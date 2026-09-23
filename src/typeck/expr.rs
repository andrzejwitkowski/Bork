use crate::ast::{BinOp, BindingKind, Block, Closure, Expr, Stmt, UnaryOp};
use crate::hir::{HirBlock, HirExpr, Ty, UseKind};

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
            let ty = binding.ty.clone();
            let use_kind = if ty.is_copy() {
                UseKind::Copy
            } else {
                UseKind::Local
            };
            Some((
                HirExpr::Ident {
                    name: name.clone(),
                    use_kind,
                },
                ty,
            ))
        }
        Expr::Move { name, span } => {
            let Some(binding) = env.binding(name) else {
                env.error(format!("unknown binding `{name}`"), Some(*span));
                return None;
            };
            Some((
                HirExpr::Ident {
                    name: name.clone(),
                    use_kind: UseKind::Move,
                },
                binding.ty.clone(),
            ))
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
                let result_ty = lhs_ty.with_nullable(false);
                let Some(result_ty) = result_ty else {
                    env.error(
                        format!("left operand of `?:` cannot have type {lhs_ty:?}"),
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
                if !lhs_ty.is_nullable() && !lhs_ty.is_copy() {
                    env.error(
                        format!("left operand of `?:` must be nullable, got {lhs_ty:?}"),
                        None,
                    );
                }
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
                    if !same_numeric_base(&lhs_ty, &rhs_ty) {
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
                    if lhs_ty != Ty::i32() || rhs_ty != Ty::i32() {
                        env.error(
                            format!(
                                "range bounds must have type i32, got {lhs_ty:?} and {rhs_ty:?}"
                            ),
                            None,
                        );
                    }
                    Ty::Range {
                        elem: Box::new(Ty::i32()),
                    }
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
        Expr::Field {
            receiver,
            name,
            safe,
        } => {
            let (receiver, receiver_ty) = check(receiver, None, return_ty, env)?;
            let result_ty = if receiver_ty.is_string() && name == "length" {
                if receiver_ty.is_nullable() && !safe {
                    env.error("field access on nullable `String?` requires `?.`", None);
                }
                Ty::i32()
                    .with_nullable(*safe && receiver_ty.is_nullable())
                    .expect("i32 supports nullable form")
            } else {
                env.error(
                    format!("unknown field `{name}` on type {receiver_ty:?}"),
                    None,
                );
                Ty::Unknown
            };
            Some((
                HirExpr::Field {
                    receiver: Box::new(receiver),
                    name: name.clone(),
                    safe: *safe,
                },
                result_ty,
            ))
        }
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            let Expr::Ident { name, span } = callee.as_ref() else {
                env.error("only named functions can be called", None);
                return None;
            };
            let (params, call_return_ty) = if let Some(signature) = env.fun_sig(name).cloned() {
                (signature.params, signature.return_ty)
            } else {
                match env.binding(name).map(|binding| binding.ty.clone()) {
                    Some(Ty::Func {
                        params,
                        ret,
                        nullable: false,
                    }) => (params, *ret),
                    Some(ty) => {
                        env.error(
                            format!("binding `{name}` is not callable (has type {ty:?})"),
                            Some(*span),
                        );
                        return None;
                    }
                    None => {
                        env.error(format!("unknown function `{name}`"), Some(*span));
                        return None;
                    }
                }
            };

            let (regular_params, trailing_signature) = if trailing.is_some() {
                match params.split_last() {
                    Some((
                        Ty::Func {
                            params,
                            ret,
                            nullable: _,
                        },
                        regular,
                    )) => (regular, Some((params.as_slice(), ret.as_ref()))),
                    _ => {
                        env.error(
                            format!(
                                "function `{name}` requires a function type as its last parameter \
                                 when called with a trailing closure"
                            ),
                            Some(*span),
                        );
                        (params.as_slice(), None)
                    }
                }
            } else {
                (params.as_slice(), None)
            };

            let checked_args: Vec<_> = args
                .iter()
                .enumerate()
                .map(|(index, arg)| check(arg, regular_params.get(index), return_ty, env))
                .collect::<Option<Vec<_>>>()?;

            if checked_args.len() != regular_params.len() {
                env.error(
                    format!(
                        "function `{name}` expects {} arguments, got {}",
                        regular_params.len(),
                        checked_args.len()
                    ),
                    Some(*span),
                );
            }
            for (index, ((_, actual), expected)) in
                checked_args.iter().zip(regular_params).enumerate()
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

            if let (Some(closure), Some((closure_params, closure_ret))) =
                (trailing, trailing_signature)
            {
                check_trailing_closure(closure, closure_params, closure_ret, env);
            }

            Some((
                HirExpr::Call {
                    callee: Box::new(HirExpr::Ident {
                        name: name.clone(),
                        use_kind: UseKind::Local,
                    }),
                    args: checked_args
                        .into_iter()
                        .map(|(argument, _)| argument)
                        .collect(),
                },
                call_return_ty,
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
            let (then_block, result_ty) = if let Some(expected) = expected {
                check_value_block(then_block, expected, return_ty, env)
            } else {
                (
                    stmt::check_block(then_block, return_ty, env, true),
                    unit_ty(),
                )
            };
            let else_block = if let Some(block) = else_block {
                if let Some(expected) = expected {
                    let (block, else_ty) = check_value_block(block, expected, return_ty, env);
                    if else_ty != result_ty {
                        env.error(
                            format!("else branch has type {else_ty:?}, expected {result_ty:?}"),
                            None,
                        );
                    }
                    Some(block)
                } else {
                    Some(stmt::check_block(block, return_ty, env, true))
                }
            } else {
                if expected.is_some() {
                    env.error(
                        "value-producing if expression requires an else branch",
                        None,
                    );
                }
                None
            };
            Some((
                HirExpr::If {
                    cond: Box::new(cond),
                    then_block,
                    else_block,
                },
                result_ty,
            ))
        }
    }
}

fn check_trailing_closure(
    closure: &Closure,
    param_tys: &[Ty],
    closure_ret: &Ty,
    env: &mut Env<'_>,
) {
    if closure.params.len() != param_tys.len() {
        env.error(
            format!(
                "trailing closure expects {} parameters, got {}",
                param_tys.len(),
                closure.params.len()
            ),
            None,
        );
    }

    env.enter_scope();
    for (param, ty) in closure.params.iter().zip(param_tys) {
        env.bind(param.name.clone(), BindingKind::Val, ty.clone());
    }
    let (_, body_ty) =
        check_value_block_in_current_scope(&closure.body, closure_ret, closure_ret, env);
    env.exit_scope();

    if &body_ty != closure_ret {
        env.error(
            format!("trailing closure body has type {body_ty:?}, expected {closure_ret:?}"),
            None,
        );
    }
}

fn check_value_block(
    block: &Block,
    expected: &Ty,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> (HirBlock, Ty) {
    env.enter_scope();
    let result = check_value_block_in_current_scope(block, expected, return_ty, env);
    env.exit_scope();
    result
}

fn check_value_block_in_current_scope(
    block: &Block,
    expected: &Ty,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> (HirBlock, Ty) {
    let Some((last, prefix)) = block.stmts.split_last() else {
        return (HirBlock { stmts: Vec::new() }, unit_ty());
    };

    let mut stmts = prefix
        .iter()
        .filter_map(|statement| stmt::check(statement, return_ty, env))
        .collect::<Vec<_>>();
    let result_ty = match last {
        Stmt::Expr(value) => match check(value, Some(expected), return_ty, env) {
            Some((value, ty)) => {
                stmts.push(crate::hir::HirStmt::Expr(value));
                ty
            }
            None => Ty::Unknown,
        },
        statement => {
            if let Some(statement) = stmt::check(statement, return_ty, env) {
                stmts.push(statement);
            }
            unit_ty()
        }
    };
    (HirBlock { stmts }, result_ty)
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

fn same_numeric_base(lhs: &Ty, rhs: &Ty) -> bool {
    matches!(
        (lhs, rhs),
        (
            Ty::Primitive { name: lhs, .. },
            Ty::Primitive { name: rhs, .. }
        ) if lhs == rhs
            && matches!(
                lhs.as_str(),
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
