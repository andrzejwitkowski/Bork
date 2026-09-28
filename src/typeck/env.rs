use std::collections::HashMap;

use crate::ast::BindingKind;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::Ty;
use crate::span::Span;

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

pub(super) struct Env<'a> {
    scopes: Vec<HashMap<String, Binding>>,
    pub(super) fun_sigs: &'a HashMap<String, FunSig>,
    pub(super) diagnostics: Vec<Diagnostic>,
    /// Types of `var`/`val` bindings in the order typeck checks them.
    pub(super) decl_tys: Vec<Ty>,
    pub(super) loop_depth: usize,
}

impl<'a> Env<'a> {
    pub(super) fn new(fun_sigs: &'a HashMap<String, FunSig>) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            fun_sigs,
            diagnostics: Vec::new(),
            decl_tys: Vec::new(),
            loop_depth: 0,
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
            .insert(name, Binding {
                kind,
                ty,
                explicit_ref,
            });
    }

    pub(super) fn binding(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    pub(super) fn fun_sig(&self, name: &str) -> Option<&FunSig> {
        self.fun_sigs.get(name)
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
