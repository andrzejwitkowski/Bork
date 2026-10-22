//! AST type → interned `TyId` lowering.

use crate::ast::Type;
use crate::defs::DefTable;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::Prim;
use crate::span::Span;
use crate::ty::{TyCtx, TyData, TyId};

pub(crate) fn lower_type(
    ty: &Type,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> TyId {
    match ty {
        Type::Array { nullable, .. } if *nullable => {
            push_err(diagnostics, "nullable array types `[T]?` are not supported", span);
            tcx.error()
        }
        Type::Array { elem, len, .. } => {
            let elem = lower_type(elem, defs, tcx, diagnostics, span);
            tcx.array(elem, *len)
        }
        Type::Named { name, nullable } if name == "String" => tcx.string(*nullable),
        Type::Named { name, nullable } if defs.lookup_class(name).is_some() => {
            let class = tcx.class(defs.lookup_class(name).expect("checked by the guard"));
            tcx.with_nullable(class, *nullable)
        }
        Type::Named { name, .. } => {
            push_err(diagnostics, format!("unknown named type `{name}`"), span);
            tcx.error()
        }
        Type::ManagedRef { inner } => {
            let inner_ty = lower_type(inner, defs, tcx, diagnostics, span);
            let is_class = matches!(tcx.data(inner_ty), TyData::Class(_));
            if !tcx.is_unknown(inner_ty) && !is_class {
                push_err(
                    diagnostics,
                    format!("managed Ref target must be a non-nullable class, got `{}`", tcx.display(inner_ty)),
                    span,
                );
            }
            if tcx.is_managed_ref(inner_ty) {
                push_err(
                    diagnostics,
                    "nested `Ref<Ref<T>>` is not supported in the first implementation",
                    span,
                );
            }
            tcx.managed_ref(inner_ty)
        }
        Type::Vec { elem } => {
            let elem_ty = lower_type(elem, defs, tcx, diagnostics, span);
            if !tcx.is_unknown(elem_ty) && !tcx.is_array_elem_supported(elem_ty) {
                push_err(
                    diagnostics,
                    format!(
                        "`Vec` element must be a Copy type, a non-null `String` or a `Ref<T>`, got `{}`",
                        tcx.display(elem_ty)
                    ),
                    span,
                );
            }
            tcx.vec_of(elem_ty)
        }
        Type::Func {
            params,
            ret,
            nullable,
        } => {
            let params = params
                .iter()
                .map(|param| lower_type(param, defs, tcx, diagnostics, span))
                .collect();
            let ret = lower_type(ret, defs, tcx, diagnostics, span);
            let func = tcx.func(params, ret);
            tcx.with_nullable(func, *nullable)
        }
        Type::Ref { inner, nullable } => lower_ref(inner, *nullable, defs, tcx, diagnostics, span),
        Type::Primitive { name, nullable } => match Prim::from_name(name) {
            Some(prim) => {
                let ty = tcx.prim(prim);
                tcx.with_nullable(ty, *nullable)
            }
            None => tcx.error(),
        },
    }
}

fn lower_ref(
    inner: &Type,
    nullable: bool,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> TyId {
    if nullable {
        push_err(
            diagnostics,
            "nullable reference types `&T?` are not supported",
            span,
        );
    }
    let inner_ty = lower_type(inner, defs, tcx, diagnostics, span);
    if !tcx.is_unknown(inner_ty) && tcx.is_copy(inner_ty) {
        push_err(
            diagnostics,
            format!("cannot borrow Copy type `{}`", tcx.display(inner_ty)),
            span,
        );
    }
    tcx.ref_to(inner_ty)
}

fn push_err(diagnostics: &mut Vec<Diagnostic>, message: impl Into<String>, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    });
}
