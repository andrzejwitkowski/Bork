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
            name,
            name_span,
            field,
            field_span,
        } => check_field_assign(name, *name_span, field, *field_span, value, return_ty, env),
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

fn check_field_assign(
    name: &str,
    name_span: Span,
    field: &str,
    field_span: Span,
    value: &Expr,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
    let bound = env.binding(name).cloned();
    if bound.is_none() {
        env.error(format!("unknown binding `{name}`"), Some(name_span));
    }
    let struct_field = bound.as_ref().and_then(|b| {
        b.ty.struct_name()
            .and_then(|struct_name| env.struct_def(struct_name))
            .and_then(|def| def.field(field))
            .cloned()
    });
    if bound
        .as_ref()
        .is_some_and(|b| !b.ty.is_unknown() && !b.ty.is_struct())
    {
        env.error(
            format!("cannot field-assign `{name}`: expected a struct"),
            Some(name_span),
        );
    } else if bound.is_some() && struct_field.is_none() {
        env.error(
            format!("unknown field `{field}` on `{name}`"),
            Some(field_span),
        );
    } else if struct_field
        .as_ref()
        .is_some_and(|f| f.kind == BindingKind::Val)
    {
        env.error(
            format!("cannot assign to immutable field `{field}`"),
            Some(field_span),
        );
    }
    let field_ty = struct_field.as_ref().map(|f| f.ty.clone());
    let value = expr::check(value, field_ty.as_ref(), return_ty, env);
    reject_type_mismatch(
        env,
        &format!("assignment to `{name}.{field}`"),
        &value.ty,
        field_ty.as_ref(),
        Some(field_span),
    );
    HirStmt::Assign {
        target: HirAssignTarget::Field {
            name: name.to_string(),
            field: field.to_string(),
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
