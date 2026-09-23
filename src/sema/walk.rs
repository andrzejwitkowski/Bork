//! AST walk and region open for ownership checking.

use super::env::{
    apply_moved_merge, bind_with, moved_names, restore_moved_flags, Analyzer, BindingOrigin,
    Shadow, Ty,
};
use super::peel_blocks;
use super::policy::{
    bare_ident_move_message, classify_use, move_source, report_move_source_err,
    MoveSourceErr, TransferSink, UseOutcome,
};
use super::region::{resolve_move_captures, RegionFrame, RegionParam};
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{BindingKind, Block, Expr, Stmt, Type};
use crate::span::{Span, SpannedName};

pub(super) fn open_ordinary(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    params: &[RegionParam],
) -> ArenaNode {
    let (body, compacted) = peel_blocks(body);
    open_frame(az, label.to_string(), body, compacted, params, &[])
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
    )
}

fn open_frame(
    az: &mut Analyzer,
    label: String,
    body: &Block,
    compacted: usize,
    params: &[RegionParam],
    captures: &[SpannedName],
) -> ArenaNode {
    let id = az.alloc_id();
    let mut frame = RegionFrame::new(id, label, compacted);
    for p in params {
        frame.bind_param(az, p);
    }
    for cap in captures {
        frame.bind_capture(az, cap);
    }
    walk_block(az, body, &mut frame.node, &mut frame.shadows);
    frame.finish(az)
}

fn walk_block(
    az: &mut Analyzer,
    block: &Block,
    node: &mut ArenaNode,
    shadows: &mut Vec<Shadow>,
) {
    for stmt in &block.stmts {
        walk_stmt(az, stmt, node, shadows);
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
                .push(open_ordinary(az, "Block", body, &[]));
        }
        Stmt::MoveBlock { captures, body } => {
            node.children
                .push(open_move(az, "MoveBlock", body, captures.as_deref(), &[]));
        }
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            let sink = TransferSink::Binding {
                dest: *kind,
                arena_id: node.id,
                arena_label: node.label.clone(),
            };
            walk(az, value, node, Some(&sink));
            let inferred = Ty::from_option(ty.clone().or_else(|| infer_type(az, value)));
            shadows.push(bind_with(
                az,
                name,
                node.id,
                &node.label,
                inferred.clone(),
                *kind,
                BindingOrigin::Declared,
            ));
            node.bindings.push(BindingInfo {
                name: name.clone(),
                ownership: Ownership::Local,
                ty: inferred.as_option(),
                span: Some(*name_span),
            });
        }
        Stmt::Assign { name, name_span, value } => {
            note_use(az, name, Some(*name_span), node);
            let sink = TransferSink::Binding {
                dest: BindingKind::Var,
                arena_id: node.id,
                arena_label: node.label.clone(),
            };
            walk(az, value, node, Some(&sink));
        }
        Stmt::For { name, iter, body } => {
            walk(az, iter, node, None);
            az.loop_move_ban
                .push(az.env.keys().cloned().collect());
            let params = [RegionParam {
                name: name.name.clone(),
                ty: Ty::Known(Type::from_ident("Int", false)),
                kind: BindingKind::Val,
                span: Some(name.span),
            }];
            node.children.push(open_ordinary(
                az,
                &format!("ForLoop ({})", name.name),
                body,
                &params,
            ));
            az.loop_move_ban.pop();
        }
        Stmt::Return(Some(e)) => walk(az, e, node, None),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk(az, e, node, None),
    }
}

fn walk(
    az: &mut Analyzer,
    expr: &Expr,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
) {
    match expr {
        Expr::Move { name, span } => apply_expr_move(az, name, Some(*span), node),
        Expr::Ident { name, span } => {
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
        Expr::Some { expr: inner, .. }
        | Expr::Unary {
            expr: inner,
            ..
        }
        | Expr::Field {
            receiver: inner, ..
        } => walk(az, inner, node, transfer),
        Expr::Binary { lhs, rhs, .. } => {
            walk(az, lhs, node, transfer);
            walk(az, rhs, node, transfer);
        }
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            walk(az, callee, node, None);
            let formals = match callee.as_ref() {
                Expr::Ident { name, .. } => az.fun_sigs.get(name).cloned(),
                _ => None,
            };
            for (i, a) in args.iter().enumerate() {
                let arg_transfer = formals
                    .as_ref()
                    .and_then(|f| f.get(i).copied())
                    .map(|formal| TransferSink::CallArg { formal });
                walk(az, a, node, arg_transfer.as_ref());
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
                        .push(open_ordinary(az, "Closure", &c.body, &params));
                }
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk(az, cond, node, None);
            let before_moved = moved_names(&az.env);
            node.children
                .push(open_ordinary(az, "IfThen", then_block, &[]));
            let then_moved = moved_names(&az.env);
            restore_moved_flags(&mut az.env, &before_moved);
            let else_moved = if let Some(else_b) = else_block {
                node.children
                    .push(open_ordinary(az, "IfElse", else_b, &[]));
                Some(moved_names(&az.env))
            } else {
                None
            };
            restore_moved_flags(&mut az.env, &before_moved);
            apply_moved_merge(
                &mut az.env,
                &before_moved,
                &then_moved,
                else_moved.as_ref(),
            );
        }
        Expr::Int(_) | Expr::Str(_) | Expr::None { .. } => {}
    }
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

fn infer_type(az: &Analyzer, expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Int(_) => Some(Type::from_ident("Int", false)),
        Expr::Str(_) => Some(Type::Named {
            name: "String".into(),
            nullable: false,
        }),
        Expr::Ident { name, .. } | Expr::Move { name, .. } => {
            az.env.get(name).and_then(|b| b.ty.as_option())
        }
        Expr::Some { expr: inner, .. } => {
            infer_type(az, inner).map(|ty| ty.with_nullable(true))
        }
        _ => None,
    }
}
