use std::collections::{HashMap, HashSet};

use crate::ast::BindingKind;
use crate::constants::ConstantTable;
use crate::defs::{DefId, DefTable};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirExpr, HirExprKind};
use crate::infer::InferCtxt;
use crate::span::{AstId, Span};
use crate::ty::{FnSigs, TyCtx, TyId, TyPrinter};

use super::{MemberResolutions, NodeTypes};

/// A checked expression: the HIR node together with its type, repeated so call sites can read
/// `.ty` without reaching through `hir`.
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
    /// Holds a closure that borrows its captures, whatever type it was annotated with (`Func` erases it).
    pub(super) view_closure: bool,
    /// A `Func`-to-be parameter: its type is still an inference variable, so it is a view only once it resolves to a callable.
    pub(super) maybe_view: bool,
}

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
    /// Names of the class type parameters.
    pub(super) generics: Vec<String>,
}

pub(super) type ClassTable = HashMap<DefId, ClassInfo>;

#[derive(Clone, Debug)]
pub(super) struct VariantInfo {
    pub(super) name: String,
    /// Payload fields; empty for a unit variant.
    pub(super) fields: Vec<ClassFieldInfo>,
}

#[derive(Clone, Debug)]
pub(super) struct EnumInfo {
    pub(super) id: DefId,
    /// Display name only; identity is `id`.
    pub(super) name: String,
    pub(super) variants: Vec<VariantInfo>,
    /// Names of the enum type parameters.
    pub(super) generics: Vec<String>,
}

pub(super) type EnumTable = HashMap<DefId, EnumInfo>;

pub(super) struct Env<'a> {
    scopes: Vec<HashMap<String, Binding>>,
    pub(super) defs: &'a DefTable,
    pub(super) tcx: &'a mut TyCtx,
    pub(super) fun_sigs: &'a FnSigs,
    pub(super) classes: &'a ClassTable,
    pub(super) enums: &'a EnumTable,
    pub(super) constants: &'a ConstantTable,
    pub(super) diagnostics: Vec<Diagnostic>,
    /// Declaration and field assignment types keyed by source span.
    pub(super) node_tys: NodeTypes,
    pub(super) loop_depth: usize,
    /// Enclosing `unsafe` blocks, plus one inside an `unsafe fun`; see [`Env::in_unsafe`].
    pub(super) unsafe_depth: usize,
    /// Class whose method body is being checked; `None` in top-level functions.
    pub(super) self_class: Option<DefId>,
    /// Member names resolved by typeck, for sema and LSP.
    pub(super) members: MemberResolutions,
    /// Binding names of `if val`s over an `Option` that were checked as a `when`.
    pub(super) option_if_vals: HashSet<AstId>,
    /// Literal variables of this function body; defaulted and resolved once at its end.
    pub(super) infcx: InferCtxt,
    /// Type parameters of the function being checked, for written types in its body.
    pub(super) tparams: Vec<(String, crate::ty::ParamIdx)>,
    /// Type parameters instantiated at calls; any still unbound at the end of the body are errors.
    pub(super) pending: Vec<PendingParam>,
    /// Traits and impls the solver selects from (#56).
    pub(super) traits: &'a HashMap<DefId, super::traits::TraitInfo>,
    pub(super) impls: &'a super::traits::ImplTable,
    /// The bounds of the function being checked: what its type parameters are known to implement.
    pub(super) param_env: Vec<crate::ty::Predicate<TyId>>,
    /// The equality bounds (`T: Container<Item = i32>`) of the function being checked.
    pub(super) proj_env: Vec<crate::ty::ProjectionPredicate<TyId>>,
    /// Projections whose base type was still unknown when they were met; `var` stands in for them.
    pub(super) projections: Vec<PendingProjection>,
}

/// A projection waiting for its base type; solved by `solve::select_where_possible`.
pub(super) struct PendingProjection {
    pub(super) projection: TyId,
    pub(super) var: TyId,
    pub(super) span: Option<Span>,
}

/// One type parameter of a generic callee, instantiated with a fresh variable at a call.
pub(super) struct PendingParam {
    pub(super) var: TyId,
    pub(super) span: Span,
    pub(super) callee: String,
    pub(super) param: String,
}

impl<'a> Env<'a> {
    pub(super) fn new(
        defs: &'a DefTable,
        tcx: &'a mut TyCtx,
        fun_sigs: &'a FnSigs,
        classes: &'a ClassTable,
        enums: &'a EnumTable,
        traits: &'a HashMap<DefId, super::traits::TraitInfo>,
        impls: &'a super::traits::ImplTable,
        constants: &'a ConstantTable,
    ) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            defs,
            tcx,
            fun_sigs,
            classes,
            enums,
            constants,
            diagnostics: Vec::new(),
            node_tys: NodeTypes::new(),
            loop_depth: 0,
            unsafe_depth: 0,
            self_class: None,
            members: MemberResolutions::new(),
            option_if_vals: HashSet::new(),
            infcx: InferCtxt::new(),
            tparams: Vec::new(),
            pending: Vec::new(),
            traits,
            impls,
            param_env: Vec::new(),
            proj_env: Vec::new(),
            projections: Vec::new(),
        }
    }

    /// True inside an `unsafe fun` body or an `unsafe { }` block.
    #[allow(dead_code)] // first caller lands with the unsafe-only primitives
    pub(super) fn in_unsafe(&self) -> bool {
        self.unsafe_depth > 0
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
                    view_closure: false,
                    maybe_view: false,
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
                ty,
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

    /// A closure that borrows its captures: its own type says so, it is a name bound to one (an annotation
    /// such as `val h: () -> i32 = { -> n }` types it as a plain `Func`), or it is the result of a call that was
    /// handed one (`forward({ -> n })` may return its argument).
    pub(super) fn is_view_closure_value(&self, value: &Checked) -> bool {
        self.hir_is_view_closure(&value.hir)
    }

    fn binding_is_view(&self, b: &Binding) -> bool {
        b.view_closure || (b.maybe_view && self.infcx.bound_shallow(self.tcx, b.ty).is_some_and(|t| self.tcx.is_callable(t)))
    }

    /// Whether `value` is a plain name of a still-undecided parameter (`val g = f`): its alias stays undecided too.
    /// Read before the alias is bound, since `val f = f` shadows the source.
    pub(super) fn is_maybe_view_alias(&self, value: &HirExpr) -> bool {
        matches!(&value.kind, HirExprKind::Ident { name, .. } if self.binding(name).is_some_and(|b| b.maybe_view))
    }

    pub(super) fn mark_maybe_view(&mut self, name: &str) {
        if let Some(b) = self.scopes.iter_mut().rev().find_map(|scope| scope.get_mut(name)) {
            b.maybe_view = true;
        }
    }

    fn hir_is_view_closure(&self, e: &HirExpr) -> bool {
        if self.tcx.is_view_closure(e.ty) {
            return true;
        }
        match &e.kind {
            HirExprKind::Ident { name, .. } => self.binding(name).is_some_and(|b| self.binding_is_view(b)),
            HirExprKind::Call { args, .. } if self.tcx.is_callable(e.ty) => {
                args.iter().any(|a| self.hir_is_view_closure(a))
            }
            // A `move` closure keeps a captured view alive: its type does not say so (`Func` erases it), the binding does.
            HirExprKind::Closure { captures, .. } => captures.iter().any(|c| self.binding(&c.name).is_some_and(|b| self.binding_is_view(b))),
            // A branch result is a view when any branch is.
            HirExprKind::If { then_block, else_block, .. } => self.block_tail_is_view(then_block) || else_block.as_ref().is_some_and(|b| self.block_tail_is_view(b)),
            HirExprKind::Match { arms, .. } => arms.iter().any(|a| self.block_tail_is_view(&a.body)),
            _ => false,
        }
    }

    /// Whether the block's result is a view. The block's own `val`s (and those of enclosing branch blocks) are gone
    /// by the time the enclosing `return` is checked, so their provenance is computed once, in declaration order,
    /// into `seen`; an outer binding of the same name must not be mistaken for them.
    pub(super) fn block_tail_is_view(&self, block: &crate::hir::HirBlock) -> bool {
        self.tail_is_view(block, &std::collections::HashMap::new())
    }

    fn tail_is_view(&self, block: &crate::hir::HirBlock, outer: &std::collections::HashMap<String, bool>) -> bool {
        let Some((crate::hir::HirStmt::Expr(tail), decls)) = block.stmts.split_last() else { return false };
        let mut seen = outer.clone();
        for stmt in decls {
            if let crate::hir::HirStmt::VarDecl { name, value, .. } = stmt {
                let view = self.is_view_in(value, &seen);
                seen.insert(name.clone(), view);
            }
        }
        self.is_view_in(tail, &seen)
    }

    fn is_view_in(&self, e: &HirExpr, seen: &std::collections::HashMap<String, bool>) -> bool {
        match &e.kind {
            HirExprKind::Ident { name, .. } if seen.contains_key(name) => seen[name],
            HirExprKind::If { then_block, else_block, .. } => self.tail_is_view(then_block, seen) || else_block.as_ref().is_some_and(|b| self.tail_is_view(b, seen)),
            HirExprKind::Match { arms, .. } => arms.iter().any(|a| self.tail_is_view(&a.body, seen)),
            _ => self.hir_is_view_closure(e),
        }
    }

    /// What calling a value of type `ty` requires of its owner: a closure's own kind, or the weakest `Fn*`
    /// bound of a type parameter. Anything else counts as `Fn`.
    pub(super) fn callable_kind(&self, ty: TyId) -> crate::ty::ClosureKind {
        self.tcx.closure_kind(ty).unwrap_or_else(|| {
            let bound = |p: &&crate::ty::Predicate<TyId>| p.self_ty == ty;
            self.param_env.iter().filter(bound).filter_map(|p| self.tcx.fn_trait_kind(p.trait_ref.trait_id)).min().unwrap_or(crate::ty::ClosureKind::Fn)
        })
    }

    /// A `Func`-typed parameter may be handed a view closure (a plain closure borrowing its captures); its type does
    /// not say so, so the callee treats it as one and cannot return or store it.
    pub(super) fn mark_func_param_view(&mut self, name: &str, ty: TyId) {
        match self.tcx.data(ty) {
            crate::ty::TyData::Func { .. } => self.mark_view_closure(name),
            // Typed only by the body (`val g: () -> i32 = f`): decided when the name is read, not here.
            crate::ty::TyData::Infer(_) => {
                if let Some(b) = self.scopes.iter_mut().rev().find_map(|scope| scope.get_mut(name)) {
                    b.maybe_view = true;
                }
            }
            _ => {}
        }
    }

    /// Remembers that the binding just made holds a view closure.
    pub(super) fn mark_view_closure(&mut self, name: &str) {
        if let Some(binding) = self.scopes.iter_mut().rev().find_map(|scope| scope.get_mut(name)) {
            binding.view_closure = true;
        }
    }

    /// A closure that borrows its captures is a view (RFC 0007 section 5): it cannot be stored where it
    /// would outlive them, i.e. in a `var`, a field, an array element or a return value.
    pub(super) fn reject_stored_view_closure(&mut self, value: &Checked, what: &str, span: Option<Span>) {
        if self.is_view_closure_value(value) {
            self.error(format!("cannot store a closure that borrows its captures in {what}; use `val`, or `move`"), span);
        }
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

    /// The enum a bare identifier names (a variant path `E.V`).
    pub(super) fn enum_named(&self, name: &str) -> Option<&EnumInfo> {
        // An implicit field of the enclosing class shadows the enum, as a local does.
        if self.implicit_field(name).is_some() {
            return None;
        }
        self.enums.get(&self.defs.lookup_enum(name)?)
    }

    pub(super) fn class(&self, id: DefId) -> Option<&ClassInfo> {
        self.classes.get(&id)
    }

    /// Message text of a type.
    pub(super) fn show(&self, mut ty: TyId) -> TyPrinter<'_> {
        // An unbound literal variable reads as the type it will default to.
        while let crate::ty::TyData::Infer(var) = *self.tcx.data(ty) {
            ty = match (self.infcx.value_of(var), self.infcx.kind_of(var)) {
                (Some(bound), _) => bound,
                (None, crate::infer::VarKind::Float) => self.tcx.prim_id(crate::hir::Prim::F64),
                (None, crate::infer::VarKind::Int) => self.tcx.prim_id(crate::hir::Prim::I32),
                (None, crate::infer::VarKind::General) => return self.tcx.display(ty),
            };
        }
        self.tcx.display(ty)
    }

    pub(super) fn expr(&self, kind: HirExprKind, ty: TyId) -> Checked {
        Checked { hir: HirExpr::new(kind, ty), ty }
    }

    pub(super) fn expr_at(&self, kind: HirExprKind, ty: TyId, span: Span) -> Checked {
        Checked { hir: HirExpr::spanned(kind, ty, span), ty }
    }

    /// Lowers a written type, reporting problems at `span`.
    pub(super) fn lower(&mut self, ty: &crate::ast::Type, span: Option<Span>) -> TyId {
        let lowered = super::lower_type(ty, self.defs, &self.tparams, self.tcx, &mut self.diagnostics, span);
        super::solve::normalize(self, lowered, span)
    }

    /// A class member type seen through `receiver`: the class parameters replaced by its arguments.
    pub(super) fn member_ty(&mut self, receiver: TyId, member: TyId) -> TyId {
        let args = self.tcx.class_args(receiver).map(|(_, args)| args.to_vec()).unwrap_or_default();
        if args.is_empty() {
            return member;
        }
        crate::ty::subst(self.tcx, member, &args).unwrap_or_else(|_| {
            let shown = self.show(receiver);
            self.error(format!("type arguments of `{shown}` would make a nullable of a nullable"), None);
            self.tcx.error()
        })
    }

    pub(super) fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    /// Whether two types are equal, binding literal variables to make them so.
    pub(super) fn same(&mut self, a: TyId, b: TyId) -> bool {
        a == b || self.infcx.unify(self.tcx, a, b).is_ok()
    }

    /// `ty` with bound literal variables replaced (shallow).
    pub(super) fn settle(&mut self, ty: TyId) -> TyId {
        self.infcx.resolve_shallow(self.tcx, ty)
    }

    /// Whether `ty` is an unbound variable of `kind`.
    pub(super) fn is_literal_var(&self, ty: TyId, kind: crate::infer::VarKind) -> bool {
        matches!(self.tcx.data(ty), crate::ty::TyData::Infer(v) if self.infcx.kind_of(*v) == kind)
    }

    /// Numeric, counting an unbound literal variable (but not an unbound general one).
    pub(super) fn is_numeric(&mut self, ty: TyId) -> bool {
        let ty = self.settle(ty);
        match self.tcx.data(ty) {
            crate::ty::TyData::Infer(var) => self.infcx.kind_of(*var) != crate::infer::VarKind::General,
            _ => self.tcx.is_numeric(ty),
        }
    }

    /// Whether `ty` is a type parameter of the function being checked.
    pub(super) fn is_param(&self, ty: TyId) -> bool {
        matches!(self.tcx.data(ty), crate::ty::TyData::Param(_))
    }

    /// Explains why an operation fails on a type parameter; empty for other types.
    pub(super) fn bounds_hint(&self, types: &[TyId]) -> &'static str {
        let bounded = |ty| self.param_env.iter().any(|p| p.self_ty == ty);
        if types.iter().any(|&ty| self.is_param(ty) && bounded(ty)) {
            "; a bound gives a type parameter trait methods only, not operators or fields"
        } else if types.iter().any(|&ty| self.is_param(ty)) {
            "; a type parameter has no bounds yet, so operators and members need a bound (not supported yet)"
        } else {
            ""
        }
    }

    /// `Self: Trait<args>` as diagnostics print it.
    pub(super) fn show_predicate(&self, predicate: &crate::ty::Predicate<TyId>) -> String {
        predicate.render(self.defs, |ty| self.show(ty).to_string())
    }

    /// The one place a type mismatch is reported: `{what} has type X, expected Y`. Unknown
    /// (already reported) types never cascade. Returns whether a diagnostic was emitted.
    pub(super) fn mismatch(&mut self, what: &str, found: TyId, expected: TyId, span: Option<Span>) -> bool {
        if self.tcx.is_unknown(found) || self.tcx.is_unknown(expected) || self.same(found, expected) {
            return false;
        }
        let (found, expected) = (self.settle(found), self.settle(expected));
        let message = format!("{what} has type {}, expected {}", self.show(found), self.show(expected));
        self.error(message, span);
        true
    }

    pub(super) fn error(&mut self, message: impl Into<String>, span: Option<Span>) {
        self.diagnostics.push(Diagnostic {
            phase: Phase::Type,
            severity: Severity::Error,
            message: message.into(),
            span,
            notes: Vec::new(),
        });
    }
}
