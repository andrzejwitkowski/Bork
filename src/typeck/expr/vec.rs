//! Built-in `Vec<T>`: construction and methods.

use crate::ast::{BindingKind, Expr, Type};
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind, VecOp};
use crate::span::Span;

use super::super::env::Env;
use super::super::MemberRef;
use super::check;

/// `Vec<T>()` and `Vec<T>(capacity)`.
pub(super) fn check_vec_new(
    elem: &Type,
    cap: Option<&Expr>,
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    // Lower the whole `Vec<T>` type so the element rule is enforced in one place.
    let lowered = super::super::lower_type(
        &Type::Vec {
            elem: Box::new(elem.clone()),
        },
        env.classes,
        &mut env.diagnostics,
        Some(span),
    );
    let elem_ty = lowered.vec_elem().cloned().unwrap_or_else(Ty::unknown);
    let cap = cap.map(|cap| {
        let cap = check(cap, Some(&Ty::i32()), return_ty, env);
        if !cap.ty.is_unknown() && cap.ty != Ty::i32() {
            env.error(
                format!("`Vec` capacity must be `i32`, got {}", cap.ty),
                cap.span.or(Some(span)),
            );
        }
        Box::new(cap)
    });
    let ty = Ty::new(TyKind::Vec(Box::new(elem_ty.clone())), false);
    HirExpr::spanned(HirExprKind::VecNew { elem: elem_ty, cap }, ty, span)
}

/// `receiver.method(args)` on a `Vec<elem>`; `borrowed` is the already-borrowed receiver.
pub(super) fn check_vec_method(
    receiver: &Expr,
    borrowed: HirExpr,
    elem: Ty,
    method: &str,
    span: Span,
    id: crate::span::AstId,
    args: &[Expr],
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
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
            signature: format!("Vec<{elem}>.{method}"),
        },
    );
    let (params, result): (Vec<Ty>, Ty) = match op {
        VecOp::Push => (vec![elem], Ty::unit()),
        VecOp::Pop if elem.is_managed_ref() => (vec![], elem),
        VecOp::Pop => (vec![], elem.with_nullable(true)),
        VecOp::Get => (vec![Ty::i32()], elem),
        VecOp::Set => (vec![Ty::i32(), elem], Ty::unit()),
        VecOp::Len | VecOp::Capacity => (vec![], Ty::i32()),
        VecOp::Clear => (vec![], Ty::unit()),
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
        .map(|(arg, param)| {
            let arg = check(arg, Some(param), return_ty, env);
            if !arg.ty.is_unknown() && &arg.ty != param {
                env.error(
                    format!("`Vec.{method}` argument has type {}, expected {param}", arg.ty),
                    arg.span.or(Some(span)),
                );
            }
            arg
        })
        .collect();
    HirExpr::spanned(
        HirExprKind::VecOp {
            op,
            receiver: Box::new(borrowed),
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
    if binding.kind == BindingKind::Val && !binding.ty.is_ref() && !binding.ty.is_managed_ref() {
        let via = if std::ptr::eq(root, receiver) { "" } else { " (through a field)" };
        env.error(format!("cannot mutate immutable `val` binding `{name}`{via}"), Some(span));
    }
}
