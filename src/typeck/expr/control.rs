use std::collections::HashSet;

use crate::ast::{BindingKind, Block, ConditionalBinding, ConditionalBindings, Expr, Stmt};
use crate::hir::{
    HirBlock, HirConditionalBinding, HirConditionalBindings, HirExpr, HirExprKind, HirStmt, Ty,
    TyKind,
};

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

pub(super) fn check_presence(
    bindings: &ConditionalBindings,
    some_block: &Block,
    none_block: Option<&Block>,
    expected: Option<&Ty>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    env.enter_scope();
    let mut names = HashSet::new();
    let head = check_conditional_binding(&bindings.head, &mut names, return_ty, env);
    let tail = bindings
        .tail
        .iter()
        .map(|binding| check_conditional_binding(binding, &mut names, return_ty, env))
        .collect();
    let binding_span = head.name.span;
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
            bindings: HirConditionalBindings {
                head: Box::new(head),
                tail,
            },
            some_block,
            none_block: none_block.map(|(block, _)| block),
        },
        result_ty,
        binding_span,
    )
}

fn check_conditional_binding(
    binding: &ConditionalBinding,
    names: &mut HashSet<String>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirConditionalBinding {
    if !names.insert(binding.name.name.clone()) {
        env.error(
            format!("duplicate conditional binding '{}'", binding.name.name),
            Some(binding.name.span),
        );
    }
    let value = check(&binding.value, None, return_ty, env);
    let binding_ty = match value.ty.managed_ref_inner() {
        Some(target) => Ty::new(TyKind::Ref(Box::new(target.clone())), false),
        None => {
            if !value.ty.is_unknown() {
                env.error(
                    format!("presence matching requires Ref<T>, got {}", value.ty),
                    Some(binding.value_span),
                );
            }
            Ty::unknown()
        }
    };
    env.span_tys.insert(binding.name.span, binding_ty.clone());
    env.bind(
        binding.name.name.clone(),
        BindingKind::Val,
        binding_ty.clone(),
        true,
    );
    HirConditionalBinding {
        name: binding.name.clone(),
        value,
        binding_ty,
    }
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
