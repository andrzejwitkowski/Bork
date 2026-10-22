use crate::ast::UnaryOp;
use crate::ast::{BindingKind, Closure, Expr};
use crate::hir::{HirExpr, HirExprKind, Prim, UseKind};
use crate::span::Span;
use crate::ty::{TyCtx, TyId};

use super::super::env::{Checked, Env};
use super::{dispatch, synthesize};
use super::control::check_value_block_in_current_scope;
use super::object::check_construct;

/// Formal parameter types of a callee, or `None` when the callee is not callable.
fn callee_signature(callee: &Expr, env: &mut Env<'_>) -> Option<(String, Span, Vec<TyId>, TyId)> {
    let Expr::Ident { name, span, .. } = callee else {
        env.error("only named functions can be called", None);
        return None;
    };
    if let Some(signature) = env.fun_sig(name).cloned() {
        return Some((name.clone(), *span, signature.params, signature.ret));
    }
    match env.binding(name).map(|binding| binding.ty) {
        Some(ty) => {
            if let Some((params, ret)) = env.tcx.func_sig(ty) {
                return Some((name.clone(), *span, params, ret));
            }
            if !env.tcx.is_unknown(ty) {
                env.error(
                    format!("binding `{name}` is not callable (has type {})", env.show(ty)),
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
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    if let Expr::Field {
        receiver,
        name,
        safe,
        span,
        id,
    } = callee
    {
        return super::method::check_method_call(
            receiver, name, *safe, *span, *id, args, trailing, return_ty, env,
        );
    }
    if let Expr::Ident { name, span, id } = callee {
        if env.implicit_method(name).is_some() {
            let this = Expr::Ident {
                name: "this".to_string(),
                span: *span,
                id: *id,
            };
            return super::method::check_method_call(
                &this, name, false, *span, *id, args, trailing, return_ty, env,
            );
        }
    }
    if let Expr::Ident { name, span, .. } = callee {
        if let Some(class) = env.class_named(name).cloned() {
            return check_construct(class, *span, args, trailing, return_ty, env);
        }
    }
    if let Expr::Ident { name, span, .. } = callee {
        if crate::builtins::is_ref_count(name) {
            return check_ref_count(name, args, *span, trailing, return_ty, env);
        }
    }
    if let Expr::Ident { name, span, .. } = callee {
        if crate::builtins::resolve(name) == Some(crate::builtins::Builtin::StringFromUtf8) {
            return check_string_from_utf8(name, args, *span, trailing, return_ty, env);
        }
    }
    let Some((name, span, params, call_return_ty)) = callee_signature(callee, env) else {
        let unknown = env.tcx.error();
        let checked_callee = match callee {
            Expr::Ident { name, span, .. } => env.expr_at(
                HirExprKind::Ident {
                    name: name.clone(),
                    use_kind: UseKind::Local,
                },
                unknown,
                *span,
            ),
            other => synthesize(other, return_ty, env),
        };
        let args = args
            .iter()
            .map(|arg| synthesize(arg, return_ty, env).hir)
            .collect();
        return env.expr(
            HirExprKind::Call {
                callee: Box::new(checked_callee.hir),
                args,
                has_trailing_closure: trailing.is_some(),
            },
            unknown,
        );
    };

    finish_call(name, span, params, call_return_ty, None, args, trailing, return_ty, env)
}

/// Checks explicit `args` against `params` (after the receiver, if any) and builds the call.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_call(
    name: String,
    span: Span,
    params: Vec<TyId>,
    call_return_ty: TyId,
    receiver: Option<Checked>,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let explicit = if receiver.is_some() { &params[1..] } else { &params[..] };
    let (regular_params, trailing_signature) =
        split_trailing_param(&name, span, explicit, trailing, env);

    let checked_args: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| dispatch(arg, regular_params.get(index).copied(), return_ty, env))
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

    for (index, (argument, &expected)) in checked_args.iter().zip(regular_params).enumerate() {
        let matches_builtin_overload =
            (crate::builtins::is_print(&name) && crate::builtins::supports_print_arg(env.tcx, argument.ty))
            || (crate::builtins::is_concat(&name)
                && env.tcx.is_string(argument.ty)
                && !env.tcx.is_nullable(argument.ty));
        if env.tcx.is_ref(expected)
            && !matches!(
                &argument.kind,
                HirExprKind::Unary {
                    op: UnaryOp::Borrow,
                    ..
                }
            )
            && !env.tcx.is_unknown(argument.ty)
        {
            env.error(
                format!(
                    "argument {} to `{name}` expects borrow `&…`, got {argument_ty}",
                    index + 1,
                    argument_ty = env.show(argument.ty)
                ),
                argument.span,
            );
        }
        let borrow_to_owned = env.tcx.is_ref(argument.ty)
            && !env.tcx.is_ref(expected)
            && !env.tcx.is_unknown(argument.ty)
            && !env.tcx.is_unknown(expected);
        if borrow_to_owned {
            env.error(
                format!(
                    "argument {} to `{name}` is borrow {argument_ty}; parameter expects owned {expected} (use `move` or take `&…` in the signature)",
                    index + 1,
                    argument_ty = env.show(argument.ty),
                    expected = env.show(expected)
                ),
                argument.span,
            );
        }
        if !borrow_to_owned
            && !env.tcx.is_unknown(argument.ty)
            && !matches_builtin_overload
            && !arg_matches_param(env.tcx, argument.ty, expected)
        {
            env.mismatch(&format!("argument {} to `{name}`", index + 1), argument.ty, expected, argument.span);
        }
    }

    if let (Some(closure), Some((closure_params, closure_ret))) = (trailing, trailing_signature) {
        check_trailing_closure(closure, &closure_params, closure_ret, env);
    }

    let callee_params = if crate::builtins::is_print(&name) && checked_args.len() == 1 {
        vec![checked_args[0].ty]
    } else {
        params
    };
    let checked_args: Vec<HirExpr> = receiver
        .into_iter()
        .chain(checked_args)
        .map(|arg| arg.hir)
        .collect();
    let callee_ty = env.tcx.func(callee_params, call_return_ty);

    build_call(
        env,
        name,
        span,
        callee_ty,
        checked_args,
        call_return_ty,
        trailing.is_some(),
    )
}

fn build_call(
    env: &Env<'_>,
    name: String,
    span: Span,
    callee_ty: TyId,
    args: Vec<HirExpr>,
    return_ty: TyId,
    has_trailing_closure: bool,
) -> Checked {
    let callee = env.expr_at(
        HirExprKind::Ident {
            name,
            use_kind: UseKind::Local,
        },
        callee_ty,
        span,
    );
    env.expr(
        HirExprKind::Call {
            callee: Box::new(callee.hir),
            args,
            has_trailing_closure,
        },
        return_ty,
    )
}

fn split_trailing_param<'a>(
    name: &str,
    span: Span,
    params: &'a [TyId],
    trailing: Option<&Closure>,
    env: &mut Env<'_>,
) -> (&'a [TyId], Option<(Vec<TyId>, TyId)>) {
    if trailing.is_none() {
        return (params, None);
    }
    // The last parameter may be a nullable function type; the closure still binds against it.
    let last = params
        .split_last()
        .and_then(|(last, regular)| Some((env.tcx.func_sig(env.tcx.base(*last))?, regular)));
    match last {
        Some((signature, regular)) => (regular, Some(signature)),
        None => {
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
    param_tys: &[TyId],
    closure_ret: TyId,
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
    for (param, &ty) in closure.params.iter().zip(param_tys) {
        let is_ref = env.tcx.is_ref(ty);
        env.bind(param.name.clone(), BindingKind::Val, ty, is_ref);
    }
    let (_, body_ty) =
        check_value_block_in_current_scope(&closure.body, Some(closure_ret), closure_ret, env);
    env.exit_scope();

    env.mismatch("trailing closure body", body_ty, closure_ret, None);
}

fn arg_matches_param(tcx: &TyCtx, got: TyId, expected: TyId) -> bool {
    if got == expected {
        return true;
    }
    if let (Some(got_inner), Some(expected_inner)) = (tcx.ref_inner(got), tcx.ref_inner(expected)) {
        return got_inner == expected_inner;
    }
    false
}

/// Shared checking for builtins whose parameters are not a fixed `FunSig` (any `Ref<T>`, any
/// `&[u8; N]`). The callee gets the instantiated function type, as `print` does in `finish_call`.
#[allow(clippy::too_many_arguments)]
fn check_builtin_call(
    name: &str,
    args: &[Expr],
    span: Span,
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
    arity: usize,
    expected: impl Fn(usize) -> Option<TyId>,
    result: TyId,
    validate: impl FnOnce(&[Checked], &mut Env<'_>),
) -> Checked {
    if trailing.is_some() {
        env.error(format!("`{name}` does not accept trailing closures"), Some(span));
    }
    let checked: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| dispatch(arg, expected(index), return_ty, env))
        .collect();
    if checked.len() != arity {
        let noun = if arity == 1 { "argument" } else { "arguments" };
        env.error(
            format!("`{name}` expects {arity} {noun}, got {}", checked.len()),
            Some(span),
        );
        let unknown = env.tcx.error();
        return env.expr_at(HirExprKind::None, unknown, span);
    }
    validate(&checked, env);
    let callee_ty = env.tcx.func(checked.iter().map(|arg| arg.ty).collect(), result);
    let args = checked.into_iter().map(|arg| arg.hir).collect();
    build_call(env, name.to_string(), span, callee_ty, args, result, trailing.is_some())
}

/// `strongCount(ref): i32` / `weakCount(ref): i32`: the argument is any `Ref<T>`.
fn check_ref_count(
    name: &str,
    args: &[Expr],
    span: Span,
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let i32_ty = env.tcx.i32();
    check_builtin_call(name, args, span, trailing, return_ty, env, 1, |_| None, i32_ty, |checked, env| {
        let reference = &checked[0];
        if !env.tcx.is_managed_ref(reference.ty) && !env.tcx.is_unknown(reference.ty) {
            env.error(
                format!("`{name}` expects a `Ref<T>`, got {}", env.show(reference.ty)),
                reference.span.or(Some(span)),
            );
        }
    })
}

/// `stringFromUtf8(&bytes, start, count): String?`: the buffer is any `&[u8; N]`.
fn check_string_from_utf8(
    name: &str,
    args: &[Expr],
    span: Span,
    trailing: Option<&Closure>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let i32_ty = env.tcx.i32();
    let result = env.tcx.string(true);
    let expected = |index| (index > 0).then_some(i32_ty);
    check_builtin_call(name, args, span, trailing, return_ty, env, 3, expected, result, |checked, env| {
        let [buffer, start, count] = checked else {
            unreachable!("argument count was checked by check_builtin_call");
        };
        let u8_ty = env.tcx.prim(Prim::U8);
        let buffer_is_borrowed_bytes = matches!(
            &buffer.kind,
            HirExprKind::Unary {
                op: UnaryOp::Borrow,
                ..
            }
        ) && env.tcx.is_ref(buffer.ty)
            && env.tcx.array_elem(buffer.ty) == Some(u8_ty);
        if !buffer_is_borrowed_bytes && !env.tcx.is_unknown(buffer.ty) {
            env.error(
                format!(
                    "`{name}` expects a borrowed byte array `&bytes`, got {}",
                    env.show(buffer.ty)
                ),
                buffer.span.or(Some(span)),
            );
        }
        for argument in [start, count] {
            if !env.tcx.is_unknown(argument.ty) && argument.ty != i32_ty {
                env.error(
                    format!(
                        "`{name}` offsets and counts must be `i32`, got {}",
                        env.show(argument.ty)
                    ),
                    argument.span.or(Some(span)),
                );
            }
        }
    })
}
