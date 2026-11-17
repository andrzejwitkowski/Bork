//! Built-in `Vec<T>`: construction and methods.

use crate::ast::{BindingKind, Expr, Type};
use crate::hir::{HirExpr, HirExprKind, VecOp};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::super::MemberRef;
use super::check;

/// `Vec::<T>()` and `Vec::<T>(capacity)`.
pub(super) fn check_vec_new(
    elem: &Type,
    cap: Option<&Expr>,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    // Lower the whole `Vec<T>` type so the element rule is enforced in one place.
    let lowered = env.lower(
        &Type::Named { name: "Vec".to_string(), args: vec![elem.clone()], nullable: false },
        Some(span),
    );
    let elem_ty = match env.tcx.vec_elem(lowered) {
        Some(elem) => elem,
        None => env.tcx.error(),
    };
    let i32_ty = env.tcx.i32();
    let cap = cap.map(|cap| {
        let cap = check(cap, i32_ty, return_ty, env);
        if !env.tcx.is_unknown(cap.ty) && !env.same(cap.ty, i32_ty) {
            env.error(
                format!("`Vec` capacity must be `i32`, got {}", env.show(cap.ty)),
                cap.span.or(Some(span)),
            );
        }
        Box::new(cap.hir)
    });
    let ty = env.tcx.vec_of(elem_ty);
    env.expr_at(HirExprKind::VecNew { elem: elem_ty, cap }, ty, span)
}

/// `receiver.method(args)` on a `Vec<elem>`; `borrowed` is the already-borrowed receiver.
pub(super) fn check_vec_method(
    receiver: &Expr,
    borrowed: Checked,
    elem: TyId,
    method: &str,
    id: crate::span::AstId,
    site: super::call::CallSite<'_>,
    env: &mut Env<'_>,
) -> Checked {
    let super::call::CallSite { span, args, return_ty, .. } = site;
    let Some(op) = VecOp::from_name(method) else {
        env.error(format!("`Vec` has no method `{method}`"), Some(span));
        return super::method::poisoned(args, span, return_ty, env);
    };
    if op.mutates() {
        reject_immutable(receiver, env, span);
    }
    env.members.insert(
        id.numbered(),
        MemberRef::Method {
            class: "Vec".to_string(),
            name: method.to_string(),
            signature: format!("Vec<{}>.{method}", env.show(elem)),
        },
    );
    // The signature is the prelude's `intrinsic fun`, with `T` replaced by the element type.
    let (params, mut result) = prelude_signature(method, elem, span, env);
    // A `Vec<Ref<T>>` pops the `Ref` itself, not an `Option` of it.
    if op == VecOp::Pop && env.tcx.is_managed_ref(elem) {
        result = elem;
    }
    if args.len() != params.len() {
        env.error(
            format!("`Vec.{method}` takes {} argument(s), got {}", params.len(), args.len()),
            Some(span),
        );
        return super::method::poisoned(args, span, return_ty, env);
    }
    let checked: Vec<HirExpr> = args
        .iter()
        .zip(&params)
        .map(|(arg, &param)| {
            let arg = check(arg, param, return_ty, env);
            env.mismatch(&format!("`Vec.{method}` argument"), arg.ty, param, arg.span.or(Some(span)));
            arg.hir
        })
        .collect();
    env.expr_at(
        HirExprKind::VecOp {
            op,
            receiver: Box::new(borrowed.hir),
            args: checked,
        },
        result,
        span,
    )
}

/// A `val` vector cannot be mutated, whether it is the binding itself or reached through fields
/// of a `val` object (`h.items.push(1)`). `this`, borrows and `Ref` handles are exempt, and
/// temporaries (call results) have no binding to protect.
fn reject_immutable(receiver: &Expr, env: &mut Env<'_>, span: Span) {
    let mut root = receiver;
    while let Expr::Field { receiver, .. } = root {
        root = receiver;
    }
    let Expr::Ident { name, .. } = root else {
        return;
    };
    if name == "this" {
        return;
    }
    let Some(binding) = env.binding(name) else {
        return;
    };
    let direct = std::ptr::eq(root, receiver);
    let (kind, ty) = (binding.kind, binding.ty);
    if kind == BindingKind::Val && !env.tcx.is_ref(ty) && !env.tcx.is_managed_ref(ty) {
        let via = if direct { "" } else { " (through a field)" };
        env.error(format!("cannot mutate immutable `val` binding `{name}`{via}"), Some(span));
    } else if env.tcx.is_ref(ty) && !env.tcx.is_rw_ref(ty) {
        let via = if direct { "" } else { " (through a field)" };
        env.error(format!("cannot mutate through the read-only borrow `{name}`{via}; take it as `&rw` (RFC 0003)"), Some(span));
    }
}

/// Parameters and result of `Vec<elem>.method`, from the prelude's `intrinsic class Vec<T>`.
fn prelude_signature(method: &str, elem: TyId, span: Span, env: &mut Env<'_>) -> (Vec<TyId>, TyId) {
    let sig = env
        .tcx
        .vec_def()
        .and_then(|class| env.defs.lookup_method(class, method))
        .and_then(|def| env.fun_sigs.get(&def))
        .cloned()
        .expect("the prelude's `Vec` declares every `VecOp`");
    let name = format!("Vec.{method}");
    let (mut params, result) = super::call::subst_sig(&name, span, &sig, &[elem], env).expect("a `Vec` signature substitutes");
    params.remove(0); // `this`
    (params, result)
}
