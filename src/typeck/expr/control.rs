use std::collections::HashSet;

use crate::ast::{BindingKind, Block, ConditionalBinding, ConditionalBindings, Expr, Stmt};
use crate::hir::{
    HirBlock, HirConditionalBinding, HirConditionalBindings, HirExprKind, HirStmt,
};
use crate::ty::TyId;

use super::super::env::{Checked, Env};
use super::super::stmt;
use super::check;

pub(super) fn check_if(
    cond: &Expr,
    then_block: &Block,
    else_block: Option<&Block>,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
    let bool_ty = env.tcx.bool();
    let cond = check(cond, Some(bool_ty), return_ty, env);
    if !env.tcx.is_unknown(cond.ty) && cond.ty != bool_ty {
        env.error(
            format!("if condition has type {}, expected bool", env.show(cond.ty)),
            cond.span,
        );
    }
    let (then_block, then_ty) = check_value_block(then_block, expected, return_ty, env);
    let else_block = else_block.map(|block| check_value_block(block, expected, return_ty, env));

    let result_ty = match (&else_block, expected) {
        (Some((_, else_ty)), _) if *else_ty == then_ty => then_ty,
        (Some((_, else_ty)), Some(_)) => {
            if !env.tcx.is_unknown(*else_ty) && !env.tcx.is_unknown(then_ty) {
                env.error(
                    format!(
                        "else branch has type {}, expected {}",
                        env.show(*else_ty),
                        env.show(then_ty)
                    ),
                    None,
                );
            }
            then_ty
        }
        (Some(_), None) => env.tcx.unit(),
        (None, Some(_)) => {
            env.error(
                "value-producing if expression requires an else branch",
                None,
            );
            then_ty
        }
        (None, None) => env.tcx.unit(),
    };

    env.expr(
        HirExprKind::If {
            cond: Box::new(cond.hir),
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
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> Checked {
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
            if !env.tcx.is_unknown(*none_ty) && !env.tcx.is_unknown(some_ty) {
                env.error(
                    format!(
                        "None branch has type {}, expected {}",
                        env.show(*none_ty),
                        env.show(some_ty)
                    ),
                    Some(binding_span),
                );
            }
            some_ty
        }
        (Some(_), None) => env.tcx.unit(),
        (None, Some(_)) => {
            env.error(
                "value-producing `if val` requires an else branch",
                Some(binding_span),
            );
            some_ty
        }
        (None, None) => env.tcx.unit(),
    };

    env.expr_at(
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
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirConditionalBinding {
    if !names.insert(binding.name.name.clone()) {
        env.error(
            format!("duplicate conditional binding '{}'", binding.name.name),
            Some(binding.name.span),
        );
    }
    let value = check(&binding.value, None, return_ty, env);
    let binding_ty = match env.tcx.managed_ref_inner(value.ty) {
        Some(target) => env.tcx.ref_to(target),
        None => {
            if !env.tcx.is_unknown(value.ty) {
                env.error(
                    format!("presence matching requires Ref<T>, got {}", env.show(value.ty)),
                    Some(binding.value_span),
                );
            }
            env.tcx.error()
        }
    };
    let legacy = env.tcx.to_ty(binding_ty);
    env.node_tys.insert(binding.name.id.numbered(), legacy.clone());
    env.bind(
        binding.name.name.clone(),
        BindingKind::Val,
        binding_ty,
        true,
    );
    HirConditionalBinding {
        name: binding.name.clone(),
        value: value.hir,
        binding_ty: legacy,
        binding_ty_id: binding_ty,
    }
}

fn check_value_block(
    block: &Block,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> (HirBlock, TyId) {
    env.enter_scope();
    let result = check_value_block_in_current_scope(block, expected, return_ty, env);
    env.exit_scope();
    result
}

pub(super) fn check_value_block_in_current_scope(
    block: &Block,
    expected: Option<TyId>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> (HirBlock, TyId) {
    let Some((last, prefix)) = block.stmts.split_last() else {
        let unit = env.tcx.unit();
        return (HirBlock { stmts: Vec::new(), span: block.span, statement_spans: Vec::new() }, unit);
    };

    let mut stmts = prefix
        .iter()
        .map(|statement| stmt::check(statement, return_ty, env))
        .collect::<Vec<_>>();
    let result_ty = match last {
        Stmt::Expr(value) => {
            let value = check(value, expected, return_ty, env);
            let ty = value.ty;
            stmts.push(HirStmt::Expr(value.hir));
            ty
        }
        statement => {
            stmts.push(stmt::check(statement, return_ty, env));
            env.tcx.unit()
        }
    };
    (HirBlock { stmts, span: block.span, statement_spans: block.statement_spans.clone() }, result_ty)
}
