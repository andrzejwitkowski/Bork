//! AST walk and region open for ownership checking.

use super::env::{
    bind_with, join_branch_moves, moved_names, restore_moved_flags, Analyzer, BindingOrigin,
    BranchEnd, Shadow, Ty,
};
use super::peel_blocks;
use super::policy::{
    bare_ident_move_message, classify_use, move_source, report_move_source_err,
    MoveSourceErr, TransferSink, UseOutcome,
};
use super::region::{resolve_move_captures, RegionFrame, RegionParam};
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{AssignTarget, BindingKind, Block, ConditionalBinding, Expr, Stmt, Type, UnaryOp};
use crate::span::{Span, SpannedName};
use std::collections::HashSet;

pub(super) fn open_ordinary(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    params: &[RegionParam],
    tail_transfer: Option<&TransferSink>,
) -> ArenaNode {
    let (body, compacted) = peel_blocks(body);
    open_frame(
        az,
        label.to_string(),
        body,
        compacted,
        params,
        &[],
        tail_transfer,
    )
}

pub(super) fn open_move(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    params: &[RegionParam],
) -> ArenaNode {
    let (body, compacted) = peel_blocks(body);
    let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
    let captures = resolve_move_captures(az, body, explicit_captures, &param_names);
    open_frame(
        az,
        format!("{label} (move)"),
        body,
        compacted,
        params,
        &captures,
        None,
    )
}

fn open_frame(
    az: &mut Analyzer,
    label: String,
    body: &Block,
    compacted: usize,
    params: &[RegionParam],
    captures: &[SpannedName],
    tail_transfer: Option<&TransferSink>,
) -> ArenaNode {
    let id = az.alloc_id();
    let mut frame = RegionFrame::new(id, label, compacted);
    for p in params {
        frame.bind_param(az, p);
    }
    for cap in captures {
        frame.bind_capture(az, cap);
    }
    walk_block(az, body, &mut frame.node, &mut frame.shadows, tail_transfer);
    frame.finish(az)
}

fn walk_block(
    az: &mut Analyzer,
    block: &Block,
    node: &mut ArenaNode,
    shadows: &mut Vec<Shadow>,
    tail_transfer: Option<&TransferSink>,
) {
    let Some((last, prefix)) = block.stmts.split_last() else {
        return;
    };
    for stmt in prefix {
        walk_stmt(az, stmt, node, shadows);
    }
    if let (Stmt::Expr(expr), Some(transfer)) = (last, tail_transfer) {
        walk(az, expr, node, Some(transfer), true);
    } else {
        walk_stmt(az, last, node, shadows);
    }
}

fn walk_stmt(
    az: &mut Analyzer,
    stmt: &Stmt,
    node: &mut ArenaNode,
    shadows: &mut Vec<Shadow>,
) {
    match stmt {
        Stmt::Block(body) => {
            node.children
                .push(open_ordinary(az, "Block", body, &[], None));
        }
        Stmt::MoveBlock { captures, body } => {
            node.children
                .push(open_move(az, "MoveBlock", body, captures.as_deref(), &[]));
        }
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty: _,
            value,
        } => {
            let inferred = az
                .span_tys
                .get(name_span)
                .cloned()
                .map(ty_from_hir)
                .unwrap_or(Ty::Unknown);
            let sink = TransferSink::Binding {
                dest: *kind,
                arena_id: node.id,
                arena_label: node.label.clone(),
            };
            walk_typed(
                az,
                value,
                node,
                Some(&sink),
                true,
                inferred.as_option().as_ref(),
            );
            let view =
                is_view_init(az, value) || matches!(&inferred, Ty::Known(ty) if ty.is_reference());
            if view && *kind == BindingKind::Var {
                az.error(
                    format!("cannot bind `var` `{name}` to a borrow; use `val`"),
                    Some(name.clone()),
                    Some(*name_span),
                );
            }
            shadows.push(bind_with(
                az,
                name,
                node.id,
                &node.label,
                inferred.clone(),
                *kind,
                if view {
                    BindingOrigin::View
                } else {
                    BindingOrigin::Declared
                },
            ));
            node.bindings.push(BindingInfo {
                name: name.clone(),
                ownership: Ownership::Local,
                ty: inferred.as_option(),
                span: Some(*name_span),
            });
        }
        Stmt::Assign { target, value } => {
            let (name, name_span, index, borrowed) = match target {
                AssignTarget::Name { name, name_span } => (name, name_span, None, false),
                AssignTarget::Index {
                    name,
                    name_span,
                    index,
                    borrowed,
                } => (name, name_span, Some(index), *borrowed),
                AssignTarget::Field {
                    receiver: crate::ast::Expr::Ident { name, span },
                    ..
                } => (name, span, None, false),
                AssignTarget::Field { receiver, span, .. } => {
                    let target_ty = az.span_tys.get(span).cloned().map(ty_from_hir);
                    walk(az, receiver, node, None, true);
                    walk_typed(
                        az,
                        value,
                        node,
                        None,
                        true,
                        target_ty.and_then(|ty| ty.as_option()).as_ref(),
                    );
                    return;
                }
            };
            let dest = az.env.get(name).cloned();
            let dest_ty = dest.as_ref().and_then(|binding| binding.ty.as_option());
            let target_ty = match (target, dest_ty) {
                (AssignTarget::Name { .. }, ty) => ty,
                (AssignTarget::Index { .. }, Some(ty)) => {
                    let ty = match ty {
                        Type::Ref { inner, .. } => *inner,
                        ty => ty,
                    };
                    match ty {
                        Type::Array { elem, .. } => Some(*elem),
                        _ => None,
                    }
                }
                (AssignTarget::Field { span, .. }, _) => az
                    .span_tys
                    .get(span)
                    .cloned()
                    .map(ty_from_hir)
                    .and_then(|ty| ty.as_option()),
                _ => None,
            };
            let assign_up = dest.as_ref().is_some_and(|b| {
                !b.moved
                    && matches!(b.kind, BindingKind::Var)
                    && b.arena_id < node.id
            });
            if borrowed {
                note_borrow(az, name, Some(*name_span), node, None);
            } else if !assign_up {
                note_use(az, name, Some(*name_span), node);
            }
            if let Some(index) = index {
                walk(az, index, node, None, true);
            }
            let (arena_id, arena_label) = if let Some(b) = dest {
                (b.arena_id, b.arena_label.clone())
            } else {
                (node.id, node.label.clone())
            };
            let sink = TransferSink::Binding {
                dest: BindingKind::Var,
                arena_id,
                arena_label,
            };
            walk_typed(az, value, node, Some(&sink), true, target_ty.as_ref());
        }
        Stmt::For { name, iter, body } => {
            walk(az, iter, node, None, true);
            az.loop_move_ban
                .push(az.env.keys().cloned().collect());
            let params = [RegionParam {
                name: name.name.clone(),
                ty: Ty::Known(Type::from_ident("Int", false)),
                kind: BindingKind::Val,
                span: Some(name.span),
                borrow_from: super::region::REF_PARAM_BORROW_FROM,
            }];
            node.children.push(open_ordinary(
                az,
                &format!("ForLoop ({})", name.name),
                body,
                &params,
                None,
            ));
            az.loop_move_ban.pop();
        }
        Stmt::While { cond, body } => {
            walk(az, cond, node, None, true);
            az.loop_move_ban
                .push(az.env.keys().cloned().collect());
            node.children.push(open_ordinary(az, "WhileLoop", body, &[], None));
            az.loop_move_ban.pop();
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Return(Some(e)) => walk(az, e, node, None, true),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk(az, e, node, None, true),
    }
}

fn walk_typed(
    az: &mut Analyzer,
    expr: &Expr,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
    expected: Option<&Type>,
) {
    if let (Expr::ArrayLit { elements, .. }, Some(Type::Array { elem, .. })) = (expr, expected) {
        for element in elements {
            walk_typed(az, element, node, transfer, record_borrow, Some(elem));
        }
    } else if matches!(expected, Some(Type::ManagedRef { .. })) {
        walk(az, expr, node, Some(&TransferSink::ManagedRef), record_borrow);
    } else {
        walk(az, expr, node, transfer, record_borrow);
    }
}

fn walk(
    az: &mut Analyzer,
    expr: &Expr,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
) {
    match expr {
        Expr::Move { name, span } => apply_expr_move(az, name, Some(*span), node),
        Expr::Promote { name, span } => apply_expr_promote(az, name, Some(*span), node, transfer),
        Expr::Ident { name, span } => {
            if matches!(transfer, Some(TransferSink::ManagedRef)) {
                if az
                    .env
                    .get(name)
                    .is_some_and(|binding| binding.ty.is_managed_ref())
                {
                    note_use(az, name, Some(*span), node);
                } else {
                    note_borrow(az, name, Some(*span), node, transfer);
                }
                return;
            }
            if let Some(sink) = transfer {
                let Some(binding) = az.env.get(name) else {
                    note_use(az, name, Some(*span), node);
                    return;
                };
                if let Some(message) = bare_ident_move_message(binding, name, sink) {
                    az.error(message, Some(name.clone()), Some(*span));
                } else {
                    note_use(az, name, Some(*span), node);
                }
            } else {
                note_use(az, name, Some(*span), node);
            }
        }
        Expr::Some { expr: inner, .. } => walk(az, inner, node, transfer, record_borrow),
        Expr::Unary {
            op,
            expr: inner,
            span,
            ..
        } => {
            if *op == UnaryOp::Borrow {
                if record_borrow {
                    match inner.as_ref() {
                        Expr::Ident { name, span: name_span } => {
                            note_borrow(az, name, Some(*name_span), node, transfer);
                        }
                        _ => {
                            az.error("`&` borrows a name", None, Some(*span));
                            walk(az, inner, node, None, true);
                        }
                    }
                } else {
                    walk(az, inner, node, None, true);
                }
            } else {
                walk(az, inner, node, transfer, record_borrow);
            }
        }
        Expr::Field { receiver: inner, .. } => walk(az, inner, node, None, record_borrow),
        Expr::ArrayLit { elements, .. } => {
            for element in elements {
                walk(az, element, node, transfer, record_borrow);
            }
        }
        Expr::Index { receiver, index, .. } => {
            walk(az, receiver, node, None, record_borrow);
            walk(az, index, node, None, record_borrow);
        }
        Expr::Slice { receiver, lo, hi, .. } => {
            walk(az, receiver, node, None, record_borrow);
            walk(az, lo, node, None, record_borrow);
            walk(az, hi, node, None, record_borrow);
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk(az, lhs, node, transfer, record_borrow);
            walk(az, rhs, node, transfer, record_borrow);
        }
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            walk(az, callee, node, None, record_borrow);
            let (formals, param_tys) = match callee.as_ref() {
                Expr::Ident { name, .. } => {
                    if let Some(class) = az.classes.get(name) {
                        (
                            None,
                            Some(class.fields.iter().map(|field| field.ty.clone()).collect()),
                        )
                    } else {
                        (
                            az.fun_sigs.get(name).cloned(),
                            az.fun_param_tys.get(name).cloned(),
                        )
                    }
                }
                _ => (None, None),
            };
            for (i, a) in args.iter().enumerate() {
                let arg_record_borrow = param_tys
                    .as_ref()
                    .and_then(|params| params.get(i))
                    .is_some_and(|ty| ty.is_reference());
                let arg_transfer = formals
                    .as_ref()
                    .and_then(|f| f.get(i).copied())
                    .map(|formal| TransferSink::CallArg { formal });
                let expected = param_tys.as_ref().and_then(|params| params.get(i));
                walk_typed(
                    az,
                    a,
                    node,
                    arg_transfer.as_ref(),
                    arg_record_borrow,
                    expected,
                );
            }
            if let Some(c) = trailing {
                let params: Vec<RegionParam> = c
                    .params
                    .iter()
                    .map(|p| RegionParam {
                        name: p.name.clone(),
                        ty: Ty::Unknown,
                        kind: BindingKind::Val,
                        span: Some(p.span),
                        borrow_from: super::region::REF_PARAM_BORROW_FROM,
                    })
                    .collect();
                if c.is_move {
                    node.children.push(open_move(
                        az,
                        "Closure",
                        &c.body,
                        c.captures.as_deref(),
                        &params,
                    ));
                } else {
                    node.children
                        .push(open_ordinary(az, "Closure", &c.body, &params, None));
                }
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk(az, cond, node, None, record_borrow);
            let tail_transfer = transfer.filter(|sink| matches!(sink, TransferSink::ManagedRef));
            let before_moved = moved_names(&az.env);
            node.children
                .push(open_ordinary(az, "IfThen", then_block, &[], tail_transfer));
            let then_end = branch_end(az, then_block, &[]);
            restore_moved_flags(&mut az.env, &before_moved);
            let else_end = else_arm_end(
                az,
                node,
                else_block.as_ref(),
                "IfElse",
                tail_transfer,
                before_moved,
            );
            join_branch_moves(&mut az.env, &[then_end, else_end]);
        }
        Expr::IfVal {
            bindings,
            then_block,
            else_block,
        } => {
            walk(az, &bindings.head.value, node, None, record_borrow);
            let tail_transfer = transfer.filter(|sink| matches!(sink, TransferSink::ManagedRef));
            let (body, compacted) = peel_blocks(then_block);
            let mut frame = RegionFrame::new(az.alloc_id(), "IfValSome".into(), compacted);
            bind_presence_guard(az, &mut frame, &bindings.head);
            for binding in &bindings.tail {
                walk(az, &binding.value, &mut frame.node, None, record_borrow);
                bind_presence_guard(az, &mut frame, binding);
            }
            // The else path sees the header's moves but not the body's.
            let header_moved = outer_moved_names(az, &frame.shadows);
            walk_block(az, body, &mut frame.node, &mut frame.shadows, tail_transfer);
            let then_end = branch_end(az, then_block, &frame.shadows);
            node.children.push(frame.finish(az));
            restore_moved_flags(&mut az.env, &header_moved);
            let else_end = else_arm_end(
                az,
                node,
                else_block.as_ref(),
                "IfValNone",
                tail_transfer,
                header_moved,
            );
            join_branch_moves(&mut az.env, &[then_end, else_end]);
        }
        Expr::Float(_) | Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::None { .. } => {}
    }
}

/// True when control never falls out of `block` (ends in `return`/`break`/`continue`, or in a
/// nested block or `if` / `if val` whose every path does).
fn block_diverges(block: &Block) -> bool {
    match block.stmts.last() {
        Some(Stmt::Return(_) | Stmt::Break { .. } | Stmt::Continue { .. }) => true,
        Some(Stmt::Block(body) | Stmt::MoveBlock { body, .. }) => block_diverges(body),
        Some(Stmt::Expr(expr)) => expr_diverges(expr),
        _ => false,
    }
}

fn expr_diverges(expr: &Expr) -> bool {
    match expr {
        Expr::If {
            then_block,
            else_block: Some(else_block),
            ..
        }
        | Expr::IfVal {
            then_block,
            else_block: Some(else_block),
            ..
        } => block_diverges(then_block) && block_diverges(else_block),
        _ => false,
    }
}

/// Names moved in the current env, with shadowed names read from the outer binding.
fn outer_moved_names(az: &Analyzer, shadows: &[Shadow]) -> HashSet<String> {
    let mut moved = moved_names(&az.env);
    for Shadow(name, prev) in shadows {
        if prev.as_ref().is_some_and(|prev| prev.moved) {
            moved.insert(name.clone());
        } else {
            moved.remove(name);
        }
    }
    moved
}

fn branch_end(az: &Analyzer, block: &Block, shadows: &[Shadow]) -> BranchEnd {
    BranchEnd {
        falls_through: !block_diverges(block),
        moved: outer_moved_names(az, shadows),
    }
}

/// Walks the optional `else` arm. Without one, control falls through with `moved_without_else`.
fn else_arm_end(
    az: &mut Analyzer,
    node: &mut ArenaNode,
    else_block: Option<&Block>,
    label: &str,
    tail_transfer: Option<&TransferSink>,
    moved_without_else: HashSet<String>,
) -> BranchEnd {
    match else_block {
        Some(block) => {
            node.children
                .push(open_ordinary(az, label, block, &[], tail_transfer));
            branch_end(az, block, &[])
        }
        None => BranchEnd {
            falls_through: true,
            moved: moved_without_else,
        },
    }
}

/// Binds one `if val` / `when` name inside its `Some` region frame.
fn bind_presence_guard(az: &mut Analyzer, frame: &mut RegionFrame, binding: &ConditionalBinding) {
    let ty = az
        .span_tys
        .get(&binding.name.span)
        .cloned()
        .map(ty_from_hir)
        .unwrap_or(Ty::Unknown);
    frame.bind_param(
        az,
        &RegionParam {
            name: binding.name.name.clone(),
            ty,
            kind: BindingKind::Val,
            span: Some(binding.name.span),
            borrow_from: "presence guard",
        },
    );
}

fn apply_expr_promote(
    az: &mut Analyzer,
    name: &str,
    span: Option<Span>,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
) {
    let Some(TransferSink::Binding {
        arena_id: sink_id,
        arena_label: sink_label,
        ..
    }) = transfer else {
        az.error(
            "`promote` is only valid when assigning to a binding in an outer region",
            Some(name.into()),
            span,
        );
        return;
    };
    let Some(peek) = az.env.get(name).cloned() else {
        report_move_source_err(az, name, MoveSourceErr::Unknown, span);
        return;
    };
    if peek.ty.is_copy() {
        az.error(
            format!("`promote` is not needed for Copy binding `{name}`"),
            Some(name.into()),
            span,
        );
        return;
    }
    if peek.moved {
        report_move_source_err(
            az,
            name,
            MoveSourceErr::AlreadyMoved {
                from: peek.arena_label.clone(),
            },
            span,
        );
        return;
    }
    if peek.arena_id <= *sink_id {
        az.error(
            format!(
                "`promote {name}` cannot lift into `{sink_label}`: value already lives in that arena or further out"
            ),
            Some(name.into()),
            span,
        );
        return;
    }
    if *sink_id >= node.id {
        az.error(
            format!(
                "`promote {name}` must target a strictly outer arena than the current `{label}`",
                label = node.label
            ),
            Some(name.into()),
            span,
        );
        return;
    }
    let binding = match move_source(az, name) {
        Ok(b) => b,
        Err(e) => {
            report_move_source_err(az, name, e, span);
            return;
        }
    };
    let from = binding.arena_label.clone();
    if let Some(binding) = az.env.get_mut(name) {
        binding.moved = true;
    }
    record_observation(
        node,
        name,
        Ownership::Moved { from },
        binding.ty.as_option(),
        span,
    );
}

fn apply_expr_move(
    az: &mut Analyzer,
    name: &str,
    span: Option<Span>,
    node: &mut ArenaNode,
) {
    let Some(peek) = az.env.get(name).cloned() else {
        report_move_source_err(az, name, MoveSourceErr::Unknown, span);
        return;
    };
    if matches!(peek.origin, BindingOrigin::View) {
        az.error(
            format!("cannot move `{name}`: it is a borrow, not an owner"),
            Some(name.into()),
            span,
        );
        return;
    }
    if peek.ty.is_copy() {
        note_use(az, name, span, node);
        return;
    }
    let binding = match move_source(az, name) {
        Ok(b) => b,
        Err(e) => {
            report_move_source_err(az, name, e, span);
            return;
        }
    };
    if matches!(binding.origin, BindingOrigin::Captured) && binding.arena_id == node.id {
        az.error(
            format!("cannot move `{name}`: it was already moved into this region as a capture"),
            Some(name.into()),
            span,
        );
        return;
    }
    let from = binding.arena_label.clone();
    if let Some(binding) = az.env.get_mut(name) {
        binding.moved = true;
    }
    if binding.arena_id == node.id {
        if let Some(b) = node.bindings.iter_mut().rev().find(|b| b.name == name) {
            b.ownership = Ownership::Moved { from };
        }
    } else {
        record_observation(
            node,
            name,
            Ownership::Moved { from },
            binding.ty.as_option(),
            span,
        );
    }
}

fn is_view_init(az: &Analyzer, value: &Expr) -> bool {
    match value {
        Expr::Unary {
            op: UnaryOp::Borrow,
            expr,
            ..
        } => matches!(expr.as_ref(), Expr::Ident { .. }),
        Expr::Ident { name, .. } => az
            .env
            .get(name)
            .is_some_and(|binding| binding.origin == BindingOrigin::View),
        _ => false,
    }
}

fn note_borrow(
    az: &mut Analyzer,
    name: &str,
    span: Option<Span>,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
) {
    let Some(binding) = az.env.get(name).cloned() else {
        return;
    };
    if binding.moved {
        az.error(
            format!("use of `{name}` after move from {}", binding.arena_label),
            Some(name.to_string()),
            span,
        );
        return;
    }
    if binding.origin == BindingOrigin::View && binding.arena_id != node.id {
        let reborrow_for_call = binding.arena_id < node.id
            && matches!(transfer, Some(TransferSink::CallArg { .. }));
        if !reborrow_for_call {
            az.error(
                format!("cannot lift borrow `{name}` out of {}", binding.arena_label),
                Some(name.to_string()),
                span,
            );
            return;
        }
        record_observation(
            node,
            name,
            Ownership::Borrow {
                from: binding.arena_label.clone(),
            },
            binding.ty.as_option(),
            span,
        );
        return;
    }
    if binding.ty.is_copy()
        || binding.arena_id == node.id
        || matches!(binding.kind, BindingKind::Val)
    {
        note_use(az, name, span, node);
        return;
    }
    record_observation(
        node,
        name,
        Ownership::Borrow {
            from: binding.arena_label,
        },
        binding.ty.as_option(),
        span,
    );
}

fn note_use(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name) else {
        return;
    };
    match classify_use(b, node.id, &node.label, name) {
        UseOutcome::Observe(ownership) => {
            record_observation(node, name, ownership, b.ty.as_option(), span)
        }
        UseOutcome::Error { message } => {
            az.error(message, Some(name.to_string()), span);
        }
    }
}

fn record_observation(
    node: &mut ArenaNode,
    name: &str,
    ownership: Ownership,
    ty: Option<Type>,
    span: Option<Span>,
) {
    let dup = node.observations.iter().any(|x| {
        if x.name != name {
            return false;
        }
        match (&ownership, &x.ownership) {
            (Ownership::Local, Ownership::Local) => x.span == span,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    });
    if dup {
        return;
    }
    node.observations.push(BindingInfo {
        name: name.to_string(),
        ownership,
        ty,
        span,
    });
}

fn ty_from_hir(ty: crate::hir::Ty) -> Ty {
    match hir_to_ast(&ty) {
        Some(ty) => Ty::Known(ty),
        None => Ty::Unknown,
    }
}

fn hir_to_ast(ty: &crate::hir::Ty) -> Option<Type> {
    if ty.is_unknown() {
        return None;
    }
    let kind = match &ty.kind {
        crate::hir::TyKind::Prim(prim) => Type::from_ident(prim.as_str(), ty.nullable),
        crate::hir::TyKind::Named(name) => Type::Named {
            name: name.clone(),
            nullable: ty.nullable,
        },
        crate::hir::TyKind::Array { elem, len } => Type::Array {
            elem: Box::new(hir_to_ast(elem)?),
            len: *len,
            nullable: ty.nullable,
        },
        crate::hir::TyKind::Ref(inner) => Type::Ref {
            inner: Box::new(hir_to_ast(inner)?),
            nullable: false,
        },
        crate::hir::TyKind::ManagedRef(inner) => Type::ManagedRef {
            inner: Box::new(hir_to_ast(inner)?),
        },
        crate::hir::TyKind::Func { .. } | crate::hir::TyKind::Range(_) | crate::hir::TyKind::Unknown => {
            return None;
        }
    };
    Some(kind)
}
