//! What a closure body captures and how it uses it (RFC 0007): the capture list and the kind
//! (`Fn` reads, `FnMut` also assigns, `FnOnce` also moves a capture out).
//!
//! shortcut: a name declared anywhere inside the closure shadows an outer one for the whole body
//! (flow-insensitive); an outer name used before such a declaration is missed. Tighten with real
//! scopes if a program needs it.

use std::collections::{BTreeSet, HashSet};

use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirParam, HirPat, HirPatKind, HirStmt, UseKind, VecOp};
use crate::ty::{ClosureKind, TyId};

#[derive(Default)]
struct Scan {
    declared: HashSet<String>,
    /// Every name read or written.
    used: BTreeSet<String>,
    assigned: HashSet<String>,
    moved: HashSet<String>,
}

/// Names the closure uses from outside (`outer_ty` gives the type of a binding outside it, `None`
/// for any other name), in name order, and the strongest kind its body needs.
pub(super) fn analyze(params: &[HirParam], body: &HirBlock, outer_ty: impl Fn(&str) -> Option<TyId>) -> (Vec<(String, TyId)>, ClosureKind) {
    let mut scan = Scan::default();
    scan.declared.extend(params.iter().map(|p| p.name.clone()));
    scan.block(body);
    let outer = |name: &String| !scan.declared.contains(name) && outer_ty(name).is_some();
    let captures: Vec<(String, TyId)> =
        scan.used.iter().filter(|name| outer(name)).filter_map(|n| Some((n.clone(), outer_ty(n)?))).collect();
    let kind = if scan.moved.iter().any(outer) {
        ClosureKind::FnOnce
    } else if scan.assigned.iter().any(outer) {
        ClosureKind::FnMut
    } else {
        ClosureKind::Fn
    };
    (captures, kind)
}

impl Scan {
    fn block(&mut self, b: &HirBlock) {
        b.stmts.iter().for_each(|s| self.stmt(s));
    }

    fn stmt(&mut self, s: &HirStmt) {
        match s {
            HirStmt::Return { value } => value.iter().for_each(|v| self.expr(v)),
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => self.block(inner),
            HirStmt::VarDecl { name, value, .. } => {
                self.declared.insert(name.clone());
                self.expr(value);
            }
            HirStmt::Expr(value) => self.expr(value),
            HirStmt::Assign { target, value } => {
                match target {
                    HirAssignTarget::Name { name } => self.write(name),
                    HirAssignTarget::Index { name, index } => {
                        self.write(name);
                        self.expr(index);
                    }
                    // Storing into a field updates the object the receiver names.
                    HirAssignTarget::Field { receiver, .. } => {
                        self.expr(receiver);
                        self.write_root(receiver);
                    }
                }
                self.expr(value);
            }
            HirStmt::For { name, iter, body } => {
                self.declared.insert(name.clone());
                self.expr(iter);
                self.block(body);
            }
            HirStmt::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }

    fn write(&mut self, name: &str) {
        self.assigned.insert(name.to_string());
        self.used.insert(name.to_string());
    }

    fn write_root(&mut self, e: &HirExpr) {
        match &e.kind {
            HirExprKind::Ident { name, .. } => {
                self.assigned.insert(name.clone());
            }
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => self.write_root(receiver),
            _ => {}
        }
    }

    fn pat(&mut self, p: &HirPat) {
        match &p.kind {
            HirPatKind::Bind { name, .. } => {
                self.declared.insert(name.name.clone());
            }
            HirPatKind::Variant { fields, .. } => fields.iter().for_each(|f| self.pat(f)),
            HirPatKind::Some(inner) => self.pat(inner),
            HirPatKind::Wildcard | HirPatKind::Lit(_) | HirPatKind::None => {}
        }
    }

    fn expr(&mut self, e: &HirExpr) {
        use HirExprKind as K;
        match &e.kind {
            K::Int { .. } | K::Float { .. } | K::Str { .. } | K::Bool { .. } | K::None => {}
            K::Ident { name, use_kind } => {
                self.used.insert(name.clone());
                if matches!(use_kind, UseKind::Move | UseKind::Promote) {
                    self.moved.insert(name.clone());
                }
            }
            K::ArrayLit { elements } => elements.iter().for_each(|x| self.expr(x)),
            K::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            K::Slice { receiver, lo, hi } => {
                self.expr(receiver);
                self.expr(lo);
                self.expr(hi);
            }
            K::VecNew { cap, .. } => cap.iter().for_each(|x| self.expr(x)),
            K::VecOp { op, receiver, args } => {
                self.expr(receiver);
                if matches!(op, VecOp::Push | VecOp::Pop | VecOp::Set | VecOp::Clear) {
                    self.write_root(receiver);
                }
                args.iter().for_each(|x| self.expr(x));
            }
            K::Some(inner) | K::RefCreate(inner) | K::Unary { expr: inner, .. } => self.expr(inner),
            K::ObjectConstruct { fields, .. } | K::EnumConstruct { fields, .. } => fields.iter().for_each(|x| self.expr(x)),
            K::ObjectField { receiver, .. } | K::Field { receiver, .. } => self.expr(receiver),
            K::PresenceMatch { bindings, some_block, none_block } => {
                for b in bindings.iter() {
                    self.expr(&b.value);
                    self.declared.insert(b.name.name.clone());
                }
                self.block(some_block);
                none_block.iter().for_each(|b| self.block(b));
            }
            K::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.pat(&arm.pat);
                    self.block(&arm.body);
                }
            }
            K::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            K::Call { callee, args, trailing } => {
                self.expr(callee);
                args.iter().for_each(|x| self.expr(x));
                trailing.iter().for_each(|x| self.expr(x));
            }
            K::If { cond, then_block, else_block } => {
                self.expr(cond);
                self.block(then_block);
                else_block.iter().for_each(|b| self.block(b));
            }
            // A nested closure's captures are uses of this one as well; its own locals stay its own.
            K::MakeClosure { captures, .. } => captures.iter().for_each(|x| self.expr(x)),
            K::Closure { captures, body, params, .. } => {
                for c in captures {
                    self.used.insert(c.name.clone());
                }
                let _ = (body, params);
            }
        }
    }
}
