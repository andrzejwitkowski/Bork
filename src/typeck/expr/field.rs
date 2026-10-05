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
    let result_ty = if let Some(struct_name) = receiver.ty.struct_name() {
        check_struct_field(&receiver.ty, struct_name, name, safe, span, env)
    } else if receiver.ty.uses_arena_storage() && name == "length" {
        require_nullable_access(&receiver.ty, safe, span, env);
        Ty::i32().with_nullable(safe && receiver.ty.is_nullable())
    } else {
        if !receiver.ty.is_unknown() {
            env.error(
                format!("unknown field `{name}` on type {}", receiver.ty),
                Some(span),
            );
        }
        Ty::unknown()
    };
    HirExpr::spanned(
        HirExprKind::Field {
            receiver: Box::new(receiver),
            name: name.to_string(),
            safe,
        },
        result_ty,
        span,
    )
}

fn check_struct_field(
    receiver_ty: &Ty,
    struct_name: &str,
    name: &str,
    safe: bool,
    span: Span,
    env: &mut Env<'_>,
) -> Ty {
    require_nullable_access(receiver_ty, safe, span, env);
    let Some(field_ty) = env.struct_def(struct_name).and_then(|def| def.field_ty(name)) else {
        if !receiver_ty.is_unknown() {
            env.error(
                format!("unknown field `{name}` on type {receiver_ty}"),
                Some(span),
            );
        }
        return Ty::unknown();
    };
    if safe && receiver_ty.is_nullable() {
        field_ty.with_nullable(true)
    } else {
        field_ty.clone()
    }
}

pub(super) fn check_struct_new(
    name: &str,
    args: &[Expr],
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let Some(def) = env.struct_def(name).cloned() else {
        env.error(format!("unknown struct `{name}`"), Some(span));
        return HirExpr::spanned(
            HirExprKind::StructNew {
                name: name.to_string(),
                args: args
                    .iter()
                    .map(|arg| check(arg, None, return_ty, env))
                    .collect(),
            },
            Ty::unknown(),
            span,
        );
    };
    if args.len() != def.fields.len() {
        env.error(
            format!(
                "`new {name}` expects {} argument(s), got {}",
                def.fields.len(),
                args.len()
            ),
            Some(span),
        );
    }
    let checked_args = args
        .iter()
        .enumerate()
        .map(|(i, arg)| {
            let expected = def.fields.get(i).map(|f| &f.ty);
            let value = check(arg, expected, return_ty, env);
            if let Some(expected) = expected {
                if !value.ty.is_unknown() && !expected.is_unknown() && &value.ty != expected {
                    env.error(
                        format!(
                            "argument {} of `new {name}` has type {}, expected {expected}",
                            i + 1,
                            value.ty
                        ),
                        value.span.or(Some(span)),
                    );
                }
            }
            value
        })
        .collect();
    HirExpr::spanned(
        HirExprKind::StructNew {
            name: name.to_string(),
            args: checked_args,
        },
        Ty::struct_ty(name),
        span,
    )
}
