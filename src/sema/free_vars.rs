//! Free-variable collection for inferred move captures.

use crate::ast::{AssignTarget, Block, Expr, Stmt};
use crate::span::{AstId, Span, SpannedName};
use std::collections::{BTreeMap, HashSet};

pub(super) fn free_vars_in_block(block: &Block) -> Vec<SpannedName> {
    let mut free: BTreeMap<String, (Span, AstId)> = BTreeMap::new();
    let mut bound = HashSet::new();
    collect_block(block, &mut free, &mut bound);
    free.into_iter()
        .map(|(name, (span, id))| SpannedName { name, span, id })
        .collect()
}

fn collect_block(
    block: &Block,
    free: &mut BTreeMap<String, (Span, AstId)>,
    bound: &mut HashSet<String>,
) {
    for stmt in &block.stmts {
        collect_stmt(stmt, free, bound);
    }
}

fn collect_stmt(
    stmt: &Stmt,
    free: &mut BTreeMap<String, (Span, AstId)>,
    bound: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Block(b) => {
            let mut inner = bound.clone();
            collect_block(b, free, &mut inner);
        }
        Stmt::MoveBlock { body: b, captures } => {
            if let Some(caps) = captures {
                for cap in caps {
                    if !bound.contains(&cap.name) {
                        free.entry(cap.name.clone()).or_insert((cap.span, cap.id));
                    }
                }
            }
            let mut inner = bound.clone();
            if let Some(caps) = captures {
                for cap in caps {
                    inner.insert(cap.name.clone());
                }
            }
            collect_block(b, free, &mut inner);
        }
        Stmt::VarDecl { name, value, .. } => {
            collect_expr(value, free, bound);
            bound.insert(name.clone());
        }
        Stmt::Assign { target, value } => {
            match target {
                AssignTarget::Name { name, name_span, id } => {
                    if !bound.contains(name) {
                        free.entry(name.clone()).or_insert((*name_span, *id));
                    }
                }
                AssignTarget::Index {
                    name,
                    name_span,
                    id,
                    index,
                    ..
                } => {
                    if !bound.contains(name) {
                        free.entry(name.clone()).or_insert((*name_span, *id));
                    }
                    collect_expr(index, free, bound);
                }
                AssignTarget::Field { receiver, .. } => {
                    collect_expr(receiver, free, bound);
                }
            }
            collect_expr(value, free, bound);
        }
        Stmt::For { name, iter, body } => {
            collect_expr(iter, free, bound);
            let mut inner = bound.clone();
            inner.insert(name.name.clone());
            collect_block(body, free, &mut inner);
        }
        Stmt::While { cond, body } => {
            collect_expr(cond, free, bound);
            let mut inner = bound.clone();
            collect_block(body, free, &mut inner);
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Return(Some(e)) | Stmt::Expr(e) => collect_expr(e, free, bound),
        Stmt::Return(None) => {}
    }
}

/// Names a pattern binds. A bare identifier that turns out to be a payload-free variant is
/// added too; that only hides a free name of the same spelling inside the arm.
fn pattern_names(pat: &crate::ast::Pattern, bound: &mut HashSet<String>) {
    use crate::ast::Pattern;
    match pat {
        Pattern::Binding(name) => {
            bound.insert(name.name.clone());
        }
        Pattern::Variant { args, .. } => args.iter().for_each(|arg| pattern_names(arg, bound)),
        Pattern::Some { inner, .. } => pattern_names(inner, bound),
        Pattern::Wildcard { .. } | Pattern::Literal { .. } | Pattern::None { .. } => {}
    }
}

fn collect_expr(
    expr: &Expr,
    free: &mut BTreeMap<String, (Span, AstId)>,
    bound: &mut HashSet<String>,
) {
    match expr {
        Expr::Ident { name, span, id } => {
            if !bound.contains(name) {
                free.entry(name.clone()).or_insert((*span, *id));
            }
        }
        // Operand of `move name` is transferred by the expression itself, not by
        // inferred `move { }` capture at block entry.
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) => {}
        Expr::Move { .. } => {}
        Expr::Promote { .. } => {}
        Expr::Some { expr, .. } => collect_expr(expr, free, bound),
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, free, bound);
            collect_expr(rhs, free, bound);
        }
        Expr::Unary { expr, .. } => collect_expr(expr, free, bound),
        Expr::Field { receiver, .. } => collect_expr(receiver, free, bound),
        Expr::Call {
            callee,
            args,
            trailing,
            ..
        } => {
            collect_expr(callee, free, bound);
            for a in args {
                collect_expr(a, free, bound);
            }
            if let Some(c) = trailing {
                let mut inner = bound.clone();
                for p in &c.params {
                    inner.insert(p.name.clone());
                }
                if let Some(caps) = &c.captures {
                    for cap in caps {
                        if !bound.contains(&cap.name) {
                            free.entry(cap.name.clone()).or_insert((cap.span, cap.id));
                        }
                        inner.insert(cap.name.clone());
                    }
                }
                collect_block(&c.body, free, &mut inner);
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_expr(cond, free, bound);
            collect_block(then_block, free, &mut bound.clone());
            if let Some(e) = else_block {
                collect_block(e, free, &mut bound.clone());
            }
        }
        Expr::IfVal {
            bindings,
            then_block,
            else_block,
        } => {
            let mut present = bound.clone();
            for binding in bindings.iter() {
                collect_expr(&binding.value, free, &mut present);
                present.insert(binding.name.name.clone());
            }
            collect_block(then_block, free, &mut present);
            if let Some(block) = else_block {
                collect_block(block, free, &mut bound.clone());
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_expr(scrutinee, free, bound);
            for arm in arms {
                let mut arm_bound = bound.clone();
                pattern_names(&arm.pat, &mut arm_bound);
                collect_block(&arm.body, free, &mut arm_bound);
            }
        }
        Expr::Str(_) | Expr::None { .. } => {}
        Expr::ArrayLit { elements, .. } => {
            for element in elements {
                collect_expr(element, free, bound);
            }
        }
        Expr::VecNew { cap, .. } => {
            if let Some(cap) = cap {
                collect_expr(cap, free, bound);
            }
        }
        Expr::Index { receiver, index, .. } => {
            collect_expr(receiver, free, bound);
            collect_expr(index, free, bound);
        }
        Expr::Slice { receiver, lo, hi, .. } => {
            collect_expr(receiver, free, bound);
            collect_expr(lo, free, bound);
            collect_expr(hi, free, bound);
        }
    }
}
