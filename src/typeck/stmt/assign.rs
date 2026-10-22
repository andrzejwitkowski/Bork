use crate::ast::{AssignTarget, BindingKind, Expr};
use crate::hir::{HirAssignTarget, HirStmt};
use crate::span::{AstId, Span};
use crate::ty::TyId;

use super::super::env::Env;
use super::super::expr;

pub(super) fn check(target: &AssignTarget, value: &Expr, return_ty: TyId, env: &mut Env<'_>) -> HirStmt {
    match target {
        AssignTarget::Name { name, name_span, id } => {
            if name == "this" {
                env.error("cannot assign to `this`", Some(*name_span));
                return HirStmt::Expr(expr::check(value, None, return_ty, env).hir);
            }
            if env.resolve_bare_field(name, *id).is_some() {
                let this = Expr::Ident {
                    name: "this".to_string(),
                    span: *name_span,
                    id: *id,
                };
                return check_receiver_field(&this, name, *name_span, *id, value, return_ty, env);
            }
            let bound = assign_binding(env, name, *name_span, *id, false);
            let value = expr::check(value, bound, return_ty, env);
            reject_type_mismatch(
                env,
                &format!("assignment to `{name}`"),
                value.ty,
                bound,
                Some(*name_span),
            );
            HirStmt::Assign {
                target: HirAssignTarget::Name { name: name.clone() },
                value: value.hir,
            }
        }
        AssignTarget::Index {
            name,
            name_span,
            id,
            index,
            ..
        } => check_index_assign(name, *name_span, *id, index, value, return_ty, env),
        AssignTarget::Field {
            receiver,
            name,
            span,
            id,
        } => check_receiver_field(receiver, name, *span, *id, value, return_ty, env),
    }
}

fn check_index_assign(
    name: &str,
    name_span: Span,
    id: AstId,
    index: &Expr,
    value: &Expr,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let bound = assign_binding(env, name, name_span, id, true);
    let elem = bound.and_then(|ty| env.tcx.array_elem(ty).or_else(|| env.tcx.vec_elem(ty)));
    if bound.is_some_and(|ty| {
        !env.tcx.is_unknown(ty) && env.tcx.array_elem(ty).is_none() && env.tcx.vec_elem(ty).is_none()
    }) {
        env.error(
            format!("cannot index-assign `{name}`: expected an array"),
            Some(name_span),
        );
    }
    let i32_ty = env.tcx.i32();
    let index = expr::check(index, Some(i32_ty), return_ty, env);
    if !env.tcx.is_unknown(index.ty) && index.ty != i32_ty {
        env.error(
            format!("array index has type {}, expected i32", env.show(index.ty)),
            index.span.or(Some(name_span)),
        );
    }
    let value = expr::check(value, elem, return_ty, env);
    reject_type_mismatch(
        env,
        &format!("assignment to `{name}` element"),
        value.ty,
        elem,
        Some(name_span),
    );
    HirStmt::Assign {
        target: HirAssignTarget::Index {
            name: name.to_string(),
            index: index.hir,
        },
        value: value.hir,
    }
}

fn check_receiver_field(
    receiver: &Expr,
    field: &str,
    field_span: Span,
    field_id: AstId,
    value: &Expr,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let receiver = expr::check(receiver, None, return_ty, env);
    if env.tcx.is_managed_ref(receiver.ty) {
        env.error(
            "managed Ref must be unwrapped before assigning one of its fields",
            Some(field_span),
        );
        let value = expr::check(value, None, return_ty, env);
        return HirStmt::Expr(value.hir);
    }
    let resolved = env.tcx.record_def(receiver.ty).and_then(|class| {
        env.class(class).and_then(|class| {
            class
                .fields
                .iter()
                .enumerate()
                .find(|(_, info)| info.name == field)
                .map(|(field_index, info)| (field_index, info.ty))
        })
    });
    let Some((field_index, field_ty)) = resolved else {
        if !env.tcx.is_unknown(receiver.ty) {
            env.error(
                format!("unknown writable field `{field}` on type {}", env.show(receiver.ty)),
                Some(field_span),
            );
        }
        let value = expr::check(value, None, return_ty, env);
        return HirStmt::Expr(value.hir);
    };
    let legacy = env.tcx.to_ty(field_ty);
    env.node_tys.insert(field_id.numbered(), legacy);
    let value = expr::check(value, Some(field_ty), return_ty, env);
    reject_type_mismatch(
        env,
        &format!("assignment to field `{field}`"),
        value.ty,
        Some(field_ty),
        Some(field_span),
    );
    HirStmt::Assign {
        target: HirAssignTarget::Field {
            receiver: receiver.hir,
            field: field.to_string(),
            field_index,
        },
        value: value.hir,
    }
}

fn assign_binding(
    env: &mut Env<'_>,
    name: &str,
    span: Span,
    id: AstId,
    index_assign: bool,
) -> Option<TyId> {
    let binding = env.binding(name).cloned();
    if binding.is_none() && env.constants.contains_key(name) && env.implicit_field(name).is_none() {
        env.error(format!("cannot assign to constant `{name}`"), Some(span));
        return None;
    }
    if binding.is_none() {
        let field = index_assign
            && env
                .reject_bare_field(name, span, id, &format!("cannot index-assign field `{name}` through `this`"))
                .is_some();
        if !field {
            env.error(format!("unknown binding `{name}`"), Some(span));
        }
    }
    if let Some(b) = binding.as_ref() {
        if b.kind == BindingKind::Val {
            if env.tcx.is_ref(b.ty) {
                if index_assign {
                    if !b.explicit_ref {
                        env.error(
                            format!("cannot index-assign through inferred view `{name}`"),
                            Some(span),
                        );
                    } else if !env.tcx.ref_inner(b.ty).is_some_and(|inner| env.tcx.is_array(inner) || env.tcx.is_vec(inner)) {
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
    got: TyId,
    expected: Option<TyId>,
    span: Option<Span>,
) {
    let Some(expected) = expected else {
        return;
    };
    if !env.tcx.is_unknown(got) && !env.tcx.is_unknown(expected) && got != expected {
        env.error(
            format!("{what} has type {}, expected {}", env.show(got), env.show(expected)),
            span,
        );
    }
}
