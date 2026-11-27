//! Typeck rules of the prelude's `Vec<T>` (a class written in Bork, #285) that an ordinary class does not have.

use crate::ast::{BindingKind, Expr};
use crate::hir::HirExprKind;
use crate::span::Span;

use super::super::env::{Checked, Env};

/// The closure stored by `push` / `set` on the Bork `Vec` lives in the vector's arena, so only a closure literal on a named
/// vector (or one that captures nothing) may be stored, and not one that borrows its captures.
pub(super) fn check_stored_closure(receiver: &Expr, call: &Checked, span: Span, env: &mut Env<'_>) {
    let HirExprKind::Call { args, .. } = &call.hir.kind else { return };
    let Some(arg) = args.last().filter(|a| env.tcx.is_callable(a.ty)) else { return };
    let value = Checked { hir: arg.clone(), ty: arg.ty };
    env.reject_stored_view_closure(&value, "a `Vec`", arg.span.or(Some(span)));
    let owned = matches!(arg.kind, HirExprKind::Closure { .. }) && is_named(receiver);
    let captureless = matches!(env.tcx.data(arg.ty), crate::ty::TyData::Closure { captures, .. } if captures.is_empty());
    if !owned && !captureless && !env.tcx.is_unknown(arg.ty) {
        env.error("a closure that captures can only be pushed into a `Vec` as a closure literal on a named vector; its environment must live in the vector's arena", arg.span.or(Some(span)));
    }
}

/// A plain variable (`v`, `&rw v`), whose arena the vector header names at run time; a field has no local.
fn is_named(receiver: &Expr) -> bool {
    match receiver {
        Expr::Ident { .. } => true,
        Expr::Unary { expr, .. } => is_named(expr),
        _ => false,
    }
}

/// A `val` vector cannot be mutated, whether it is the binding itself or reached through fields
/// of a `val` object (`h.items.push(1)`). `this`, borrows and `Ref` handles are exempt, and
/// temporaries (call results) have no binding to protect.
pub(super) fn reject_immutable(receiver: &Expr, env: &mut Env<'_>, span: Span) {
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
