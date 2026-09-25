use crate::ast::UnaryOp;
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind, UseKind};
use crate::span::Span;

use super::check_ident;
use super::super::env::Env;

pub(super) fn check_borrow(
    name: &str,
    name_span: Span,
    span: Span,
    expected: Option<&Ty>,
    env: &mut Env<'_>,
) -> HirExpr {
    let inner = check_ident(name, name_span, UseKind::Borrow, env);
    if inner.ty.is_copy() {
        env.error(format!("cannot borrow Copy type `{}`", inner.ty), Some(span));
    }
    let ref_ty = Ty::new(TyKind::Ref(Box::new(inner.ty.clone())), false);
    if let Some(expected) = expected {
        if !expected.is_unknown() && expected != &ref_ty {
            if expected.ref_inner() != Some(&inner.ty) {
                env.error(
                    format!("borrow has type {ref_ty}, expected {expected}"),
                    Some(span),
                );
            }
        }
    }
    HirExpr::spanned(
        HirExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(inner),
        },
        ref_ty,
        span,
    )
}
