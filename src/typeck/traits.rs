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
use crate::ty::{subst, ParamIdx, Predicate, TraitRef, TyCtx, TyData, TyId};

/// A trait method signature without the implicit `this: &Self`. In `params`/`ret`, `Param(0)` is
/// `Self`, then come the trait's parameters, then the method's own.
#[derive(Debug, Clone)]
pub(super) struct TraitMethodInfo {
    pub name: String,
    pub generics: usize,
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
    pub id: DefId,
    pub generics: Vec<String>,
    pub trait_args: Vec<TyId>,
    pub self_ty: TyId,
    /// The impl's own bounds (`impl<T: Show> ..`); they become obligations when it is selected.
    pub predicates: Vec<Predicate<TyId>>,
    pub methods: HashMap<String, DefId>,
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
            let span = Some(bound.name.span);
            let Some(trait_id) = defs.lookup_trait(&bound.name.name).filter(|id| traits.contains_key(id)) else {
                diagnostics.push(type_error(format!("unknown trait `{}`", bound.name.name), span));
                continue;
            };
            let wanted = traits[&trait_id].generics.len();
            if bound.args.len() != wanted {
                let message = format!(
                    "trait `{}` takes {wanted} type argument(s) but {} were supplied",
                    bound.name.name,
                    bound.args.len()
                );
                diagnostics.push(type_error(message, span));
                continue;
            }
            let args: Vec<TyId> = bound.args.iter().map(|a| lower_type(a, defs, scope, tcx, diagnostics, span)).collect();
            if args.iter().any(|&a| tcx.is_error(a)) {
                continue;
            }
            let self_ty = tcx.param(idx);
            predicates.push(Predicate { self_ty, trait_ref: TraitRef { trait_id, args } });
        }
    }
    predicates
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
        if let Some(info) = check_impl(i, index, &traits[&trait_id], traits, defs, tcx, diagnostics) {
            table.entry(trait_id).or_default().push(info);
        }
    }
    table
}

fn check_impl(
    i: &Impl,
    index: usize,
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
        check_method(i, method, expected, self_ty, &trait_args, &scope, defs, tcx, diagnostics);
    }
    let missing: Vec<String> =
        t.methods.iter().filter(|m| !implemented.contains_key(&m.name)).map(|m| format!("`{}`", m.name)).collect();
    if !missing.is_empty() {
        let message = format!("not all trait items implemented, missing: {}", missing.join(", "));
        diagnostics.push(type_error(message, head));
    }
    tcx.set_param_names(Vec::new());
    Some(ImplInfo {
        id: defs.impl_id(index),
        generics: i.generics.iter().map(|g| g.name.name.clone()).collect(),
        trait_args,
        self_ty,
        predicates,
        methods: implemented,
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
) {
    let (name, trait_name, span) = (&method.name, &i.trait_name.name, Some(method.name_span));
    if method.generics.len() != expected.generics {
        let message = format!(
            "method `{name}` has {} type parameter(s) but trait `{trait_name}` declares {}",
            method.generics.len(),
            expected.generics
        );
        diagnostics.push(type_error(message, span));
        return;
    }
    if method.params.len() != expected.params.len() {
        let message = format!(
            "method `{name}` has {} parameters but trait `{trait_name}` declares {}",
            method.params.len(),
            expected.params.len()
        );
        diagnostics.push(type_error(message, span));
        return;
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
    for (k, param) in method.params.iter().enumerate() {
        let found = lower_type(&param.ty.with_self(&i.self_ty), defs, &scope, tcx, diagnostics, Some(param.name.span));
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
    let found = lower_type(&method.return_type.with_self(&i.self_ty), defs, &scope, tcx, diagnostics, span);
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
}
