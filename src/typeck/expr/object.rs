use crate::ast::{Closure, Expr};
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind};
use crate::span::Span;

use super::super::env::{ClassInfo, Env};
use super::check;

pub(super) fn check_construct(
    class: ClassInfo,
    span: Span,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
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

    let mut values = Vec::with_capacity(class.fields.len());
    for (index, field) in class.fields.iter().enumerate() {
        if let Some(arg) = args.get(index) {
            let value = check(arg, Some(&field.ty), return_ty, env);
            if !value.ty.is_unknown() && value.ty != field.ty {
                env.error(
                    format!(
                        "constructor field `{}` has type {}, expected {}",
                        field.name, value.ty, field.ty
                    ),
                    value.span.or(Some(field.span)),
                );
            }
            values.push(value);
        } else if field.ty.is_managed_ref() {
            values.push(HirExpr::spanned(HirExprKind::None, field.ty.clone(), field.span));
        } else {
            env.error(
                format!("missing constructor argument for field `{}`", field.name),
                Some(field.span),
            );
            values.push(HirExpr::spanned(HirExprKind::None, Ty::unknown(), field.span));
        }
    }
    for extra in args.iter().skip(class.fields.len()) {
        let _ = check(extra, None, return_ty, env);
    }

    HirExpr::spanned(
        HirExprKind::ObjectConstruct {
            class_name: class.name.clone(),
            fields: values,
        },
        Ty::new(TyKind::Named(class.name), false),
        span,
    )
}
