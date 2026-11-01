//! Trait and impl declarations (#55): lowers trait method signatures and checks that every
//! `impl Trait for Type` provides exactly the trait's methods with matching signatures.
//!
//! Nothing is solved or dispatched yet (#56, #58), and impl bodies are not type-checked until a
//! call can reach them.

use std::collections::{HashMap, HashSet};

use super::{generics, lower::lower_type, type_error};
use crate::ast::{Impl, Program, Trait};
use crate::defs::{DefId, DefTable};
use crate::diag::Diagnostic;
use crate::infer::{InferCtxt, VarKind};
use crate::span::Span;
use crate::ty::{subst, FnSig, ParamIdx, Predicate, ProjectionPredicate, TraitRef, TyCtx, TyData, TyId};

/// A trait method signature without the implicit `this: &Self`. In `params`/`ret`, `Param(0)` is
/// `Self`, then come the trait's parameters, then the method's own.
#[derive(Debug, Clone)]
pub(super) struct TraitMethodInfo {
    pub name: String,
    pub generics: usize,
    pub generic_names: Vec<String>,
    pub param_names: Vec<String>,
    pub params: Vec<TyId>,
    pub ret: TyId,
}

#[derive(Debug, Clone)]
pub(super) struct TraitInfo {
    pub generics: Vec<String>,
    /// `type Item` declarations.
    pub assoc_types: Vec<String>,
    pub methods: Vec<TraitMethodInfo>,
}

/// One `impl Trait<args> for self_ty`; read by the solver (#56) and by dispatch (#58).
#[derive(Debug, Clone)]
pub(super) struct ImplInfo {
    pub trait_id: DefId,
    /// Program order among all impls; coherence reports in this order.
    pub index: usize,
    /// The impl's trait name, where its diagnostics point.
    pub span: Span,
    pub generics: Vec<String>,
    pub trait_args: Vec<TyId>,
    pub self_ty: TyId,
    /// The impl's own bounds (`impl<T: Show> ..`); they become obligations when it is selected.
    pub predicates: Vec<Predicate<TyId>>,
    pub methods: HashMap<String, DefId>,
    /// `type Item = X`, as written in the impl (may mention other projections of `self_ty`).
    pub assoc: HashMap<String, TyId>,
    /// Signature of each method whose header matches the trait, `this: &Self` first; only those get bodies checked.
    pub sigs: HashMap<DefId, FnSig<TyId>>,
}

/// The trait and impl signatures monomorphization resolves a `Trait.method` call with.
pub(super) fn hir_signatures(
    program: &Program,
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    impls: &ImplTable,
    tcx: &mut TyCtx,
) -> (Vec<crate::hir::HirTrait>, Vec<crate::hir::HirImpl>) {
    let this_ty = tcx.param(ParamIdx(0));
    let this_ref = tcx.ref_to(this_ty);
    let mut hir_traits = Vec::new();
    for (index, t) in program.traits.iter().enumerate() {
        let id = defs.trait_id(index);
        // A rejected or repeated trait name keeps the first declaration.
        let Some(info) = traits.get(&id).filter(|_| defs.lookup_trait(&t.name.name) == Some(id)) else { continue };
        let methods = info
            .methods
            .iter()
            .map(|m| crate::hir::HirTraitMethod {
                name: m.name.clone(),
                generics: std::iter::once("Self".to_string()).chain(info.generics.clone()).chain(m.generic_names.clone()).collect(),
                own_generics: m.generic_names.len(),
                params: std::iter::once(this_ref).chain(m.params.iter().copied()).collect(),
                ret: m.ret,
            })
            .collect();
        hir_traits.push(crate::hir::HirTrait { name: t.name.name.clone(), methods });
    }
    let mut infos: Vec<&ImplInfo> = impls.values().flatten().collect();
    infos.sort_by_key(|info| info.index);
    let hir_impls = infos
        .into_iter()
        .map(|info| crate::hir::HirImpl {
            trait_name: defs.get(info.trait_id).name.clone(),
            generics: info.generics.clone(),
            trait_args: info.trait_args.clone(),
            self_ty: info.self_ty,
            assoc: {
                let mut assoc: Vec<_> = info.assoc.iter().map(|(name, &ty)| (name.clone(), ty)).collect();
                assoc.sort_by(|a, b| a.0.cmp(&b.0));
                assoc
            },
            methods: program.impls[info.index]
                .methods
                .iter()
                .map(|m| (m.name.clone(), impl_method_name(&defs.get(info.trait_id).name, info.index, &m.name)))
                .collect(),
        })
        .collect();
    (hir_traits, hir_impls)
}

/// The HIR name of a method of the `index`th impl of `trait_name`. `@` cannot appear in an
/// identifier, so it never collides with a user function, and the index keeps `impl Show for
/// Box<i32>` apart from `impl Show for Box<String>`.
pub(super) fn impl_method_name(trait_name: &str, index: usize, method: &str) -> String {
    format!("{trait_name}@{index}.{method}")
}

impl ImplInfo {
    /// The header with a fresh inference variable for each impl parameter: self type, trait
    /// arguments and the impl's own bounds. `None` if a substitution would make `T??`.
    pub(super) fn instantiate(
        &self,
        infcx: &mut InferCtxt,
        tcx: &mut TyCtx,
    ) -> Option<(TyId, Vec<TyId>, Vec<Predicate<TyId>>)> {
        let fresh: Vec<TyId> = self.generics.iter().map(|_| infcx.new_var(tcx, VarKind::General)).collect();
        let self_ty = subst(tcx, self.self_ty, &fresh).ok()?;
        let args = self.trait_args.iter().map(|&a| subst(tcx, a, &fresh)).collect::<Result<_, _>>().ok()?;
        let predicates = self.predicates.iter().map(|p| p.subst(tcx, &fresh)).collect::<Result<_, _>>().ok()?;
        Some((self_ty, args, predicates))
    }
}

/// The predicates of `T: Trait<..>` bounds on `generics`, and the equality bounds of `Item = X`
/// bindings; `scope` numbers the parameters. An unknown trait or a wrong argument count is
/// reported and the bound dropped.
pub(super) fn lower_bounds(
    generics: &[crate::ast::TypeParam],
    scope: &[(String, ParamIdx)],
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<Predicate<TyId>>, Vec<ProjectionPredicate<TyId>>) {
    let mut predicates = Vec::new();
    let mut proj_predicates = Vec::new();
    for param in generics {
        let Some(&(_, idx)) = scope.iter().find(|(name, _)| *name == param.name.name) else { continue };
        for bound in &param.bounds {
            let Some(trait_ref) = lower_trait_ref(bound, scope, defs, traits, tcx, diagnostics) else { continue };
            let self_ty = tcx.param(idx);
            for (name, ty) in &bound.bindings {
                if !traits[&trait_ref.trait_id].assoc_types.contains(&name.name) {
                    let message = format!("trait `{}` has no associated type `{}`", bound.name.name, name.name);
                    diagnostics.push(type_error(message, Some(name.span)));
                    continue;
                }
                let ty = lower_type(ty, defs, scope, tcx, diagnostics, Some(name.span));
                let projection = tcx.projection(self_ty, Some(&bound.name.name), &name.name);
                if proj_predicates.iter().any(|p: &ProjectionPredicate<TyId>| p.projection == projection) {
                    let message = format!("duplicate binding of `{}` on `{}`", name.name, param.name.name);
                    diagnostics.push(type_error(message, Some(name.span)));
                    continue;
                }
                proj_predicates.push(ProjectionPredicate { projection, ty });
            }
            let predicate = Predicate { self_ty, trait_ref };
            if predicates.contains(&predicate) {
                let message = format!("duplicate bound `{}` on `{}`", bound.name.name, param.name.name);
                diagnostics.push(type_error(message, Some(bound.name.span)));
            } else {
                predicates.push(predicate);
            }
        }
    }
    (predicates, proj_predicates)
}

/// One written bound `Trait<args>`; `None` once an error is reported (or an argument is already one).
fn lower_trait_ref(
    bound: &crate::ast::TraitRef,
    scope: &[(String, ParamIdx)],
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<TraitRef<TyId>> {
    let span = Some(bound.name.span);
    let trait_id = resolve_trait(bound, defs, traits, diagnostics)?;
    check_trait_arity(bound, traits[&trait_id].generics.len(), diagnostics)?;
    let args: Vec<TyId> = bound.args.iter().map(|a| lower_type(a, defs, scope, tcx, diagnostics, span)).collect();
    (!args.iter().any(|&a| tcx.is_error(a))).then_some(TraitRef { trait_id, args })
}

fn resolve_trait(
    bound: &crate::ast::TraitRef,
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<DefId> {
    let found = defs.lookup_trait(&bound.name.name).filter(|id| traits.contains_key(id));
    if found.is_none() {
        diagnostics.push(type_error(format!("unknown trait `{}`", bound.name.name), Some(bound.name.span)));
    }
    found
}

fn check_trait_arity(bound: &crate::ast::TraitRef, wanted: usize, diagnostics: &mut Vec<Diagnostic>) -> Option<()> {
    if bound.args.len() == wanted {
        return Some(());
    }
    let message = format!(
        "trait `{}` takes {wanted} type argument(s) but {} were supplied",
        bound.name.name,
        bound.args.len()
    );
    diagnostics.push(type_error(message, Some(bound.name.span)));
    None
}

/// Impls by trait, in program order.
pub(super) type ImplTable = HashMap<DefId, Vec<ImplInfo>>;

pub(super) fn build_traits(
    program: &Program,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<DefId, TraitInfo> {
    let mut traits = HashMap::new();
    for (index, t) in program.traits.iter().enumerate() {
        let id = defs.trait_id(index);
        diagnostics.extend(defs.diagnostics(id, |_| true));
        for method in 0..t.methods.len() {
            diagnostics.extend(defs.diagnostics(defs.trait_method_id(index, method), |_| true));
        }
        // A rejected or repeated trait name keeps the first declaration.
        if defs.lookup_trait(&t.name.name) == Some(id) {
            traits.insert(id, lower_trait(t, defs, tcx, diagnostics));
        }
    }
    traits
}

fn lower_trait(t: &Trait, defs: &DefTable, tcx: &mut TyCtx, diagnostics: &mut Vec<Diagnostic>) -> TraitInfo {
    let mut outer = vec![crate::ast::TypeParam { name: crate::span::SpannedName::new("Self", t.name.span), bounds: Vec::new() }];
    outer.extend(t.generics.iter().cloned());
    let mut seen = HashSet::new();
    let mut methods = Vec::new();
    for method in &t.methods {
        // The first declaration of a name wins; the duplicate is reported by the def table.
        if !seen.insert(method.name.name.clone()) {
            continue;
        }
        let scope = generics::scope(&outer, &method.generics);
        tcx.set_param_names(generics::names(&outer, &method.generics));
        let params = method
            .params
            .iter()
            .map(|p| lower_type(&p.ty, defs, &scope, tcx, diagnostics, Some(p.name.span)))
            .collect();
        let ret = lower_type(&method.return_type, defs, &scope, tcx, diagnostics, Some(method.name.span));
        methods.push(TraitMethodInfo {
            name: method.name.name.clone(),
            generics: method.generics.len(),
            generic_names: method.generics.iter().map(|g| g.name.name.clone()).collect(),
            param_names: method.params.iter().map(|p| p.name.name.clone()).collect(),
            params,
            ret,
        });
    }
    tcx.set_param_names(Vec::new());
    let mut assoc_types: Vec<String> = Vec::new();
    for a in &t.assoc_types {
        if assoc_types.contains(&a.name) {
            diagnostics.push(type_error(format!("duplicate associated type `{}` in trait `{}`", a.name, t.name.name), Some(a.span)));
        } else {
            assoc_types.push(a.name.clone());
        }
    }
    // `Self::Item` in a method is this trait's own `Item`.
    let this = tcx.param(ParamIdx(0));
    for m in &mut methods {
        let declares = |base: TyId, name: &str| -> Vec<String> {
            if base == this && assoc_types.iter().any(|n| n == name) { vec![t.name.name.clone()] } else { Vec::new() }
        };
        let mut qualify = Qualify { declares: &declares, names: &[], errors: Vec::new() };
        m.params = m.params.iter().map(|&p| qualify.apply(tcx, p)).collect();
        m.ret = qualify.apply(tcx, m.ret);
    }
    TraitInfo { generics: t.generics.iter().map(|g| g.name.name.clone()).collect(), assoc_types, methods }
}

pub(super) fn build_impls(
    program: &Program,
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> ImplTable {
    let mut table = ImplTable::new();
    for (index, i) in program.impls.iter().enumerate() {
        let Some(trait_id) = defs.lookup_trait(&i.trait_name.name).filter(|id| traits.contains_key(id)) else {
            diagnostics.push(type_error(format!("unknown trait `{}`", i.trait_name.name), Some(i.trait_name.span)));
            continue;
        };
        if let Some(info) = check_impl(i, index, trait_id, &traits[&trait_id], traits, defs, tcx, diagnostics) {
            table.entry(trait_id).or_default().push(info);
        }
    }
    table
}

fn check_impl(
    i: &Impl,
    index: usize,
    trait_id: DefId,
    t: &TraitInfo,
    traits: &HashMap<DefId, TraitInfo>,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ImplInfo> {
    let head = Some(i.trait_name.span);
    let trait_name = &i.trait_name.name;
    if i.trait_args.len() != t.generics.len() {
        let message = format!(
            "trait `{trait_name}` takes {} type argument(s) but {} were supplied",
            t.generics.len(),
            i.trait_args.len()
        );
        diagnostics.push(type_error(message, head));
        return None;
    }
    let scope = generics::scope(&[], &i.generics);
    tcx.set_param_names(generics::names(&[], &i.generics));
    let (predicates, proj_predicates) = lower_bounds(&i.generics, &scope, defs, traits, tcx, diagnostics);
    // shortcut: the impl is rejected rather than enforcing the bound on selection; add it to the nested obligations when needed.
    if !proj_predicates.is_empty() {
        diagnostics.push(type_error("equality bounds (`Item = X`) on an impl's parameters are not supported yet", head));
        tcx.set_param_names(Vec::new());
        return None;
    }
    let self_ty = lower_type(&i.self_ty, defs, &scope, tcx, diagnostics, Some(i.self_span));
    let trait_args: Vec<TyId> =
        i.trait_args.iter().map(|arg| lower_type(arg, defs, &scope, tcx, diagnostics, head)).collect();
    if tcx.is_error(self_ty) || trait_args.iter().any(|&arg| tcx.is_error(arg)) {
        tcx.set_param_names(Vec::new());
        return None;
    }
    if !matches!(tcx.data(self_ty), TyData::Class(..) | TyData::Enum(..)) {
        let message = format!("`impl {trait_name} for {}`: only classes and enums can implement a trait yet", tcx.display(self_ty));
        diagnostics.push(type_error(message, Some(i.self_span)));
        tcx.set_param_names(Vec::new());
        return None;
    }

    let assoc = lower_assoc(i, t, self_ty, &scope, defs, tcx, diagnostics);
    let mut implemented = HashMap::new();
    let mut sigs = HashMap::new();
    for (method_index, method) in i.methods.iter().enumerate() {
        let id = defs.impl_method_id(index, method_index);
        if implemented.contains_key(&method.name) {
            let message = format!("duplicate method `{}` in impl of `{trait_name}`", method.name);
            diagnostics.push(type_error(message, Some(method.name_span)));
            continue;
        }
        implemented.insert(method.name.clone(), id);
        let Some(expected) = t.methods.iter().find(|m| m.name == method.name) else {
            let message = format!("method `{}` is not a member of trait `{trait_name}`", method.name);
            diagnostics.push(type_error(message, Some(method.name_span)));
            continue;
        };
        if let Some(mut sig) = check_method(i, method, expected, self_ty, &trait_args, &assoc, &scope, defs, tcx, diagnostics) {
            sig.predicates = predicates.clone();
            sigs.insert(id, sig);
        }
    }
    let missing: Vec<String> =
        t.methods.iter().filter(|m| !implemented.contains_key(&m.name)).map(|m| format!("`{}`", m.name)).collect();
    if !missing.is_empty() {
        let message = format!("not all trait items implemented, missing: {}", missing.join(", "));
        diagnostics.push(type_error(message, head));
    }
    tcx.set_param_names(Vec::new());
    Some(ImplInfo {
        trait_id,
        index,
        span: i.trait_name.span,
        generics: i.generics.iter().map(|g| g.name.name.clone()).collect(),
        trait_args,
        self_ty,
        predicates,
        methods: implemented,
        assoc,
        sigs,
    })
}

#[allow(clippy::too_many_arguments)]
fn check_method(
    i: &Impl,
    method: &crate::ast::Function,
    expected: &TraitMethodInfo,
    self_ty: TyId,
    trait_args: &[TyId],
    assoc: &HashMap<String, TyId>,
    impl_scope: &[(String, ParamIdx)],
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<FnSig<TyId>> {
    let errors_before = diagnostics.len();
    let (name, trait_name, span) = (&method.name, &i.trait_name.name, Some(method.name_span));
    if method.generics.len() != expected.generics {
        let message = format!(
            "method `{name}` has {} type parameter(s) but trait `{trait_name}` declares {}",
            method.generics.len(),
            expected.generics
        );
        diagnostics.push(type_error(message, span));
        return None;
    }
    // Trait methods cannot declare bounds yet, so any bound here is stricter than the trait.
    if let Some(bound) = method.generics.iter().flat_map(|g| &g.bounds).next() {
        let message = format!("method `{name}` has bounds that trait `{trait_name}` does not declare");
        diagnostics.push(type_error(message, Some(bound.name.span)));
    }
    if method.params.len() != expected.params.len() {
        let message = format!(
            "method `{name}` has {} parameters but trait `{trait_name}` declares {}",
            method.params.len(),
            expected.params.len()
        );
        diagnostics.push(type_error(message, span));
        return None;
    }
    let scope: Vec<_> = impl_scope
        .iter()
        .cloned()
        .chain(method.generics.iter().enumerate().map(|(j, g)| (g.name.name.clone(), ParamIdx((impl_scope.len() + j) as u32))))
        .collect();
    tcx.set_param_names(scope.iter().map(|(n, _)| n.clone()).collect());
    // Trait view of this impl: Self, the trait's arguments, then this method's own parameters.
    let args: Vec<TyId> = std::iter::once(self_ty)
        .chain(trait_args.iter().copied())
        .chain((0..expected.generics).map(|j| tcx.param(ParamIdx((impl_scope.len() + j) as u32))))
        .collect();
    let shown = |tcx: &TyCtx, ty: TyId| tcx.display(ty).to_string();
    let mut params = Vec::new();
    for (k, param) in method.params.iter().enumerate() {
        let found = lower_type(&param.ty.with_self(&i.self_ty), defs, &scope, tcx, diagnostics, Some(param.name.span));
        let found = normalize_in_impl(tcx, found, trait_name, self_ty, assoc);
        params.push(found);
        let Ok(want) = subst(tcx, expected.params[k], &args) else { continue };
        let want = normalize_in_impl(tcx, want, trait_name, self_ty, assoc);
        if found != want && !tcx.is_error(found) && !tcx.is_error(want) && !tcx.has_projection_of(want, self_ty) {
            let message = format!(
                "parameter `{}` of `{name}` has type `{}` but trait `{trait_name}` declares `{}`",
                expected.param_names[k],
                shown(tcx, found),
                shown(tcx, want)
            );
            diagnostics.push(type_error(message, Some(param.name.span)));
        }
    }
    let ret = lower_type(&method.return_type.with_self(&i.self_ty), defs, &scope, tcx, diagnostics, span);
    let ret = normalize_in_impl(tcx, ret, trait_name, self_ty, assoc);
    let found = ret;
    if let Ok(want) = subst(tcx, expected.ret, &args) {
        let want = normalize_in_impl(tcx, want, trait_name, self_ty, assoc);
        if found != want && !tcx.is_error(found) && !tcx.is_error(want) && !tcx.has_projection_of(want, self_ty) {
            let message = format!(
                "`{name}` returns `{}` but trait `{trait_name}` declares `{}`",
                shown(tcx, found),
                shown(tcx, want)
            );
            diagnostics.push(type_error(message, span));
        }
    }
    if diagnostics.len() != errors_before {
        return None;
    }
    let this_ty = tcx.ref_to(self_ty);
    let mut sig = FnSig::new(std::iter::once(this_ty).chain(params).collect(), ret);
    sig.generics = scope.iter().map(|(_, idx)| *idx).collect();
    sig.generic_names = scope.iter().map(|(name, _)| name.clone()).collect();
    Some(sig)
}

/// Lowers the `type Item = X` items of `i`, reporting duplicate, unknown and missing ones.
fn lower_assoc(
    i: &Impl,
    t: &TraitInfo,
    self_ty: TyId,
    scope: &[(String, ParamIdx)],
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<String, TyId> {
    let trait_name = &i.trait_name.name;
    let mut assoc = HashMap::new();
    for (name, ty) in &i.assoc_types {
        if assoc.contains_key(&name.name) {
            diagnostics.push(type_error(format!("duplicate associated type `{}` in impl of `{trait_name}`", name.name), Some(name.span)));
        } else if !t.assoc_types.contains(&name.name) {
            diagnostics.push(type_error(format!("`{}` is not an associated type of trait `{trait_name}`", name.name), Some(name.span)));
        } else {
            let ty = lower_type(&ty.with_self(&i.self_ty), defs, scope, tcx, diagnostics, Some(name.span));
            assoc.insert(name.name.clone(), ty);
        }
    }
    let missing: Vec<String> =
        t.assoc_types.iter().filter(|n| !assoc.contains_key(*n)).map(|n| format!("type `{n}`")).collect();
    if !missing.is_empty() {
        let message = format!("not all trait items implemented, missing: {}", missing.join(", "));
        diagnostics.push(type_error(message, Some(i.trait_name.span)));
    }
    // A cycle (`type Item = Self::Item`) is reported once, here, not at every use.
    for (name, &ty) in &assoc {
        let normalized = normalize_in_impl(tcx, ty, trait_name, self_ty, &assoc);
        if tcx.has_projection_of(normalized, self_ty) {
            let message = format!("the associated type `{name}` of `{}` refers to itself", tcx.display(self_ty));
            diagnostics.push(type_error(message, Some(i.trait_name.span)));
        }
    }
    assoc
}

/// How many `type Item = ..` definitions may be followed to normalize one type.
const MAX_NORMALIZE_DEPTH: u32 = 32;

/// Gives each `T::Item` written on a type parameter the trait that declares `Item`, found through
/// `declares(T, "Item")`; reports the ones no bound declares, or several do.
pub(super) struct Qualify<'a> {
    pub declares: &'a dyn Fn(TyId, &str) -> Vec<String>,
    /// Names of the type parameters, for messages.
    pub names: &'a [String],
    pub errors: Vec<String>,
}

impl Qualify<'_> {
    pub fn apply(&mut self, tcx: &mut TyCtx, ty: TyId) -> TyId {
        use crate::ty::TypeFolder;
        self.fold(tcx, ty).unwrap_or(ty)
    }
}

impl crate::ty::TypeFolder for Qualify<'_> {
    fn fold(&mut self, ctx: &mut TyCtx, id: TyId) -> Result<TyId, crate::ty::NestedNullable> {
        let id = crate::ty::super_fold(self, ctx, id)?;
        let TyData::Projection(base, None, name) = ctx.data(id).clone() else { return Ok(id) };
        if !matches!(ctx.data(base), TyData::Param(_)) {
            return Ok(id);
        }
        let mut declaring = (self.declares)(base, &name);
        declaring.sort();
        declaring.dedup();
        let shown = ctx.display_with(base, self.names).to_string();
        match declaring.as_slice() {
            [only] => Ok(ctx.projection(base, Some(only), &name)),
            [] => {
                self.errors.push(format!(
                    "no bound on `{shown}` declares an associated type `{name}`; add a bound such as `{shown}: Trait`"
                ));
                Ok(ctx.error())
            }
            _ => {
                let traits = declaring.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ");
                self.errors.push(format!("`{shown}::{name}` is ambiguous: the bounds {traits} on `{shown}` all declare `{name}`"));
                Ok(ctx.error())
            }
        }
    }
}

/// Qualifies the projections in the parameters and return type of `sig` against its own bounds.
pub(super) fn qualify_sig(
    sig: &mut FnSig<TyId>,
    traits: &HashMap<DefId, TraitInfo>,
    tcx: &mut TyCtx,
    span: Option<Span>,
    diagnostics: &mut Vec<Diagnostic>,
    defs: &DefTable,
) {
    let declares = |base: TyId, name: &str| -> Vec<String> {
        sig.predicates
            .iter()
            .filter(|p| p.self_ty == base && traits[&p.trait_ref.trait_id].assoc_types.iter().any(|n| n == name))
            .map(|p| defs.get(p.trait_ref.trait_id).name.clone())
            .collect()
    };
    let names = sig.generic_names.clone();
    let mut qualify = Qualify { declares: &declares, names: &names, errors: Vec::new() };
    let params: Vec<TyId> = sig.params.iter().map(|&p| qualify.apply(tcx, p)).collect();
    let ret = qualify.apply(tcx, sig.ret);
    for predicate in &mut sig.proj_predicates {
        predicate.ty = qualify.apply(tcx, predicate.ty);
    }
    let errors = std::mem::take(&mut qualify.errors);
    // The same type is often written twice (`this` and the return); say it once.
    let mut seen = Vec::new();
    for message in errors {
        if !seen.contains(&message) {
            diagnostics.push(type_error(message.clone(), span));
            seen.push(message);
        }
    }
    sig.params = params;
    sig.ret = ret;
}

struct ImplNormalizer<'a> {
    trait_name: &'a str,
    self_ty: TyId,
    assoc: &'a HashMap<String, TyId>,
    depth: u32,
}

impl crate::ty::TypeFolder for ImplNormalizer<'_> {
    fn fold(&mut self, ctx: &mut TyCtx, id: TyId) -> Result<TyId, crate::ty::NestedNullable> {
        let id = crate::ty::super_fold(self, ctx, id)?;
        if let TyData::Projection(base, trait_name, name) = ctx.data(id).clone() {
            let ours = trait_name.as_deref().is_none_or(|t| t == self.trait_name);
            if let (true, true, Some(&value)) = (ours, base == self.self_ty, self.assoc.get(&name)) {
                self.depth += 1;
                if self.depth <= MAX_NORMALIZE_DEPTH {
                    return self.fold(ctx, value);
                }
            }
        }
        Ok(id)
    }
}

/// `ty` with the projections of the impl's own `self_ty` replaced by the impl's `type` items. A
/// cycle stops at the depth limit and leaves a projection behind.
fn normalize_in_impl(tcx: &mut TyCtx, ty: TyId, trait_name: &str, self_ty: TyId, assoc: &HashMap<String, TyId>) -> TyId {
    use crate::ty::TypeFolder;
    ImplNormalizer { trait_name, self_ty, assoc, depth: 0 }.fold(tcx, ty).unwrap_or(ty)
}
