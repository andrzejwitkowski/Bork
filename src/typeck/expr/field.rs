use crate::ast::Expr;
use crate::hir::HirExprKind;
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::synthesize;

fn require_nullable_access(receiver_ty: TyId, safe: bool, span: Span, env: &mut Env<'_>) {
    let nullable = env.tcx.is_nullable(receiver_ty);
    if nullable && !safe {
        env.error(
            "field access on a nullable type requires `?.`",
            Some(span),
        );
    }
    if safe && !nullable {
        env.error("`?.` requires a nullable receiver", Some(span));
    }
}

pub(super) fn check_field(
    receiver: &Expr,
    name: &str,
    safe: bool,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let receiver = synthesize(receiver, return_ty, env);
    let inner = {
        let deref = env.tcx.ref_inner(receiver.ty).unwrap_or(receiver.ty);
        env.tcx.with_nullable(deref, false)
    };
    if (env.tcx.is_string(inner) || env.tcx.is_array(inner)) && name == "length" {
        require_nullable_access(receiver.ty, safe, span, env);
        let i32_ty = env.tcx.i32();
        let result_ty = env
            .tcx
            .with_nullable(i32_ty, safe && env.tcx.is_nullable(receiver.ty));
        return env.expr_at(
            HirExprKind::Field {
                receiver: Box::new(receiver.hir),
                name: name.to_string(),
                safe,
            },
            result_ty,
            span,
        );
    }

    let (named, class, through_managed_ref) = env.tcx.field_receiver(receiver.ty);

    if through_managed_ref && !safe {
        env.error(
            format!(
                "field access on managed Ref `{}` requires `if val`, `when`, or `?.`",
                env.show(receiver.ty)
            ),
            Some(span),
        );
    }
    if safe && !through_managed_ref && named && !env.tcx.is_unknown(receiver.ty) {
        env.error("?. on a class field requires Ref<T>", Some(span));
    }

    let field = class.and_then(|class| {
        env.class(class).and_then(|info| {
            info.fields
                .iter()
                .enumerate()
                .find(|(_, field)| field.name == name)
                .map(|(index, field)| (class, index, field.clone()))
        })
    });
    let Some((class, field_index, field)) = field else {
        if !env.tcx.is_unknown(receiver.ty) {
            env.error(
                format!("unknown field `{name}` on type {}", env.show(receiver.ty)),
                Some(span),
            );
        }
        let unknown = env.tcx.error();
        return env.expr_at(
            HirExprKind::Field {
                receiver: Box::new(receiver.hir),
                name: name.to_string(),
                safe,
            },
            unknown,
            span,
        );
    };

    let result_ty = if env.tcx.is_ref(receiver.ty) && env.tcx.record_def(field.ty).is_some() {
        env.tcx.ref_to(field.ty)
    } else if through_managed_ref && safe && !env.tcx.is_managed_ref(field.ty) {
        if env.tcx.supports_nullable(field.ty) {
            env.tcx.with_nullable(field.ty, true)
        } else {
            env.error(
                format!("safe field result `{}` cannot be represented as optional", env.show(field.ty)),
                Some(span),
            );
            env.tcx.error()
        }
    } else {
        field.ty
    };
    let class_name = env.tcx.class_name(class).to_string();
    env.expr_at(
        HirExprKind::ObjectField {
            receiver: Box::new(receiver.hir),
            class_name,
            field_index,
            name: name.to_string(),
            safe,
        },
        result_ty,
        span,
    )
}
