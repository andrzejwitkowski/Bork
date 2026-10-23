use crate::ast::{Closure, Expr};
use crate::hir::{HirExprKind, HirExpr};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, ClassInfo, Env};
use super::call::fresh_params;
use super::{check, synthesize};

pub(super) fn check_construct(
    class: ClassInfo,
    span: Span,
    args: &[Expr],
    trailing: Option<&Closure>,
    expected: Option<TyId>,
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

    // A generic class is instantiated with fresh variables that the expected type and the field
    // arguments fix; the expected type is only a hint, so the caller reports a mismatch.
    let type_args = fresh_params(&class.generics, &class.name, span, env);
    let ty = env.tcx.class(class.id, type_args.clone());
    if let Some(expected) = expected {
        env.same(ty, expected);
    }

    let mut values: Vec<HirExpr> = Vec::with_capacity(class.fields.len());
    for (index, field) in class.fields.iter().enumerate() {
        let field_ty = crate::ty::subst(env.tcx, field.ty, &type_args).unwrap_or_else(|_| env.tcx.error());
        if let Some(arg) = args.get(index) {
            let value = check(arg, field_ty, return_ty, env);
            let what = format!("constructor field `{}`", field.name);
            env.mismatch(&what, value.ty, field_ty, value.span.or(Some(field.span)));
            values.push(value.hir);
        } else if env.tcx.is_managed_ref(field_ty) {
            values.push(env.expr_at(HirExprKind::None, field_ty, field.span).hir);
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
        let _ = synthesize(extra, return_ty, env);
    }

    env.expr_at(
        HirExprKind::ObjectConstruct {
            class_name: class.name,
            fields: values,
        },
        ty,
        span,
    )
}
