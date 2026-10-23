//! Method calls: `recv.m(args)` and bare `m(args)` inside a method.

use crate::ast::{Closure, Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, UseKind};
use crate::span::{AstId, Span};
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::super::MemberRef;
use super::call::finish_call;
use super::synthesize;

/// `receiver.method(args)`; the receiver is borrowed for the call.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_method_call(
    receiver: &Expr,
    method: &str,
    safe: bool,
    span: Span,
    id: AstId,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
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
        return super::vec::check_vec_method(
            receiver, borrowed, elem, method, span, id, args, return_ty, env,
        );
    }
    let Some(class) = env.tcx.record_def(checked.ty) else {
        let hint = env.bounds_hint(&[checked.ty]);
        env.error(format!("only named functions can be called{hint}"), Some(span));
        return poisoned(args, span, return_ty, env);
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
    // The receiver fixes the class parameters (they come first in `sig.generics`); the method's own
    // parameters are fresh variables that the arguments and the expected type fix.
    let (mut params, mut ret) = (sig.params.clone(), sig.ret);
    if !sig.generics.is_empty() {
        let class_arity = env.class(class).map_or(0, |info| info.generics.len());
        let mut type_args = env.tcx.class_args(checked.ty).map(|(_, args)| args.to_vec()).unwrap_or_default();
        type_args.extend(super::call::fresh_params(&sig.generic_names[class_arity..], &format!("{class_name}.{method}"), span, env));
        let substituted: Result<Vec<_>, _> =
            sig.params.iter().map(|&ty| crate::ty::subst(env.tcx, ty, &type_args)).collect();
        match (substituted, crate::ty::subst(env.tcx, sig.ret, &type_args)) {
            (Ok(p), Ok(r)) => (params, ret) = (p, r),
            _ => {
                env.error(format!("type arguments of `{class_name}.{method}` would make a nullable of a nullable"), Some(span));
                return poisoned(args, span, return_ty, env);
            }
        }
    }
    let receiver = borrow(checked, env);
    finish_call(
        format!("{class_name}.{method}"),
        span,
        params,
        ret,
        Some(receiver),
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
