use crate::ast::{Closure, Expr};
use crate::hir::{HirExprKind, HirExpr};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, ClassInfo, EnumInfo, Env};
use super::call::{fresh_params, CallSite};
use super::{check, synthesize};

/// A declared field type with the owner's type parameters replaced by `type_args`.
fn instantiate_field(ty: TyId, type_args: &[TyId], owner: &str, span: Span, env: &mut Env<'_>) -> TyId {
    if type_args.is_empty() {
        return ty;
    }
    crate::ty::subst(env.tcx, ty, type_args).unwrap_or_else(|_| {
        env.error(format!("type arguments of `{owner}` would make a nullable of a nullable"), Some(span));
        env.tcx.error()
    })
}

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
        let field_ty = instantiate_field(field.ty, &type_args, &class.name, span, env);
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

/// The enum `receiver` names when it is a bare identifier that no local binding shadows, so
/// `E.V` is a variant path rather than a field access.
pub(super) fn variant_path(receiver: &Expr, safe: bool, env: &Env<'_>) -> Option<EnumInfo> {
    let Expr::Ident { name, .. } = receiver else { return None };
    if safe || env.binding(name).is_some() {
        return None;
    }
    env.enum_named(name).cloned()
}

/// `E.V(args)` or a bare `E.V`: every payload field takes exactly one argument.
pub(super) fn check_variant(
    e: EnumInfo,
    variant: &str,
    site: CallSite<'_>,
    expected: Option<TyId>,
    env: &mut Env<'_>,
) -> Checked {
    let CallSite { span, args, trailing, return_ty } = site;
    let Some(info) = e.variants.iter().find(|v| v.name == variant) else {
        env.error(format!("enum `{}` has no variant `{variant}`", e.name), Some(span));
        for arg in args {
            let _ = synthesize(arg, return_ty, env);
        }
        let unknown = env.tcx.error();
        return env.expr_at(HirExprKind::None, unknown, span);
    };
    if trailing.is_some() {
        env.error("enum variants do not accept trailing closures", Some(span));
    }
    if args.len() != info.fields.len() {
        let hint = if args.is_empty() && !info.fields.is_empty() { format!(" (write `{}.{variant}(..)`)", e.name) } else { String::new() };
        env.error(
            format!(
                "variant `{}.{variant}` takes {} argument(s), got {}{hint}",
                e.name,
                info.fields.len(),
                args.len()
            ),
            Some(span),
        );
        for arg in args {
            let _ = synthesize(arg, return_ty, env);
        }
        let unknown = env.tcx.error();
        return env.expr_at(HirExprKind::None, unknown, span);
    }

    let type_args = fresh_params(&e.generics, &e.name, span, env);
    let ty = env.tcx.enum_ty(e.id, type_args.clone());
    if let Some(expected) = expected {
        env.same(ty, expected);
    }

    let mut values: Vec<HirExpr> = Vec::with_capacity(info.fields.len());
    for (field, arg) in info.fields.iter().zip(args) {
        let field_ty = instantiate_field(field.ty, &type_args, &e.name, span, env);
        let value = check(arg, field_ty, return_ty, env);
        let what = format!("field `{}` of variant `{}.{variant}`", field.name, e.name);
        env.mismatch(&what, value.ty, field_ty, value.span.or(Some(field.span)));
        values.push(value.hir);
    }
    env.expr_at(
        HirExprKind::EnumConstruct { enum_name: e.name, variant: variant.to_string(), fields: values },
        ty,
        span,
    )
}
