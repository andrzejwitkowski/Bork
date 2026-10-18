//! AST type → HIR `Ty` lowering.

use crate::ast::Type;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{Ty, TyKind};
use crate::span::Span;

use super::env::ClassTable;

pub(crate) fn lower_type(
    ty: &Type,
    classes: &ClassTable,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> Ty {
    match ty {
        Type::Array { nullable, .. } if *nullable => {
            push_err(diagnostics, "nullable array types `[T]?` are not supported", span);
            Ty::unknown()
        }
        Type::Array { elem, len, .. } => Ty::new(
            TyKind::Array {
                elem: Box::new(lower_type(elem, classes, diagnostics, span)),
                len: *len,
            },
            false,
        ),
        Type::Named { name, nullable } if name == "String" => Ty::string(*nullable),
        Type::Named { name, nullable } if classes.contains_key(name) => {
            Ty::new(TyKind::Named(name.clone()), *nullable)
        }
        Type::Named { name, .. } => {
            push_err(diagnostics, format!("unknown named type `{name}`"), span);
            Ty::unknown()
        }
        Type::ManagedRef { inner } => {
            let inner_ty = lower_type(inner, classes, diagnostics, span);
            let is_class = matches!(&inner_ty.kind, TyKind::Named(name) if classes.contains_key(name))
                && !inner_ty.nullable;
            if !inner_ty.is_unknown() && !is_class {
                push_err(
                    diagnostics,
                    format!("managed Ref target must be a non-nullable class, got `{inner_ty}`"),
                    span,
                );
            }
            if inner_ty.is_managed_ref() {
                push_err(
                    diagnostics,
                    "nested `Ref<Ref<T>>` is not supported in the first implementation",
                    span,
                );
            }
            Ty::new(TyKind::ManagedRef(Box::new(inner_ty)), false)
        }
        Type::Vec { elem } => {
            let elem_ty = lower_type(elem, classes, diagnostics, span);
            if !elem_ty.is_unknown() && !elem_ty.is_array_elem_supported() {
                push_err(
                    diagnostics,
                    format!(
                        "`Vec` element must be a Copy type, a non-null `String` or a `Ref<T>`, got `{elem_ty}`"
                    ),
                    span,
                );
            }
            Ty::new(TyKind::Vec(Box::new(elem_ty)), false)
        }
        Type::Func {
            params,
            ret,
            nullable,
        } => Ty::new(
            TyKind::Func {
                params: params
                    .iter()
                    .map(|param| lower_type(param, classes, diagnostics, span))
                    .collect(),
                ret: Box::new(lower_type(ret, classes, diagnostics, span)),
            },
            *nullable,
        ),
        Type::Ref { inner, nullable } => {
            lower_ref(inner, *nullable, classes, diagnostics, span)
        }
        _ => Ty::from_ast(ty),
    }
}

fn lower_ref(
    inner: &Type,
    nullable: bool,
    classes: &ClassTable,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> Ty {
    if nullable {
        push_err(
            diagnostics,
            "nullable reference types `&T?` are not supported",
            span,
        );
    }
    let inner_ty = lower_type(inner, classes, diagnostics, span);
    if !inner_ty.is_unknown() && inner_ty.is_copy() {
        push_err(
            diagnostics,
            format!("cannot borrow Copy type `{inner_ty}`"),
            span,
        );
    }
    Ty::new(TyKind::Ref(Box::new(inner_ty)), false)
}

fn push_err(diagnostics: &mut Vec<Diagnostic>, message: impl Into<String>, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    });
}
