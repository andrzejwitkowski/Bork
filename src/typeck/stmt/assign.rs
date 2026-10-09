use crate::ast::{AssignTarget, BindingKind, Expr};
use crate::hir::{HirAssignTarget, HirStmt, Ty};
use crate::span::Span;

use super::super::env::Env;
use super::super::expr;

pub(super) fn check(target: &AssignTarget, value: &Expr, return_ty: &Ty, env: &mut Env<'_>) -> HirStmt {
    match target {
        AssignTarget::Name { name, name_span } => {
            let bound = assign_binding(env, name, *name_span, false);
            let value = expr::check(value, bound.as_ref(), return_ty, env);
            reject_type_mismatch(
                env,
                &format!("assignment to `{name}`"),
                &value.ty,
                bound.as_ref(),
                Some(*name_span),
            );
            HirStmt::Assign {
                target: HirAssignTarget::Name { name: name.clone() },
                value,
            }
        }
        AssignTarget::Index {
            name,
            name_span,
            index,
            ..
        } => check_index_assign(name, *name_span, index, value, return_ty, env),
        AssignTarget::Field {
            receiver,
            name,
            span,
        } => check_receiver_field(receiver, name, *span, value, return_ty, env),
    }
}

fn check_index_assign(
    name: &str,
    name_span: Span,
    index: &Expr,
    value: &Expr,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
    let bound = assign_binding(env, name, name_span, true);
    let elem = bound.as_ref().and_then(|ty| ty.array_elem().cloned());
    if bound
        .as_ref()
        .is_some_and(|ty| !ty.is_unknown() && ty.array_elem().is_none())
    {
        env.error(
            format!("cannot index-assign `{name}`: expected an array"),
            Some(name_span),
        );
    }
    let index = expr::check(index, Some(&Ty::i32()), return_ty, env);
    if !index.ty.is_unknown() && index.ty != Ty::i32() {
        env.error(
            format!("array index has type {}, expected i32", index.ty),
            index.span.or(Some(name_span)),
        );
    }
    let value = expr::check(value, elem.as_ref(), return_ty, env);
    reject_type_mismatch(
        env,
        &format!("assignment to `{name}` element"),
        &value.ty,
        elem.as_ref(),
        Some(name_span),
    );
    HirStmt::Assign {
        target: HirAssignTarget::Index {
            name: name.to_string(),
            index,
        },
        value,
    }
}

fn check_receiver_field(
    receiver: &Expr,
    field: &str,
    field_span: Span,
    value: &Expr,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
    let receiver = expr::check(receiver, None, return_ty, env);
    if matches!(receiver.ty.kind, crate::hir::TyKind::ManagedRef(_)) {
        env.error(
            "managed Ref must be unwrapped before assigning one of its fields",
            Some(field_span),
        );
        let value = expr::check(value, None, return_ty, env);
        return HirStmt::Expr(value);
    }
    let resolved = receiver.ty.record_name().and_then(|class_name| {
        env.class(class_name).and_then(|class| {
            class
                .fields
                .iter()
                .enumerate()
                .find(|(_, info)| info.name == field)
                .map(|(field_index, info)| (field_index, info.ty.clone()))
        })
    });
    let Some((field_index, field_ty)) = resolved else {
        if !receiver.ty.is_unknown() {
            env.error(
                format!("unknown writable field `{field}` on type {}", receiver.ty),
                Some(field_span),
            );
        }
        let value = expr::check(value, None, return_ty, env);
        return HirStmt::Expr(value);
    };
    env.decl_tys.insert(field_span, field_ty.clone());
    let value = expr::check(value, Some(&field_ty), return_ty, env);
    reject_type_mismatch(
        env,
        &format!("assignment to field `{field}`"),
        &value.ty,
        Some(&field_ty),
        Some(field_span),
    );
    HirStmt::Assign {
        target: HirAssignTarget::Field {
            receiver,
            field: field.to_string(),
            field_index,
        },
        value,
    }
}

fn assign_binding(
    env: &mut Env<'_>,
    name: &str,
    span: Span,
    index_assign: bool,
) -> Option<Ty> {
    let binding = env.binding(name).cloned();
    if binding.is_none() {
        env.error(format!("unknown binding `{name}`"), Some(span));
    }
    if let Some(b) = binding.as_ref() {
        if b.kind == BindingKind::Val {
            if b.ty.is_ref() {
                if index_assign {
                    if !b.explicit_ref {
                        env.error(
                            format!("cannot index-assign through inferred view `{name}`"),
                            Some(span),
                        );
                    } else if !b.ty.ref_inner().is_some_and(|inner| inner.is_array()) {
                        env.error(
                            format!("cannot index-assign through borrow `{name}`"),
                            Some(span),
                        );
                    }
                } else {
                    env.error(
                        format!("cannot assign to borrow binding `{name}`"),
                        Some(span),
                    );
                }
            } else {
                env.error(
                    format!("cannot assign to immutable `val` binding `{name}`"),
                    Some(span),
                );
            }
        }
    }
    binding.map(|b| b.ty)
}

fn reject_type_mismatch(
    env: &mut Env<'_>,
    what: &str,
    got: &Ty,
    expected: Option<&Ty>,
    span: Option<Span>,
) {
    let Some(expected) = expected else {
        return;
    };
    if !got.is_unknown() && !expected.is_unknown() && got != expected {
        env.error(
            format!("{what} has type {got}, expected {expected}"),
            span,
        );
    }
}
