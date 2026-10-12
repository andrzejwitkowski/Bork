use std::collections::HashMap;

use crate::ast::BindingKind;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::Ty;
use crate::span::Span;

use super::MemberResolutions;

#[derive(Clone)]
pub(super) struct Binding {
    pub(super) kind: BindingKind,
    pub(super) ty: Ty,
    /// Written `&T` in a type annotation or a `&T` parameter (not inferred `val b = &a`).
    pub(super) explicit_ref: bool,
}

#[derive(Clone)]
pub(crate) struct FunSig {
    pub(crate) params: Vec<Ty>,
    pub(crate) return_ty: Ty,
}

#[derive(Clone, Debug)]
pub(super) struct ClassFieldInfo {
    pub(super) name: String,
    pub(super) ty: Ty,
    pub(super) span: Span,
}

#[derive(Clone, Debug)]
pub(super) struct ClassInfo {
    pub(super) name: String,
    pub(super) fields: Vec<ClassFieldInfo>,
    /// Method name -> display signature (`fun C.m(a: i32): i32`). Signatures are in `fun_sigs` as `C.m`.
    pub(super) methods: HashMap<String, String>,
}

pub(super) type ClassTable = HashMap<String, ClassInfo>;

pub(super) struct Env<'a> {
    scopes: Vec<HashMap<String, Binding>>,
    pub(super) fun_sigs: &'a HashMap<String, FunSig>,
    pub(super) classes: &'a ClassTable,
    pub(super) diagnostics: Vec<Diagnostic>,
    /// Declaration and field assignment types keyed by source span.
    pub(super) span_tys: HashMap<Span, Ty>,
    pub(super) loop_depth: usize,
    /// Class whose method body is being checked; `None` in top-level functions.
    pub(super) self_class: Option<String>,
    /// Member names resolved by typeck, for sema and LSP.
    pub(super) members: MemberResolutions,
}

impl<'a> Env<'a> {
    pub(super) fn new(
        fun_sigs: &'a HashMap<String, FunSig>,
        classes: &'a ClassTable,
    ) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            fun_sigs,
            classes,
            diagnostics: Vec::new(),
            span_tys: HashMap::new(),
            loop_depth: 0,
            self_class: None,
            members: MemberResolutions::new(),
        }
    }

    pub(super) fn bind(
        &mut self,
        name: String,
        kind: BindingKind,
        ty: Ty,
        explicit_ref: bool,
    ) {
        self.scopes
            .last_mut()
            .expect("type environment always has a scope")
            .insert(
                name,
                Binding {
                    kind,
                    ty,
                    explicit_ref,
                },
            );
    }

    pub(super) fn binding(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Field of the enclosing class that a bare `name` refers to. Locals and params shadow fields.
    pub(super) fn implicit_field(&self, name: &str) -> Option<(String, usize, Ty)> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.classes.get(self.self_class.as_deref()?)?;
        let index = class.fields.iter().position(|field| field.name == name)?;
        Some((class.name.clone(), index, class.fields[index].ty.clone()))
    }

    /// Bare field in a position that needs a binding (`move x`, `&x`, `x[i] = v`, capture list):
    /// records the member and reports `message`. Returns the field type, or None if `name` is no field.
    pub(super) fn reject_bare_field(&mut self, name: &str, span: Span, message: &str) -> Option<Ty> {
        let (class, index, ty) = self.implicit_field(name)?;
        self.members.insert(
            span,
            super::MemberRef::Field {
                class,
                name: name.to_string(),
                index,
                ty: ty.clone(),
            },
        );
        self.error(message, Some(span));
        Some(ty)
    }

    /// Explicit `move(..)` capture lists name bindings; a bare field there is an error.
    pub(super) fn reject_field_captures(&mut self, captures: &[crate::span::SpannedName]) {
        for capture in captures {
            let message = format!(
                "cannot capture field `{}`; fields are reached through the `this` borrow",
                capture.name
            );
            self.reject_bare_field(&capture.name, capture.span, &message);
        }
    }

    /// Class of the enclosing method when a bare call `name(..)` is one of its methods.
    pub(super) fn implicit_method(&self, name: &str) -> Option<String> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.self_class.as_ref()?;
        self.fun_sigs
            .contains_key(&format!("{class}.{name}"))
            .then(|| class.clone())
    }

    pub(super) fn fun_sig(&self, name: &str) -> Option<&FunSig> {
        self.fun_sigs.get(name)
    }

    pub(super) fn class(&self, name: &str) -> Option<&ClassInfo> {
        self.classes.get(name)
    }

    pub(super) fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    pub(super) fn error(&mut self, message: impl Into<String>, span: Option<Span>) {
        self.diagnostics.push(Diagnostic {
            phase: Phase::Type,
            severity: Severity::Error,
            message: message.into(),
            span,
        });
    }
}
