//! Free-variable collection for inferred move captures.

use crate::ast::{AssignTarget, Block, Expr, Stmt};
use crate::span::{Span, SpannedName};
use std::collections::{BTreeMap, HashSet};

pub(super) fn free_vars_in_block(block: &Block) -> Vec<SpannedName> {
    let mut free: BTreeMap<String, Span> = BTreeMap::new();
    let mut bound = HashSet::new();
    collect_block(block, &mut free, &mut bound);
    free.into_iter()
        .map(|(name, span)| SpannedName::new(name, span))
        .collect()
}

fn collect_block(
    block: &Block,
    free: &mut BTreeMap<String, Span>,
    bound: &mut HashSet<String>,
) {
    for stmt in &block.stmts {
        collect_stmt(stmt, free, bound);
    }
}

fn collect_stmt(
    stmt: &Stmt,
    free: &mut BTreeMap<String, Span>,
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
                        free.entry(cap.name.clone()).or_insert(cap.span);
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
                AssignTarget::Name { name, name_span } => {
                    if !bound.contains(name) {
                        free.entry(name.clone()).or_insert(*name_span);
                    }
                }
                AssignTarget::Index {
                    name,
                    name_span,
                    index,
                } => {
                    if !bound.contains(name) {
                        free.entry(name.clone()).or_insert(*name_span);
                    }
                    collect_expr(index, free, bound);
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
            collect_block(body, free, bound);
        }
        Stmt::Break { .. } | Stmt::Continue { .. } => {}
        Stmt::Return(Some(e)) | Stmt::Expr(e) => collect_expr(e, free, bound),
        Stmt::Return(None) => {}
    }
}

fn collect_expr(
    expr: &Expr,
    free: &mut BTreeMap<String, Span>,
    bound: &mut HashSet<String>,
) {
    match expr {
        Expr::Ident { name, span } => {
            if !bound.contains(name) {
                free.entry(name.clone()).or_insert(*span);
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
                            free.entry(cap.name.clone()).or_insert(cap.span);
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
        Expr::Str(_) | Expr::None { .. } => {}
        Expr::ArrayLit { elements, .. } => {
            for element in elements {
                collect_expr(element, free, bound);
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
