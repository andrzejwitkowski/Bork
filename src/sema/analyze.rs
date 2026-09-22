//! Region walk and ownership checking.

use super::collapse_block;
use super::env::{
    bind, restore_shadows, shadow_insert, Analyzer, EnvBinding, Shadow,
};
use super::free_vars::free_vars_in_block;
use super::report::{ArenaNode, ArenaReport, BindingInfo, Ownership, SemaError};
use crate::ast::{BindingKind, Block, Expr, Function, Program, Stmt, Type};
use crate::span::{Span, SpannedName};
use std::collections::HashMap;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionKind {
    Ordinary,
    Move,
}

fn analyze_function(az: &mut Analyzer, func: &Function) -> ArenaNode {
    let params: Vec<(String, Option<Type>)> = func
        .params
        .iter()
        .map(|p| (p.name.clone(), Some(p.ty.clone())))
        .collect();
    open_region(
        az,
        &format!("fun {}", func.name),
        &func.body,
        None,
        &params,
        RegionKind::Ordinary,
    )
}

fn resolve_move_captures(
    az: &Analyzer,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    param_names: &[String],
) -> Vec<SpannedName> {
    match explicit_captures {
        Some(caps) => caps.to_vec(),
        None => free_vars_in_block(body)
            .into_iter()
            .filter(|n| !param_names.contains(&n.name))
            .filter(|n| az.env.get(&n.name).is_some_and(|b| !b.moved))
            .collect(),
    }
}

fn open_region(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    params: &[(String, Option<Type>)],
    kind: RegionKind,
) -> ArenaNode {
    let (body, compacted) = collapse_block(body.clone());
    let id = az.alloc_id();
    let is_move = matches!(kind, RegionKind::Move);
    let label = if is_move {
        format!("{label} (move)")
    } else {
        label.to_string()
    };

    let param_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
    let captures = if is_move {
        resolve_move_captures(az, &body, explicit_captures, &param_names)
    } else {
        Vec::new()
    };

    let mut node = ArenaNode {
        id,
        label: label.clone(),
        compacted_braces: compacted,
        bindings: Vec::new(),
        children: Vec::new(),
    };

    let mut shadows = Vec::new();
    let mut moved_parents = Vec::new();

    for (name, ty) in params {
        shadows.push(bind(
            az,
            name,
            id,
            &label,
            ty.clone(),
            BindingKind::Val,
        ));
        node.bindings.push(BindingInfo {
            name: name.clone(),
            ownership: Ownership::Local,
            ty: ty.clone(),
            span: None,
        });
    }

    for cap in &captures {
        match az.env.get(&cap.name).cloned() {
            None => az.error(
                format!("cannot move unknown name `{}`", cap.name),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) if b.moved => az.error(
                format!(
                    "cannot move `{}`: already moved from {}",
                    cap.name, b.arena_label
                ),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) => {
                moved_parents.push(cap.name.clone());
                shadows.push(bind(
                    az,
                    &cap.name,
                    id,
                    &label,
                    b.ty.clone(),
                    BindingKind::Val,
                ));
                node.bindings.push(BindingInfo {
                    name: cap.name.clone(),
                    ownership: Ownership::Moved {
                        from: b.arena_label,
                    },
                    ty: b.ty,
                    span: Some(cap.span),
                });
            }
        }
    }

    walk_block(az, &body, &mut node, &mut shadows);
    restore_shadows(az, shadows);
    for cap in moved_parents {
        if let Some(b) = az.env.get_mut(&cap) {
            b.moved = true;
        }
    }

    node
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
            node.children.push(open_region(
                az,
                "Block",
                body,
                None,
                &[],
                RegionKind::Ordinary,
            ));
        }
        Stmt::MoveBlock { captures, body } => {
            node.children.push(open_region(
                az,
                "MoveBlock",
                body,
                captures.as_deref(),
                &[],
                RegionKind::Move,
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
            node.children.push(open_region(
                az,
                &format!("ForLoop ({name})"),
                body,
                None,
                &params,
                RegionKind::Ordinary,
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
                node.children.push(open_region(
                    az,
                    "Closure",
                    &c.body,
                    c.captures.as_deref(),
                    &params,
                    if c.is_move {
                        RegionKind::Move
                    } else {
                        RegionKind::Ordinary
                    },
                ));
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(az, cond, node);
            let env_before = az.env.clone();
            node.children.push(open_region(
                az,
                "IfThen",
                then_block,
                None,
                &[],
                RegionKind::Ordinary,
            ));
            let env_after_then = az.env.clone();
            az.env = env_before.clone();
            let env_after_else = if let Some(else_b) = else_block {
                node.children.push(open_region(
                    az,
                    "IfElse",
                    else_b,
                    None,
                    &[],
                    RegionKind::Ordinary,
                ));
                Some(az.env.clone())
            } else {
                None
            };
            az.env = env_before;
            merge_branch_moves(&mut az.env, &env_after_then, env_after_else.as_ref());
        }
        Expr::Int(_) | Expr::Str(_) | Expr::None => {}
    }
}

fn merge_branch_moves(
    env: &mut HashMap<String, EnvBinding>,
    after_then: &HashMap<String, EnvBinding>,
    after_else: Option<&HashMap<String, EnvBinding>>,
) {
    for (name, binding) in env.iter_mut() {
        let then_moved = after_then.get(name).is_some_and(|b| b.moved);
        binding.moved = match after_else {
            Some(env_else) => then_moved && env_else.get(name).is_some_and(|b| b.moved),
            None => then_moved && binding.moved,
        };
    }
}

fn note_use(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name).cloned() else {
        return;
    };
    if b.moved {
        az.error(
            format!("use of `{name}` after move from {}", b.arena_label),
            Some(name.to_string()),
            span,
        );
        return;
    }
    if b.arena_id == node.id {
        return;
    }
    let is_copy = b.ty.as_ref().is_some_and(Type::is_copy);
    if is_copy {
        record_obs(node, name, Ownership::Copy, b.ty, span);
        return;
    }
    if matches!(b.kind, BindingKind::Val) {
        record_obs(
            node,
            name,
            Ownership::Shared {
                from: b.arena_label,
            },
            b.ty,
            span,
        );
        return;
    }
    az.error(
        format!(
            "`{name}` is not Copy; move it into `{}` with `move`",
            node.label
        ),
        Some(name.to_string()),
        span,
    );
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
