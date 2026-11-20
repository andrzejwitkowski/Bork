//! What a closure body captures and how it uses it (RFC 0007): the capture list and the kind
//! (`Fn` reads, `FnMut` also assigns, `FnOnce` also moves a capture out).
//!
//! shortcut: a name bound by a pattern, a `for` or a presence binding shadows the outer one to the end of the
//! enclosing block, not just of its arm or loop body; tighten with real scopes if a program needs it.

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
    /// Names a `move (..)` block inside the body moves in on entry, which the scan of its body does not show.
    block_moved: Vec<String>,
    /// Names called as a local callable value, calling it is as strong as its kind.
    called: Vec<String>,
    /// `val alias = captured`: calling the alias calls the captured value.
    aliases: std::collections::HashMap<String, String>,
}

/// Names the closure uses from outside (`outer_ty` gives the type of a binding outside it, `None`
/// for any other name), in name order, and the strongest kind its body needs.
pub(super) fn analyze(params: &[HirParam], body: &HirBlock, outer_ty: impl Fn(&str) -> Option<TyId>, is_copy: impl Fn(TyId) -> bool, call_kind: impl Fn(TyId) -> ClosureKind) -> (Vec<(String, TyId)>, ClosureKind) {
    let mut scan = Scan::default();
    scan.declared.extend(params.iter().map(|p| p.name.clone()));
    scan.block(body);
    for name in std::mem::take(&mut scan.block_moved) {
        if let Some(ty) = outer_ty(&name) {
            scan.moved.push((name, ty));
        }
    }
    // The call node carries a plain `Fn` function type; the binding holds the real kind: a closure type, a `Func` written with `FnMut`/`FnOnce`, or a bounded parameter.
    for name in std::mem::take(&mut scan.called) {
        let Some(ty) = outer_ty(&name) else { continue };
        match call_kind(ty) {
            ClosureKind::Fn => {}
            ClosureKind::FnMut => {
                scan.assigned.insert(name);
            }
            ClosureKind::FnOnce => scan.moved.push((name, ty)),
        }
    }
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
        // Names declared inside a block go out of scope with it, so an outer name or alias it shadowed is back.
        let (declared, aliases) = (self.declared.clone(), self.aliases.clone());
        b.stmts.iter().for_each(|s| self.stmt(s));
        (self.declared, self.aliases) = (declared, aliases);
    }

    fn stmt(&mut self, s: &HirStmt) {
        match s {
            HirStmt::Return { value } => value.iter().for_each(|v| self.expr(v)),
            HirStmt::Block(inner) => self.block(inner),
            HirStmt::MoveBlock { captures, body } => {
                for name in captures.iter().flatten() {
                    if self.note(name) {
                        self.block_moved.push(name.clone());
                    }
                }
                let outer = std::mem::take(&mut self.used);
                self.block(body);
                // An omitted list moves in every outer name the body uses (sema's `resolve_move_captures`).
                if captures.is_none() {
                    self.block_moved.extend(self.used.iter().cloned());
                }
                self.used.extend(outer);
            }
            HirStmt::VarDecl { name, value, .. } => {
                self.expr(value);
                let alias = match &value.kind {
                    HirExprKind::Ident { name: from, .. } if !self.declared.contains(from) => Some(from.clone()),
                    _ => None,
                };
                self.declared.insert(name.clone());
                match alias {
                    Some(from) => self.aliases.insert(name.clone(), from),
                    None => self.aliases.remove(name),
                };
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
                if self.note(name) && matches!(use_kind, UseKind::Move | UseKind::Promote | UseKind::RefMove) {
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
                if let K::Ident { name, use_kind: UseKind::Copy } = &callee.kind {
                    match self.aliases.get(name) {
                        Some(from) => self.called.push(from.clone()),
                        None if !self.declared.contains(name) => self.called.push(name.clone()),
                        None => {}
                    }
                }
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
            K::Closure { captures, kind, is_move, .. } => {
                for c in captures {
                    if self.note(&c.name) {
                        // Building a `move` closure moves its captures out of ours, whatever it does with them.
                        if *is_move {
                            self.moved.push((c.name.clone(), c.ty));
                        }
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
