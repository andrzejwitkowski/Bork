use crate::ast::{BindingKind, Block, Stmt};
use crate::hir::{HirBlock, HirStmt, Ty, TyKind};

use super::env::Env;
use super::expr;

pub(super) fn check_block(
    block: &Block,
    return_ty: &Ty,
    env: &mut Env<'_>,
    nested: bool,
) -> HirBlock {
    if nested {
        env.enter_scope();
    }

    let stmts = block
        .stmts
        .iter()
        .map(|stmt| check(stmt, return_ty, env))
        .collect();

    if nested {
        env.exit_scope();
    }

    HirBlock { stmts }
}

pub(super) fn block_always_returns(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_always_returns)
}

fn stmt_always_returns(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return { .. } => true,
        HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => block_always_returns(body),
        HirStmt::Expr(expr) => expr_always_returns(expr),
        HirStmt::VarDecl { .. } | HirStmt::Assign { .. } | HirStmt::For { .. } => false,
    }
}

fn expr_always_returns(expr: &crate::hir::HirExpr) -> bool {
    match &expr.kind {
        crate::hir::HirExprKind::If {
            then_block,
            else_block: Some(else_block),
            ..
        } => block_always_returns(then_block) && block_always_returns(else_block),
        _ => false,
    }
}

pub(super) fn check(stmt: &Stmt, return_ty: &Ty, env: &mut Env<'_>) -> HirStmt {
    match stmt {
        Stmt::Block(block) => HirStmt::Block(check_block(block, return_ty, env, true)),
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            let declared_ty = ty
                .as_ref()
                .map(|ty| super::lower_type(ty, &mut env.diagnostics));
            let value = expr::check(value, declared_ty.as_ref(), return_ty, env);
            // Bind whatever type we can settle on, even after a bad initializer,
            // so later uses of `name` are not reported as unknown bindings.
            let declared_ty = declared_ty.unwrap_or_else(|| value.ty.clone());
            if !value.ty.is_unknown() && !declared_ty.is_unknown() && declared_ty != value.ty {
                env.error(
                    format!(
                        "initializer for `{name}` has type {}, expected {declared_ty}",
                        value.ty
                    ),
                    Some(*name_span),
                );
            }
            env.bind(name.clone(), *kind, declared_ty.clone());
            HirStmt::VarDecl {
                kind: *kind,
                name: name.clone(),
                ty: declared_ty,
                value,
                alloc_in_binding: None,
            }
        }
        Stmt::Assign {
            name,
            name_span,
            value,
        } => {
            let binding = env.binding(name).cloned();
            if binding.is_none() {
                env.error(format!("unknown binding `{name}`"), Some(*name_span));
            }
            if binding.as_ref().is_some_and(|b| b.kind == BindingKind::Val) {
                env.error(
                    format!("cannot assign to immutable `val` binding `{name}`"),
                    Some(*name_span),
                );
            }
            let expected = binding.as_ref().map(|b| b.ty.clone());
            let value = expr::check(value, expected.as_ref(), return_ty, env);
            if let Some(expected) = expected {
                if !value.ty.is_unknown() && !expected.is_unknown() && expected != value.ty {
                    env.error(
                        format!(
                            "assignment to `{name}` has type {}, expected {expected}",
                            value.ty
                        ),
                        Some(*name_span),
                    );
                }
            }
            HirStmt::Assign {
                name: name.clone(),
                value,
            }
        }
        Stmt::Return(value) => {
            let checked = match value {
                Some(value) => {
                    let value = expr::check(value, Some(return_ty), return_ty, env);
                    if !value.ty.is_unknown() && &value.ty != return_ty {
                        env.error(
                            format!("return value has type {}, expected {return_ty}", value.ty),
                            value.span,
                        );
                    }
                    Some(value)
                }
                None => {
                    let unit = Ty::unit();
                    if &unit != return_ty {
                        env.error(
                            format!("empty return has type {unit}, expected {return_ty}"),
                            None,
                        );
                    }
                    None
                }
            };
            HirStmt::Return { value: checked }
        }
        Stmt::Expr(value) => HirStmt::Expr(expr::check(value, None, return_ty, env)),
        Stmt::For { name, iter, body } => {
            let iter = expr::check(iter, None, return_ty, env);
            let TyKind::Range(elem) = iter.ty.kind.clone() else {
                if !iter.ty.is_unknown() {
                    env.error("for-loop iterator must be a range", None);
                }
                return HirStmt::Expr(iter);
            };
            if *elem != Ty::i32() {
                env.error("for-loop range elements must have type i32", None);
            }

            env.enter_scope();
            env.bind(name.name.clone(), BindingKind::Val, (*elem).clone());
            let body = check_block(body, return_ty, env, false);
            env.exit_scope();
            HirStmt::For {
                name: name.name.clone(),
                iter,
                body,
            }
        }
        Stmt::MoveBlock { captures, body } => HirStmt::MoveBlock {
            captures: captures.as_ref().map(|names| {
                names
                    .iter()
                    .map(|capture| capture.name.clone())
                    .collect::<Vec<_>>()
            }),
            body: check_block(body, return_ty, env, true),
        },
    }
}
