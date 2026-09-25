use crate::ast::{BinOp, Expr};
use crate::hir::{HirExpr, HirExprKind, Ty};
use crate::span::Span;

use super::super::env::Env;
use super::check;

pub(super) fn check_binary(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expr_span: Span,
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
    let guide_from_rhs = matches!(lhs, Expr::Int(_) | Expr::None { .. })
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
        BinOp::And | BinOp::Or => {
            if !poisoned && (*lhs_ty != Ty::bool() || *rhs_ty != Ty::bool()) {
                env.error(
                    format!(
                        "logical operands must have type bool, got {lhs_ty} and {rhs_ty}"
                    ),
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

    HirExpr::spanned(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
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
                Some(expr_span),
            );
        }
        lhs.ty.with_nullable(false)
    } else {
        if !lhs.ty.is_unknown() {
            env.error(
                format!("left operand of `?:` cannot have type {}", lhs.ty),
                Some(expr_span),
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
            Some(expr_span),
        );
    }

    HirExpr::spanned(
        HirExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        result_ty,
        expr_span,
    )
}
