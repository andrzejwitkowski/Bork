//! What a closure body captures and how it uses it (RFC 0007): the capture list and the kind
//! (`Fn` reads, `FnMut` also assigns, `FnOnce` also moves a capture out).
//!
//! shortcut: a name declared inside the closure shadows an outer one from its declaration to the end of the
//! closure (not to the end of its block); tighten with real scopes if a program needs it.

use std::collections::{BTreeSet, HashSet};

use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirParam, HirPat, HirPatKind, HirStmt, UseKind, VecOp};
use crate::ty::{ClosureKind, TyId};

#[derive(Default)]
struct Scan {
    declared: HashSet<String>,
    /// Every name read or written.
    used: BTreeSet<String>,
    assigned: HashSet<String>,
    /// Names moved out, with the moved value's type.
    moved: Vec<(String, TyId)>,
}

/// Names the closure uses from outside (`outer_ty` gives the type of a binding outside it, `None`
/// for any other name), in name order, and the strongest kind its body needs.
pub(super) fn analyze(params: &[HirParam], body: &HirBlock, outer_ty: impl Fn(&str) -> Option<TyId>, is_copy: impl Fn(TyId) -> bool) -> (Vec<(String, TyId)>, ClosureKind) {
    let mut scan = Scan::default();
    scan.declared.extend(params.iter().map(|p| p.name.clone()));
    scan.block(body);
    let captures: Vec<(String, TyId)> = scan
        .used
        .iter()
        .filter_map(|name| Some((name.clone(), outer_ty(name)?)))
        .collect();
    let captured = |name: &String| captures.iter().any(|(c, _)| c == name);
    // Moving a Copy value out of its binding only copies it, which is a read.
    let kind = if scan.moved.iter().any(|(n, t)| captured(n) && !is_copy(*t)) {
        ClosureKind::FnOnce
    } else if scan.assigned.iter().any(captured) {
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
                self.expr(value);
                self.declared.insert(name.clone());
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
                self.expr(iter);
                self.declared.insert(name.clone());
                self.block(body);
            }
            HirStmt::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }

    /// A use of `name` counts only while no declaration inside the closure shadows it.
    fn note(&mut self, name: &str) -> bool {
        let outer = !self.declared.contains(name);
        if outer {
            self.used.insert(name.to_string());
        }
        outer
    }

    fn write(&mut self, name: &str) {
        if self.note(name) {
            self.assigned.insert(name.to_string());
        }
    }

    fn write_root(&mut self, e: &HirExpr) {
        match &e.kind {
            HirExprKind::Ident { name, .. } => self.write(name),
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
                if self.note(name) && matches!(use_kind, UseKind::Move | UseKind::Promote) {
                    self.moved.push((name.clone(), e.ty));
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
            // `&rw x` hands out an editable borrow: the callee may write through it, so this is a write of `x`.
            K::Unary { op: crate::ast::UnaryOp::Borrow { rw: true }, expr: inner } => {
                self.expr(inner);
                self.write_root(inner);
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
            // What a nested closure does to a capture it does to ours: it makes this closure at least as strong.
            // shortcut: the nested kind applies to all its captures, not just the one it assigns or moves.
            K::Closure { captures, kind, .. } => {
                for c in captures {
                    if self.note(&c.name) {
                        match kind {
                            ClosureKind::Fn => {}
                            ClosureKind::FnMut => {
                                self.assigned.insert(c.name.clone());
                            }
                            ClosureKind::FnOnce => {
                                self.moved.push((c.name.clone(), c.ty));
                            }
                        }
                    }
                }
            }
        }
    }
}
