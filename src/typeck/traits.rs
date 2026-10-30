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
use crate::ty::{subst, FnSig, ParamIdx, Predicate, TraitRef, TyCtx, TyData, TyId};

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

/// The predicates of `T: Trait<..>` bounds on `generics`; `scope` numbers the parameters. An unknown
/// trait or a wrong argument count is reported and the bound dropped.
pub(super) fn lower_bounds(
    generics: &[crate::ast::TypeParam],
    scope: &[(String, ParamIdx)],
    defs: &DefTable,
    traits: &HashMap<DefId, TraitInfo>,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Predicate<TyId>> {
    let mut predicates = Vec::new();
    for param in generics {
        let Some(&(_, idx)) = scope.iter().find(|(name, _)| *name == param.name.name) else { continue };
        for bound in &param.bounds {
            let Some(trait_ref) = lower_trait_ref(bound, scope, defs, traits, tcx, diagnostics) else { continue };
            let predicate = Predicate { self_ty: tcx.param(idx), trait_ref };
            if predicates.contains(&predicate) {
                let message = format!("duplicate bound `{}` on `{}`", bound.name.name, param.name.name);
                diagnostics.push(type_error(message, Some(bound.name.span)));
            } else {
                predicates.push(predicate);
            }
        }
    }
    predicates
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
    TraitInfo { generics: t.generics.iter().map(|g| g.name.name.clone()).collect(), methods }
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
    let predicates = lower_bounds(&i.generics, &scope, defs, traits, tcx, diagnostics);
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
        if let Some(mut sig) = check_method(i, method, expected, self_ty, &trait_args, &scope, defs, tcx, diagnostics) {
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
        params.push(found);
        let Ok(want) = subst(tcx, expected.params[k], &args) else { continue };
        if found != want && !tcx.is_error(found) && !tcx.is_error(want) {
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
    let found = ret;
    if let Ok(want) = subst(tcx, expected.ret, &args) {
        if found != want && !tcx.is_error(found) && !tcx.is_error(want) {
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
