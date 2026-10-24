//! Expression type checking, bidirectional.
//!
//! Mode per `Expr` kind (RFC 0001 §5):
//! - **check** (uses the expected type): `Int`, `Float`, `ArrayLit`, `None`, `Some`, `If`/`IfVal`
//!   branches, `&` borrow, `!!`, `?:`, arithmetic `Binary`, call/constructor/`Vec` arguments, and a
//!   managed `Ref<T>` target (wraps `T` in `RefCreate`).
//! - **synthesize**: `Bool`, `Str`, `Index`, `Slice`, `VecNew`, `Ident`, `Move`, `Promote`,
//!   `Field`, `Call`, `!`, other `Binary`.
//!
//! Mode switch: a synthesized expression met with an expected type is compared by the caller
//! with `Env::mismatch`, the one place that words a type mismatch.

pub(super) mod array;
mod binary;
mod borrow;
mod call;
mod control;
mod field;
mod method;
mod object;
mod vec;

use crate::ast::{BinOp, BindingKind, Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, Prim, UseKind};
use crate::infer::VarKind;
use crate::span::{AstId, Span};
use crate::ty::TyId;

use super::env::{Checked, Env};

use array::{check_array_lit, check_index, check_slice};

use binary::{check_binary, check_elvis};
use call::check_call;
use control::{check_if, check_presence};
use field::check_field;

/// Synthesize: the type comes from the expression alone.
pub(super) fn synthesize(expr: &Expr, return_ty: TyId, env: &mut Env<'_>) -> Checked {
    dispatch(expr, None, return_ty, env)
}

/// Check: push `expected` into the expression. Callers report a remaining mismatch with
/// `Env::mismatch` (synthesize-then-subsume), so there is a single message format.
pub(super) fn check(expr: &Expr, expected: TyId, return_ty: TyId, env: &mut Env<'_>) -> Checked {
    dispatch(expr, Some(expected), return_ty, env)
}

/// Shared body of `synthesize` and `check`; `expected` is only a hint when `None`.
/// Always yields an expression: a failed check is reported once and
/// poisoned with the error type, which suppresses follow-on mismatches.
pub(super) fn dispatch(
    expr: &Expr,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    if let Some(expected) = expected.filter(|&ty| env.tcx.is_managed_ref(ty)) {
        return check_managed_ref(expr, expected, return_ty, env);
    }
    let expected = expected.map(|ty| env.settle(ty));
    match expr {
        Expr::Int(value) => check_int(*value, expected, env),
        Expr::Bool(value) => {
            let ty = env.tcx.bool();
            env.expr(HirExprKind::Bool { value: *value }, ty)
        }
        Expr::Float(value) => check_float(*value, expected, env),
        Expr::ArrayLit { elements, span } => {
            check_array_lit(elements, *span, expected, return_ty, env)
        }
        Expr::Index {
            receiver,
            index,
            span,
        } => check_index(receiver, index, *span, return_ty, env),
        Expr::Slice {
            receiver,
            lo,
            hi,
            span,
        } => check_slice(receiver, lo, hi, *span, return_ty, env),
        Expr::VecNew { elem, cap, span } => vec::check_vec_new(elem, cap.as_deref(), *span, return_ty, env),
        Expr::Str(value) => {
            let ty = env.tcx.string(false);
            env.expr(HirExprKind::Str { value: value.clone() }, ty)
        }
        Expr::Ident { name, span, id } => method::implicit_field(name, *span, *id, return_ty, env)
            .unwrap_or_else(|| check_ident(name, *span, *id, UseKind::Local, env)),
        Expr::Unary {
            op: UnaryOp::Borrow,
            expr,
            span,
        } => check_borrow_operand(expr, *span, expected, env),
        Expr::Move { name, span, id } => check_ident(name, *span, *id, UseKind::Move, env),
        Expr::Promote { name, span, id } => check_ident(name, *span, *id, UseKind::Promote, env),
        Expr::None { span } => check_none(*span, expected, env),
        Expr::Some { expr: inner, span } => check_some(inner, *span, expected, return_ty, env),
        Expr::Binary { op, lhs, rhs, span } => {
            if *op == BinOp::Elvis {
                check_elvis(op, lhs, rhs, *span, expected, return_ty, env)
            } else {
                check_binary(op, lhs, rhs, *span, expected, return_ty, env)
            }
        }
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            span,
        } => check_not(expr, *span, return_ty, env),
        Expr::Unary {
            op: UnaryOp::NotNullAssert,
            expr,
            span,
        } => check_not_null_assert(expr, *span, expected, return_ty, env),
        Expr::Field {
            receiver,
            name,
            safe,
            span,
            ..
        } => match object::variant_path(receiver, *safe, env) {
            Some(e) => object::check_variant(e, name, *span, &[], None, expected, return_ty, env),
            None => check_field(receiver, name, *safe, *span, return_ty, env),
        },
        Expr::Call {
            callee,
            type_args,
            args,
            trailing,
        } => check_call(callee, type_args, args, trailing.as_ref(), expected, return_ty, env),
        Expr::If {
            cond,
            then_block,
            else_block,
        } => check_if(
            cond,
            then_block,
            else_block.as_ref(),
            expected,
            return_ty,
            env,
        ),
        Expr::IfVal {
            bindings,
            then_block,
            else_block,
        } => check_presence(
            bindings,
            then_block,
            else_block.as_ref(),
            expected,
            return_ty,
            env,
        ),
    }
}

/// An integer literal takes the expected integer type (or literal variable), else `i32`.
fn check_int(value: i64, expected: Option<TyId>, env: &mut Env<'_>) -> Checked {
    let ty = match expected.filter(|&ty| env.tcx.is_integer(ty) || env.is_literal_var(ty, VarKind::Int)) {
        Some(ty) => ty,
        None => env.tcx.i32(),
    };
    env.expr(HirExprKind::Int { value }, ty)
}

/// A float literal takes the expected float type (or literal variable), else `f64`.
fn check_float(value: f64, expected: Option<TyId>, env: &mut Env<'_>) -> Checked {
    let ty = match expected.filter(|&ty| {
        (env.tcx.is_numeric(ty) && !env.tcx.is_integer(ty)) || env.is_literal_var(ty, VarKind::Float)
    }) {
        Some(ty) => ty,
        None => env.tcx.prim(Prim::F64),
    };
    env.expr(HirExprKind::Float { value }, ty)
}

/// `&name`; anything else after `&` is an error.
fn check_borrow_operand(operand: &Expr, span: Span, expected: Option<TyId>, env: &mut Env<'_>) -> Checked {
    match operand {
        Expr::Ident { name, span: name_span, id } => {
            borrow::check_borrow(name, *name_span, *id, span, expected, env)
        }
        _ => {
            env.error("`&` borrows a name", Some(span));
            let unknown = env.tcx.error();
            env.expr_at(HirExprKind::None, unknown, span)
        }
    }
}

/// `None` needs an expected presence-capable type to take its type from.
fn check_none(span: Span, expected: Option<TyId>, env: &mut Env<'_>) -> Checked {
    let Some(ty) = expected.filter(|&ty| env.tcx.is_presence_capable(ty)) else {
        env.error(
            "cannot infer type of `None`; add a type annotation, e.g. `val x: i32? = None`",
            Some(span),
        );
        let unknown = env.tcx.error();
        return env.expr_at(HirExprKind::None, unknown, span);
    };
    env.expr_at(HirExprKind::None, ty, span)
}

fn check_some(inner: &Expr, span: Span, expected: Option<TyId>, return_ty: TyId, env: &mut Env<'_>) -> Checked {
    let expected_inner = expected
        .filter(|&ty| env.tcx.is_nullable(ty))
        .map(|ty| env.tcx.with_nullable(ty, false));
    let inner = dispatch(inner, expected_inner, return_ty, env);
    let ty = if env.tcx.supports_nullable(inner.ty) {
        env.tcx.with_nullable(inner.ty, true)
    } else {
        if !env.tcx.is_unknown(inner.ty) {
            env.error(format!("`Some` value cannot have type {}", env.show(inner.ty)), Some(span));
        }
        env.tcx.error()
    };
    env.expr_at(HirExprKind::Some(Box::new(inner.hir)), ty, span)
}

fn check_not(operand: &Expr, span: Span, return_ty: TyId, env: &mut Env<'_>) -> Checked {
    let bool_ty = env.tcx.bool();
    let operand = check(operand, bool_ty, return_ty, env);
    if !env.tcx.is_unknown(operand.ty) && !env.same(operand.ty, bool_ty) {
        env.error(
            format!("operand of `!` must have type bool, got {}", env.show(operand.ty)),
            operand.span.or(Some(span)),
        );
    }
    env.expr_at(
        HirExprKind::Unary {
            op: UnaryOp::Not,
            expr: Box::new(operand.hir),
        },
        bool_ty,
        span,
    )
}

fn check_not_null_assert(
    operand: &Expr,
    span: Span,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let expected_operand = expected
        .filter(|&ty| env.tcx.supports_nullable(ty))
        .map(|ty| env.tcx.with_nullable(ty, true));
    let operand = dispatch(operand, expected_operand, return_ty, env);
    let result_ty = if env.tcx.is_nullable(operand.ty) {
        env.tcx.with_nullable(operand.ty, false)
    } else {
        if !env.tcx.is_unknown(operand.ty) {
            env.error(format!("operand of `!!` must be nullable, got {}", env.show(operand.ty)), Some(span));
        }
        env.tcx.error()
    };
    env.expr_at(
        HirExprKind::Unary {
            op: UnaryOp::NotNullAssert,
            expr: Box::new(operand.hir),
        },
        result_ty,
        span,
    )
}

pub(super) fn check_ident(name: &str, span: Span, id: AstId, requested: UseKind, env: &mut Env<'_>) -> Checked {
    let ident = |use_kind| HirExprKind::Ident {
        name: name.to_string(),
        use_kind,
    };
    if name == "this" {
        if env.self_class.is_none() {
            env.error("`this` is only available inside a class method", Some(span));
            let unknown = env.tcx.error();
            return env.expr_at(ident(requested), unknown, span);
        }
        if matches!(requested, UseKind::Move | UseKind::Promote) {
            env.error("cannot move `this`; it is a borrow", Some(span));
        }
    }
    let field_message = match requested {
        UseKind::Move => Some(format!("cannot move field `{name}`; fields are reached through the `this` borrow")),
        UseKind::Promote => Some(format!("cannot promote field `{name}`; fields are reached through the `this` borrow")),
        UseKind::Borrow => Some(format!("cannot borrow field `{name}`: `&` borrows a name; bind the field to a `val` first")),
        _ => None,
    };
    if let Some(ty) = field_message.and_then(|message| env.reject_bare_field(name, span, id, &message)) {
        return env.expr_at(ident(requested), ty, span);
    }
    let Some(binding) = env.binding(name) else {
        if let Some(constant) = env.constants.get(name) {
            return constant_use(name, constant, span, id, requested, env);
        }
        env.error(format!("unknown binding `{name}`"), Some(span));
        let unknown = env.tcx.error();
        return env.expr_at(ident(requested), unknown, span);
    };
    let (binding_ty, binding_kind) = (binding.ty, binding.kind);
    let ty = env.settle(binding_ty);
    let use_kind = if requested == UseKind::Move && env.tcx.is_managed_ref(ty) {
        UseKind::RefMove
    } else if requested == UseKind::Move
        || requested == UseKind::Promote
        || requested == UseKind::Borrow
    {
        requested
    } else if env.tcx.is_managed_ref(ty) {
        UseKind::RefCopy
    } else if env.tcx.is_copy(ty) || env.is_literal_var(ty, VarKind::Int) || env.is_literal_var(ty, VarKind::Float) {
        UseKind::Copy
    } else if binding_kind == BindingKind::Val {
        UseKind::Shared
    } else {
        UseKind::Local
    };
    env.expr_at(ident(use_kind), ty, span)
}

/// A constant read lowers to its literal; `move`/`promote` are not meaningful on a constant.
fn constant_use(
    name: &str,
    constant: &crate::constants::Constant,
    span: Span,
    id: AstId,
    requested: UseKind,
    env: &mut Env<'_>,
) -> Checked {
    if matches!(requested, UseKind::Move | UseKind::Promote) {
        env.error(format!("cannot move or promote constant `{name}`"), Some(span));
    }
    env.members.insert(id.numbered(), constant.member(name));
    let ty = constant.ty;
    env.expr_at(constant.value.clone(), ty, span)
}

fn check_managed_ref(
    expr: &Expr,
    expected: TyId,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    if let Expr::None { span } = expr {
        return env.expr_at(HirExprKind::None, expected, *span);
    }

    let expected_inner = env.tcx.managed_ref_inner(expected);
    let source = match expr {
        Expr::Some { expr, .. } => dispatch(expr, expected_inner, return_ty, env),
        other => synthesize(other, return_ty, env),
    };
    if source.ty == expected {
        return source;
    }
    // An object of the target class becomes a `Ref`; the class arguments may still be inference
    // variables (`Node(1, None)` for a `Ref<Node<?>>` field), so unify instead of comparing ids.
    let source_ty = env.settle(source.ty);
    let is_target_object = env.tcx.class_def(source_ty).is_some()
        && expected_inner.is_some_and(|inner| inner == source_ty || env.same(inner, source_ty));
    if is_target_object {
        let span = source.span;
        return Checked {
            hir: HirExpr {
                id: crate::hir::NodeId::DUMMY,
                ty: expected,
                span,
                kind: HirExprKind::RefCreate(Box::new(source.hir)),
            },
            ty: expected,
        };
    }
    source
}
