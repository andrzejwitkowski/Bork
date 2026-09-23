use std::collections::HashMap;

use crate::ast::BindingKind;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{RegionId, Ty};
use crate::span::Span;

#[derive(Clone)]
pub(super) struct Binding {
    pub(super) kind: BindingKind,
    pub(super) ty: Ty,
}

#[derive(Clone)]
pub(super) struct FunSig {
    pub(super) params: Vec<Ty>,
    pub(super) return_ty: Ty,
}

pub(super) struct Env<'a> {
    scopes: Vec<HashMap<String, Binding>>,
    pub(super) fun_sigs: &'a HashMap<String, FunSig>,
    pub(super) diagnostics: Vec<Diagnostic>,
    next_region: RegionId,
}

impl<'a> Env<'a> {
    pub(super) fn new(fun_sigs: &'a HashMap<String, FunSig>, next_region: RegionId) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            fun_sigs,
            diagnostics: Vec::new(),
            next_region,
        }
    }

    pub(super) fn bind(&mut self, name: String, kind: BindingKind, ty: Ty) {
        self.scopes
            .last_mut()
            .expect("type environment always has a scope")
            .insert(name, Binding { kind, ty });
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

    pub(super) fn alloc_region(&mut self) -> RegionId {
        let region = self.next_region;
        self.next_region += 1;
        region
    }

    pub(super) fn next_region(&self) -> RegionId {
        self.next_region
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
