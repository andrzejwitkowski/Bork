use crate::ast::{BinOp, BindingKind, Block, Closure, Expr, Stmt, UnaryOp};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, Ty, TyKind, UseKind};
use crate::span::Span;

use super::env::Env;
use super::stmt;

/// Check always yields an expression: a failed check is reported once and
/// poisoned with `TyKind::Unknown`, which suppresses follow-on mismatches.
pub(super) fn check(
    expr: &Expr,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    match expr {
        Expr::Int(value) => {
            let ty = expected
                .filter(|ty| ty.is_integer())
                .cloned()
                .unwrap_or_else(Ty::i32);
            HirExpr::new(HirExprKind::Int { value: *value }, ty)
        }
        Expr::Str(value) => HirExpr::new(
            HirExprKind::Str {
                value: value.clone(),
            },
            Ty::string(false),
        ),
        Expr::Ident { name, span } => check_ident(name, *span, UseKind::Local, env),
        Expr::Move { name, span } => check_ident(name, *span, UseKind::Move, env),
        Expr::None => {
            let Some(ty) = expected.filter(|ty| ty.is_nullable()) else {
                env.error("cannot infer type of `None`", None);
                return HirExpr::new(HirExprKind::None, Ty::unknown());
            };
            HirExpr::new(HirExprKind::None, ty.clone())
        }
        Expr::Some(inner) => {
            let expected_inner = expected
                .filter(|ty| ty.is_nullable())
                .map(|ty| ty.with_nullable(false));
            let inner = check(inner, expected_inner.as_ref(), return_ty, env);
            let ty = if inner.ty.supports_nullable() {
                inner.ty.with_nullable(true)
            } else {
                if !inner.ty.is_unknown() {
                    env.error(format!("`Some` value cannot have type {}", inner.ty), None);
                }
                Ty::unknown()
            };
            HirExpr::new(HirExprKind::Some(Box::new(inner)), ty)
        }
        Expr::Binary { op, lhs, rhs } => {
            if *op == BinOp::Elvis {
                check_elvis(op, lhs, rhs, expected, return_ty, env)
            } else {
                check_binary(op, lhs, rhs, expected, return_ty, env)
            }
        }
        Expr::Unary {
            op: UnaryOp::NotNullAssert,
            expr,
        } => {
            let expected_operand = expected
                .filter(|ty| ty.supports_nullable())
                .map(|ty| ty.with_nullable(true));
            let operand = check(expr, expected_operand.as_ref(), return_ty, env);
            let result_ty = if operand.ty.is_nullable() {
                operand.ty.with_nullable(false)
            } else {
                if !operand.ty.is_unknown() {
                    env.error(
                        format!("operand of `!!` must be nullable, got {}", operand.ty),
                        None,
                    );
                }
                Ty::unknown()
            };
            HirExpr::new(
                HirExprKind::Unary {
                    op: UnaryOp::NotNullAssert,
                    expr: Box::new(operand),
                },
                result_ty,
            )
        }
        Expr::Field {
            receiver,
            name,
            safe,
        } => check_field(receiver, name, *safe, return_ty, env),
        Expr::Call {
            callee,
            args,
            trailing,
        } => check_call(callee, args, trailing.as_ref(), return_ty, env),
        Expr::If {
            cond,
            then_block,
            else_block,
        } => check_if(
            cond,
            then_block,
            else_block.as_ref(),
            expected,
            return_ty,
            env,
        ),
    }
}

fn check_ident(name: &str, span: Span, requested: UseKind, env: &mut Env<'_>) -> HirExpr {
    let Some(binding) = env.binding(name) else {
        env.error(format!("unknown binding `{name}`"), Some(span));
        return HirExpr::spanned(
            HirExprKind::Ident {
                name: name.to_string(),
                use_kind: requested,
            },
            Ty::unknown(),
            span,
        );
    };
    let ty = binding.ty.clone();
    let use_kind = if requested == UseKind::Move {
        UseKind::Move
    } else if ty.is_copy() {
        UseKind::Copy
    } else {
        UseKind::Local
    };
    HirExpr::spanned(
        HirExprKind::Ident {
            name: name.to_string(),
            use_kind,
        },
        ty,
        span,
    )
}

fn check_binary(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let outer_numeric = match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            expected.filter(|ty| ty.is_numeric())
        }
        _ => None,
    };

    // Guide Int/None from the peer when the left side cannot invent a type alone.
    let guide_from_rhs = matches!(lhs, Expr::Int(_) | Expr::None)
        && outer_numeric.is_none()
        && *op != BinOp::RangeTo;

    let (lhs, rhs) = if guide_from_rhs {
        let rhs = check(rhs, None, return_ty, env);
        let peer = (!rhs.ty.is_unknown()).then_some(rhs.ty.clone());
        let lhs = check(lhs, peer.as_ref(), return_ty, env);
        (lhs, rhs)
    } else {
        let lhs = check(lhs, outer_numeric, return_ty, env);
        let peer = (!lhs.ty.is_unknown() && *op != BinOp::RangeTo).then_some(lhs.ty.clone());
        let rhs_expected = outer_numeric.or(peer.as_ref());
        let rhs = check(rhs, rhs_expected, return_ty, env);
        (lhs, rhs)
    };

    let span = lhs.span.or(rhs.span);
    let (lhs_ty, rhs_ty) = (&lhs.ty, &rhs.ty);
    let poisoned = lhs_ty.is_unknown() || rhs_ty.is_unknown();

    let result_ty = match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            if poisoned {
                Ty::unknown()
            } else {
                if lhs_ty != rhs_ty || !lhs_ty.is_numeric() {
                    env.error(
                        format!(
                            "arithmetic operands must have the same numeric type, got \
                             {lhs_ty} and {rhs_ty}"
                        ),
                        span,
                    );
                }
                lhs_ty.clone()
            }
        }
        BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => {
            if !poisoned && !(lhs_ty == rhs_ty && lhs_ty.is_numeric()) {
                env.error(
                    format!(
                        "ordered comparison operands must have the same non-nullable \
                         numeric type, got {lhs_ty} and {rhs_ty}"
                    ),
                    span,
                );
            }
            Ty::bool()
        }
        BinOp::Eq | BinOp::Ne => {
            if !poisoned && lhs_ty != rhs_ty {
                env.error(
                    format!("equality operands must have the same type, got {lhs_ty} and {rhs_ty}"),
                    span,
                );
            }
            Ty::bool()
        }
        BinOp::RangeTo => {
            if !poisoned && (*lhs_ty != Ty::i32() || *rhs_ty != Ty::i32()) {
                env.error(
                    format!("range bounds must have type i32, got {lhs_ty} and {rhs_ty}"),
                    span,
                );
            }
            Ty::range(Ty::i32())
        }
        BinOp::Elvis => unreachable!("Elvis is handled before other binary operators"),
    };

    HirExpr::new(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        result_ty,
    )
}

fn check_elvis(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let expected_lhs = expected
        .filter(|ty| ty.supports_nullable())
        .map(|ty| ty.with_nullable(true));
    let lhs = check(lhs, expected_lhs.as_ref(), return_ty, env);

    let result_ty = if lhs.ty.supports_nullable() {
        if !lhs.ty.is_nullable() {
            env.error(
                format!("left operand of `?:` must be nullable, got {}", lhs.ty),
                None,
            );
        }
        lhs.ty.with_nullable(false)
    } else {
        if !lhs.ty.is_unknown() {
            env.error(
                format!("left operand of `?:` cannot have type {}", lhs.ty),
                None,
            );
        }
        Ty::unknown()
    };

    let rhs = check(rhs, Some(&result_ty), return_ty, env);
    if !result_ty.is_unknown() && !rhs.ty.is_unknown() && rhs.ty != result_ty {
        env.error(
            format!(
                "right operand of `?:` has type {}, expected {result_ty}",
                rhs.ty
            ),
            None,
        );
    }

    HirExpr::new(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        result_ty,
    )
}

fn check_field(
    receiver: &Expr,
    name: &str,
    safe: bool,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let receiver = check(receiver, None, return_ty, env);
    let result_ty = if receiver.ty.is_string() && name == "length" {
        if receiver.ty.is_nullable() && !safe {
            env.error("field access on nullable `String?` requires `?.`", None);
        }
        Ty::i32().with_nullable(safe && receiver.ty.is_nullable())
    } else {
        if !receiver.ty.is_unknown() {
            env.error(
                format!("unknown field `{name}` on type {}", receiver.ty),
                None,
            );
        }
        Ty::unknown()
    };
    HirExpr::new(
        HirExprKind::Field {
            receiver: Box::new(receiver),
            name: name.to_string(),
            safe,
        },
        result_ty,
    )
}

/// Formal parameter types of a callee, or `None` when the callee is not callable.
fn callee_signature(callee: &Expr, env: &mut Env<'_>) -> Option<(String, Span, Vec<Ty>, Ty)> {
    let Expr::Ident { name, span } = callee else {
        env.error("only named functions can be called", None);
        return None;
    };
    if let Some(signature) = env.fun_sig(name).cloned() {
        return Some((name.clone(), *span, signature.params, signature.return_ty));
    }
    match env.binding(name).map(|binding| binding.ty.clone()) {
        Some(Ty {
            kind: TyKind::Func { params, ret },
            nullable: false,
        }) => Some((name.clone(), *span, params, *ret)),
        Some(ty) => {
            if !ty.is_unknown() {
                env.error(
                    format!("binding `{name}` is not callable (has type {ty})"),
                    Some(*span),
                );
            }
            None
        }
        None => {
            env.error(format!("unknown function `{name}`"), Some(*span));
            None
        }
    }
}

fn check_call(
    callee: &Expr,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let Some((name, span, params, call_return_ty)) = callee_signature(callee, env) else {
        // An unresolvable named callee was already reported; anything else is
        // still worth checking for errors of its own.
        let checked_callee = match callee {
            Expr::Ident { name, span } => HirExpr::spanned(
                HirExprKind::Ident {
                    name: name.clone(),
                    use_kind: UseKind::Local,
                },
                Ty::unknown(),
                *span,
            ),
            other => check(other, None, return_ty, env),
        };
        return HirExpr::new(
            HirExprKind::Call {
                callee: Box::new(checked_callee),
                args: args
                    .iter()
                    .map(|arg| check(arg, None, return_ty, env))
                    .collect(),
            },
            Ty::unknown(),
        );
    };

    let callee_ty = Ty::new(
        TyKind::Func {
            params: params.clone(),
            ret: Box::new(call_return_ty.clone()),
        },
        false,
    );

    let (regular_params, trailing_signature) =
        split_trailing_param(&name, span, &params, trailing, env);

    let checked_args: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| check(arg, regular_params.get(index), return_ty, env))
        .collect();

    if checked_args.len() != regular_params.len() {
        env.error(
            format!(
                "function `{name}` expects {} arguments, got {}",
                regular_params.len(),
                checked_args.len()
            ),
            Some(span),
        );
    }
    for (index, (argument, expected)) in checked_args.iter().zip(regular_params).enumerate() {
        if !argument.ty.is_unknown() && &argument.ty != expected {
            env.error(
                format!(
                    "argument {} to `{name}` has type {}, expected {expected}",
                    index + 1,
                    argument.ty
                ),
                argument.span,
            );
        }
    }

    if let (Some(closure), Some((closure_params, closure_ret))) = (trailing, trailing_signature) {
        check_trailing_closure(closure, closure_params, closure_ret, env);
    }

    HirExpr::new(
        HirExprKind::Call {
            callee: Box::new(HirExpr::spanned(
                HirExprKind::Ident {
                    name,
                    use_kind: UseKind::Local,
                },
                callee_ty,
                span,
            )),
            args: checked_args,
        },
        call_return_ty,
    )
}

/// Split the formals into the positional ones and the trailing closure's signature.
fn split_trailing_param<'a>(
    name: &str,
    span: Span,
    params: &'a [Ty],
    trailing: Option<&Closure>,
    env: &mut Env<'_>,
) -> (&'a [Ty], Option<(&'a [Ty], &'a Ty)>) {
    if trailing.is_none() {
        return (params, None);
    }
    match params.split_last() {
        Some((
            Ty {
                kind: TyKind::Func { params, ret },
                ..
            },
            regular,
        )) => (regular, Some((params.as_slice(), ret.as_ref()))),
        _ => {
            env.error(
                format!(
                    "function `{name}` requires a function type as its last parameter \
                     when called with a trailing closure"
                ),
                Some(span),
            );
            (params, None)
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
        check_value_block_in_current_scope(&closure.body, Some(closure_ret), closure_ret, env);
    env.exit_scope();

    if !body_ty.is_unknown() && &body_ty != closure_ret {
        env.error(
            format!("trailing closure body has type {body_ty}, expected {closure_ret}"),
            None,
        );
    }
}

fn check_if(
    cond: &Expr,
    then_block: &Block,
    else_block: Option<&Block>,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let cond = check(cond, Some(&Ty::bool()), return_ty, env);
    if !cond.ty.is_unknown() && cond.ty != Ty::bool() {
        env.error(
            format!("if condition has type {}, expected bool", cond.ty),
            cond.span,
        );
    }
    let (then_block, then_ty) = check_value_block(then_block, expected, return_ty, env);
    let else_block = else_block.map(|block| check_value_block(block, expected, return_ty, env));

    let result_ty = match (&else_block, expected) {
        (Some((_, else_ty)), _) if *else_ty == then_ty => then_ty,
        (Some((_, else_ty)), Some(_)) => {
            if !else_ty.is_unknown() && !then_ty.is_unknown() {
                env.error(
                    format!("else branch has type {else_ty}, expected {then_ty}"),
                    None,
                );
            }
            then_ty
        }
        (Some(_), None) => Ty::unit(),
        (None, Some(_)) => {
            env.error(
                "value-producing if expression requires an else branch",
                None,
            );
            then_ty
        }
        (None, None) => Ty::unit(),
    };

    HirExpr::new(
        HirExprKind::If {
            cond: Box::new(cond),
            then_block,
            else_block: else_block.map(|(block, _)| block),
        },
        result_ty,
    )
}

fn check_value_block(
    block: &Block,
    expected: Option<&Ty>,
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
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> (HirBlock, Ty) {
    let Some((last, prefix)) = block.stmts.split_last() else {
        return (HirBlock { stmts: Vec::new() }, Ty::unit());
    };

    let mut stmts = prefix
        .iter()
        .map(|statement| stmt::check(statement, return_ty, env))
        .collect::<Vec<_>>();
    let result_ty = match last {
        Stmt::Expr(value) => {
            let value = check(value, expected, return_ty, env);
            let ty = value.ty.clone();
            stmts.push(HirStmt::Expr(value));
            ty
        }
        statement => {
            stmts.push(stmt::check(statement, return_ty, env));
            Ty::unit()
        }
    };
    (HirBlock { stmts }, result_ty)
}
