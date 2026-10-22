mod assign;

use crate::ast::{BindingKind, Block, Stmt};
use crate::hir::{HirBlock, HirStmt};
use crate::ty::TyId;

use super::env::Env;
use super::expr;

pub(super) fn check_block(
    block: &Block,
    return_ty: TyId,
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

    HirBlock { stmts, span: block.span, statement_spans: block.statement_spans.clone() }
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

pub(super) fn check(stmt: &Stmt, return_ty: TyId, env: &mut Env<'_>) -> HirStmt {
    match stmt {
        Stmt::Block(block) => HirStmt::Block(check_block(block, return_ty, env, true)),
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            id,
            ty,
            value,
        } => check_var_decl(*kind, name, *name_span, *id, ty.as_ref(), value, return_ty, env),
        Stmt::Assign { target, value } => assign::check(target, value, return_ty, env),
        Stmt::Return(value) => check_return(value.as_ref(), return_ty, env),
        Stmt::Expr(value) => HirStmt::Expr(expr::check(value, None, return_ty, env).hir),
        Stmt::For { name, iter, body } => check_for(&name.name, iter, body, return_ty, env),
        Stmt::While { cond, body } => check_while(cond, body, return_ty, env),
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
                env.reject_field_captures(names);
                names
                    .iter()
                    .map(|capture| capture.name.clone())
                    .collect::<Vec<_>>()
            }),
            body: check_block(body, return_ty, env, true),
        },
    }
}

fn check_var_decl(
    kind: BindingKind,
    name: &str,
    name_span: crate::span::Span,
    id: crate::span::AstId,
    ty: Option<&crate::ast::Type>,
    value: &crate::ast::Expr,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let declared_ty = ty.map(|ty| env.lower(ty, Some(name_span)));
    let value = expr::check(value, declared_ty, return_ty, env);
    // Bind whatever type we can settle on, even after a bad initializer,
    // so later uses of `name` are not reported as unknown bindings.
    let declared_ty = declared_ty.unwrap_or(value.ty);
    if env.tcx.is_ref(declared_ty) && kind == BindingKind::Var {
        env.error(
            format!("cannot bind `var` `{name}` to a reference type; use `val`"),
            Some(name_span),
        );
    }
    if !env.tcx.is_unknown(value.ty) && !env.tcx.is_unknown(declared_ty) && declared_ty != value.ty {
        env.error(
            format!(
                "initializer for `{name}` has type {}, expected {}",
                env.show(value.ty),
                env.show(declared_ty)
            ),
            Some(name_span),
        );
    }
    let explicit_ref = ty.is_some() && env.tcx.is_ref(declared_ty);
    env.node_tys.insert(id.numbered(), declared_ty);
    env.bind(name.to_string(), kind, declared_ty, explicit_ref);
    HirStmt::VarDecl {
        kind,
        name: name.to_string(),
        ty: declared_ty,
        value: value.hir,
        alloc_in_binding: None,
    }
}

fn check_return(
    value: Option<&crate::ast::Expr>,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let checked = match value {
        Some(value) => {
            let value = expr::check(value, Some(return_ty), return_ty, env);
            if !env.tcx.is_unknown(value.ty) && value.ty != return_ty {
                env.error(
                    format!(
                        "return value has type {}, expected {}",
                        env.show(value.ty),
                        env.show(return_ty)
                    ),
                    value.span,
                );
            }
            Some(value.hir)
        }
        None => {
            let unit = env.tcx.unit();
            if unit != return_ty {
                env.error(
                    format!(
                        "empty return has type {}, expected {}",
                        env.show(unit),
                        env.show(return_ty)
                    ),
                    None,
                );
            }
            None
        }
    };
    HirStmt::Return { value: checked }
}

fn check_for(
    name: &str,
    iter: &crate::ast::Expr,
    body: &Block,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let iter = expr::check(iter, None, return_ty, env);
    let elem = match env.tcx.range_elem(iter.ty) {
        Some(elem) => Some(elem),
        None => {
            if !env.tcx.is_unknown(iter.ty) {
                env.error("for-loop iterator must be a range", None);
            }
            None
        }
    };
    let i32_ty = env.tcx.i32();
    if elem.is_some_and(|elem| elem != i32_ty) {
        env.error("for-loop range elements must have type i32", None);
    }

    env.loop_depth += 1;
    env.enter_scope();
    let elem = match elem {
        Some(elem) => elem,
        None => env.tcx.error(),
    };
    env.bind(name.to_string(), BindingKind::Val, elem, false);
    let body = check_block(body, return_ty, env, false);
    env.exit_scope();
    env.loop_depth -= 1;
    HirStmt::For {
        name: name.to_string(),
        iter: iter.hir,
        body,
    }
}

fn check_while(
    cond: &crate::ast::Expr,
    body: &Block,
    return_ty: TyId,
    env: &mut Env<'_>,
) -> HirStmt {
    let bool_ty = env.tcx.bool();
    let cond = expr::check(cond, Some(bool_ty), return_ty, env);
    if !env.tcx.is_unknown(cond.ty) && cond.ty != bool_ty {
        env.error(
            format!("while condition has type {}, expected bool", env.show(cond.ty)),
            cond.span,
        );
    }
    env.loop_depth += 1;
    let body = check_block(body, return_ty, env, true);
    env.loop_depth -= 1;
    HirStmt::While { cond: cond.hir, body }
}
