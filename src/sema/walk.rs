//! AST walk and region open for ownership checking.

use super::env::{bind_with, Analyzer, BindingOrigin, Shadow, Ty};
use super::peel_blocks;
use super::policy::{
    bare_ident_move_message, classify_use, move_source, report_move_source_err,
    MoveSourceErr, TransferSink, UseOutcome,
};
use super::region::{resolve_move_captures, RegionFrame, RegionParam};
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{AssignTarget, BinOp, BindingKind, Block, Expr, Stmt, Type, UnaryOp};
use crate::span::{AstId, Span, SpannedName};
use crate::typeck::MemberRef;

mod branches;
use branches::{walk_if, walk_if_val};

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
            id,
            ty: _,
            value,
        } => walk_var_decl(az, kind, name, name_span, id, value, node, shadows),
        Stmt::Assign { target, value } => walk_assign(az, target, value, node),
        Stmt::For { name, iter, body } => walk_for(az, name, iter, body, node),
        Stmt::While { cond, body } => walk_while(az, cond, body, node),
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Return(Some(e)) => walk(az, e, node, None, true),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk(az, e, node, None, true),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_var_decl(
    az: &mut Analyzer,
    kind: &BindingKind,
    name: &String,
    name_span: &Span,
    id: &AstId,
    value: &Expr,
    node: &mut ArenaNode,
    shadows: &mut Vec<Shadow>,
) {
    let inferred = az
        .node_tys
        .get(id)
        .cloned()
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

fn walk_assign(az: &mut Analyzer, target: &AssignTarget, value: &Expr, node: &mut ArenaNode) {
    // Bare field inside a method: same path as `this.field = value`.
    let this = "this".to_string();
    let bare_field =
        matches!(target, AssignTarget::Name { id, .. } if az.is_bare_field(*id));
    let (name, name_span, index, borrowed) = match target {
        AssignTarget::Name { name_span, .. } if bare_field => {
            (&this, name_span, None, false)
        }
        AssignTarget::Name { name, name_span, .. } => (name, name_span, None, false),
        AssignTarget::Index {
            name,
            name_span,
            index,
            borrowed,
            ..
        } => (name, name_span, Some(index), *borrowed),
        AssignTarget::Field {
            receiver: crate::ast::Expr::Ident { name, span, id },
            ..
        } if !az.is_bare_field(*id) => {
            (name, span, None, false)
        }
        AssignTarget::Field { receiver, id, .. } => {
            let target_ty = az.node_tys.get(id).cloned();
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
        (AssignTarget::Name { id, .. }, _) if bare_field => az
            .node_tys
            .get(id)
            .cloned()
            .and_then(|ty| ty.as_option()),
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
        (AssignTarget::Field { id, .. }, _) => az
            .node_tys
            .get(id)
            .cloned()
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

fn walk_for(az: &mut Analyzer, name: &SpannedName, iter: &Expr, body: &Block, node: &mut ArenaNode) {
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

fn walk_while(az: &mut Analyzer, cond: &Expr, body: &Block, node: &mut ArenaNode) {
    walk(az, cond, node, None, true);
    az.loop_move_ban
        .push(az.env.keys().cloned().collect());
    node.children.push(open_ordinary(az, "WhileLoop", body, &[], None));
    az.loop_move_ban.pop();
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
        Expr::Move { name, span, id } => apply_expr_move(az, name, Some(*span), *id, node),
        Expr::Promote { name, span, id } => apply_expr_promote(az, name, Some(*span), *id, node, transfer),
        Expr::Ident { name, span, id } => walk_ident(az, name, span, id, node, transfer),
        Expr::Some { expr: inner, .. } => walk(az, inner, node, transfer, record_borrow),
        Expr::Unary {
            op,
            expr: inner,
            span,
            ..
        } => walk_unary(az, op, inner, span, node, transfer, record_borrow),
        Expr::Field { receiver: inner, .. } => walk(az, inner, node, None, record_borrow),
        Expr::ArrayLit { elements, .. } => {
            for element in elements {
                walk(az, element, node, transfer, record_borrow);
            }
        }
        Expr::VecNew { cap, .. } => {
            if let Some(cap) = cap {
                walk(az, cap, node, None, record_borrow);
            }
        }
        Expr::Index { receiver, index, .. } => {
            if receiver_is_vec(az, receiver) {
                // Reading `v[i]` borrows the vector like `v.get(i)`.
                walk_receiver(az, receiver, node);
            } else {
                walk(az, receiver, node, None, record_borrow);
            }
            walk(az, index, node, None, record_borrow);
        }
        Expr::Slice { receiver, lo, hi, .. } => {
            walk(az, receiver, node, None, record_borrow);
            walk(az, lo, node, None, record_borrow);
            walk(az, hi, node, None, record_borrow);
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            // Comparisons observe their operands; they never transfer them.
            let transfer = if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le) {
                None
            } else {
                transfer
            };
            walk(az, lhs, node, transfer, record_borrow);
            walk(az, rhs, node, transfer, record_borrow);
        }
        Expr::Call {
            callee,
            args,
            trailing,
            ..
        } => walk_call(az, callee, args, trailing, node, record_borrow),
        Expr::If {
            cond,
            then_block,
            else_block,
        } => walk_if(az, cond, then_block, else_block.as_ref(), node, transfer, record_borrow),
        Expr::IfVal {
            bindings,
            then_block,
            else_block,
        } => walk_if_val(az, bindings, then_block, else_block.as_ref(), node, transfer, record_borrow),
        Expr::Float(_) | Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::None { .. } => {}
    }
}

fn walk_ident(az: &mut Analyzer, name: &String, span: &Span, id: &AstId, node: &mut ArenaNode, transfer: Option<&TransferSink>) {
    if az.is_bare_field(*id) {
        // Bare field inside a method: a read through the `this` borrow.
        note_use(az, "this", Some(*span), node);
        return;
    }
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

fn walk_unary(
    az: &mut Analyzer,
    op: &UnaryOp,
    inner: &Expr,
    span: &Span,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
) {
    if *op == UnaryOp::Borrow {
        if record_borrow {
            match inner {
                Expr::Ident { name, span: name_span, .. } => {
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

fn walk_call(
    az: &mut Analyzer,
    callee: &Expr,
    args: &[Expr],
    trailing: &Option<crate::ast::Closure>,
    node: &mut ArenaNode,
    record_borrow: bool,
) {
    let (formals, param_tys) = if let Some((receiver, qualified)) = method_call(az, callee) {
        walk_receiver(az, &receiver, node);
        (
            az.fun_sigs.get(&qualified).map(|f| f[1..].to_vec()),
            az.fun_param_tys.get(&qualified).map(|p| p[1..].to_vec()),
        )
    } else {
        walk(az, callee, node, None, record_borrow);
        match callee {
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
        }
    };
    for (i, a) in args.iter().enumerate() {
        let builtin_borrow = matches!(callee, Expr::Ident { name, .. }
            if crate::builtins::is_borrowed_arg(name, i));
        let arg_record_borrow = builtin_borrow
            || param_tys
            .as_ref()
            .and_then(|params| params.get(i))
            .is_some_and(|ty| ty.is_reference());
        let arg_transfer = formals
            .as_ref()
            .and_then(|f| f.get(i).copied())
            .map(|formal| TransferSink::CallArg { formal })
            .or_else(|| {
                builtin_borrow.then_some(TransferSink::CallArg {
                    formal: BindingKind::Val,
                })
            });
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

fn apply_expr_promote(
    az: &mut Analyzer,
    name: &str,
    span: Option<Span>,
    id: crate::span::AstId,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
) {
    if az.is_constant(id) {
        return; // typeck reports `promote` of a constant
    }
    if az.is_bare_field(id) {
        return; // typeck reports `promote` of a field
    }
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
    id: crate::span::AstId,
    node: &mut ArenaNode,
) {
    if az.is_constant(id) {
        return; // typeck reports `move` of a constant
    }
    if az.is_bare_field(id) {
        return; // typeck reports `move` of a field
    }
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

/// Receiver expression and `C.m` name when typeck resolved `callee` to a method.
fn method_call(az: &Analyzer, callee: &Expr) -> Option<(Expr, String)> {
    let (receiver, id) = match callee {
        Expr::Field { receiver, id, .. } => ((**receiver).clone(), *id),
        // The synthetic `this` shares the callee's span and id, as the callee names the member.
        Expr::Ident { span, id, .. } => (
            Expr::Ident {
                name: "this".to_string(),
                span: *span,
                id: *id,
            },
            *id,
        ),
        _ => return None,
    };
    match az.members.get(&id)? {
        MemberRef::Method { class, name, .. } => Some((receiver, format!("{class}.{name}"))),
        MemberRef::Field { .. } | MemberRef::Constant { .. } => None,
    }
}

fn receiver_is_vec(az: &Analyzer, receiver: &Expr) -> bool {
    matches!(receiver, Expr::Ident { name, .. }
        if az.env.get(name).is_some_and(|b| matches!(b.ty.as_option(), Some(Type::Vec { .. }))))
}

/// A method receiver is borrowed for the call, like an `&T` argument; a field chain borrows its root.
fn walk_receiver(az: &mut Analyzer, receiver: &Expr, node: &mut ArenaNode) {
    let mut root = receiver;
    while let Expr::Field { receiver, .. } = root {
        root = receiver;
    }
    let Expr::Ident { name, span, id } = root else {
        walk(az, root, node, None, true);
        return;
    };
    let name = if az.is_bare_field(*id) { "this" } else { name };
    let sink = TransferSink::CallArg {
        formal: BindingKind::Val,
    };
    note_borrow(az, name, Some(*span), node, Some(&sink));
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

/// Typeck's type of a node as sema's own `Ty`; types sema does not model (functions, ranges,
/// errors) are `Unknown`.
pub(super) fn ty_from_id(tcx: &crate::ty::TyCtx, id: crate::ty::TyId) -> Ty {
    match id_to_ast(tcx, id) {
        Some(ty) => Ty::Known(ty),
        None => Ty::Unknown,
    }
}

fn id_to_ast(tcx: &crate::ty::TyCtx, id: crate::ty::TyId) -> Option<Type> {
    use crate::ty::TyData;
    let nullable = tcx.is_nullable(id);
    let kind = match tcx.data(tcx.base(id)) {
        TyData::Prim(prim) => Type::from_ident(prim.as_str(), nullable),
        TyData::String => Type::Named { name: "String".into(), args: Vec::new(), nullable },
        TyData::Class(def, args) => Type::Named {
            name: tcx.class_name(*def).to_string(),
            args: args.iter().map(|&arg| id_to_ast(tcx, arg)).collect::<Option<_>>()?,
            nullable,
        },
        TyData::Array { elem, len } => Type::Array {
            elem: Box::new(id_to_ast(tcx, *elem)?),
            len: *len,
            nullable,
        },
        TyData::Ref(inner) => Type::Ref {
            inner: Box::new(id_to_ast(tcx, *inner)?),
            nullable: false,
        },
        TyData::ManagedRef(inner) => Type::ManagedRef {
            inner: Box::new(id_to_ast(tcx, *inner)?),
        },
        TyData::Vec(elem) => Type::Vec {
            elem: Box::new(id_to_ast(tcx, *elem)?),
        },
        TyData::Func { .. }
        | TyData::Range(_)
        | TyData::Nullable(_)
        | TyData::Param(_)
        | TyData::Infer(_)
        | TyData::Error => return None,
    };
    Some(kind)
}
