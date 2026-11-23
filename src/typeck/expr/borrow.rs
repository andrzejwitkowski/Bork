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
            let hint = if env.tcx.is_rw_ref(expected) && !env.tcx.is_rw_ref(ref_ty) { "; write `&rw` to borrow it editable" } else { "" };
            env.error(
                format!("borrow has type {}, expected {}{hint}", env.show(ref_ty), env.show(expected)),
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
    if rw {
        require_var(name, span, env);
    }
    env.tcx.ref_to_flag(operand, rw)
}

/// `&v[i]` / `&rw v[i]` on a `Vec`: a view of one element, typed `&T` / `&rw T`. Only elements with a borrow form
/// (not scalars, which `v[i]` already copies); `&rw` needs a mutable `v`. Locking `v` is `mir::borrows`' job.
pub(super) fn check_element_borrow(
    receiver: &crate::ast::Expr,
    index: &crate::ast::Expr,
    index_span: Span,
    span: Span,
    rw: bool,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let element = super::array::check_index(receiver, index, index_span, return_ty, env);
    let unknown = env.tcx.is_unknown(element.ty);
    let is_vec = matches!(&element.hir.kind, HirExprKind::Index { receiver, .. } if env.tcx.vec_elem(receiver.ty).is_some());
    if !is_vec && !unknown {
        env.error(format!("`{}` borrows a name; only a `Vec` element can be borrowed by index", if rw { "&rw" } else { "&" }), Some(span));
    } else if env.tcx.is_managed_ref(element.ty) {
        // shortcut: the MIR backend has no `&Ref<T>` local yet; lift once it does (#282 follow-up).
        env.error("cannot borrow a `Ref` element; `v[i]` gives you a `Ref` of its own", Some(span));
    } else if env.tcx.is_trivially_copy(element.ty) {
        env.error(format!("cannot borrow Copy type `{}`; read it with `v[i]`", env.show(element.ty)), Some(span));
    }
    if let (true, crate::ast::Expr::Ident { name, .. }) = (rw, receiver) {
        if env.binding(name).is_some_and(|b| !env.tcx.is_ref(b.ty)) {
            require_var(name, span, env);
        }
    }
    let ref_ty = if unknown { element.ty } else { env.tcx.ref_to_flag(element.ty, rw) };
    env.expr_at(HirExprKind::Unary { op: UnaryOp::Borrow { rw }, expr: Box::new(element.hir) }, ref_ty, span)
}

/// `&rw name` needs a `var` (the one site of this rule).
fn require_var(name: &str, span: Span, env: &mut Env<'_>) {
    if env.binding(name).is_some_and(|b| b.kind == BindingKind::Val) {
        env.error(format!("cannot borrow `val` `{name}` as `&rw`; declare it `var`"), Some(span));
    }
}
