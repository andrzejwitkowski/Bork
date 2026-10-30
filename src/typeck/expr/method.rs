//! Method calls: `recv.m(args)` and bare `m(args)` inside a method.

use crate::ast::{Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, UseKind};
use crate::span::{AstId, Span};
use crate::defs::DefId;
use crate::ty::{FnSig, Predicate, TraitRef, TyData, TyId};

use super::super::env::{Checked, Env};
use super::super::MemberRef;
use super::call::{finish_call, fresh_params, subst_sig, CallSite};
use super::synthesize;

/// `receiver.method(args)`; the receiver is borrowed for the call.
pub(super) fn check_method_call(
    receiver: &Expr,
    method: &str,
    safe: bool,
    id: AstId,
    site: CallSite<'_>,
    env: &mut Env<'_>,
) -> Checked {
    let CallSite { span, args, return_ty, .. } = site;
    if !is_place(receiver) {
        env.error(
            "method receiver must be a named value; bind it to a `val` first",
            Some(span),
        );
        return poisoned(args, span, return_ty, env);
    }
    let checked = synthesize(receiver, return_ty, env);
    if env.tcx.is_unknown(checked.ty) {
        return poisoned(args, span, return_ty, env);
    }
    if env.tcx.is_managed_ref(checked.ty) {
        env.error("method call on managed Ref requires `if val` or `when`", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    if env.tcx.is_nullable(checked.ty) || safe {
        env.error("method call on a nullable receiver", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    if let Some(elem) = env.tcx.vec_elem(checked.ty) {
        let borrowed = borrow(checked, env);
        return super::vec::check_vec_method(receiver, borrowed, elem, method, id, site, env);
    }
    // An inherent method shadows a trait method of the same name.
    let inherent = env.tcx.record_def(checked.ty).filter(|&class| env.defs.lookup_method(class, method).is_some());
    let Some(class) = inherent else {
        return check_trait_method_call(checked, method, id, site, env);
    };
    let class_name = env.tcx.class_name(class).to_string();
    let method_def = env.defs.lookup_method(class, method);
    let (Some(sig), Some(signature)) = (
        method_def.and_then(|def| env.fun_sigs.get(&def)).cloned(),
        method_def.and_then(|def| env.class(class)?.methods.get(&def).cloned()),
    ) else {
        env.error(format!("class `{class_name}` has no method `{method}`"), Some(span));
        return poisoned(args, span, return_ty, env);
    };
    env.members.insert(
        id.numbered(),
        MemberRef::Method {
            class: class_name.clone(),
            name: method.to_string(),
            signature,
        },
    );
    let name = format!("{class_name}.{method}");
    let Some((params, ret)) = instantiate_method(&name, class, &sig, &checked, span, env) else {
        return poisoned(args, span, return_ty, env);
    };
    let receiver = borrow(checked, env);
    finish_call(name, params, ret, Some(receiver), site, env)
}

/// A trait that provides the method being called, with the trait arguments of the bound or impl.
struct TraitCandidate {
    trait_id: DefId,
    args: Vec<TyId>,
}

/// `recv.m(args)` where `m` is not an inherent method: a trait method from the receiver's bounds
/// (type parameter) or from an impl (class or enum), compiled to a call of `Trait.m`.
fn check_trait_method_call(
    checked: Checked,
    method: &str,
    id: AstId,
    site: CallSite<'_>,
    env: &mut Env<'_>,
) -> Checked {
    let CallSite { span, args, return_ty, .. } = site;
    let self_ty = env.tcx.ref_inner(checked.ty).unwrap_or(checked.ty);
    let candidates = trait_candidates(self_ty, method, env);
    let [candidate] = candidates.as_slice() else {
        report_unresolved_method(&checked, self_ty, method, &candidates, span, env);
        return poisoned(args, span, return_ty, env);
    };
    let trait_id = candidate.trait_id;
    let trait_name = env.defs.get(trait_id).name.clone();
    let info = &env.traits[&trait_id];
    let tm = info.methods.iter().find(|m| m.name == method).expect("the candidate declares the method");
    let (own_names, mut sig_params, sig_ret) = (tm.generic_names.clone(), tm.params.clone(), tm.ret);
    let this_ty = env.tcx.param(crate::ty::ParamIdx(0));
    sig_params.insert(0, env.tcx.ref_to(this_ty));
    let name = format!("{trait_name}.{method}");
    // `Self`, the trait's arguments, then the method's own parameters, as the signature was lowered.
    let mut subst_args = vec![self_ty];
    subst_args.extend(&candidate.args);
    subst_args.extend(super::call::fresh_params(&own_names, &name, span, env));
    let mut sig = FnSig::new(sig_params, sig_ret);
    sig.generics = (0..subst_args.len() as u32).map(crate::ty::ParamIdx).collect();
    sig.generic_names = subst_args.iter().map(|_| String::new()).collect();
    let Some((params, ret)) = subst_sig(&name, span, &sig, &subst_args, env) else {
        return poisoned(args, span, return_ty, env);
    };
    let required = Predicate { self_ty, trait_ref: TraitRef { trait_id, args: candidate.args.clone() } };
    super::super::solve::require(env, required, span, &name);
    let shown: Vec<String> = params[1..].iter().map(|&p| env.show(p).to_string()).collect();
    let signature = format!("fun {name}({}): {}", shown.join(", "), env.show(ret));
    env.members.insert(id.numbered(), MemberRef::Method { class: trait_name, name: method.to_string(), signature });
    let receiver = borrow(checked, env);
    finish_call(name, params, ret, Some(receiver), site, env)
}

/// The traits that could provide `method` for `self_ty`, in a stable order.
fn trait_candidates(self_ty: TyId, method: &str, env: &mut Env<'_>) -> Vec<TraitCandidate> {
    let mut candidates = Vec::new();
    if env.is_param(self_ty) {
        for bound in env.param_env.iter().filter(|p| p.self_ty == self_ty) {
            if env.traits[&bound.trait_ref.trait_id].methods.iter().any(|m| m.name == method) {
                candidates.push(TraitCandidate { trait_id: bound.trait_ref.trait_id, args: bound.trait_ref.args.clone() });
            }
        }
        return candidates;
    }
    if !matches!(env.tcx.data(self_ty), TyData::Class(..) | TyData::Enum(..)) {
        return candidates;
    }
    let mut trait_ids: Vec<DefId> = env.traits.keys().copied().collect();
    trait_ids.sort_by_key(|id| env.defs.get(*id).name.clone());
    for trait_id in trait_ids {
        let info = &env.traits[&trait_id];
        if !info.methods.iter().any(|m| m.name == method) {
            continue;
        }
        let arity = info.generics.len();
        let impls = env.impls;
        let applies = impls.get(&trait_id).is_some_and(|all| {
            all.iter().any(|imp| {
                let snap = env.infcx.snapshot();
                let unified = imp
                    .instantiate(&mut env.infcx, env.tcx)
                    .is_some_and(|(head, _, _)| env.infcx.unify(env.tcx, head, self_ty).is_ok());
                env.infcx.rollback_to(snap);
                unified
            })
        });
        if applies {
            let args = (0..arity).map(|_| env.infcx.new_var(env.tcx, crate::infer::VarKind::General)).collect();
            candidates.push(TraitCandidate { trait_id, args });
        }
    }
    candidates
}

fn report_unresolved_method(
    checked: &Checked,
    self_ty: TyId,
    method: &str,
    candidates: &[TraitCandidate],
    span: Span,
    env: &mut Env<'_>,
) {
    if candidates.len() > 1 {
        let names: Vec<String> = candidates.iter().map(|c| format!("`{}`", env.defs.get(c.trait_id).name)).collect();
        env.error(format!("ambiguous method `{method}`: it is provided by the traits {}", names.join(", ")), Some(span));
    } else if env.is_param(self_ty) && env.param_env.iter().any(|p| p.self_ty == self_ty) {
        let bounds: Vec<String> = env
            .param_env
            .iter()
            .filter(|p| p.self_ty == self_ty)
            .map(|p| format!("`{}`", env.defs.get(p.trait_ref.trait_id).name))
            .collect();
        env.error(
            format!("type parameter `{}` has no method `{method}`; its bounds are: {}", env.show(self_ty), bounds.join(", ")),
            Some(span),
        );
    } else if let Some(class) = env.tcx.record_def(checked.ty) {
        env.error(format!("class `{}` has no method `{method}`", env.tcx.class_name(class)), Some(span));
    } else {
        let hint = env.bounds_hint(&[self_ty]);
        env.error(format!("only named functions can be called{hint}"), Some(span));
    }
}

/// The signature of a call to the method `sig` of `class`: the receiver fixes the class parameters
/// (they come first in `sig.generics`), and the method's own parameters are fresh variables that
/// the arguments and the expected type fix. `None` when the instantiation is unusable (reported).
fn instantiate_method(
    name: &str,
    class: DefId,
    sig: &FnSig<TyId>,
    receiver: &Checked,
    span: Span,
    env: &mut Env<'_>,
) -> Option<(Vec<TyId>, TyId)> {
    if sig.generics.is_empty() {
        return Some((sig.params.clone(), sig.ret));
    }
    let class_arity = env.class(class).map_or(0, |info| info.generics.len());
    let mut args = env.tcx.class_args(receiver.ty).map(|(_, args)| args.to_vec()).unwrap_or_default();
    args.extend(fresh_params(&sig.generic_names[class_arity..], name, span, env));
    subst_sig(name, span, sig, &args, env)
}

/// `name(.field)*` — something that has an address the method can borrow.
pub(super) fn is_place(expr: &Expr) -> bool {
    match expr {
        Expr::Ident { .. } => true,
        Expr::Field {
            receiver,
            safe: false,
            ..
        } => is_place(receiver),
        _ => false,
    }
}

fn borrow(receiver: Checked, env: &mut Env<'_>) -> Checked {
    let ty = if env.tcx.is_ref(receiver.ty) {
        receiver.ty
    } else {
        env.tcx.ref_to(receiver.ty)
    };
    let span = receiver.hir.span;
    let receiver = match receiver.hir.kind {
        HirExprKind::Ident { name, .. } => HirExpr {
            kind: HirExprKind::Ident {
                name,
                use_kind: UseKind::Borrow,
            },
            ..receiver.hir
        },
        _ => receiver.hir,
    };
    Checked {
        hir: HirExpr {
            id: crate::hir::NodeId::DUMMY,
            kind: HirExprKind::Unary {
                op: UnaryOp::Borrow,
                expr: Box::new(receiver),
            },
            ty,
            span,
        },
        ty,
    }
}

pub(super) fn poisoned(args: &[Expr], span: Span, return_ty: TyId, env: &mut Env<'_>) -> Checked {
    for arg in args {
        synthesize(arg, return_ty, env);
    }
    let unknown = env.tcx.error();
    env.expr_at(HirExprKind::None, unknown, span)
}

/// Bare `name` inside a method that names a field of the enclosing class: `this.name`.
pub(super) fn implicit_field(name: &str, span: Span, id: AstId, return_ty: TyId, env: &mut Env<'_>) -> Option<Checked> {
    env.resolve_bare_field(name, id)?;
    let this = Expr::Ident {
        name: "this".to_string(),
        span,
        id,
    };
    Some(super::field::check_field(&this, name, false, span, return_ty, env))
}
