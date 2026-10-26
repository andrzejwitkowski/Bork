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
    LowerCx { defs, scope, tcx, diagnostics, span }.lower(ty)
}

/// What lowering one written type reads and reports to: the definitions and type parameters in
/// scope, the type interner, the diagnostics, and the span every problem is reported at.
struct LowerCx<'a> {
    defs: &'a DefTable,
    scope: &'a Scope,
    tcx: &'a mut TyCtx,
    diagnostics: &'a mut Vec<Diagnostic>,
    span: Option<Span>,
}

impl LowerCx<'_> {
    fn error(&mut self, message: impl Into<String>) {
        push_err(self.diagnostics, message, self.span);
    }

    fn lower(&mut self, ty: &Type) -> TyId {
        match ty {
            Type::Array { nullable, .. } if *nullable => {
                self.error("nullable array types `[T]?` are not supported");
                self.tcx.error()
            }
            Type::Array { elem, len, .. } => {
                let elem = self.lower(elem);
                self.tcx.array(elem, *len)
            }
            Type::Named { name, args, nullable } => self.lower_named(name, args, *nullable),
            Type::ManagedRef { inner } => self.lower_managed_ref(inner),
            Type::Vec { elem } => self.lower_vec(elem),
            Type::Func { params, ret, nullable } => {
                let params = params.iter().map(|param| self.lower(param)).collect();
                let ret = self.lower(ret);
                let func = self.tcx.func(params, ret);
                self.nullable_if(func, *nullable)
            }
            Type::Ref { inner, nullable } => self.lower_ref(inner, *nullable),
            Type::Primitive { name, nullable } => match Prim::from_name(name) {
                Some(prim) => {
                    let ty = self.tcx.prim(prim);
                    self.nullable_if(ty, *nullable)
                }
                None => self.tcx.error(),
            },
        }
    }

    /// `ty?` when `nullable`, else `ty` untouched: an explicit `Option<T>` is already `T?`.
    fn nullable_if(&mut self, ty: TyId, nullable: bool) -> TyId {
        if nullable { self.tcx.with_nullable(ty, true) } else { ty }
    }

    fn lower_managed_ref(&mut self, inner: &Type) -> TyId {
        let inner_ty = self.lower(inner);
        // A parameter may turn out to be a class; checked at instantiation.
        let is_class =
            self.tcx.managed_ref_target_ok(inner_ty) || matches!(self.tcx.data(inner_ty), TyData::Param(_));
        if !self.tcx.is_unknown(inner_ty) && !is_class {
            let message = self.tcx.managed_ref_target_message(inner_ty);
            self.error(message);
        }
        if self.tcx.is_managed_ref(inner_ty) {
            self.error("nested `Ref<Ref<T>>` is not supported in the first implementation");
        }
        self.tcx.managed_ref(inner_ty)
    }

    fn lower_vec(&mut self, elem: &Type) -> TyId {
        let elem_ty = self.lower(elem);
        let is_param = matches!(self.tcx.data(elem_ty), TyData::Param(_));
        if !self.tcx.is_unknown(elem_ty) && !is_param && !self.tcx.is_array_elem_supported(elem_ty) {
            let message = self.tcx.vec_elem_message(elem_ty);
            self.error(message);
        }
        self.tcx.vec_of(elem_ty)
    }

    fn lower_named(&mut self, name: &str, args: &[Type], nullable: bool) -> TyId {
        if let Some(&(_, idx)) = self.scope.iter().find(|(param, _)| param == name) {
            return self.lower_param(name, idx, args, nullable);
        }
        let lowered: Vec<TyId> = args.iter().map(|arg| self.lower(arg)).collect();
        if name == "String" {
            if !args.is_empty() {
                self.error("`String` takes no type arguments");
                return self.tcx.error();
            }
            return self.tcx.string(nullable);
        }
        self.lower_class(name, lowered, nullable)
    }

    fn lower_param(&mut self, name: &str, idx: ParamIdx, args: &[Type], nullable: bool) -> TyId {
        if !args.is_empty() {
            self.error(format!("type parameter `{name}` takes no type arguments"));
            return self.tcx.error();
        }
        let param = self.tcx.param(idx);
        self.nullable_if(param, nullable)
    }

    /// A class or enum name applied to the lowered type arguments `args`.
    fn lower_class(&mut self, name: &str, args: Vec<TyId>, nullable: bool) -> TyId {
        let enum_def = self.defs.lookup_enum(name).filter(|_| self.defs.lookup_class(name).is_none());
        let Some(class) = self.defs.lookup_class(name).or(enum_def) else {
            self.error(format!("unknown named type `{name}`"));
            return self.tcx.error();
        };
        let arity = self.defs.get(class).arity;
        let given = args.len();
        if given != arity {
            let message = match arity {
                0 => format!("`{name}` takes no type arguments"),
                1 => format!("`{name}` expects 1 type argument, got {given}"),
                _ => format!("`{name}` expects {arity} type arguments, got {given}"),
            };
            self.error(message);
            return self.tcx.error();
        }
        // An argument that already has a diagnostic must not leave a `Class(_, [Error])` behind.
        if args.iter().any(|&arg| self.tcx.is_error(arg)) {
            return self.tcx.error();
        }
        let ty = if enum_def.is_some() { self.tcx.enum_ty(class, args) } else { self.tcx.class(class, args) };
        self.nullable_if(ty, nullable)
    }

    fn lower_ref(&mut self, inner: &Type, nullable: bool) -> TyId {
        if nullable {
            self.error("nullable reference types `&T?` are not supported");
        }
        let inner_ty = self.lower(inner);
        if !self.tcx.is_unknown(inner_ty) && self.tcx.is_copy(inner_ty) {
            let message = format!("cannot borrow Copy type `{}`", self.tcx.display(inner_ty));
            self.error(message);
        }
        self.tcx.ref_to(inner_ty)
    }
}

fn push_err(diagnostics: &mut Vec<Diagnostic>, message: impl Into<String>, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    });
}
