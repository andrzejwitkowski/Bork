use crate::ast::{BindingKind, Closure, Expr};
use crate::ast::UnaryOp;
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind, UseKind};
use crate::span::Span;

use super::super::env::Env;
use super::check;
use super::control::check_value_block_in_current_scope;
use super::object::check_construct;

/// Formal parameter types of a callee, or `None` when the callee is not callable.
fn callee_signature(callee: &Expr, env: &mut Env<'_>) -> Option<(String, Span, Vec<Ty>, Ty)> {
    let Expr::Ident { name, span } = callee else {
        env.error("only named functions can be called", None);
        return None;
    };
    if let Some(signature) = env.fun_sig(name).cloned() {
        return Some((name.clone(), *span, signature.params, signature.return_ty));
    }
    match env.binding(name).map(|binding| binding.ty.clone()) {
        Some(Ty {
            kind: TyKind::Func { params, ret },
            nullable: false,
        }) => Some((name.clone(), *span, params, *ret)),
        Some(ty) => {
            if !ty.is_unknown() {
                env.error(
                    format!("binding `{name}` is not callable (has type {ty})"),
                    Some(*span),
                );
            }
            None
        }
        None => {
            env.error(format!("unknown function `{name}`"), Some(*span));
            None
        }
    }
}

pub(super) fn check_call(
    callee: &Expr,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    if let Expr::Field {
        receiver,
        name,
        safe,
        span,
    } = callee
    {
        return super::method::check_method_call(
            receiver, name, *safe, *span, args, trailing, return_ty, env,
        );
    }
    if let Expr::Ident { name, span } = callee {
        if env.implicit_method(name).is_some() {
            let this = Expr::Ident {
                name: "this".to_string(),
                span: *span,
            };
            return super::method::check_method_call(
                &this, name, false, *span, args, trailing, return_ty, env,
            );
        }
    }
    if let Expr::Ident { name, span } = callee {
        if let Some(class) = env.class(name).cloned() {
            return check_construct(class, *span, args, trailing, return_ty, env);
        }
    }
    if let Expr::Ident { name, span } = callee {
        if crate::builtins::resolve(name) == Some(crate::builtins::Builtin::StringFromUtf8) {
            return check_string_from_utf8(name, args, *span, return_ty, env);
        }
    }
    let Some((name, span, params, call_return_ty)) = callee_signature(callee, env) else {
        let checked_callee = match callee {
            Expr::Ident { name, span } => HirExpr::spanned(
                HirExprKind::Ident {
                    name: name.clone(),
                    use_kind: UseKind::Local,
                },
                Ty::unknown(),
                *span,
            ),
            other => check(other, None, return_ty, env),
        };
        return HirExpr::new(
            HirExprKind::Call {
                callee: Box::new(checked_callee),
                args: args
                    .iter()
                    .map(|arg| check(arg, None, return_ty, env))
                    .collect(),
                has_trailing_closure: trailing.is_some(),
            },
            Ty::unknown(),
        );
    };

    finish_call(name, span, params, call_return_ty, None, args, trailing, return_ty, env)
}

/// Checks explicit `args` against `params` (after the receiver, if any) and builds the call.
pub(super) fn finish_call(
    name: String,
    span: Span,
    params: Vec<Ty>,
    call_return_ty: Ty,
    receiver: Option<HirExpr>,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let explicit = if receiver.is_some() { &params[1..] } else { &params[..] };
    let (regular_params, trailing_signature) =
        split_trailing_param(&name, span, explicit, trailing, env);

    let checked_args: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| check(arg, regular_params.get(index), return_ty, env))
        .collect();

    if checked_args.len() != regular_params.len() {
        env.error(
            format!(
                "function `{name}` expects {} arguments, got {}",
                regular_params.len(),
                checked_args.len()
            ),
            Some(span),
        );
    }
    for (index, (argument, expected)) in checked_args.iter().zip(regular_params).enumerate() {
        let matches_builtin_overload =
            (crate::builtins::is_print(&name) && crate::builtins::supports_print_arg(&argument.ty))
            || (crate::builtins::is_concat(&name)
                && argument.ty.is_string()
                && !argument.ty.is_nullable());
        if expected.is_ref()
            && !matches!(
                &argument.kind,
                HirExprKind::Unary {
                    op: UnaryOp::Borrow,
                    ..
                }
            )
            && !argument.ty.is_unknown()
        {
            env.error(
                format!(
                    "argument {} to `{name}` expects borrow `&…`, got {argument_ty}",
                    index + 1,
                    argument_ty = argument.ty
                ),
                argument.span,
            );
        }
        let borrow_to_owned = argument.ty.is_ref()
            && !expected.is_ref()
            && !argument.ty.is_unknown()
            && !expected.is_unknown();
        if borrow_to_owned {
            env.error(
                format!(
                    "argument {} to `{name}` is borrow {argument_ty}; parameter expects owned {expected} (use `move` or take `&…` in the signature)",
                    index + 1,
                    argument_ty = argument.ty
                ),
                argument.span,
            );
        }
        if !borrow_to_owned
            && !argument.ty.is_unknown()
            && !matches_builtin_overload
            && !arg_matches_param(&argument.ty, expected)
        {
            env.error(
                format!(
                    "argument {} to `{name}` has type {}, expected {expected}",
                    index + 1,
                    argument.ty
                ),
                argument.span,
            );
        }
    }

    if let (Some(closure), Some((closure_params, closure_ret))) = (trailing, trailing_signature) {
        check_trailing_closure(closure, closure_params, closure_ret, env);
    }

    let callee_params = if crate::builtins::is_print(&name) && checked_args.len() == 1 {
        vec![checked_args[0].ty.clone()]
    } else {
        params
    };
    let checked_args: Vec<HirExpr> = receiver.into_iter().chain(checked_args).collect();
    let callee_ty = Ty::new(
        TyKind::Func {
            params: callee_params,
            ret: Box::new(call_return_ty.clone()),
        },
        false,
    );

    HirExpr::new(
        HirExprKind::Call {
            callee: Box::new(HirExpr::spanned(
                HirExprKind::Ident {
                    name,
                    use_kind: UseKind::Local,
                },
                callee_ty,
                span,
            )),
            args: checked_args,
            has_trailing_closure: trailing.is_some(),
        },
        call_return_ty,
    )
}

fn split_trailing_param<'a>(
    name: &str,
    span: Span,
    params: &'a [Ty],
    trailing: Option<&Closure>,
    env: &mut Env<'_>,
) -> (&'a [Ty], Option<(&'a [Ty], &'a Ty)>) {
    if trailing.is_none() {
        return (params, None);
    }
    match params.split_last() {
        Some((
            Ty {
                kind: TyKind::Func { params, ret },
                ..
            },
            regular,
        )) => (regular, Some((params.as_slice(), ret.as_ref()))),
        _ => {
            env.error(
                format!(
                    "function `{name}` requires a function type as its last parameter \
                     when called with a trailing closure"
                ),
                Some(span),
            );
            (params, None)
        }
    }
}

fn check_trailing_closure(
    closure: &Closure,
    param_tys: &[Ty],
    closure_ret: &Ty,
    env: &mut Env<'_>,
) {
    if closure.params.len() != param_tys.len() {
        env.error(
            format!(
                "trailing closure expects {} parameters, got {}",
                param_tys.len(),
                closure.params.len()
            ),
            None,
        );
    }

    if let Some(captures) = &closure.captures {
        env.reject_field_captures(captures);
    }
    env.enter_scope();
    for (param, ty) in closure.params.iter().zip(param_tys) {
        env.bind(param.name.clone(), BindingKind::Val, ty.clone(), ty.is_ref());
    }
    let (_, body_ty) =
        check_value_block_in_current_scope(&closure.body, Some(closure_ret), closure_ret, env);
    env.exit_scope();

    if !body_ty.is_unknown() && &body_ty != closure_ret {
        env.error(
            format!("trailing closure body has type {body_ty}, expected {closure_ret}"),
            None,
        );
    }
}

fn arg_matches_param(got: &Ty, expected: &Ty) -> bool {
    if got == expected {
        return true;
    }
    if let (Some(got_inner), Some(expected_inner)) = (got.ref_inner(), expected.ref_inner()) {
        return got_inner == expected_inner;
    }
    false
}

/// `stringFromUtf8(&bytes, start, count): String?`: the buffer is any `&[u8; N]`, so the
/// signature cannot be a fixed `FunSig`.
fn check_string_from_utf8(
    name: &str,
    args: &[Expr],
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let checked: Vec<HirExpr> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| {
            let expected = if index == 0 { None } else { Some(&Ty::i32()) };
            check(arg, expected, return_ty, env)
        })
        .collect();
    if checked.len() != 3 {
        env.error(
            format!("`{name}` expects 3 arguments, got {}", checked.len()),
            Some(span),
        );
        return HirExpr::spanned(HirExprKind::None, Ty::unknown(), span);
    }
    let buffer_ok = matches!(&checked[0].kind, HirExprKind::Unary { op: UnaryOp::Borrow, .. })
        && checked[0].ty.is_ref()
        && checked[0].ty.array_elem() == Some(&Ty::prim(crate::hir::Prim::U8));
    if !buffer_ok && !checked[0].ty.is_unknown() {
        env.error(
            format!("`{name}` expects a borrowed byte array `&bytes`, got {}", checked[0].ty),
            checked[0].span.or(Some(span)),
        );
    }
    for argument in &checked[1..] {
        if !argument.ty.is_unknown() && argument.ty != Ty::i32() {
            env.error(
                format!("`{name}` offsets and counts must be `i32`, got {}", argument.ty),
                argument.span.or(Some(span)),
            );
        }
    }
    HirExpr::new(
        HirExprKind::Call {
            callee: Box::new(HirExpr::spanned(
                HirExprKind::Ident {
                    name: name.to_string(),
                    use_kind: UseKind::Local,
                },
                Ty::unknown(),
                span,
            )),
            args: checked,
            has_trailing_closure: false,
        },
        Ty::string(true),
    )
}
