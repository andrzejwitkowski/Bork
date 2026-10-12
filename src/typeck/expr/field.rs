use crate::ast::Expr;
use crate::hir::{HirExpr, HirExprKind, Ty};
use crate::span::Span;

use super::super::env::Env;
use super::check;

fn require_nullable_access(receiver_ty: &Ty, safe: bool, span: Span, env: &mut Env<'_>) {
    if receiver_ty.is_nullable() && !safe {
        env.error(
            "field access on a nullable type requires `?.`",
            Some(span),
        );
    }
    if safe && !receiver_ty.is_nullable() {
        env.error("`?.` requires a nullable receiver", Some(span));
    }
}

pub(super) fn check_field(
    receiver: &Expr,
    name: &str,
    safe: bool,
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let receiver = check(receiver, None, return_ty, env);
    let inner = receiver.ty.deref_ty().with_nullable(false);
    if (inner.is_string() || inner.is_array()) && name == "length" {
        require_nullable_access(&receiver.ty, safe, span, env);
        let result_ty = Ty::i32().with_nullable(safe && receiver.ty.is_nullable());
        return HirExpr::spanned(
            HirExprKind::Field {
                receiver: Box::new(receiver),
                name: name.to_string(),
                safe,
            },
            result_ty,
            span,
        );
    }

    let (class_name, through_managed_ref) = match &receiver.ty.kind {
        crate::hir::TyKind::Named(class_name) => (Some(class_name.clone()), false),
        crate::hir::TyKind::Ref(inner) => match &inner.kind {
            crate::hir::TyKind::Named(class_name) => (Some(class_name.clone()), false),
            _ => (None, false),
        },
        crate::hir::TyKind::ManagedRef(inner) => match &inner.kind {
            crate::hir::TyKind::Named(class_name) => (Some(class_name.clone()), true),
            _ => (None, true),
        },
        _ => (None, false),
    };

    if through_managed_ref && !safe {
        env.error(
            format!(
                "field access on managed Ref `{}` requires `if val`, `when`, or `?.`",
                receiver.ty
            ),
            Some(span),
        );
    }
    if safe && !through_managed_ref && class_name.is_some() && !receiver.ty.is_unknown() {
        env.error("?. on a class field requires Ref<T>", Some(span));
    }

    let field = class_name.and_then(|class_name| {
        env.class(&class_name).and_then(|class| {
            class
                .fields
                .iter()
                .enumerate()
                .find(|(_, field)| field.name == name)
                .map(|(index, field)| (class_name, index, field.clone()))
        })
    });
    let Some((class_name, field_index, field)) = field else {
        if !receiver.ty.is_unknown() {
            env.error(
                format!("unknown field `{name}` on type {}", receiver.ty),
                Some(span),
            );
        }
        return HirExpr::spanned(
            HirExprKind::Field {
                receiver: Box::new(receiver),
                name: name.to_string(),
                safe,
            },
            Ty::unknown(),
            span,
        );
    };

    let result_ty = if receiver.ty.is_ref() && field.ty.record_name().is_some() {
        Ty::new(crate::hir::TyKind::Ref(Box::new(field.ty.clone())), false)
    } else if through_managed_ref && safe && !field.ty.is_managed_ref() {
        if field.ty.supports_nullable() {
            field.ty.with_nullable(true)
        } else {
            env.error(
                format!("safe field result `{}` cannot be represented as optional", field.ty),
                Some(span),
            );
            Ty::unknown()
        }
    } else {
        field.ty.clone()
    };
    HirExpr::spanned(
        HirExprKind::ObjectField {
            receiver: Box::new(receiver),
            class_name,
            field_index,
            name: name.to_string(),
            safe,
        },
        result_ty,
        span,
    )
}
