//! AST type → interned `TyId` lowering.

use crate::ast::Type;
use crate::defs::DefTable;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::Prim;
use crate::span::Span;
use crate::ty::{ParamIdx, TyCtx, TyData, TyId};

/// Type parameters visible to a declaration, in index order.
pub(crate) type Scope = [(String, ParamIdx)];

pub(crate) fn lower_type(
    ty: &Type,
    defs: &DefTable,
    scope: &Scope,
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
            let elem = lower_type(elem, defs, scope, tcx, diagnostics, span);
            tcx.array(elem, *len)
        }
        Type::Named { name, args, nullable } => {
            lower_named(name, args, *nullable, defs, scope, tcx, diagnostics, span)
        }
        Type::ManagedRef { inner } => {
            let inner_ty = lower_type(inner, defs, scope, tcx, diagnostics, span);
            // A parameter may turn out to be a class; checked at instantiation.
            let is_class = matches!(tcx.data(inner_ty), TyData::Class(_) | TyData::Param(_));
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
            let elem_ty = lower_type(elem, defs, scope, tcx, diagnostics, span);
            let is_param = matches!(tcx.data(elem_ty), TyData::Param(_));
            if !tcx.is_unknown(elem_ty) && !is_param && !tcx.is_array_elem_supported(elem_ty) {
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
                .map(|param| lower_type(param, defs, scope, tcx, diagnostics, span))
                .collect();
            let ret = lower_type(ret, defs, scope, tcx, diagnostics, span);
            let func = tcx.func(params, ret);
            tcx.with_nullable(func, *nullable)
        }
        Type::Ref { inner, nullable } => lower_ref(inner, *nullable, defs, scope, tcx, diagnostics, span),
        Type::Primitive { name, nullable } => match Prim::from_name(name) {
            Some(prim) => {
                let ty = tcx.prim(prim);
                tcx.with_nullable(ty, *nullable)
            }
            None => tcx.error(),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_named(
    name: &str,
    args: &[Type],
    nullable: bool,
    defs: &DefTable,
    scope: &Scope,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> TyId {
    if let Some(&(_, idx)) = scope.iter().find(|(param, _)| param == name) {
        return lower_param(name, idx, args, nullable, tcx, diagnostics, span);
    }
    // Lowered for their own errors (unknown names); instantiation itself is #44.
    for arg in args {
        lower_type(arg, defs, scope, tcx, diagnostics, span);
    }
    if name == "String" {
        if !args.is_empty() {
            push_err(diagnostics, "`String` takes no type arguments", span);
            return tcx.error();
        }
        return tcx.string(nullable);
    }
    lower_class(name, args.len(), nullable, defs, tcx, diagnostics, span)
}

fn lower_param(
    name: &str,
    idx: ParamIdx,
    args: &[Type],
    nullable: bool,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> TyId {
    if !args.is_empty() {
        push_err(diagnostics, format!("type parameter `{name}` takes no type arguments"), span);
        return tcx.error();
    }
    let param = tcx.param(idx);
    tcx.with_nullable(param, nullable)
}

/// A class name with `given` type arguments; only non-generic classes can be used until #44.
fn lower_class(
    name: &str,
    given: usize,
    nullable: bool,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> TyId {
    let Some(class) = defs.lookup_class(name) else {
        push_err(diagnostics, format!("unknown named type `{name}`"), span);
        return tcx.error();
    };
    let arity = defs.get(class).arity;
    if given != arity {
        let message = match arity {
            0 => format!("`{name}` takes no type arguments"),
            1 => format!("`{name}` expects 1 type argument, got {given}"),
            _ => format!("`{name}` expects {arity} type arguments, got {given}"),
        };
        push_err(diagnostics, message, span);
        return tcx.error();
    }
    if arity > 0 {
        push_err(diagnostics, "instantiating generic types is not supported yet (#44)", span);
        return tcx.error();
    }
    let class = tcx.class(class);
    tcx.with_nullable(class, nullable)
}

fn lower_ref(
    inner: &Type,
    nullable: bool,
    defs: &DefTable,
    scope: &Scope,
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
    let inner_ty = lower_type(inner, defs, scope, tcx, diagnostics, span);
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
