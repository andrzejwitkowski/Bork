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
    expected: Option<TyId>,
    env: &mut Env<'_>,
) -> Checked {
    let inner = check_ident(name, name_span, name_id, UseKind::Borrow, env);
    if env.tcx.is_copy(inner.ty) {
        env.error(format!("cannot borrow Copy type `{}`", env.show(inner.ty)), Some(span));
    }
    let ref_ty = if env.tcx.is_ref(inner.ty) {
        inner.ty
    } else {
        env.tcx.ref_to(inner.ty)
    };
    if let Some(expected) = expected {
        if !env.tcx.is_unknown(expected) && expected != ref_ty {
            if env.tcx.ref_inner(expected) != Some(inner.ty) {
                env.error(
                    format!("borrow has type {}, expected {}", env.show(ref_ty), env.show(expected)),
                    Some(span),
                );
            }
        }
    }
    env.expr_at(
        HirExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(inner.hir),
        },
        ref_ty,
        span,
    )
}
