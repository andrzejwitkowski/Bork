//! Rewrites every type, and optionally every expression, in one function body: used to replace
//! literal inference variables and to substitute type arguments in monomorphized instances.

use super::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirStmt};
use crate::ty::TyId;

/// Applies `f` to the type of every expression, declaration and presence binding in `body`.
pub fn resolve_types(body: &mut HirBlock, f: &mut impl FnMut(TyId) -> TyId) {
    rewrite(body, f, &mut |_| {});
}

/// Like [`resolve_types`], and calls `on_expr` on every expression after its children (and its own
/// type) were rewritten, so a callee's type is already final when the call is visited.
pub fn rewrite(body: &mut HirBlock, f: &mut impl FnMut(TyId) -> TyId, on_expr: &mut impl FnMut(&mut HirExpr)) {
    block(body, f, on_expr);
}

fn block(b: &mut HirBlock, f: &mut impl FnMut(TyId) -> TyId, g: &mut impl FnMut(&mut HirExpr)) {
    for stmt in &mut b.stmts {
        match stmt {
            HirStmt::Return { value } => value.iter_mut().for_each(|v| expr(v, f, g)),
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => block(inner, f, g),
            HirStmt::VarDecl { value, ty, .. } => {
                *ty = f(*ty);
                expr(value, f, g);
            }
            HirStmt::Expr(value) => expr(value, f, g),
            HirStmt::Assign { target, value } => {
                match target {
                    HirAssignTarget::Name { .. } => {}
                    HirAssignTarget::Index { index, .. } => expr(index, f, g),
                    HirAssignTarget::Field { receiver, .. } => expr(receiver, f, g),
                }
                expr(value, f, g);
            }
            HirStmt::For { iter, body, .. } => {
                expr(iter, f, g);
                block(body, f, g);
            }
            HirStmt::While { cond, body } => {
                expr(cond, f, g);
                block(body, f, g);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }
}

fn expr(e: &mut HirExpr, f: &mut impl FnMut(TyId) -> TyId, g: &mut impl FnMut(&mut HirExpr)) {
    e.ty = f(e.ty);
    match &mut e.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => {}
        HirExprKind::ArrayLit { elements } => elements.iter_mut().for_each(|x| expr(x, f, g)),
        HirExprKind::Index { receiver, index, .. } => {
            expr(receiver, f, g);
            expr(index, f, g);
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            expr(receiver, f, g);
            expr(lo, f, g);
            expr(hi, f, g);
        }
        HirExprKind::VecNew { cap, elem } => {
            *elem = f(*elem);
            cap.iter_mut().for_each(|x| expr(x, f, g));
        }
        HirExprKind::VecOp { receiver, args, .. } => {
            expr(receiver, f, g);
            args.iter_mut().for_each(|x| expr(x, f, g));
        }
        HirExprKind::Some(inner) | HirExprKind::RefCreate(inner) => expr(inner, f, g),
        HirExprKind::ObjectConstruct { fields, .. } => fields.iter_mut().for_each(|x| expr(x, f, g)),
        HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => expr(receiver, f, g),
        HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
            for b in std::iter::once(&mut *bindings.head).chain(&mut bindings.tail) {
                b.binding_ty = f(b.binding_ty);
                expr(&mut b.value, f, g);
            }
            block(some_block, f, g);
            none_block.iter_mut().for_each(|b| block(b, f, g));
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs, f, g);
            expr(rhs, f, g);
        }
        HirExprKind::Unary { expr: inner, .. } => expr(inner, f, g),
        HirExprKind::Call { callee, args, .. } => {
            expr(callee, f, g);
            args.iter_mut().for_each(|x| expr(x, f, g));
        }
        HirExprKind::If { cond, then_block, else_block } => {
            expr(cond, f, g);
            block(then_block, f, g);
            else_block.iter_mut().for_each(|b| block(b, f, g));
        }
    }
    g(e);
}
