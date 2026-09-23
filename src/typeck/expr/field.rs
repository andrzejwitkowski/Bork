use crate::ast::Expr;
use crate::hir::{HirExpr, HirExprKind, Ty};
use crate::span::Span;

use super::super::env::Env;
use super::check;

pub(super) fn check_field(
    receiver: &Expr,
    name: &str,
    safe: bool,
    span: Span,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let receiver = check(receiver, None, return_ty, env);
    let result_ty = if receiver.ty.is_string() && name == "length" {
        if receiver.ty.is_nullable() && !safe {
            env.error("field access on nullable `String?` requires `?.`", Some(span));
        }
        Ty::i32().with_nullable(safe && receiver.ty.is_nullable())
    } else {
        if !receiver.ty.is_unknown() {
            env.error(
                format!("unknown field `{name}` on type {}", receiver.ty),
                Some(span),
            );
        }
        Ty::unknown()
    };
    HirExpr::spanned(
        HirExprKind::Field {
            receiver: Box::new(receiver),
            name: name.to_string(),
            safe,
        },
        result_ty,
        span,
    )
}
