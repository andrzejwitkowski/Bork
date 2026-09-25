use crate::ast::{AssignTarget, BindingKind, Block, Stmt};
use crate::hir::HirAssignTarget;
use crate::hir::{HirBlock, HirStmt, Ty, TyKind};

use super::env::Env;
use super::expr;

pub(super) fn check_block(
    block: &Block,
    return_ty: &Ty,
    env: &mut Env<'_>,
    nested: bool,
) -> HirBlock {
    if nested {
        env.enter_scope();
    }

    let stmts = block
        .stmts
        .iter()
        .map(|stmt| check(stmt, return_ty, env))
        .collect();

    if nested {
        env.exit_scope();
    }

    HirBlock { stmts }
}

pub(super) fn block_always_returns(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_always_returns)
}

fn stmt_always_returns(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return { .. } => true,
        HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => block_always_returns(body),
        HirStmt::Expr(expr) => expr_always_returns(expr),
        HirStmt::VarDecl { .. }
        | HirStmt::Assign { .. }
        | HirStmt::For { .. }
        | HirStmt::While { .. }
        | HirStmt::Break { .. }
        | HirStmt::Continue { .. } => false,
    }
}

fn assign_binding(env: &mut Env<'_>, name: &str, span: crate::span::Span) -> Option<Ty> {
    let binding = env.binding(name).cloned();
    if binding.is_none() {
        env.error(format!("unknown binding `{name}`"), Some(span));
    }
    if binding.as_ref().is_some_and(|b| b.kind == BindingKind::Val) {
        env.error(
            format!("cannot assign to immutable `val` binding `{name}`"),
            Some(span),
        );
    }
    binding.map(|b| b.ty)
}

fn reject_type_mismatch(
    env: &mut Env<'_>,
    what: &str,
    got: &Ty,
    expected: Option<&Ty>,
    span: Option<crate::span::Span>,
) {
    let Some(expected) = expected else {
        return;
    };
    if !got.is_unknown() && !expected.is_unknown() && got != expected {
        env.error(
            format!("{what} has type {got}, expected {expected}"),
            span,
        );
    }
}

fn expr_always_returns(expr: &crate::hir::HirExpr) -> bool {
    match &expr.kind {
        crate::hir::HirExprKind::If {
            then_block,
            else_block: Some(else_block),
            ..
        } => block_always_returns(then_block) && block_always_returns(else_block),
        _ => false,
    }
}

pub(super) fn check(stmt: &Stmt, return_ty: &Ty, env: &mut Env<'_>) -> HirStmt {
    match stmt {
        Stmt::Block(block) => HirStmt::Block(check_block(block, return_ty, env, true)),
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            let declared_ty = ty
                .as_ref()
                .map(|ty| super::lower_type(ty, &mut env.diagnostics));
            let value = expr::check(value, declared_ty.as_ref(), return_ty, env);
            // Bind whatever type we can settle on, even after a bad initializer,
            // so later uses of `name` are not reported as unknown bindings.
            let declared_ty = declared_ty.unwrap_or_else(|| value.ty.clone());
            if !value.ty.is_unknown() && !declared_ty.is_unknown() && declared_ty != value.ty {
                env.error(
                    format!(
                        "initializer for `{name}` has type {}, expected {declared_ty}",
                        value.ty
                    ),
                    Some(*name_span),
                );
            }
            env.decl_tys.push(declared_ty.clone());
            env.bind(name.clone(), *kind, declared_ty.clone());
            HirStmt::VarDecl {
                kind: *kind,
                name: name.clone(),
                ty: declared_ty,
                value,
                alloc_in_binding: None,
            }
        }
        Stmt::Assign { target, value } => {
            let (name, name_span) = match target {
                AssignTarget::Name { name, name_span }
                | AssignTarget::Index { name, name_span, .. } => (name, name_span),
            };
            let bound = assign_binding(env, name, *name_span);
            match target {
                AssignTarget::Name { name, name_span } => {
                    let value = expr::check(value, bound.as_ref(), return_ty, env);
                    reject_type_mismatch(
                        env,
                        &format!("assignment to `{name}`"),
                        &value.ty,
                        bound.as_ref(),
                        Some(*name_span),
                    );
                    HirStmt::Assign {
                        target: HirAssignTarget::Name { name: name.clone() },
                        value,
                    }
                }
                AssignTarget::Index {
                    name,
                    name_span,
                    index,
                } => {
                    let elem = bound.as_ref().and_then(|ty| ty.array_elem().cloned());
                    if bound.as_ref().is_some_and(|ty| {
                        !ty.is_unknown() && ty.array_elem().is_none()
                    }) {
                        env.error(
                            format!("cannot index-assign `{name}`: expected an array"),
                            Some(*name_span),
                        );
                    }
                    let index = expr::check(index, Some(&Ty::i32()), return_ty, env);
                    if !index.ty.is_unknown() && index.ty != Ty::i32() {
                        env.error(
                            format!("array index has type {}, expected i32", index.ty),
                            index.span.or(Some(*name_span)),
                        );
                    }
                    let value = expr::check(value, elem.as_ref(), return_ty, env);
                    reject_type_mismatch(
                        env,
                        &format!("assignment to `{name}` element"),
                        &value.ty,
                        elem.as_ref(),
                        Some(*name_span),
                    );
                    HirStmt::Assign {
                        target: HirAssignTarget::Index {
                            name: name.clone(),
                            index,
                        },
                        value,
                    }
                }
            }
        }
        Stmt::Return(value) => {
            let checked = match value {
                Some(value) => {
                    let value = expr::check(value, Some(return_ty), return_ty, env);
                    if !value.ty.is_unknown() && &value.ty != return_ty {
                        env.error(
                            format!("return value has type {}, expected {return_ty}", value.ty),
                            value.span,
                        );
                    }
                    Some(value)
                }
                None => {
                    let unit = Ty::unit();
                    if &unit != return_ty {
                        env.error(
                            format!("empty return has type {unit}, expected {return_ty}"),
                            None,
                        );
                    }
                    None
                }
            };
            HirStmt::Return { value: checked }
        }
        Stmt::Expr(value) => HirStmt::Expr(expr::check(value, None, return_ty, env)),
        Stmt::For { name, iter, body } => {
            let iter = expr::check(iter, None, return_ty, env);
            let elem = match &iter.ty.kind {
                TyKind::Range(elem) => Some((**elem).clone()),
                _ => {
                    if !iter.ty.is_unknown() {
                        env.error("for-loop iterator must be a range", None);
                    }
                    None
                }
            };
            if elem.as_ref().is_some_and(|elem| elem != &Ty::i32()) {
                env.error("for-loop range elements must have type i32", None);
            }

            env.loop_depth += 1;
            env.enter_scope();
            env.bind(
                name.name.clone(),
                BindingKind::Val,
                elem.unwrap_or_else(Ty::unknown),
            );
            let body = check_block(body, return_ty, env, false);
            env.exit_scope();
            env.loop_depth -= 1;
            HirStmt::For {
                name: name.name.clone(),
                iter,
                body,
            }
        }
        Stmt::While { cond, body } => {
            let cond = expr::check(cond, Some(&Ty::bool()), return_ty, env);
            if !cond.ty.is_unknown() && cond.ty != Ty::bool() {
                env.error(
                    format!("while condition has type {}, expected bool", cond.ty),
                    cond.span,
                );
            }
            env.loop_depth += 1;
            let body = check_block(body, return_ty, env, true);
            env.loop_depth -= 1;
            HirStmt::While {
                cond,
                body,
            }
        }
        Stmt::Break { span } => {
            if env.loop_depth == 0 {
                env.error("`break` outside of a loop", Some(*span));
            }
            HirStmt::Break { span: *span }
        }
        Stmt::Continue { span } => {
            if env.loop_depth == 0 {
                env.error("`continue` outside of a loop", Some(*span));
            }
            HirStmt::Continue { span: *span }
        }
        Stmt::MoveBlock { captures, body } => HirStmt::MoveBlock {
            captures: captures.as_ref().map(|names| {
                names
                    .iter()
                    .map(|capture| capture.name.clone())
                    .collect::<Vec<_>>()
            }),
            body: check_block(body, return_ty, env, true),
        },
    }
}
