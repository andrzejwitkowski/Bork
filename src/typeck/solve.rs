//! The trait solver (#56): decides the obligations that calls register in `InferCtxt`.
//!
//! An obligation `Self: Trait<args>` is solved from the function's own bounds when `Self` is a type
//! parameter, and from the impl table when it is a class or enum. A requirement whose type is still
//! an unbound variable waits. There is no negative reasoning and no specialization.

use std::collections::{HashMap, HashSet};

use super::env::{Env, PendingProjection};
use crate::infer::Obligation;
use crate::span::Span;
use crate::ty::{Predicate, ProjectionPredicate, TyData, TyId};

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
        progress |= solve_projections(env);
        if !progress {
            return;
        }
    }
}

/// What is still pending at the end of the body is waiting for a type that nothing fixes.
pub(super) fn report_unsolved(env: &mut Env<'_>) {
    for pending in std::mem::take(&mut env.projections) {
        let Ok(projection) = env.infcx.resolve(env.tcx, pending.projection) else { continue };
        if !env.tcx.is_unknown(projection) {
            let message = format!("cannot normalize `{}`: a type is not known; add a type annotation", env.show(projection));
            env.error(message, pending.span);
        }
    }
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

/// An impl header instantiated with fresh variables and unified with `self_ty` and, when known,
/// the trait arguments. Returns the impl's own bounds; the bindings stay until the caller rolls back.
fn unify_header(
    env: &mut Env<'_>,
    info: &super::traits::ImplInfo,
    self_ty: TyId,
    trait_args: Option<&[TyId]>,
) -> Option<Vec<Predicate<TyId>>> {
    let (head, args, nested) = info.instantiate(&mut env.infcx, env.tcx)?;
    env.infcx.unify(env.tcx, head, self_ty).ok()?;
    for (&want, &have) in args.iter().zip(trait_args.unwrap_or(&[])) {
        env.infcx.unify(env.tcx, want, have).ok()?;
    }
    Some(nested)
}

/// Whether the impl could apply to `self_ty` for some trait arguments. Leaves no bindings.
pub(super) fn impl_applies(env: &mut Env<'_>, info: &super::traits::ImplInfo, self_ty: TyId) -> bool {
    let snap = env.infcx.snapshot();
    let applies = unify_header(env, info, self_ty, None).is_some();
    env.infcx.rollback_to(snap);
    applies
}

/// `commit` keeps the bindings and returns the impl's own bounds as nested requirements.
fn try_impl(
    env: &mut Env<'_>,
    info: &super::traits::ImplInfo,
    predicate: &Predicate<TyId>,
    commit: bool,
) -> Option<Vec<Predicate<TyId>>> {
    let snap = env.infcx.snapshot();
    let header = unify_header(env, info, predicate.self_ty, Some(&predicate.trait_ref.args));
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

/// What a projection `base::name` turned out to be.
enum Projected {
    Done(TyId),
    /// A type parameter's own associated type: no further than itself.
    Rigid,
    /// Its base is still an unbound variable.
    Stalled,
    Failed(String),
}

/// Registers the equality bounds `Item = X` of a callee, instantiated with `args`.
pub(super) fn register_projections(
    env: &mut Env<'_>,
    predicates: &[ProjectionPredicate<TyId>],
    args: &[TyId],
    span: Span,
) {
    for predicate in predicates {
        let Ok(ProjectionPredicate { projection, ty }) = predicate.subst(env.tcx, args) else { continue };
        let actual = normalize(env, projection, Some(span));
        let ty = normalize(env, ty, Some(span));
        unify_projection(env, actual, ty, Some(span));
    }
}

fn unify_projection(env: &mut Env<'_>, actual: TyId, wanted: TyId, span: Option<Span>) {
    if env.infcx.unify(env.tcx, actual, wanted).is_err() {
        let message = format!("expected the associated type to be `{}`, but it is `{}`", env.show(wanted), env.show(actual));
        env.error(message, span);
    }
}

/// Retries the pending projections; true when one was settled.
fn solve_projections(env: &mut Env<'_>) -> bool {
    let mut progress = false;
    for pending in std::mem::take(&mut env.projections) {
        let Ok(projection) = env.infcx.resolve(env.tcx, pending.projection) else { continue };
        let TyData::Projection(base, trait_name, name) = env.tcx.data(projection).clone() else {
            // The base turned out to be an error, which collapsed the projection.
            progress = true;
            unify_projection(env, pending.var, projection, pending.span);
            continue;
        };
        let found = match project(env, projection, base, trait_name.as_deref(), &name) {
            Projected::Stalled => {
                env.projections.push(pending);
                continue;
            }
            Projected::Done(ty) => normalize(env, ty, pending.span),
            Projected::Rigid => projection,
            Projected::Failed(message) => {
                env.error(message, pending.span);
                env.tcx.error()
            }
        };
        progress = true;
        unify_projection(env, found, pending.var, pending.span);
    }
    progress
}

/// `ty` with every projection replaced by what it names. A projection of a type parameter that is
/// only bounded stays; one whose base is not yet known becomes a variable that is solved later.
pub(super) fn normalize(env: &mut Env<'_>, ty: TyId, span: Option<Span>) -> TyId {
    if !env.tcx.has_projection(ty) {
        return ty;
    }
    let mut ty = ty;
    let mut rigid = HashSet::new();
    let mut replaced_any = false;
    for _ in 0..=MAX_DEPTH {
        // Leaves are keyed by their interned id, so the type must carry the resolved bases.
        ty = env.infcx.resolve(env.tcx, ty).unwrap_or(ty);
        let mut leaves = Vec::new();
        collect_leaves(env, ty, &rigid, &mut leaves);
        if leaves.is_empty() {
            // The writer's `lower_class` check could not see through `Holder<W::Item>` (RFC 0003 section 2).
            if replaced_any {
                if let Some(message) = env.tcx.borrow_type_arg_in(ty) {
                    env.error(message, span);
                    return env.tcx.error();
                }
            }
            return ty;
        }
        replaced_any = true;
        let mut replaced = HashMap::new();
        for leaf in leaves {
            let TyData::Projection(base, trait_name, name) = env.tcx.data(leaf).clone() else { continue };
            match project(env, leaf, base, trait_name.as_deref(), &name) {
                Projected::Done(found) => {
                    replaced.insert(leaf, found);
                }
                Projected::Rigid => {
                    rigid.insert(leaf);
                }
                Projected::Stalled => {
                    let var = env.infcx.new_var(env.tcx, crate::infer::VarKind::General);
                    env.projections.push(PendingProjection { projection: leaf, var, span });
                    replaced.insert(leaf, var);
                }
                Projected::Failed(message) => {
                    env.error(message, span);
                    replaced.insert(leaf, env.tcx.error());
                }
            }
        }
        let Ok(next) = crate::ty::subst_with(env.tcx, ty, &replaced) else {
            let message = format!("`{}`: an associated type here would make a nullable of a nullable", env.show(ty));
            env.error(message, span);
            return env.tcx.error();
        };
        ty = next;
    }
    let message = format!(
        "overflow normalizing `{}`: associated types are followed at most {MAX_DEPTH} levels deep",
        env.show(ty)
    );
    env.error(message, span);
    env.tcx.error()
}

/// The projections in `ty` not yet known to be rigid whose base holds no other such projection.
fn collect_leaves(env: &Env<'_>, ty: TyId, rigid: &HashSet<TyId>, out: &mut Vec<TyId>) {
    let before = out.len();
    for child in env.tcx.data(ty).children() {
        collect_leaves(env, child, rigid, out);
    }
    if matches!(env.tcx.data(ty), TyData::Projection(..)) && !rigid.contains(&ty) && out.len() == before && !out.contains(&ty) {
        out.push(ty);
    }
}

fn project(env: &mut Env<'_>, projection: TyId, base: TyId, trait_name: Option<&str>, name: &str) -> Projected {
    match env.tcx.data(base).clone() {
        TyData::Infer(_) => Projected::Stalled,
        TyData::Error => Projected::Done(base),
        TyData::Param(_) => project_param(env, projection, base, trait_name, name),
        // A projection of a projection (`T::A::B`) has no bounds to look in.
        TyData::Projection(..) => Projected::Rigid,
        TyData::Class(..) | TyData::Enum(..) => project_from_impls(env, base, trait_name, name),
        _ => Projected::Failed(format!(
            "cannot normalize `{}::{name}`: only classes and enums implement traits yet",
            env.show(base)
        )),
    }
}

/// A type parameter's projection is fixed by an equality bound, else it is its own rigid type.
/// Signatures are already qualified (`qualify_sig`); a type written in a body is qualified here.
fn project_param(env: &mut Env<'_>, projection: TyId, base: TyId, trait_name: Option<&str>, name: &str) -> Projected {
    if let Some(bound) = env.proj_env.iter().find(|p| p.projection == projection) {
        return Projected::Done(bound.ty);
    }
    let mut declaring: Vec<_> = env
        .param_env
        .iter()
        .filter(|p| p.self_ty == base && env.traits[&p.trait_ref.trait_id].assoc_types.iter().any(|n| n == name))
        .map(|p| env.defs.get(p.trait_ref.trait_id).name.clone())
        .filter(|t| trait_name.is_none_or(|want| want == t))
        .collect();
    declaring.sort();
    declaring.dedup();
    match (declaring.as_slice(), trait_name) {
        (_, Some(_)) if declaring.len() == 1 => Projected::Rigid,
        ([only], None) => Projected::Done(env.tcx.projection(base, Some(only), name)),
        ([], _) => Projected::Failed(format!(
            "no bound on `{}` declares an associated type `{name}`; add a bound such as `{}: Trait`",
            env.show(base),
            env.show(base)
        )),
        _ => Projected::Failed(format!(
            "`{0}::{name}` is ambiguous: the bounds {1} on `{0}` all declare `{name}`",
            env.show(base),
            declaring.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn project_from_impls(env: &mut Env<'_>, base: TyId, trait_name: Option<&str>, name: &str) -> Projected {
    let (impls, traits) = (env.impls, env.traits);
    let mut candidates: Vec<(&str, &super::traits::ImplInfo)> = Vec::new();
    for (trait_id, infos) in impls {
        let declared = &env.defs.get(*trait_id).name;
        if !traits[trait_id].assoc_types.iter().any(|n| n == name) || trait_name.is_some_and(|want| want != declared) {
            continue;
        }
        for info in infos {
            if impl_applies(env, info, base) {
                candidates.push((declared, info));
            }
        }
    }
    candidates.sort_by_key(|(trait_name, info)| (*trait_name, info.index));
    let shown = env.show(base).to_string();
    // With a variable in `base`, a later statement may still pick the impl.
    if candidates.len() != 1 && super::has_infer(env.tcx, base) {
        return Projected::Stalled;
    }
    match candidates.as_slice() {
        [] => Projected::Failed(format!(
            "cannot normalize `{shown}::{name}`: `{shown}` implements no trait with an associated type `{name}`"
        )),
        [(_, info)] => {
            let Some(&value) = info.assoc.get(name) else { return Projected::Done(env.tcx.error()) };
            // Bind the impl's variables only to read its value; they must not constrain `base`.
            let snap = env.infcx.snapshot();
            let fresh: Vec<TyId> = info.generics.iter().map(|_| env.infcx.new_var(env.tcx, crate::infer::VarKind::General)).collect();
            let value = match (crate::ty::subst(env.tcx, info.self_ty, &fresh), crate::ty::subst(env.tcx, value, &fresh)) {
                (Ok(head), Ok(value)) if env.infcx.unify(env.tcx, head, base).is_ok() => env.infcx.resolve(env.tcx, value).ok(),
                _ => None,
            };
            env.infcx.rollback_to(snap);
            // A variable the header leaves open would outlive the rollback.
            let Some(value) = value.filter(|&v| !fresh.iter().any(|&f| mentions(env.tcx, v, f))) else {
                return Projected::Failed(format!(
                    "cannot normalize `{shown}::{name}`: a type parameter of the impl is not determined by its type"
                ));
            };
            Projected::Done(value)
        }
        many => Projected::Failed(format!(
            "`{shown}::{name}` is ambiguous: it is declared by {}",
            many.iter().map(|(t, _)| format!("`{t}`")).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn mentions(tcx: &crate::ty::TyCtx, ty: TyId, var: TyId) -> bool {
    ty == var || tcx.data(ty).children().into_iter().any(|c| mentions(tcx, c, var))
}
