use crate::ast::{BinOp, Expr, UnaryOp};
use crate::hir::HirExprKind;
use crate::span::Span;
use crate::ty::{Predicate, TraitRef, TyData, TyId};

use super::super::env::{Checked, Env};
use super::super::solve;
use super::call::build_call;
use super::method::{borrow, trait_method_sig};
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
    let (lhs, rhs) = check_operands(op, lhs, rhs, expected, return_ty, env);
    let span = lhs.span.or(rhs.span);
    let (lhs, rhs) = match trait_operator(op, lhs, rhs, span, expr_span, env) {
        Ok(call) => return call,
        Err(operands) => operands,
    };
    let result_ty = binary_result_ty(op, lhs.ty, rhs.ty, span, env);
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

/// `a == b`, `a != b` and `a < b` (…) on a class, an enum or a type parameter: a call of
/// `Eq.eq` / `Ord.cmp` (#274), compared with 0 for `<`. Any other operands are given back for the
/// builtin rules.
fn trait_operator(
    op: &BinOp,
    lhs: Checked,
    rhs: Checked,
    span: Option<Span>,
    expr_span: Span,
    env: &mut Env<'_>,
) -> Result<Checked, (Checked, Checked)> {
    let (trait_name, method, symbol) = match op {
        BinOp::Eq => ("Eq", "eq", "=="),
        BinOp::Ne => ("Eq", "eq", "!="),
        BinOp::Lt => ("Ord", "cmp", "<"),
        BinOp::Le => ("Ord", "cmp", "<="),
        BinOp::Gt => ("Ord", "cmp", ">"),
        BinOp::Ge => ("Ord", "cmp", ">="),
        _ => return Err((lhs, rhs)),
    };
    // `a == b` on `&T` operands compares the targets.
    let settled = env.settle(lhs.ty);
    let self_ty = env.tcx.ref_inner(settled).unwrap_or(settled);
    let object = matches!(env.tcx.data(self_ty), TyData::Class(..) | TyData::Enum(..)) && !env.tcx.is_nullable(self_ty);
    if !(object || env.is_param(self_ty)) || !env.same(lhs.ty, rhs.ty) {
        return Err((lhs, rhs));
    }
    let Some(trait_id) = env.traits.keys().copied().find(|&id| env.defs.get(id).name == trait_name && env.traits[&id].methods.iter().any(|m| m.name == method)) else {
        return Err((lhs, rhs));
    };
    let span = span.unwrap_or(expr_span);
    let Some((name, params, ret)) = trait_method_sig(trait_id, method, self_ty, &[], span, env) else {
        let unknown = env.tcx.error();
        return Ok(env.expr_at(HirExprKind::None, unknown, expr_span));
    };
    let required = Predicate { self_ty, trait_ref: TraitRef { trait_id, args: Vec::new() } };
    solve::require(env, required, span, &format!("operator `{symbol}`"));
    let by_ref = |side: Checked, env: &mut Env<'_>| if env.tcx.is_ref(side.ty) { side.hir } else { borrow(side, env).hir };
    let args = vec![by_ref(lhs, env), by_ref(rhs, env)];
    let callee_ty = env.tcx.func(params, ret);
    let call = build_call(env, name, expr_span, callee_ty, args, ret, false);
    let kind = match op {
        BinOp::Eq => return Ok(call),
        BinOp::Ne => HirExprKind::Unary { op: UnaryOp::Not, expr: Box::new(call.hir) },
        _ => {
            let zero = env.expr_at(HirExprKind::Int { value: 0 }, ret, expr_span);
            HirExprKind::Binary { op: op.clone(), lhs: Box::new(call.hir), rhs: Box::new(zero.hir) }
        }
    };
    let bool_ty = env.tcx.bool();
    Ok(env.expr_at(kind, bool_ty, expr_span))
}

/// Checks both operands, letting each one's type guide the other where a literal could not
/// invent a type alone (`1 + x`, `None == opt`).
fn check_operands(
    op: &BinOp,
    lhs: &Expr,
    rhs: &Expr,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> (Checked, Checked) {
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

    if guide_from_rhs {
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
    }
}

/// The type of `lhs op rhs`, reporting operands the operator does not accept.
fn binary_result_ty(op: &BinOp, lhs_ty: TyId, rhs_ty: TyId, span: Option<Span>, env: &mut Env<'_>) -> TyId {
    let poisoned = env.tcx.is_unknown(lhs_ty) || env.tcx.is_unknown(rhs_ty);
    let (bool_ty, i32_ty) = (env.tcx.bool(), env.tcx.i32());

    match op {
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
            // shortcut: no structural equality for enums until traits give `==` a source, so say so here.
            let settled = env.settle(lhs_ty);
            if !poisoned
                && matches!(env.tcx.data(settled), crate::ty::TyData::Enum(..))
                && !env.tcx.is_nullable(settled)
            {
                env.error(format!("`==` is not supported for the enum {} yet; match on it with `when`", env.show(lhs_ty)), span);
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
    }
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
