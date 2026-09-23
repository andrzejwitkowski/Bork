use crate::ast::{Block, Expr, Stmt};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, Ty};

use super::super::env::Env;
use super::super::stmt;
use super::check;

pub(super) fn check_if(
    cond: &Expr,
    then_block: &Block,
    else_block: Option<&Block>,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let cond = check(cond, Some(&Ty::bool()), return_ty, env);
    if !cond.ty.is_unknown() && cond.ty != Ty::bool() {
        env.error(
            format!("if condition has type {}, expected bool", cond.ty),
            cond.span,
        );
    }
    let (then_block, then_ty) = check_value_block(then_block, expected, return_ty, env);
    let else_block = else_block.map(|block| check_value_block(block, expected, return_ty, env));

    let result_ty = match (&else_block, expected) {
        (Some((_, else_ty)), _) if *else_ty == then_ty => then_ty,
        (Some((_, else_ty)), Some(_)) => {
            if !else_ty.is_unknown() && !then_ty.is_unknown() {
                env.error(
                    format!("else branch has type {else_ty}, expected {then_ty}"),
                    None,
                );
            }
            then_ty
        }
        (Some(_), None) => Ty::unit(),
        (None, Some(_)) => {
            env.error(
                "value-producing if expression requires an else branch",
                None,
            );
            then_ty
        }
        (None, None) => Ty::unit(),
    };

    HirExpr::new(
        HirExprKind::If {
            cond: Box::new(cond),
            then_block,
            else_block: else_block.map(|(block, _)| block),
        },
        result_ty,
    )
}

fn check_value_block(
    block: &Block,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> (HirBlock, Ty) {
    env.enter_scope();
    let result = check_value_block_in_current_scope(block, expected, return_ty, env);
    env.exit_scope();
    result
}

pub(super) fn check_value_block_in_current_scope(
    block: &Block,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> (HirBlock, Ty) {
    let Some((last, prefix)) = block.stmts.split_last() else {
        return (HirBlock { stmts: Vec::new() }, Ty::unit());
    };

    let mut stmts = prefix
        .iter()
        .map(|statement| stmt::check(statement, return_ty, env))
        .collect::<Vec<_>>();
    let result_ty = match last {
        Stmt::Expr(value) => {
            let value = check(value, expected, return_ty, env);
            let ty = value.ty.clone();
            stmts.push(HirStmt::Expr(value));
            ty
        }
        statement => {
            stmts.push(stmt::check(statement, return_ty, env));
            Ty::unit()
        }
    };
    (HirBlock { stmts }, result_ty)
}
