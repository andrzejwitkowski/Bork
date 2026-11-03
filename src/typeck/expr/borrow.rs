use crate::ast::{BindingKind, UnaryOp};
use crate::hir::{HirExprKind, UseKind};
use crate::span::{AstId, Span};
use crate::ty::TyId;

use super::check_ident;
use super::super::env::{Checked, Env};

pub(super) fn check_borrow(
    name: &str,
    name_span: Span,
    name_id: AstId,
    span: Span,
    rw: bool,
    expected: Option<TyId>,
    env: &mut Env<'_>,
) -> Checked {
    let inner = check_ident(name, name_span, name_id, UseKind::Borrow, env);
    if env.tcx.is_trivially_copy(inner.ty) {
        env.error(format!("cannot borrow Copy type `{}`", env.show(inner.ty)), Some(span));
    }
    let ref_ty = borrow_type(name, inner.ty, rw, span, env);
    if let Some(expected) = expected {
        // `&T` and `&rw T` are different types, so the target only matches in the same form.
        let same_form = env.tcx.is_rw_ref(expected) == env.tcx.is_rw_ref(ref_ty);
        let target = env.tcx.ref_inner(expected);
        if !env.tcx.is_unknown(expected)
            && !env.same(expected, ref_ty)
            && !target.is_some_and(|target| same_form && env.same(target, inner.ty))
        {
            env.error(
                format!("borrow has type {}, expected {}", env.show(ref_ty), env.show(expected)),
                Some(span),
            );
        }
    }
    env.expr_at(
        HirExprKind::Unary {
            op: UnaryOp::Borrow { rw },
            expr: Box::new(inner.hir),
        },
        ref_ty,
        span,
    )
}

/// The type of `&name` / `&rw name` (RFC 0003 section 5). An owned value is borrowed in the written
/// form, and `&rw` needs a `var`. A borrow can be passed on as `&` (read-only, from either form),
/// but only an `&rw` borrow can be passed on as `&rw`.
fn borrow_type(name: &str, operand: TyId, rw: bool, span: Span, env: &mut Env<'_>) -> TyId {
    if let Some(target) = env.tcx.ref_inner(operand) {
        if rw && !env.tcx.is_rw_ref(operand) && !env.tcx.is_unknown(operand) {
            env.error(format!("cannot take `&rw` of the read-only borrow `{name}`"), Some(span));
        }
        return env.tcx.ref_to_flag(target, rw);
    }
    if rw && env.binding(name).is_some_and(|b| b.kind == BindingKind::Val) {
        env.error(format!("cannot borrow `val` `{name}` as `&rw`; declare it `var`"), Some(span));
    }
    env.tcx.ref_to_flag(operand, rw)
}
