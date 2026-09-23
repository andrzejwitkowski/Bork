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

fn check(stmt: &Stmt, return_ty: &Ty, env: &mut Env<'_>) -> Option<HirStmt> {
    match stmt {
        Stmt::Block(block) => Some(HirStmt::Block(check_block(block, return_ty, env, true))),
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            let (value, value_ty) = expr::check(value, env)?;
            let declared_ty = ty.as_ref().map(Ty::from_ast).unwrap_or(value_ty.clone());
            if declared_ty != value_ty {
                env.error(
                    format!(
                        "initializer for `{name}` has type {value_ty:?}, expected {declared_ty:?}"
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
            let (value, value_ty) = expr::check(value, env)?;
            if binding.ty != value_ty {
                env.error(
                    format!(
                        "assignment to `{name}` has type {value_ty:?}, expected {:?}",
                        binding.ty
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
                    let (value, value_ty) = expr::check(value, env)?;
                    if &value_ty != return_ty {
                        env.error(
                            format!("return value has type {value_ty:?}, expected {return_ty:?}"),
                            None,
                        );
                    }
                    Some(value)
                }
                None => {
                    let unit = Ty::from_ast(&crate::ast::Type::unit(false));
                    if &unit != return_ty {
                        env.error(
                            format!("empty return has type {unit:?}, expected {return_ty:?}"),
                            None,
                        );
                    }
                    None
                }
            };
            Some(HirStmt::Return { value: checked })
        }
        _ => {
            env.error("statement is not supported by type checking yet", None);
            None
        }
    }
}
