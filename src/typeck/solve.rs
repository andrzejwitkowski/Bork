//! The trait solver (#56): decides the obligations that calls register in `InferCtxt`.
//!
//! An obligation `Self: Trait<args>` is solved from the function's own bounds when `Self` is a type
//! parameter, and from the impl table when it is a class or enum. A requirement whose type is still
//! an unbound variable waits. There is no negative reasoning and no specialization.

use super::env::Env;
use crate::infer::Obligation;
use crate::span::Span;
use crate::ty::{Predicate, TyData, TyId};

/// How many impl bounds may be followed to solve one requirement.
const MAX_DEPTH: u32 = 32;

/// Registers the bounds `predicates` of a callee, instantiated with `args`, and solves what it can.
pub(super) fn register(env: &mut Env<'_>, predicates: &[Predicate<TyId>], args: &[TyId], span: Span, origin: &str) {
    for predicate in predicates {
        // A bound that would make `T??` is already reported by the caller's own substitution.
        if let Ok(predicate) = predicate.subst(env.tcx, args) {
            env.infcx.register(Obligation { predicate, span: Some(span), depth: 0, origin: origin.to_string() });
        }
    }
    select_where_possible(env);
}

/// Registers a requirement that is already in terms of the current function's types.
pub(super) fn require(env: &mut Env<'_>, predicate: Predicate<TyId>, span: Span, origin: &str) {
    env.infcx.register(Obligation { predicate, span: Some(span), depth: 0, origin: origin.to_string() });
    select_where_possible(env);
}

enum Outcome {
    /// Solved; these requirements of the chosen impl remain.
    Solved(Vec<Obligation>),
    /// Waiting for a variable to be bound.
    Stalled,
    Failed(String),
}

/// Solves obligations until none makes progress; the rest stay pending.
pub(super) fn select_where_possible(env: &mut Env<'_>) {
    loop {
        let pending = env.infcx.take_obligations();
        let mut stalled = Vec::new();
        let mut progress = false;
        for obligation in pending {
            match solve_one(env, &obligation) {
                Outcome::Solved(nested) => {
                    progress = true;
                    nested.into_iter().for_each(|n| env.infcx.register(n));
                }
                Outcome::Stalled => stalled.push(obligation),
                Outcome::Failed(message) => {
                    progress = true;
                    env.error(message, obligation.span);
                }
            }
        }
        stalled.into_iter().for_each(|o| env.infcx.register(o));
        if !progress {
            return;
        }
    }
}

/// What is still pending at the end of the body is waiting for a type that nothing fixes.
pub(super) fn report_unsolved(env: &mut Env<'_>) {
    for obligation in env.infcx.take_obligations() {
        let Some(predicate) = resolved(env, &obligation) else { continue };
        // A variable already reported by `report_uninferred` was poisoned and resolves to an error.
        if has_unknown(env, &predicate) {
            continue;
        }
        let message = format!(
            "cannot check the bound `{}` of `{}`: a type is not known; add a type annotation",
            env.show_predicate(&predicate),
            obligation.origin
        );
        env.error(message, obligation.span);
    }
}

fn resolved(env: &mut Env<'_>, obligation: &Obligation) -> Option<Predicate<TyId>> {
    let Predicate { self_ty, trait_ref } = &obligation.predicate;
    let self_ty = env.infcx.resolve(env.tcx, *self_ty).ok()?;
    let args = trait_ref.args.iter().map(|&a| env.infcx.resolve(env.tcx, a)).collect::<Result<_, _>>().ok()?;
    Some(Predicate { self_ty, trait_ref: crate::ty::TraitRef { trait_id: trait_ref.trait_id, args } })
}

fn has_unknown(env: &Env<'_>, predicate: &Predicate<TyId>) -> bool {
    std::iter::once(&predicate.self_ty).chain(&predicate.trait_ref.args).any(|&t| env.tcx.is_unknown(t))
}

fn solve_one(env: &mut Env<'_>, obligation: &Obligation) -> Outcome {
    let Some(predicate) = resolved(env, obligation) else { return Outcome::Solved(Vec::new()) };
    // Already reported: do not pile a second error on an unknown type.
    if has_unknown(env, &predicate) {
        return Outcome::Solved(Vec::new());
    }
    if obligation.depth > MAX_DEPTH {
        let message = format!(
            "overflow evaluating the requirement `{}`: bounds are followed at most {MAX_DEPTH} levels deep",
            env.show_predicate(&predicate)
        );
        return Outcome::Failed(message);
    }
    match env.tcx.data(predicate.self_ty).clone() {
        TyData::Infer(_) => Outcome::Stalled,
        TyData::Param(_) => solve_from_bounds(env, &predicate),
        TyData::Class(..) | TyData::Enum(..) => solve_from_impls(env, &predicate, obligation),
        _ => Outcome::Failed(unsatisfied(env, &predicate, "only classes and enums implement traits yet")),
    }
}

/// The function's own bound `T: Trait<..>` whose arguments unify with the requirement's.
fn solve_from_bounds(env: &mut Env<'_>, predicate: &Predicate<TyId>) -> Outcome {
    let candidates: Vec<Predicate<TyId>> = env
        .param_env
        .iter()
        .filter(|p| p.self_ty == predicate.self_ty && p.trait_ref.trait_id == predicate.trait_ref.trait_id)
        .cloned()
        .collect();
    for candidate in candidates {
        let snap = env.infcx.snapshot();
        let unified = candidate
            .trait_ref
            .args
            .iter()
            .zip(&predicate.trait_ref.args)
            .all(|(&a, &b)| env.infcx.unify(env.tcx, a, b).is_ok());
        if unified {
            env.infcx.commit(snap);
            return Outcome::Solved(Vec::new());
        }
        env.infcx.rollback_to(snap);
    }
    Outcome::Failed(unsatisfied(env, predicate, "a type parameter only implements the traits in its bounds"))
}

/// An impl header instantiated with fresh variables, unified with the requirement.
/// `commit` keeps the bindings and returns the impl's own bounds as nested requirements.
fn try_impl(
    env: &mut Env<'_>,
    info: &super::traits::ImplInfo,
    predicate: &Predicate<TyId>,
    commit: bool,
) -> Option<Vec<Predicate<TyId>>> {
    let snap = env.infcx.snapshot();
    let header = (|| {
        let (self_ty, trait_args, nested) = info.instantiate(&mut env.infcx, env.tcx)?;
        env.infcx.unify(env.tcx, self_ty, predicate.self_ty).ok()?;
        for (&want, &have) in trait_args.iter().zip(&predicate.trait_ref.args) {
            env.infcx.unify(env.tcx, want, have).ok()?;
        }
        Some(nested)
    })();
    match header {
        Some(nested) if commit => {
            env.infcx.commit(snap);
            Some(nested)
        }
        Some(_) => {
            env.infcx.rollback_to(snap);
            Some(Vec::new())
        }
        None => {
            env.infcx.rollback_to(snap);
            None
        }
    }
}

fn solve_from_impls(env: &mut Env<'_>, predicate: &Predicate<TyId>, obligation: &Obligation) -> Outcome {
    let impls = env.impls;
    let all: &[super::traits::ImplInfo] = impls.get(&predicate.trait_ref.trait_id).map_or(&[], Vec::as_slice);
    let matching: Vec<&super::traits::ImplInfo> =
        all.iter().filter(|info| try_impl(env, info, predicate, false).is_some()).collect();
    match matching.as_slice() {
        [] => {
            let known: Vec<String> = all.iter().map(|info| impl_self(env, info)).collect();
            let hint = if known.is_empty() {
                "it has no implementations".to_string()
            } else {
                format!("it is implemented for: {}", known.join(", "))
            };
            Outcome::Failed(unsatisfied(env, predicate, &hint))
        }
        [only] => {
            let nested = try_impl(env, only, predicate, true).expect("the impl unified a moment ago");
            let depth = obligation.depth + 1;
            Outcome::Solved(
                nested
                    .into_iter()
                    .map(|predicate| Obligation { predicate, span: obligation.span, depth, origin: obligation.origin.clone() })
                    .collect(),
            )
        }
        // Impls cannot overlap (coherence), so several can match only while a variable is still open.
        _ => Outcome::Stalled,
    }
}

/// An impl's self type with the impl's own parameter names (`Box<T>`).
fn impl_self(env: &Env<'_>, info: &super::traits::ImplInfo) -> String {
    env.tcx.display_with(info.self_ty, &info.generics).to_string()
}

fn unsatisfied(env: &Env<'_>, predicate: &Predicate<TyId>, hint: &str) -> String {
    format!("the trait bound `{}` is not satisfied; {hint}", env.show_predicate(predicate))
}
