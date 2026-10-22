use crate::ast::Expr;
use crate::hir::{HirExpr, HirExprKind, UseKind};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::{check, dispatch, synthesize};

fn const_i32(expr: &HirExpr) -> Option<i32> {
    match &expr.kind {
        HirExprKind::Int { value } => {
            if *value < i32::MIN as i64 || *value > i32::MAX as i64 {
                None
            } else {
                Some(*value as i32)
            }
        }
        _ => None,
    }
}

fn slice_bounds(lo: &HirExpr, hi: &HirExpr) -> Option<(i32, i32, u32)> {
    let lo = const_i32(lo)?;
    let hi = const_i32(hi)?;
    (hi >= lo && lo >= 0).then_some((lo, hi, (hi - lo) as u32))
}

pub(super) fn check_array_lit(
    elements: &[Expr],
    span: Span,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let expected_len = expected.and_then(|ty| env.tcx.array_len(ty));
    let elem_hint = expected
        .and_then(|ty| env.tcx.array_elem(ty))
        .filter(|&ty| env.tcx.is_array_elem_supported(ty));
    let mut inferred = elem_hint;
    let mut checked = Vec::with_capacity(elements.len());
    for element in elements {
        let hir = dispatch(element, inferred, return_ty, env);
        if env.tcx.is_unknown(hir.ty) {
            checked.push(hir);
            continue;
        }
        if !env.tcx.is_array_elem_supported(hir.ty) {
            env.error(
                format!("array element type `{}` is not supported", env.show(hir.ty)),
                hir.span.or(Some(span)),
            );
        }
        if let Some(elem) = inferred {
            if !env.tcx.is_unknown(hir.ty) && !env.same(hir.ty, elem) {
                env.error(
                    format!(
                        "array elements must have the same type, got {} and {}",
                        env.show(elem),
                        env.show(hir.ty)
                    ),
                    hir.span.or(Some(span)),
                );
            }
        } else {
            inferred = Some(hir.ty);
        }
        checked.push(hir);
    }
    let elem_ty = match inferred {
        Some(ty) => ty,
        None => {
            if elements.is_empty() {
                env.error(
                    "empty array literal requires an explicit type, e.g. `val a: [i32; 0] = []`",
                    Some(span),
                );
            }
            env.tcx.error()
        }
    };
    let count = checked.len();
    if let Some(n) = expected_len {
        if n as usize != count {
            env.error(
                format!("array literal has {count} elements, expected {n}"),
                Some(span),
            );
        }
    }
    if count > u32::MAX as usize {
        env.error("array literal is too large", Some(span));
    }
    let len = count as u32;
    let ty = env.tcx.array(elem_ty, len);
    env.expr_at(
        HirExprKind::ArrayLit { elements: checked.into_iter().map(|e| e.hir).collect() },
        ty,
        span,
    )
}

pub(super) fn check_index(
    receiver: &Expr,
    index: &Expr,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let receiver_hir = synthesize(receiver, return_ty, env);
    let i32_ty = env.tcx.i32();
    let index_hir = check(index, i32_ty, return_ty, env);
    let elem_ty = if env.tcx.is_text(receiver_hir.ty) {
        env.tcx.prim(crate::hir::Prim::U8)
    } else if let Some(elem) = env
        .tcx
        .array_elem(receiver_hir.ty)
        .or_else(|| env.tcx.vec_elem(receiver_hir.ty))
    {
        elem
    } else {
        if !env.tcx.is_unknown(receiver_hir.ty) {
            env.error(
                format!("indexing requires an array type, got {}", env.show(receiver_hir.ty)),
                Some(span),
            );
        }
        env.tcx.error()
    };
    if !env.tcx.is_unknown(index_hir.ty) && !env.same(index_hir.ty, i32_ty) {
        env.error(
            format!("array index must be `i32`, got {}", env.show(index_hir.ty)),
            index_hir.span.or(Some(span)),
        );
    }
    let use_kind = if env.tcx.is_unknown(elem_ty) {
        UseKind::Local
    } else if env.tcx.is_copy(elem_ty) {
        UseKind::Copy
    } else {
        UseKind::Shared
    };
    env.expr_at(
        HirExprKind::Index {
            receiver: Box::new(receiver_hir.hir),
            index: Box::new(index_hir.hir),
            use_kind,
        },
        elem_ty,
        span,
    )
}

pub(super) fn check_slice(
    receiver: &Expr,
    lo: &Expr,
    hi: &Expr,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let receiver = synthesize(receiver, return_ty, env);
    let i32_ty = env.tcx.i32();
    let lo = check(lo, i32_ty, return_ty, env);
    let hi = check(hi, i32_ty, return_ty, env);
    if !env.tcx.is_array(receiver.ty) && !env.tcx.is_unknown(receiver.ty) {
        env.error(
            format!("slicing requires an array type, got {}", env.show(receiver.ty)),
            Some(span),
        );
    }
    for (bound, label) in [(&lo, "lo"), (&hi, "hi")] {
        if !env.tcx.is_unknown(bound.ty) && !env.same(bound.ty, i32_ty) {
            env.error(
                format!("slice bound `{label}` must be `i32`, got {}", env.show(bound.ty)),
                bound.span.or(Some(span)),
            );
        }
    }
    let array_ty = match (env.tcx.array_elem(receiver.ty), slice_bounds(&lo, &hi)) {
        (Some(elem), Some((lo_v, hi_v, len))) => {
            if let Some(max) = env.tcx.array_len(receiver.ty) {
                if hi_v > max as i32 {
                    env.error(
                        format!("slice [{lo_v}..{hi_v}] is out of bounds for `[{}; {max}]`", env.show(elem)),
                        Some(span),
                    );
                }
            }
            env.tcx.array(elem, len)
        }
        (Some(_), None) => {
            let msg = if const_i32(&lo).is_some() && const_i32(&hi).is_some() {
                "slice lower bound must not exceed upper bound"
            } else {
                "slice bounds must be integer literals so the result type is `[T; N]`"
            };
            env.error(msg, Some(span));
            env.tcx.error()
        }
        _ => env.tcx.error(),
    };
    env.expr_at(
        HirExprKind::Slice {
            receiver: Box::new(receiver.hir),
            lo: Box::new(lo.hir),
            hi: Box::new(hi.hir),
        },
        array_ty,
        span,
    )
}
