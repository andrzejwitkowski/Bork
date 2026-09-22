//! Region walk and ownership checking.

use super::env::{moved_names, shadow_insert, Analyzer, EnvBinding, Shadow};
use super::policy::{classify_use, UseOutcome};
use super::region::{open_move, open_ordinary};
use super::report::{ArenaNode, ArenaReport, BindingInfo, Ownership, SemaError};
use crate::ast::{Block, Expr, Function, Program, Stmt, Type};
use crate::span::Span;
use std::collections::HashSet;

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    let roots = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f))
        .collect();
    (ArenaReport { roots }, az.errors)
}

fn analyze_function(az: &mut Analyzer, func: &Function) -> ArenaNode {
    let params: Vec<(String, Option<Type>)> = func
        .params
        .iter()
        .map(|p| (p.name.clone(), Some(p.ty.clone())))
        .collect();
    open_ordinary(
        az,
        &format!("fun {}", func.name),
        &func.body,
        &params,
        walk_block,
    )
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
                .push(open_ordinary(az, "Block", body, &[], walk_block));
        }
        Stmt::MoveBlock { captures, body } => {
            node.children.push(open_move(
                az,
                "MoveBlock",
                body,
                captures.as_deref(),
                &[],
                walk_block,
            ));
        }
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            walk_expr(az, value, node);
            let inferred = ty.clone().or_else(|| infer_type(az, value));
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
                ty: inferred,
                span: Some(*name_span),
            });
        }
        Stmt::Assign { name, name_span, value } => {
            note_use(az, name, Some(*name_span), node);
            walk_expr(az, value, node);
        }
        Stmt::For { name, iter, body } => {
            walk_expr(az, iter, node);
            let params = [(name.clone(), Some(Type::from_ident("Int", false)))];
            node.children.push(open_ordinary(
                az,
                &format!("ForLoop ({name})"),
                body,
                &params,
                walk_block,
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
                let params: Vec<(String, Option<Type>)> =
                    c.params.iter().map(|p| (p.name.clone(), None)).collect();
                if c.is_move {
                    node.children.push(open_move(
                        az,
                        "Closure",
                        &c.body,
                        c.captures.as_deref(),
                        &params,
                        walk_block,
                    ));
                } else {
                    node.children
                        .push(open_ordinary(az, "Closure", &c.body, &params, walk_block));
                }
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(az, cond, node);
            let env_before = az.env.clone();
            let before_moved = moved_names(&az.env);
            node.children
                .push(open_ordinary(az, "IfThen", then_block, &[], walk_block));
            let then_moved = moved_names(&az.env);
            az.env = env_before.clone();
            let else_moved = if let Some(else_b) = else_block {
                node.children
                    .push(open_ordinary(az, "IfElse", else_b, &[], walk_block));
                Some(moved_names(&az.env))
            } else {
                None
            };
            az.env = env_before;
            merge_branch_moves(&mut az.env, &before_moved, &then_moved, else_moved.as_ref());
        }
        Expr::Int(_) | Expr::Str(_) | Expr::None => {}
    }
}

fn merge_branch_moves(
    env: &mut std::collections::HashMap<String, EnvBinding>,
    before_moved: &HashSet<String>,
    then_moved: &HashSet<String>,
    else_moved: Option<&HashSet<String>>,
) {
    for (name, binding) in env.iter_mut() {
        let then = then_moved.contains(name);
        binding.moved = match else_moved {
            Some(else_set) => before_moved.contains(name) || (then && else_set.contains(name)),
            // if-without-else: then-only moves do not stick
            None => before_moved.contains(name),
        };
    }
}

fn note_use(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name).cloned() else {
        return;
    };
    match classify_use(&b, node.id, &node.label, name) {
        UseOutcome::Ignore => {}
        UseOutcome::Observe(ownership) => record_obs(node, name, ownership, b.ty, span),
        UseOutcome::Error { message } => {
            az.error(message, Some(name.to_string()), span);
        }
    }
}

fn record_obs(
    node: &mut ArenaNode,
    name: &str,
    ownership: Ownership,
    ty: Option<Type>,
    span: Option<Span>,
) {
    if node.bindings.iter().any(|x| x.name == name) {
        return;
    }
    node.bindings.push(BindingInfo {
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
        Expr::Ident { name, .. } => az.env.get(name).and_then(|b| b.ty.clone()),
        Expr::Some(inner) => infer_type(az, inner).map(|ty| ty.with_nullable(true)),
        _ => None,
    }
}
