use crate::ast::Expr;
use crate::hir::{HirExpr, Ty};

use super::env::Env;

pub(super) fn check(expr: &Expr, env: &mut Env<'_>) -> Option<(HirExpr, Ty)> {
    match expr {
        Expr::Int(value) => Some((HirExpr::Int { value: *value }, Ty::i32())),
        Expr::Str(value) => Some((
            HirExpr::Str {
                value: value.clone(),
            },
            Ty::string(false),
        )),
        Expr::Ident { name, span } => {
            let Some(binding) = env.binding(name) else {
                env.error(format!("unknown binding `{name}`"), Some(*span));
                return None;
            };
            Some((HirExpr::Ident { name: name.clone() }, binding.ty.clone()))
        }
        _ => {
            env.error("expression is not supported by type checking yet", None);
            None
        }
    }
}
