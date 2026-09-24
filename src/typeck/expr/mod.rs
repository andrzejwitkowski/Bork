//! Expression type checking.

mod binary;
mod call;
mod control;
mod field;

use crate::ast::{BinOp, BindingKind, Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, Ty, UseKind};
use crate::span::Span;

use super::env::Env;

use binary::{check_binary, check_elvis};
use call::check_call;
use control::check_if;
use field::check_field;

/// Check always yields an expression: a failed check is reported once and
/// poisoned with `TyKind::Unknown`, which suppresses follow-on mismatches.
pub(super) fn check(
    expr: &Expr,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    match expr {
        Expr::Int(value) => {
            let ty = expected
                .filter(|ty| ty.is_integer())
                .cloned()
                .unwrap_or_else(Ty::i32);
            HirExpr::new(HirExprKind::Int { value: *value }, ty)
        }
        Expr::Str(value) => HirExpr::new(
            HirExprKind::Str {
                value: value.clone(),
            },
            Ty::string(false),
        ),
        Expr::Ident { name, span } => check_ident(name, *span, UseKind::Local, env),
        Expr::Move { name, span } => check_ident(name, *span, UseKind::Move, env),
        Expr::Promote { name, span } => check_ident(name, *span, UseKind::Promote, env),
        Expr::None { span } => {
            let Some(ty) = expected.filter(|ty| ty.is_nullable()) else {
                env.error("cannot infer type of `None`", Some(*span));
                return HirExpr::spanned(HirExprKind::None, Ty::unknown(), *span);
            };
            HirExpr::spanned(HirExprKind::None, ty.clone(), *span)
        }
        Expr::Some { expr: inner, span } => {
            let expected_inner = expected
                .filter(|ty| ty.is_nullable())
                .map(|ty| ty.with_nullable(false));
            let inner = check(inner, expected_inner.as_ref(), return_ty, env);
            let ty = if inner.ty.supports_nullable() {
                inner.ty.with_nullable(true)
            } else {
                if !inner.ty.is_unknown() {
                    env.error(
                        format!("`Some` value cannot have type {}", inner.ty),
                        Some(*span),
                    );
                }
                Ty::unknown()
            };
            HirExpr::spanned(HirExprKind::Some(Box::new(inner)), ty, *span)
        }
        Expr::Binary { op, lhs, rhs, span } => {
            if *op == BinOp::Elvis {
                check_elvis(op, lhs, rhs, *span, expected, return_ty, env)
            } else {
                check_binary(op, lhs, rhs, *span, expected, return_ty, env)
            }
        }
        Expr::Unary {
            op: UnaryOp::NotNullAssert,
            expr,
            span,
        } => {
            let expected_operand = expected
                .filter(|ty| ty.supports_nullable())
                .map(|ty| ty.with_nullable(true));
            let operand = check(expr, expected_operand.as_ref(), return_ty, env);
            let result_ty = if operand.ty.is_nullable() {
                operand.ty.with_nullable(false)
            } else {
                if !operand.ty.is_unknown() {
                    env.error(
                        format!("operand of `!!` must be nullable, got {}", operand.ty),
                        Some(*span),
                    );
                }
                Ty::unknown()
            };
            HirExpr::spanned(
                HirExprKind::Unary {
                    op: UnaryOp::NotNullAssert,
                    expr: Box::new(operand),
                },
                result_ty,
                *span,
            )
        }
        Expr::Field {
            receiver,
            name,
            safe,
            span,
        } => check_field(receiver, name, *safe, *span, return_ty, env),
        Expr::Call {
            callee,
            args,
            trailing,
        } => check_call(callee, args, trailing.as_ref(), return_ty, env),
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
    }
}

fn check_ident(name: &str, span: Span, requested: UseKind, env: &mut Env<'_>) -> HirExpr {
    let Some(binding) = env.binding(name) else {
        env.error(format!("unknown binding `{name}`"), Some(span));
        return HirExpr::spanned(
            HirExprKind::Ident {
                name: name.to_string(),
                use_kind: requested,
            },
            Ty::unknown(),
            span,
        );
    };
    let ty = binding.ty.clone();
    let use_kind = if requested == UseKind::Move || requested == UseKind::Promote {
        requested
    } else if ty.is_copy() {
        UseKind::Copy
    } else if binding.kind == BindingKind::Val {
        UseKind::Shared
    } else {
        UseKind::Local
    };
    HirExpr::spanned(
        HirExprKind::Ident {
            name: name.to_string(),
            use_kind,
        },
        ty,
        span,
    )
}
