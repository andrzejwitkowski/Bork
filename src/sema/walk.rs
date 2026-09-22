//! AST walk and region open for ownership checking.

use super::env::{
    apply_moved_merge, moved_names, restore_moved_flags, shadow_insert, Analyzer, EnvBinding,
    Shadow, Ty,
};
use super::peel_blocks;
use super::policy::{classify_use, UseOutcome};
use super::region::{resolve_move_captures, RegionFrame, RegionParam};
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{Block, Expr, Stmt, Type};
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
            walk_expr(az, value, node);
        }
        Stmt::For { name, iter, body } => {
            walk_expr(az, iter, node);
            let params = [RegionParam {
                name: name.clone(),
                ty: Ty::Known(Type::from_ident("Int", false)),
                span: None,
            }];
            node.children.push(open_ordinary(
                az,
                &format!("ForLoop ({name})"),
                body,
                &params,
            ));
        }
        Stmt::Return(Some(e)) => walk_expr(az, e, node),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk_expr(az, e, node),
    }
}

fn walk_expr(az: &mut Analyzer, expr: &Expr, node: &mut ArenaNode) {
    match expr {
        Expr::Ident { name, span } => note_use(az, name, Some(*span), node),
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
            for a in args {
                walk_expr(az, a, node);
            }
            if let Some(c) = trailing {
                let params: Vec<RegionParam> = c
                    .params
                    .iter()
                    .map(|p| RegionParam {
                        name: p.name.clone(),
                        ty: Ty::Unknown,
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

fn note_use(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name).cloned() else {
        return;
    };
    match classify_use(&b, node.id, &node.label, name) {
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
        Expr::Ident { name, .. } => az.env.get(name).and_then(|b| b.ty.as_option()),
        Expr::Some(inner) => infer_type(az, inner).map(|ty| ty.with_nullable(true)),
        _ => None,
    }
}
