use crate::ast::{BinOp, Expr, UnaryOp};
use crate::hir::{HirExpr, Ty};

use super::env::Env;
use super::stmt;

pub(super) fn check(
    expr: &Expr,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> Option<(HirExpr, Ty)> {
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
        Expr::None => {
            let Some(ty) = expected.filter(|ty| ty.is_nullable()) else {
                env.error("cannot infer type of `None`", None);
                return Some((HirExpr::None, Ty::Unknown));
            };
            Some((HirExpr::None, ty.clone()))
        }
        Expr::Some(inner) => {
            let expected_inner = expected
                .filter(|ty| ty.is_nullable())
                .and_then(|ty| ty.with_nullable(false));
            let (inner, inner_ty) = check(inner, expected_inner.as_ref(), return_ty, env)?;
            let Some(nullable_ty) = inner_ty.with_nullable(true) else {
                env.error(
                    format!("`Some` value has non-nullable-incompatible type {inner_ty:?}"),
                    None,
                );
                return Some((HirExpr::Some(Box::new(inner)), Ty::Unknown));
            };
            Some((HirExpr::Some(Box::new(inner)), nullable_ty))
        }
        Expr::Binary { op, lhs, rhs } => {
            if *op == BinOp::Elvis {
                let expected_lhs = expected.and_then(|ty| ty.with_nullable(true));
                let (lhs, lhs_ty) = check(lhs, expected_lhs.as_ref(), return_ty, env)?;
                let Some(result_ty) = lhs_ty.with_nullable(false).filter(|_| lhs_ty.is_nullable())
                else {
                    env.error(
                        format!("left operand of `?:` must be nullable, got {lhs_ty:?}"),
                        None,
                    );
                    let (rhs, _) = check(rhs, expected, return_ty, env)?;
                    return Some((
                        HirExpr::Binary {
                            op: op.clone(),
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        Ty::Unknown,
                    ));
                };
                let (rhs, rhs_ty) = check(rhs, Some(&result_ty), return_ty, env)?;
                if rhs_ty != result_ty {
                    env.error(
                        format!(
                            "right operand of `?:` has type {rhs_ty:?}, expected {result_ty:?}"
                        ),
                        None,
                    );
                }
                return Some((
                    HirExpr::Binary {
                        op: op.clone(),
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    result_ty,
                ));
            }

            let (lhs, lhs_ty) = check(lhs, None, return_ty, env)?;
            let (rhs, rhs_ty) = check(rhs, None, return_ty, env)?;

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
                BinOp::RangeTo => {
                    env.error(
                        "binary operator is not supported by type checking yet",
                        None,
                    );
                    Ty::Unknown
                }
                BinOp::Elvis => unreachable!("Elvis is handled before other binary operators"),
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
        Expr::Unary {
            op: UnaryOp::NotNullAssert,
            expr,
        } => {
            let expected_operand = expected.and_then(|ty| ty.with_nullable(true));
            let (expr, expr_ty) = check(expr, expected_operand.as_ref(), return_ty, env)?;
            let Some(result_ty) = expr_ty
                .with_nullable(false)
                .filter(|_| expr_ty.is_nullable())
            else {
                env.error(
                    format!("operand of `!!` must be nullable, got {expr_ty:?}"),
                    None,
                );
                return Some((
                    HirExpr::Unary {
                        op: UnaryOp::NotNullAssert,
                        expr: Box::new(expr),
                    },
                    Ty::Unknown,
                ));
            };
            Some((
                HirExpr::Unary {
                    op: UnaryOp::NotNullAssert,
                    expr: Box::new(expr),
                },
                result_ty,
            ))
        }
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            if trailing.is_some() {
                env.error("trailing closures not typed yet", None);
            }

            let Expr::Ident { name, span } = callee.as_ref() else {
                env.error("only named functions can be called", None);
                return None;
            };
            let Some(signature) = env.fun_sig(name).cloned() else {
                env.error(format!("unknown function `{name}`"), Some(*span));
                return None;
            };

            let checked_args: Vec<_> = args
                .iter()
                .enumerate()
                .map(|(index, arg)| check(arg, signature.params.get(index), return_ty, env))
                .collect::<Option<Vec<_>>>()?;

            if checked_args.len() != signature.params.len() {
                env.error(
                    format!(
                        "function `{name}` expects {} arguments, got {}",
                        signature.params.len(),
                        checked_args.len()
                    ),
                    Some(*span),
                );
            }
            for (index, ((_, actual), expected)) in
                checked_args.iter().zip(&signature.params).enumerate()
            {
                if actual != expected {
                    env.error(
                        format!(
                            "argument {} to `{name}` has type {actual:?}, expected {expected:?}",
                            index + 1
                        ),
                        None,
                    );
                }
            }

            Some((
                HirExpr::Call {
                    callee: Box::new(HirExpr::Ident { name: name.clone() }),
                    args: checked_args
                        .into_iter()
                        .map(|(argument, _)| argument)
                        .collect(),
                },
                signature.return_ty,
            ))
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            let expected_cond = bool_ty();
            let (cond, cond_ty) = check(cond, Some(&expected_cond), return_ty, env)?;
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
