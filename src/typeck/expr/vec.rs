//! Built-in `Vec<T>`: construction and methods.

use crate::ast::{BindingKind, Expr, Type};
use crate::hir::{HirExpr, HirExprKind, VecOp};
use crate::span::Span;
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::super::MemberRef;
use super::check;

/// `Vec<T>()` and `Vec<T>(capacity)`.
pub(super) fn check_vec_new(
    elem: &Type,
    cap: Option<&Expr>,
    span: Span,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    // Lower the whole `Vec<T>` type so the element rule is enforced in one place.
    let lowered = env.lower(
        &Type::Vec {
            elem: Box::new(elem.clone()),
        },
        Some(span),
    );
    let elem_ty = match env.tcx.vec_elem(lowered) {
        Some(elem) => elem,
        None => env.tcx.error(),
    };
    let i32_ty = env.tcx.i32();
    let cap = cap.map(|cap| {
        let cap = check(cap, Some(i32_ty), return_ty, env);
        if !env.tcx.is_unknown(cap.ty) && cap.ty != i32_ty {
            env.error(
                format!("`Vec` capacity must be `i32`, got {}", env.show(cap.ty)),
                cap.span.or(Some(span)),
            );
        }
        Box::new(cap.hir)
    });
    let ty = env.tcx.vec_of(elem_ty);
    let elem = env.tcx.to_ty(elem_ty);
    env.expr_at(HirExprKind::VecNew { elem, cap }, ty, span)
}

/// `receiver.method(args)` on a `Vec<elem>`; `borrowed` is the already-borrowed receiver.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_vec_method(
    receiver: &Expr,
    borrowed: Checked,
    elem: TyId,
    method: &str,
    span: Span,
    id: crate::span::AstId,
    args: &[Expr],
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
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
    let (i32_ty, unit) = (env.tcx.i32(), env.tcx.unit());
    let (params, result): (Vec<TyId>, TyId) = match op {
        VecOp::Push => (vec![elem], unit),
        VecOp::Pop if env.tcx.is_managed_ref(elem) => (vec![], elem),
        VecOp::Pop => (vec![], env.tcx.with_nullable(elem, true)),
        VecOp::Get => (vec![i32_ty], elem),
        VecOp::Set => (vec![i32_ty, elem], unit),
        VecOp::Len | VecOp::Capacity => (vec![], i32_ty),
        VecOp::Clear => (vec![], unit),
    };
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
            let arg = check(arg, Some(param), return_ty, env);
            if !env.tcx.is_unknown(arg.ty) && arg.ty != param {
                env.error(
                    format!(
                        "`Vec.{method}` argument has type {}, expected {}",
                        env.show(arg.ty),
                        env.show(param)
                    ),
                    arg.span.or(Some(span)),
                );
            }
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
    if binding.kind == BindingKind::Val && !env.tcx.is_ref(binding.ty) && !env.tcx.is_managed_ref(binding.ty) {
        let via = if std::ptr::eq(root, receiver) { "" } else { " (through a field)" };
        env.error(format!("cannot mutate immutable `val` binding `{name}`{via}"), Some(span));
    }
}
