mod assign;

use crate::ast::{BindingKind, Block, Stmt};
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
        } => check_var_decl(*kind, name, *name_span, ty.as_ref(), value, return_ty, env),
        Stmt::Assign { target, value } => assign::check(target, value, return_ty, env),
        Stmt::Return(value) => check_return(value.as_ref(), return_ty, env),
        Stmt::Expr(value) => HirStmt::Expr(expr::check(value, None, return_ty, env)),
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
    ty: Option<&crate::ast::Type>,
    value: &crate::ast::Expr,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
    let declared_ty = ty.map(|ty| super::lower_type(ty, env.structs, &mut env.diagnostics));
    let value = expr::check(value, declared_ty.as_ref(), return_ty, env);
    // Bind whatever type we can settle on, even after a bad initializer,
    // so later uses of `name` are not reported as unknown bindings.
    let declared_ty = declared_ty.unwrap_or_else(|| value.ty.clone());
    if declared_ty.is_ref() && kind == BindingKind::Var {
        env.error(
            format!("cannot bind `var` `{name}` to a reference type; use `val`"),
            Some(name_span),
        );
    }
    if !value.ty.is_unknown() && !declared_ty.is_unknown() && declared_ty != value.ty {
        env.error(
            format!(
                "initializer for `{name}` has type {}, expected {declared_ty}",
                value.ty
            ),
            Some(name_span),
        );
    }
    let explicit_ref = ty.is_some() && declared_ty.is_ref();
    env.decl_tys.push(declared_ty.clone());
    env.bind(name.to_string(), kind, declared_ty.clone(), explicit_ref);
    HirStmt::VarDecl {
        kind,
        name: name.to_string(),
        ty: declared_ty,
        value,
        alloc_in_binding: None,
    }
}

fn check_return(
    value: Option<&crate::ast::Expr>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
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

fn check_for(
    name: &str,
    iter: &crate::ast::Expr,
    body: &Block,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
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
        name.to_string(),
        BindingKind::Val,
        elem.unwrap_or_else(Ty::unknown),
        false,
    );
    let body = check_block(body, return_ty, env, false);
    env.exit_scope();
    env.loop_depth -= 1;
    HirStmt::For {
        name: name.to_string(),
        iter,
        body,
    }
}

fn check_while(
    cond: &crate::ast::Expr,
    body: &Block,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirStmt {
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
    HirStmt::While { cond, body }
}
