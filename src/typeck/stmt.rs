use crate::ast::{BindingKind, Block, Stmt};
use crate::hir::{HirBlock, HirStmt, Ty};

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
        .filter_map(|stmt| check(stmt, return_ty, env))
        .collect();

    if nested {
        env.exit_scope();
    }

    HirBlock { stmts }
}

pub(super) fn check(stmt: &Stmt, return_ty: &Ty, env: &mut Env<'_>) -> Option<HirStmt> {
    match stmt {
        Stmt::Block(block) => Some(HirStmt::Block(check_block(block, return_ty, env, true))),
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
            let value = expr::check(value, declared_ty.as_ref(), return_ty, env)?;
            let declared_ty = declared_ty.unwrap_or_else(|| value.ty.clone());
            if declared_ty != value.ty {
                env.error(
                    format!(
                        "initializer for `{name}` has type {}, expected {declared_ty}",
                        value.ty
                    ),
                    Some(*name_span),
                );
            }
            env.bind(name.clone(), *kind, declared_ty.clone());
            Some(HirStmt::VarDecl {
                kind: *kind,
                name: name.clone(),
                ty: declared_ty,
                value,
            })
        }
        Stmt::Assign {
            name,
            name_span,
            value,
        } => {
            let binding = env.binding(name).cloned();
            let Some(binding) = binding else {
                env.error(format!("unknown binding `{name}`"), Some(*name_span));
                return None;
            };
            if binding.kind == BindingKind::Val {
                env.error(
                    format!("cannot assign to immutable `val` binding `{name}`"),
                    Some(*name_span),
                );
            }
            let value = expr::check(value, Some(&binding.ty), return_ty, env)?;
            if binding.ty != value.ty {
                env.error(
                    format!(
                        "assignment to `{name}` has type {}, expected {}",
                        value.ty, binding.ty
                    ),
                    Some(*name_span),
                );
            }
            Some(HirStmt::Assign {
                name: name.clone(),
                value,
            })
        }
        Stmt::Return(value) => {
            let checked = match value {
                Some(value) => {
                    let value = expr::check(value, Some(return_ty), return_ty, env)?;
                    if &value.ty != return_ty {
                        env.error(
                            format!("return value has type {}, expected {return_ty}", value.ty),
                            None,
                        );
                    }
                    Some(value)
                }
                None => {
                    let unit = Ty::from_ast(&crate::ast::Type::unit(false));
                    if &unit != return_ty {
                        env.error(
                            format!("empty return has type {unit}, expected {return_ty}"),
                            None,
                        );
                    }
                    None
                }
            };
            Some(HirStmt::Return { value: checked })
        }
        Stmt::Expr(value) => {
            let value = expr::check(value, None, return_ty, env)?;
            Some(HirStmt::Expr(value))
        }
        Stmt::For { name, iter, body } => {
            let iter = expr::check(iter, None, return_ty, env)?;
            let Ty::Range { elem } = iter.ty.clone() else {
                env.error("for-loop iterator must be a range", None);
                return Some(HirStmt::Expr(iter));
            };
            if *elem != Ty::i32() {
                env.error("for-loop range elements must have type i32", None);
            }

            let region = env.alloc_region();
            env.enter_scope();
            env.bind(name.name.clone(), BindingKind::Val, (*elem).clone());
            let body = check_block(body, return_ty, env, false);
            env.exit_scope();
            Some(HirStmt::For {
                name: name.name.clone(),
                iter,
                body,
                region,
            })
        }
        Stmt::MoveBlock { captures, body } => {
            let region = env.alloc_region();
            let body = check_block(body, return_ty, env, true);
            Some(HirStmt::MoveBlock {
                captures: captures.as_ref().map(|names| {
                    names
                        .iter()
                        .map(|capture| capture.name.clone())
                        .collect::<Vec<_>>()
                }),
                body,
                region,
            })
        }
    }
}
