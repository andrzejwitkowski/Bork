use crate::ast::{BinOp, Expr};
use crate::hir::HirExprKind;
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::{check, dispatch, synthesize};

pub(super) fn check_binary(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expr_span: Span,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let outer_numeric = match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            expected.filter(|&ty| env.is_numeric(ty))
        }
        _ => None,
    };

    // Guide Int/None from the peer when the left side cannot invent a type alone.
    let guide_from_rhs = matches!(lhs, Expr::Int(_) | Expr::None { .. })
        && outer_numeric.is_none()
        && *op != BinOp::RangeTo;

    let (lhs, rhs) = if guide_from_rhs {
        let rhs = synthesize(rhs, return_ty, env);
        let peer = (!env.tcx.is_unknown(rhs.ty)).then_some(rhs.ty);
        let lhs = dispatch(lhs, peer, return_ty, env);
        (lhs, rhs)
    } else {
        let lhs = dispatch(lhs, outer_numeric, return_ty, env);
        // Text needs no literal hint, and `s == &t` must not demand `rhs: String`.
        let hintless = *op == BinOp::RangeTo
            || (matches!(op, BinOp::Eq | BinOp::Ne) && env.tcx.is_text(lhs.ty));
        let peer = (!env.tcx.is_unknown(lhs.ty) && !hintless).then_some(lhs.ty);
        let rhs_expected = outer_numeric.or(peer);
        let rhs = dispatch(rhs, rhs_expected, return_ty, env);
        (lhs, rhs)
    };

    let span = lhs.span.or(rhs.span);
    let (lhs_ty, rhs_ty) = (lhs.ty, rhs.ty);
    let poisoned = env.tcx.is_unknown(lhs_ty) || env.tcx.is_unknown(rhs_ty);
    let (bool_ty, i32_ty) = (env.tcx.bool(), env.tcx.i32());

    let result_ty = match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            if poisoned {
                env.tcx.error()
            } else {
                if !env.same(lhs_ty, rhs_ty) || !env.is_numeric(lhs_ty) {
                    env.error(
                        format!(
                            "arithmetic operands must have the same numeric type, got \
                             {} and {}{}",
                            env.show(lhs_ty),
                            env.show(rhs_ty),
                            env.bounds_hint(&[lhs_ty, rhs_ty])
                        ),
                        span,
                    );
                }
                lhs_ty
            }
        }
        BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => {
            if !poisoned && !(env.same(lhs_ty, rhs_ty) && env.is_numeric(lhs_ty)) {
                env.error(
                    format!(
                        "ordered comparison operands must have the same non-nullable \
                         numeric type, got {} and {}{}",
                        env.show(lhs_ty),
                        env.show(rhs_ty),
                        env.bounds_hint(&[lhs_ty, rhs_ty])
                    ),
                    span,
                );
            }
            bool_ty
        }
        BinOp::Eq | BinOp::Ne => {
            let opaque = env.is_param(lhs_ty) || env.is_param(rhs_ty);
            if !poisoned
                && (opaque
                    || (!env.same(lhs_ty, rhs_ty) && !(env.tcx.is_text(lhs_ty) && env.tcx.is_text(rhs_ty))))
            {
                env.error(
                    format!(
                        "equality operands must have the same type, got {} and {}{}",
                        env.show(lhs_ty),
                        env.show(rhs_ty),
                        env.bounds_hint(&[lhs_ty, rhs_ty])
                    ),
                    span,
                );
            }
            bool_ty
        }
        BinOp::And | BinOp::Or => {
            if !poisoned && (!env.same(lhs_ty, bool_ty) || !env.same(rhs_ty, bool_ty)) {
                env.error(
                    format!(
                        "logical operands must have type bool, got {} and {}",
                        env.show(lhs_ty),
                        env.show(rhs_ty)
                    ),
                    span,
                );
            }
            bool_ty
        }
        BinOp::RangeTo => {
            if !poisoned && (!env.same(lhs_ty, i32_ty) || !env.same(rhs_ty, i32_ty)) {
                env.error(
                    format!(
                        "range bounds must have type i32, got {} and {}",
                        env.show(lhs_ty),
                        env.show(rhs_ty)
                    ),
                    span,
                );
            }
            env.tcx.range(i32_ty)
        }
        BinOp::Elvis => unreachable!("Elvis is handled before other binary operators"),
    };

    env.expr_at(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs.hir),
            rhs: Box::new(rhs.hir),
        },
        result_ty,
        expr_span,
    )
}

pub(super) fn check_elvis(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expr_span: Span,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let expected_lhs = expected
        .filter(|&ty| env.tcx.supports_nullable(ty))
        .map(|ty| env.tcx.with_nullable(ty, true));
    let lhs = dispatch(lhs, expected_lhs, return_ty, env);
    if env.tcx.is_managed_ref(lhs.ty) {
        let result_ty = lhs.ty;
        let rhs = check(rhs, result_ty, return_ty, env);
        env.mismatch("right operand of `?:`", rhs.ty, result_ty, Some(expr_span));
        return env.expr_at(
            HirExprKind::Binary {
                op: op.clone(),
                lhs: Box::new(lhs.hir),
                rhs: Box::new(rhs.hir),
            },
            result_ty,
            expr_span,
        );
    }

    let result_ty = if env.tcx.supports_nullable(lhs.ty) {
        if !env.tcx.is_nullable(lhs.ty) {
            env.error(
                format!("left operand of `?:` must be nullable, got {}", env.show(lhs.ty)),
                Some(expr_span),
            );
        }
        env.tcx.with_nullable(lhs.ty, false)
    } else {
        if !env.tcx.is_unknown(lhs.ty) {
            env.error(
                format!("left operand of `?:` cannot have type {}", env.show(lhs.ty)),
                Some(expr_span),
            );
        }
        env.tcx.error()
    };

    let rhs = check(rhs, result_ty, return_ty, env);
    env.mismatch("right operand of `?:`", rhs.ty, result_ty, Some(expr_span));

    env.expr_at(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs.hir),
            rhs: Box::new(rhs.hir),
        },
        result_ty,
        expr_span,
    )
}
