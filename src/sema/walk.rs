//! AST walk and region open for ownership checking.

use super::env::{
    apply_moved_merge, moved_names, restore_moved_flags, shadow_insert, Analyzer, EnvBinding,
    Shadow, Ty,
};
use super::peel_blocks;
use super::policy::{classify_use, UseOutcome};
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
            check_transfer_rhs(az, value, *kind, Some(*name_span));
            walk_expr(az, value, node);
            let inferred = Ty::from_option(ty.clone().or_else(|| infer_type(az, value)));
            shadows.push(shadow_insert(
                az,
                name.clone(),
                EnvBinding {
                    arena_id: node.id,
                    arena_label: node.label.clone(),
                    ty: inferred.clone(),
                    kind: *kind,
                    moved: false,
                    from_capture: false,
                },
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
            check_transfer_rhs(az, value, BindingKind::Var, Some(*name_span));
            walk_expr(az, value, node);
        }
        Stmt::For { name, iter, body } => {
            walk_expr(az, iter, node);
            let before_moved = moved_names(&az.env);
            let outer_names: std::collections::HashSet<String> =
                az.env.keys().cloned().collect();
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
            // A move of an outer binding inside the loop would already be spent
            // on later iterations — reject at compile time.
            for n in &outer_names {
                let was = before_moved.contains(n);
                let now = az.env.get(n).is_some_and(|b| b.moved);
                if !was && now {
                    az.error(
                        format!(
                            "cannot move `{n}` inside a loop: it would already be moved on later iterations"
                        ),
                        Some(n.clone()),
                        Some(name.span),
                    );
                }
            }
        }
        Stmt::Return(Some(e)) => walk_expr(az, e, node),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk_expr(az, e, node),
    }
}

fn walk_expr(az: &mut Analyzer, expr: &Expr, node: &mut ArenaNode) {
    match expr {
        Expr::Ident { name, span } => note_use(az, name, Some(*span), node),
        Expr::Move { name, span } => apply_expr_move(az, name, Some(*span), node),
        Expr::Some(e) => walk_expr(az, e, node),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(az, lhs, node);
            walk_expr(az, rhs, node);
        }
        Expr::Unary { expr, .. } => walk_expr(az, expr, node),
        Expr::Field { receiver, .. } => walk_expr(az, receiver, node),
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            walk_expr(az, callee, node);
            let formals: Option<Vec<(BindingKind, Type)>> = match callee.as_ref() {
                Expr::Ident { name, .. } => az.fun_sigs.get(name).cloned(),
                _ => None,
            };
            for (i, a) in args.iter().enumerate() {
                let formal = formals.as_ref().and_then(|f| f.get(i));
                check_call_arg(az, a, formal, node);
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
            walk_expr(az, cond, node);
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
        Expr::Int(_) | Expr::Str(_) | Expr::None => {}
    }
}

fn check_call_arg(
    az: &mut Analyzer,
    arg: &Expr,
    formal: Option<&(BindingKind, Type)>,
    node: &mut ArenaNode,
) {
    let Some((formal_kind, _)) = formal else {
        walk_expr(az, arg, node);
        return;
    };
    match arg {
        Expr::Move { name, span } => apply_expr_move(az, name, Some(*span), node),
        Expr::Ident { name, span } => {
            let Some(binding) = az.env.get(name) else {
                note_use(az, name, Some(*span), node);
                return;
            };
            if binding.ty.is_copy() {
                note_use(az, name, Some(*span), node);
            } else if matches!(formal_kind, BindingKind::Var)
                || matches!(binding.kind, BindingKind::Var)
            {
                az.error(
                    format!("use `move {name}` to pass ownership"),
                    Some(name.clone()),
                    Some(*span),
                );
            } else {
                note_use(az, name, Some(*span), node);
            }
        }
        _ => walk_expr(az, arg, node),
    }
}

fn apply_expr_move(
    az: &mut Analyzer,
    name: &str,
    span: Option<Span>,
    _node: &mut ArenaNode,
) {
    let Some(binding) = az.env.get(name).cloned() else {
        az.error(
            format!("cannot move unknown name `{name}`"),
            Some(name.into()),
            span,
        );
        return;
    };
    if binding.moved {
        az.error(
            format!(
                "cannot move `{name}`: already moved from {}",
                binding.arena_label
            ),
            Some(name.into()),
            span,
        );
        return;
    }
    if binding.from_capture {
        az.error(
            format!("cannot move `{name}`: it was already moved into this region as a capture"),
            Some(name.into()),
            span,
        );
        return;
    }
    if let Some(binding) = az.env.get_mut(name) {
        binding.moved = true;
    }
}

fn check_transfer_rhs(
    az: &mut Analyzer,
    value: &Expr,
    dest_kind: BindingKind,
    _span: Option<Span>,
) {
    let Expr::Ident { name, span } = value else {
        return;
    };
    let Some(binding) = az.env.get(name) else {
        return;
    };
    if binding.ty.is_copy() || binding.moved {
        return;
    }
    let src_var = matches!(binding.kind, BindingKind::Var);
    let dest_var = matches!(dest_kind, BindingKind::Var);
    if src_var || dest_var {
        az.error(
            format!("use `move {name}` to transfer ownership"),
            Some(name.clone()),
            Some(*span),
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
        match ownership {
            Ownership::Local => matches!(x.ownership, Ownership::Local) && x.span == span,
            _ => !matches!(x.ownership, Ownership::Local),
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
        Expr::Some(inner) => infer_type(az, inner).map(|ty| ty.with_nullable(true)),
        _ => None,
    }
}
