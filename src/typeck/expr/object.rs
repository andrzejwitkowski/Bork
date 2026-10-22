use crate::ast::{Closure, Expr};
use crate::hir::{HirExprKind, HirExpr};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, ClassInfo, Env};
use super::check;

pub(super) fn check_construct(
    class: ClassInfo,
    span: Span,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    if trailing.is_some() {
        env.error("record constructors do not accept trailing closures", Some(span));
    }
    if args.len() > class.fields.len() {
        env.error(
            format!(
                "constructor `{}` accepts at most {} arguments, got {}",
                class.name,
                class.fields.len(),
                args.len()
            ),
            Some(span),
        );
    }

    let mut values: Vec<HirExpr> = Vec::with_capacity(class.fields.len());
    for (index, field) in class.fields.iter().enumerate() {
        if let Some(arg) = args.get(index) {
            let value = check(arg, Some(field.ty), return_ty, env);
            if !env.tcx.is_unknown(value.ty) && value.ty != field.ty {
                env.error(
                    format!(
                        "constructor field `{}` has type {}, expected {}",
                        field.name,
                        env.show(value.ty),
                        env.show(field.ty)
                    ),
                    value.span.or(Some(field.span)),
                );
            }
            values.push(value.hir);
        } else if env.tcx.is_managed_ref(field.ty) {
            values.push(env.expr_at(HirExprKind::None, field.ty, field.span).hir);
        } else {
            env.error(
                format!("missing constructor argument for field `{}`", field.name),
                Some(field.span),
            );
            let unknown = env.tcx.error();
            values.push(env.expr_at(HirExprKind::None, unknown, field.span).hir);
        }
    }
    for extra in args.iter().skip(class.fields.len()) {
        let _ = check(extra, None, return_ty, env);
    }

    let ty = env.tcx.class(class.id);
    env.expr_at(
        HirExprKind::ObjectConstruct {
            class_name: class.name,
            fields: values,
        },
        ty,
        span,
    )
}
