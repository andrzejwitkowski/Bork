//! Rewrites every type, and optionally every expression, in one function body: used to replace
//! literal inference variables and to substitute type arguments in monomorphized instances.

use super::{ChildMut, HirBlock, HirExpr, HirExprKind, HirPat, HirPatKind, HirStmt};
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
        if let HirStmt::VarDecl { ty, .. } = stmt {
            *ty = f(*ty);
        }
        stmt.children_mut().into_iter().for_each(|c| child(c, f, g));
    }
}

fn child(c: ChildMut<'_>, f: &mut impl FnMut(TyId) -> TyId, g: &mut impl FnMut(&mut HirExpr)) {
    match c {
        ChildMut::Expr(e) => expr(e, f, g),
        ChildMut::Block(b) => block(b, f, g),
    }
}

fn pat(p: &mut HirPat, f: &mut impl FnMut(TyId) -> TyId) {
    p.ty = f(p.ty);
    match &mut p.kind {
        HirPatKind::Bind { binding_ty, .. } => *binding_ty = f(*binding_ty),
        HirPatKind::Variant { fields, .. } => fields.iter_mut().for_each(|x| pat(x, f)),
        HirPatKind::Some(inner) => pat(inner, f),
        HirPatKind::Wildcard | HirPatKind::Lit(_) | HirPatKind::None => {}
    }
}

/// The types an expression holds besides its children's.
fn own_types(e: &mut HirExpr, f: &mut impl FnMut(TyId) -> TyId) {
    e.ty = f(e.ty);
    match &mut e.kind {
        HirExprKind::VecNew { elem, .. } => *elem = f(*elem),
        HirExprKind::PresenceMatch { bindings, .. } => {
            for b in std::iter::once(&mut *bindings.head).chain(&mut bindings.tail) {
                b.binding_ty = f(b.binding_ty);
            }
        }
        HirExprKind::Match { arms, .. } => arms.iter_mut().for_each(|arm| pat(&mut arm.pat, f)),
        HirExprKind::Closure { params, captures, .. } => {
            params.iter_mut().for_each(|p| p.ty = f(p.ty));
            captures.iter_mut().for_each(|c| c.ty = f(c.ty));
        }
        _ => {}
    }
}

fn expr(e: &mut HirExpr, f: &mut impl FnMut(TyId) -> TyId, g: &mut impl FnMut(&mut HirExpr)) {
    own_types(e, f);
    e.children_mut().into_iter().for_each(|c| child(c, f, g));
    g(e);
}
