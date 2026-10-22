//! Rewrites every type in one function body, used to replace literal inference variables.

use super::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirStmt};
use crate::ty::TyId;

/// Applies `f` to the type of every expression, declaration and presence binding in `body`.
pub fn resolve_types(body: &mut HirBlock, f: &mut impl FnMut(TyId) -> TyId) {
    block(body, f);
}

fn block(b: &mut HirBlock, f: &mut impl FnMut(TyId) -> TyId) {
    for stmt in &mut b.stmts {
        match stmt {
            HirStmt::Return { value } => value.iter_mut().for_each(|v| expr(v, f)),
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => block(inner, f),
            HirStmt::VarDecl { value, ty, .. } => {
                *ty = f(*ty);
                expr(value, f);
            }
            HirStmt::Expr(value) => expr(value, f),
            HirStmt::Assign { target, value } => {
                match target {
                    HirAssignTarget::Name { .. } => {}
                    HirAssignTarget::Index { index, .. } => expr(index, f),
                    HirAssignTarget::Field { receiver, .. } => expr(receiver, f),
                }
                expr(value, f);
            }
            HirStmt::For { iter, body, .. } => {
                expr(iter, f);
                block(body, f);
            }
            HirStmt::While { cond, body } => {
                expr(cond, f);
                block(body, f);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }
}

fn expr(e: &mut HirExpr, f: &mut impl FnMut(TyId) -> TyId) {
    e.ty = f(e.ty);
    match &mut e.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => {}
        HirExprKind::ArrayLit { elements } => elements.iter_mut().for_each(|x| expr(x, f)),
        HirExprKind::Index { receiver, index, .. } => {
            expr(receiver, f);
            expr(index, f);
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            expr(receiver, f);
            expr(lo, f);
            expr(hi, f);
        }
        HirExprKind::VecNew { cap, elem } => {
            *elem = f(*elem);
            cap.iter_mut().for_each(|x| expr(x, f));
        }
        HirExprKind::VecOp { receiver, args, .. } => {
            expr(receiver, f);
            args.iter_mut().for_each(|x| expr(x, f));
        }
        HirExprKind::Some(inner) | HirExprKind::RefCreate(inner) => expr(inner, f),
        HirExprKind::ObjectConstruct { fields, .. } => fields.iter_mut().for_each(|x| expr(x, f)),
        HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => expr(receiver, f),
        HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
            for b in std::iter::once(&mut *bindings.head).chain(&mut bindings.tail) {
                b.binding_ty = f(b.binding_ty);
                expr(&mut b.value, f);
            }
            block(some_block, f);
            none_block.iter_mut().for_each(|b| block(b, f));
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs, f);
            expr(rhs, f);
        }
        HirExprKind::Unary { expr: inner, .. } => expr(inner, f),
        HirExprKind::Call { callee, args, .. } => {
            expr(callee, f);
            args.iter_mut().for_each(|x| expr(x, f));
        }
        HirExprKind::If { cond, then_block, else_block } => {
            expr(cond, f);
            block(then_block, f);
            else_block.iter_mut().for_each(|b| block(b, f));
        }
    }
}
