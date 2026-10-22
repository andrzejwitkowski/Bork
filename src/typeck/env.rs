use std::collections::HashMap;

use crate::ast::BindingKind;
use crate::constants::ConstantTable;
use crate::defs::{DefId, DefTable};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirExpr, HirExprKind, Ty};
use crate::span::{AstId, Span};
use crate::ty::{FnSigs, TyCtx, TyId, TyPrinter};

use super::{MemberResolutions, NodeTypes};

/// A checked expression: the legacy HIR node (`hir.ty` is the `hir::Ty` view that later passes
/// read) together with its interned type, which is what typeck itself compares and queries.
pub(super) struct Checked {
    pub(super) hir: HirExpr,
    pub(super) ty: TyId,
}

impl std::ops::Deref for Checked {
    type Target = HirExpr;

    fn deref(&self) -> &HirExpr {
        &self.hir
    }
}

#[derive(Clone)]
pub(super) struct Binding {
    pub(super) kind: BindingKind,
    pub(super) ty: TyId,
    /// Written `&T` in a type annotation or a `&T` parameter (not inferred `val b = &a`).
    pub(super) explicit_ref: bool,
}

/// Name-keyed signature in the legacy `hir::Ty` form, for `builtins::signatures`.
pub(crate) type FunSig = crate::ty::FnSig<Ty>;

#[derive(Clone, Debug)]
pub(super) struct ClassFieldInfo {
    pub(super) name: String,
    pub(super) ty: TyId,
    pub(super) span: Span,
}

#[derive(Clone, Debug)]
pub(super) struct ClassInfo {
    pub(super) id: DefId,
    /// Display name only; identity is `id`.
    pub(super) name: String,
    pub(super) fields: Vec<ClassFieldInfo>,
    /// Method definition -> display signature (`fun C.m(a: i32): i32`). Signatures are in `fun_sigs`.
    pub(super) methods: HashMap<DefId, String>,
}

pub(super) type ClassTable = HashMap<DefId, ClassInfo>;

pub(super) struct Env<'a> {
    scopes: Vec<HashMap<String, Binding>>,
    pub(super) defs: &'a DefTable,
    pub(super) tcx: &'a mut TyCtx,
    pub(super) fun_sigs: &'a FnSigs,
    pub(super) classes: &'a ClassTable,
    pub(super) constants: &'a ConstantTable,
    pub(super) diagnostics: Vec<Diagnostic>,
    /// Declaration and field assignment types keyed by source span.
    pub(super) node_tys: NodeTypes,
    pub(super) loop_depth: usize,
    /// Class whose method body is being checked; `None` in top-level functions.
    pub(super) self_class: Option<DefId>,
    /// Member names resolved by typeck, for sema and LSP.
    pub(super) members: MemberResolutions,
}

impl<'a> Env<'a> {
    pub(super) fn new(
        defs: &'a DefTable,
        tcx: &'a mut TyCtx,
        fun_sigs: &'a FnSigs,
        classes: &'a ClassTable,
        constants: &'a ConstantTable,
    ) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            defs,
            tcx,
            fun_sigs,
            classes,
            constants,
            diagnostics: Vec::new(),
            node_tys: NodeTypes::new(),
            loop_depth: 0,
            self_class: None,
            members: MemberResolutions::new(),
        }
    }

    pub(super) fn bind(
        &mut self,
        name: String,
        kind: BindingKind,
        ty: TyId,
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
    pub(super) fn implicit_field(&self, name: &str) -> Option<(DefId, usize, TyId)> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.classes.get(&self.self_class?)?;
        let index = class.fields.iter().position(|field| field.name == name)?;
        Some((class.id, index, class.fields[index].ty))
    }

    /// Records a bare `name` at `span` as a field read through `this`. Returns its type, or None
    /// if `name` is not a field of the enclosing class.
    pub(super) fn resolve_bare_field(&mut self, name: &str, id: AstId) -> Option<TyId> {
        let (class, index, ty) = self.implicit_field(name)?;
        self.members.insert(
            id.numbered(),
            super::MemberRef::Field {
                class: self.defs.get(class).name.clone(),
                name: name.to_string(),
                index,
                ty: self.tcx.to_ty(ty),
            },
        );
        Some(ty)
    }

    /// Bare field in a position that needs a binding (`move x`, `&x`, `x[i] = v`, capture list):
    /// records the member and reports `message`. Returns the field type, or None if `name` is no field.
    pub(super) fn reject_bare_field(&mut self, name: &str, span: Span, id: AstId, message: &str) -> Option<TyId> {
        let ty = self.resolve_bare_field(name, id)?;
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
            self.reject_bare_field(&capture.name, capture.span, capture.id, &message);
        }
    }

    /// Class of the enclosing method when a bare call `name(..)` is one of its methods.
    pub(super) fn implicit_method(&self, name: &str) -> Option<DefId> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.self_class?;
        let method = self.defs.lookup_method(class, name)?;
        self.fun_sigs.contains_key(&method).then_some(class)
    }

    /// Signature of a user function or a builtin that has one, by callee name.
    pub(super) fn fun_sig(&self, name: &str) -> Option<&crate::ty::FnSig<TyId>> {
        let def = self.defs.lookup_fn(name).or_else(|| self.defs.lookup_intrinsic(name))?;
        self.fun_sigs.get(&def)
    }

    /// The class a bare identifier names (a constructor call).
    pub(super) fn class_named(&self, name: &str) -> Option<&ClassInfo> {
        self.classes.get(&self.defs.lookup_class(name)?)
    }

    pub(super) fn class(&self, id: DefId) -> Option<&ClassInfo> {
        self.classes.get(&id)
    }

    /// Message text of a type.
    pub(super) fn show(&self, ty: TyId) -> TyPrinter<'_> {
        self.tcx.display(ty)
    }

    pub(super) fn expr(&self, kind: HirExprKind, ty: TyId) -> Checked {
        Checked { hir: HirExpr::new(kind, self.tcx.to_ty(ty), ty), ty }
    }

    pub(super) fn expr_at(&self, kind: HirExprKind, ty: TyId, span: Span) -> Checked {
        Checked { hir: HirExpr::spanned(kind, self.tcx.to_ty(ty), ty, span), ty }
    }

    /// Lowers a written type, reporting problems at `span`.
    pub(super) fn lower(&mut self, ty: &crate::ast::Type, span: Option<Span>) -> TyId {
        super::lower_type(ty, self.defs, self.tcx, &mut self.diagnostics, span)
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
