use crate::ast::Expr;
use crate::hir::{HirExpr, HirExprKind, Ty, UseKind};
use crate::span::Span;

use super::super::env::Env;
use super::check;

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
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let expected_len = expected.and_then(|ty| ty.array_len());
    let elem_hint = expected
        .and_then(|ty| ty.array_elem())
        .cloned()
        .filter(|ty| ty.is_array_elem_supported());
    let mut inferred = elem_hint.clone();
    let mut checked = Vec::with_capacity(elements.len());
    for element in elements {
        let hir = check(element, inferred.as_ref(), return_ty, env);
        if hir.ty.is_unknown() {
            checked.push(hir);
            continue;
        }
        if !hir.ty.is_array_elem_supported() {
            env.error(
                format!("array element type `{}` is not supported", hir.ty),
                hir.span.or(Some(span)),
            );
        }
        if let Some(ref elem) = inferred {
            if !hir.ty.is_unknown() && &hir.ty != elem {
                env.error(
                    format!(
                        "array elements must have the same type, got {} and {}",
                        elem,
                        hir.ty
                    ),
                    hir.span.or(Some(span)),
                );
            }
        } else {
            inferred = Some(hir.ty.clone());
        }
        checked.push(hir);
    }
    let elem_ty = inferred.unwrap_or_else(|| {
        if elements.is_empty() {
            env.error(
                "empty array literal requires an explicit type, e.g. `val a: [i32; 0] = []`",
                Some(span),
            );
        }
        Ty::unknown()
    });
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
    HirExpr::spanned(
        HirExprKind::ArrayLit { elements: checked },
        Ty::array(elem_ty, len),
        span,
    )
}

pub(super) fn check_index(
    receiver: &Expr,
    index: &Expr,
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let receiver_hir = check(receiver, None, return_ty, env);
    let index_hir = check(index, Some(&Ty::i32()), return_ty, env);
    let elem_ty = if receiver_hir.ty.is_array() {
        receiver_hir
            .ty
            .array_elem()
            .cloned()
            .unwrap_or(Ty::unknown())
    } else if !receiver_hir.ty.is_unknown() {
        env.error(
            format!("indexing requires an array type, got {}", receiver_hir.ty),
            Some(span),
        );
        Ty::unknown()
    } else {
        Ty::unknown()
    };
    if !index_hir.ty.is_unknown() && index_hir.ty != Ty::i32() {
        env.error(
            format!("array index must be `i32`, got {}", index_hir.ty),
            index_hir.span.or(Some(span)),
        );
    }
    let use_kind = if elem_ty.is_unknown() {
        UseKind::Local
    } else if elem_ty.is_copy() {
        UseKind::Copy
    } else {
        UseKind::Shared
    };
    HirExpr::spanned(
        HirExprKind::Index {
            receiver: Box::new(receiver_hir),
            index: Box::new(index_hir),
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
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let receiver = check(receiver, None, return_ty, env);
    let lo = check(lo, Some(&Ty::i32()), return_ty, env);
    let hi = check(hi, Some(&Ty::i32()), return_ty, env);
    if !receiver.ty.is_array() && !receiver.ty.is_unknown() {
        env.error(
            format!("slicing requires an array type, got {}", receiver.ty),
            Some(span),
        );
    }
    for (bound, label) in [(&lo, "lo"), (&hi, "hi")] {
        if !bound.ty.is_unknown() && bound.ty != Ty::i32() {
            env.error(
                format!("slice bound `{label}` must be `i32`, got {}", bound.ty),
                bound.span.or(Some(span)),
            );
        }
    }
    let array_ty = match (receiver.ty.array_elem().cloned(), slice_bounds(&lo, &hi)) {
        (Some(elem), Some((lo_v, hi_v, len))) => {
            if let Some(max) = receiver.ty.array_len() {
                if hi_v > max as i32 {
                    env.error(
                        format!("slice [{lo_v}..{hi_v}] is out of bounds for `[{elem}; {max}]`"),
                        Some(span),
                    );
                }
            }
            Ty::array(elem, len)
        }
        (Some(_), None) => {
            let msg = if const_i32(&lo).is_some() && const_i32(&hi).is_some() {
                "slice lower bound must not exceed upper bound"
            } else {
                "slice bounds must be integer literals so the result type is `[T; N]`"
            };
            env.error(msg, Some(span));
            Ty::unknown()
        }
        _ => Ty::unknown(),
    };
    HirExpr::spanned(
        HirExprKind::Slice {
            receiver: Box::new(receiver),
            lo: Box::new(lo),
            hi: Box::new(hi),
        },
        array_ty,
        span,
    )
}
