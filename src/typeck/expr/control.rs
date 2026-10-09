use crate::ast::{BindingKind, Block, Expr, Stmt};
use crate::hir::{HirBlock, HirConditionalBinding, HirExpr, HirExprKind, HirStmt, Ty, TyKind};
use crate::span::SpannedName;

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

pub(super) fn check_presence<'a>(
    bindings: impl IntoIterator<Item = (&'a Expr, &'a SpannedName, Option<crate::span::Span>)>,
    some_block: &Block,
    none_block: Option<&Block>,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    env.enter_scope();
    let mut names = std::collections::HashSet::new();
    let mut checked = Vec::new();
    for (value, binding, value_span) in bindings {
        if !names.insert(binding.name.clone()) {
            env.error(
                format!("duplicate conditional binding '{}'", binding.name),
                Some(binding.span),
            );
        }
        let value = check(value, None, return_ty, env);
        let value_span = value_span.or(value.span).unwrap_or(binding.span);
        let binding_ty = match value.ty.managed_ref_inner() {
            Some(target) => Ty::new(TyKind::Ref(Box::new(target.clone())), false),
            None => {
                if !value.ty.is_unknown() {
                    env.error(
                        format!("presence matching requires Ref<T>, got {}", value.ty),
                        Some(value_span),
                    );
                }
                Ty::unknown()
            }
        };
        env.decl_tys.insert(binding.span, binding_ty.clone());
        env.bind(
            binding.name.clone(),
            BindingKind::Val,
            binding_ty.clone(),
            true,
        );
        checked.push(HirConditionalBinding {
            name: binding.clone(),
            value,
            value_span,
            binding_ty,
        });
    }
    let binding_span = checked
        .first()
        .expect("presence match requires a binding")
        .name
        .span;
    let (some_block, some_ty) =
        check_value_block_in_current_scope(some_block, expected, return_ty, env);
    env.exit_scope();
    let none_block = none_block.map(|block| check_value_block(block, expected, return_ty, env));

    let result_ty = match (&none_block, expected) {
        (Some((_, none_ty)), _) if *none_ty == some_ty => some_ty,
        (Some((_, none_ty)), Some(_)) => {
            if !none_ty.is_unknown() && !some_ty.is_unknown() {
                env.error(
                    format!("None branch has type {none_ty}, expected {some_ty}"),
                    Some(binding_span),
                );
            }
            some_ty
        }
        (Some(_), None) => Ty::unit(),
        (None, Some(_)) => {
            env.error(
                "value-producing `if val` requires an else branch",
                Some(binding_span),
            );
            some_ty
        }
        (None, None) => Ty::unit(),
    };

    HirExpr::spanned(
        HirExprKind::PresenceMatch {
            bindings: checked,
            some_block,
            none_block: none_block.map(|(block, _)| block),
        },
        result_ty,
        binding_span,
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
