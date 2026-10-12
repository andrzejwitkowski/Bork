//! Method calls: `recv.m(args)` and bare `m(args)` inside a method.

use crate::ast::{Closure, Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind, UseKind};
use crate::span::Span;

use super::super::env::Env;
use super::super::MemberRef;
use super::call::finish_call;
use super::check;

/// `receiver.method(args)`; the receiver is borrowed for the call.
pub(super) fn check_method_call(
    receiver: &Expr,
    method: &str,
    safe: bool,
    span: Span,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    if !is_place(receiver) {
        env.error(
            "method receiver must be a named value; bind it to a `val` first",
            Some(span),
        );
        return poisoned(args, span, return_ty, env);
    }
    let checked = check(receiver, None, return_ty, env);
    if checked.ty.is_unknown() {
        return poisoned(args, span, return_ty, env);
    }
    if checked.ty.is_managed_ref() {
        env.error("method call on managed Ref requires `if val` or `when`", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    if checked.ty.is_nullable() || safe {
        env.error("method call on a nullable receiver", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    let Some(class) = checked.ty.record_name().map(str::to_string) else {
        env.error("only named functions can be called", Some(span));
        return poisoned(args, span, return_ty, env);
    };
    let qualified = format!("{class}.{method}");
    let (Some(sig), Some(signature)) = (
        env.fun_sig(&qualified).cloned(),
        env.class(&class).and_then(|info| info.methods.get(method).cloned()),
    ) else {
        env.error(format!("class `{class}` has no method `{method}`"), Some(span));
        return poisoned(args, span, return_ty, env);
    };
    env.members.insert(
        span,
        MemberRef::Method {
            class,
            name: method.to_string(),
            signature,
        },
    );
    finish_call(
        qualified,
        span,
        sig.params,
        sig.return_ty,
        Some(borrow(checked)),
        args,
        trailing,
        return_ty,
        env,
    )
}

/// `name(.field)*` — something that has an address the method can borrow.
fn is_place(expr: &Expr) -> bool {
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

fn borrow(receiver: HirExpr) -> HirExpr {
    let ty = if receiver.ty.is_ref() {
        receiver.ty.clone()
    } else {
        Ty::new(TyKind::Ref(Box::new(receiver.ty.clone())), false)
    };
    let span = receiver.span;
    let receiver = match receiver.kind {
        HirExprKind::Ident { name, .. } => HirExpr {
            kind: HirExprKind::Ident {
                name,
                use_kind: UseKind::Borrow,
            },
            ..receiver
        },
        _ => receiver,
    };
    HirExpr {
        kind: HirExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(receiver),
        },
        ty,
        span,
    }
}

fn poisoned(args: &[Expr], span: Span, return_ty: &Ty, env: &mut Env<'_>) -> HirExpr {
    for arg in args {
        check(arg, None, return_ty, env);
    }
    HirExpr::spanned(HirExprKind::None, Ty::unknown(), span)
}
