use crate::ast::{BinOp, Expr};
use crate::hir::{HirExpr, Ty};

use super::env::Env;
use super::stmt;

pub(super) fn check(expr: &Expr, return_ty: &Ty, env: &mut Env<'_>) -> Option<(HirExpr, Ty)> {
    match expr {
        Expr::Int(value) => Some((HirExpr::Int { value: *value }, Ty::i32())),
        Expr::Str(value) => Some((
            HirExpr::Str {
                value: value.clone(),
            },
            Ty::string(false),
        )),
        Expr::Ident { name, span } => {
            let Some(binding) = env.binding(name) else {
                env.error(format!("unknown binding `{name}`"), Some(*span));
                return None;
            };
            Some((HirExpr::Ident { name: name.clone() }, binding.ty.clone()))
        }
        Expr::Binary { op, lhs, rhs } => {
            let (lhs, lhs_ty) = check(lhs, return_ty, env)?;
            let (rhs, rhs_ty) = check(rhs, return_ty, env)?;

            let result_ty = match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                    if lhs_ty != rhs_ty || !is_numeric(&lhs_ty) {
                        env.error(
                            format!(
                                "arithmetic operands must have the same numeric type, got \
                                 {lhs_ty:?} and {rhs_ty:?}"
                            ),
                            None,
                        );
                    }
                    lhs_ty
                }
                BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => {
                    if lhs_ty != rhs_ty || !is_numeric(&lhs_ty) {
                        env.error(
                            format!(
                                "ordered comparison operands must have the same numeric type, got \
                                 {lhs_ty:?} and {rhs_ty:?}"
                            ),
                            None,
                        );
                    }
                    bool_ty()
                }
                BinOp::Eq | BinOp::Ne => {
                    if lhs_ty != rhs_ty {
                        env.error(
                            format!(
                                "equality operands must have the same type, got {lhs_ty:?} and \
                                 {rhs_ty:?}"
                            ),
                            None,
                        );
                    }
                    bool_ty()
                }
                BinOp::RangeTo | BinOp::Elvis => {
                    env.error(
                        "binary operator is not supported by type checking yet",
                        None,
                    );
                    Ty::Unknown
                }
            };

            Some((
                HirExpr::Binary {
                    op: op.clone(),
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                result_ty,
            ))
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            let (cond, cond_ty) = check(cond, return_ty, env)?;
            if cond_ty != bool_ty() {
                env.error(
                    format!("if condition has type {cond_ty:?}, expected bool"),
                    None,
                );
            }
            let then_block = stmt::check_block(then_block, return_ty, env, true);
            let else_block = else_block
                .as_ref()
                .map(|block| stmt::check_block(block, return_ty, env, true));
            Some((
                HirExpr::If {
                    cond: Box::new(cond),
                    then_block,
                    else_block,
                },
                unit_ty(),
            ))
        }
        _ => {
            env.error("expression is not supported by type checking yet", None);
            None
        }
    }
}

fn is_numeric(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Primitive {
            name,
            nullable: false,
        } if matches!(
            name.as_str(),
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64"
        )
    )
}

fn bool_ty() -> Ty {
    Ty::Primitive {
        name: "bool".into(),
        nullable: false,
    }
}

fn unit_ty() -> Ty {
    Ty::Primitive {
        name: "unit".into(),
        nullable: false,
    }
}
