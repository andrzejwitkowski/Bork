use crate::ast::UnaryOp;
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
    let ref_ty = if env.tcx.is_ref(inner.ty) {
        inner.ty
    } else {
        env.tcx.ref_to_flag(inner.ty, rw)
    };
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
