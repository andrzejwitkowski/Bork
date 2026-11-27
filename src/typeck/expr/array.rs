use crate::ast::Expr;
use crate::hir::{HirExpr, HirExprKind, UseKind};
use crate::span::{AstId, Span};
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::call::CallSite;
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
            env.error(env.tcx.array_elem_message(hir.ty), hir.span.or(Some(span)));
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

/// The prelude's Bork `Vec` (`Vec` until the switch, #285): an `opaque` class, so a user class of that name never matches.
pub(in crate::typeck) fn is_vec_class(ty: TyId, env: &Env<'_>) -> bool {
    env.tcx.is_vec_class(ty)
}

pub(super) fn check_index(
    receiver: &Expr,
    index: &Expr,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    check_index_as(receiver, index, span, return_ty, false, env)
}

/// `check_index`; with `keep_index` the Bork `Vec` stays an `Index` node (for `&v[i]`) instead of becoming `v.get(i)`.
pub(super) fn check_index_as(
    receiver: &Expr,
    index: &Expr,
    span: Span,
    return_ty: TyId,
    keep_index: bool,
    env: &mut Env<'_>,
) -> Checked {
    // Peek at a plain name's type first: synthesizing it as a value would count as a use (a move, inside a loop).
    let peeked = match receiver {
        Expr::Ident { name, .. } => env.binding(name).map(|b| b.ty),
        _ => None,
    };
    if !keep_index && peeked.is_some_and(|ty| is_vec_class(ty, env)) {
        // `v[i]` on the Bork `Vec` is `v.get(i)`.
        let get = Expr::Field { receiver: Box::new(receiver.clone()), name: "get".to_string(), safe: false, span, id: AstId::DUMMY };
        return dispatch(&Expr::call(get, Vec::new(), vec![index.clone()], None, span), None, return_ty, env);
    }
    let diagnostics_before = env.diagnostics.len();
    let receiver_hir = synthesize(receiver, return_ty, env);
    if !keep_index && is_vec_class(receiver_hir.ty, env) {
        // `v[i]` on the Bork `Vec` is `v.get(i)`.
        let site = CallSite { span, args: std::slice::from_ref(index), return_ty };
        if super::method::is_place(receiver) {
            // The method call checks the place receiver again, so what this first pass reported is dropped to report it once.
            env.diagnostics.truncate(diagnostics_before);
            let get = Expr::Field { receiver: Box::new(receiver.clone()), name: "get".to_string(), safe: false, span, id: AstId::DUMMY };
            return dispatch(&Expr::call(get, Vec::new(), vec![index.clone()], None, span), None, return_ty, env);
        }
        // A receiver that is not a place (`make()[i]`) is borrowed through a temporary; it is checked once, here.
        return super::method::check_method_call_on(receiver, "get", false, AstId::DUMMY, site, Some(receiver_hir), env);
    }
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
    } else if env.tcx.is_trivially_copy(elem_ty) {
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
