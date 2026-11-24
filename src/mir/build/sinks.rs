//! Where a `val`/`var` initialiser must be allocated so that it outlives its region (RFC 0006 3.3): the arena of the
//! outer binding it is moved into on the next statement, or the caller's result arena when it is returned. Computed
//! on the monomorphic HIR, one decision per declaration keyed by its initialiser's node id.

use std::collections::{HashMap, HashSet};

use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, NodeId, UseKind};
use crate::ty::TyCtx;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Sink {
    /// The caller's result arena.
    Result,
    /// The arena of this outer binding.
    Outer(String),
    /// The current region: a callable call result bound in a loop and only ever called, so it cannot outlive the
    /// iteration (#327).
    Local,
}

pub(crate) fn decl_sinks(function: &HirFunction, tcx: &TyCtx) -> HashMap<NodeId, Sink> {
    let mut sinks = HashMap::new();
    let visible: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
    outer_hoists(&function.body, &visible, &mut sinks);
    result_hoists(&function.body, tcx, &mut sinks);
    local_loop_calls(&function.body, tcx, &function.body, false, &mut sinks);
    sinks
}

/// `val f = call()` in a loop body whose result is callable or moves a handle stays in the iteration's region when every
/// mention of `f` in the function is the callee of a call or a `when`/`if val` scrutinee: nothing can carry the value
/// out of the iteration (`return f` would hand out the loop slot, which `materialize` does not copy).
/// shortcut: name-based (a shadowing or reused name just keeps the result sink), refine with binding ids if it matters.
fn local_loop_calls(block: &HirBlock, tcx: &TyCtx, root: &HirBlock, in_loop: bool, out: &mut HashMap<NodeId, Sink>) {
    for stmt in &block.stmts {
        if let HirStmt::VarDecl { name, value, .. } = stmt {
            let called = matches!(value.kind, HirExprKind::Call { .. }) && (tcx.is_callable(value.ty) || moves_handle(tcx, value.ty));
            if in_loop && called && !out.contains_key(&value.id) && mentions_only_as_callee(root, name) {
                out.insert(value.id, Sink::Local);
            }
        }
        // `when (call())` in a loop: the arms bind clones, so the scrutinee itself never leaves the iteration.
        if let (true, HirStmt::Expr(e)) = (in_loop, stmt) {
            if let HirExprKind::Match { scrutinee, .. } = &e.kind {
                let call = matches!(scrutinee.kind, HirExprKind::Call { .. });
                if call && (tcx.is_callable(scrutinee.ty) || moves_handle(tcx, scrutinee.ty)) {
                    out.entry(scrutinee.id).or_insert(Sink::Local);
                }
            }
        }
        let inner_loop = in_loop || matches!(stmt, HirStmt::For { .. } | HirStmt::While { .. });
        for inner in nested_blocks(stmt) {
            local_loop_calls(inner, tcx, root, inner_loop, out);
        }
    }
}

/// Mirrors `Builder::moves_handle`.
fn moves_handle(t: &TyCtx, ty: crate::ty::TyId) -> bool {
    let payload = if t.is_nullable(ty) { t.enum_args(ty).and_then(|(_, a)| a.first().copied()).unwrap_or(ty) } else { ty };
    t.enum_args(payload).is_some() || (t.record_def(ty).is_some() && !t.is_managed_ref(ty))
}

/// Whether `name` is declared once in `block` and every mention of it is a callee or a match scrutinee.
fn mentions_only_as_callee(block: &HirBlock, name: &str) -> bool {
    use crate::hir::ChildMut;
    fn walk(children: Vec<ChildMut<'_>>, name: &str, uses: &mut usize) -> bool {
        for child in children {
            let ok = match child {
                ChildMut::Block(b) => b.stmts.iter_mut().all(|s| walk(s.children_mut(), name, uses)),
                ChildMut::Expr(e) => match &mut e.kind {
                    HirExprKind::Ident { name: n, .. } => n != name,
                    HirExprKind::Call { callee, args } => {
                        let callee_is_name = matches!(&callee.kind, HirExprKind::Ident { name: n, .. } if n == name);
                        *uses += usize::from(callee_is_name);
                        let callee_ok = callee_is_name || walk(vec![ChildMut::Expr(callee)], name, uses);
                        callee_ok && args.iter_mut().all(|a| walk(vec![ChildMut::Expr(a)], name, uses))
                    }
                    HirExprKind::Match { scrutinee, arms } if matches!(&scrutinee.kind, HirExprKind::Ident { name: n, .. } if n == name) => {
                        arms.iter_mut().all(|a| walk(vec![ChildMut::Block(&mut a.body)], name, uses))
                    }
                    HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
                        let head_is_name = bindings.tail.is_empty() && matches!(&bindings.head.value.kind, HirExprKind::Ident { name: n, .. } if n == name);
                        if head_is_name {
                            walk(std::iter::once(ChildMut::Block(some_block)).chain(none_block.iter_mut().map(ChildMut::Block)).collect(), name, uses)
                        } else {
                            walk(e.children_mut(), name, uses)
                        }
                    }
                    _ => walk(e.children_mut(), name, uses),
                },
            };
            if !ok {
                return false;
            }
        }
        true
    }
    let mut decls = Vec::new();
    collect_decls(block, &mut decls);
    if decls.iter().filter(|(n, _)| n == name).count() != 1 {
        return false;
    }
    let mut uses = 0;
    let mut body = block.clone();
    body.stmts.iter_mut().all(|s| walk(s.children_mut(), name, &mut uses))
}

fn outer_hoists(block: &HirBlock, outer: &HashSet<String>, out: &mut HashMap<NodeId, Sink>) {
    let mut local = HashSet::new();
    for (index, stmt) in block.stmts.iter().enumerate() {
        if let (Some(name), HirStmt::VarDecl { value, .. }) = (hoist_target(stmt, block.stmts.get(index + 1), outer, &local), stmt) {
            out.insert(value.id, Sink::Outer(name));
        }
        match stmt {
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } | HirStmt::For { body: inner, .. } | HirStmt::While { body: inner, .. } => {
                let mut nested = outer.clone();
                nested.extend(local.iter().cloned());
                outer_hoists(inner, &nested, out);
            }
            HirStmt::VarDecl { name, .. } => {
                local.insert(name.clone());
            }
            _ => {}
        }
    }
}

/// `val inner = <init>` immediately followed by `outer = move inner`, with `outer` already in scope.
fn hoist_target(decl: &HirStmt, next: Option<&HirStmt>, outer: &HashSet<String>, local: &HashSet<String>) -> Option<String> {
    let HirStmt::VarDecl { name: inner, value, .. } = decl else { return None };
    let HirStmt::Assign { target: HirAssignTarget::Name { name: outer_name }, value: rhs } = next? else { return None };
    if !value_may_hoist(value) || !assign_moves_ident(rhs, inner) {
        return None;
    }
    (outer.contains(outer_name) || local.contains(outer_name)).then(|| outer_name.clone())
}

fn value_may_hoist(value: &HirExpr) -> bool {
    matches!(
        value.kind,
        HirExprKind::Str { .. } | HirExprKind::Call { .. } | HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. }
    )
}

fn assign_moves_ident(value: &HirExpr, name: &str) -> bool {
    matches!(&value.kind, HirExprKind::Ident { name: rhs, use_kind: UseKind::Move } if rhs == name)
}

/// A `val` stored in (or returned as) any `return` of the function, at any depth, is allocated in the result sink, so
/// it outlives the function with its owner. Declarations inside loop bodies are skipped.
fn result_hoists(body: &HirBlock, tcx: &TyCtx, out: &mut HashMap<NodeId, Sink>) {
    let mut returns = Vec::new();
    collect_returns(body, &mut returns);
    let mut used = Vec::new();
    let mut tails: Vec<&HirExpr> = Vec::new();
    for ret in &returns {
        value_tails(ret, &mut tails);
    }
    for ret in &tails {
        if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &ret.kind {
            collect_idents(fields, tcx, &mut used);
        }
    }
    let mut decls = Vec::new();
    collect_decls(body, &mut decls);
    for ret in &tails {
        // Only a construct: a call's result may own bytes (a `String?`) that this does not move.
        if let HirExprKind::Ident { name, .. } = &ret.kind {
            if decls.iter().any(|(n, v)| n == name && matches!(v.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::MakeClosure { .. })) {
                used.push(name.clone());
            }
        }
    }
    loop {
        let before = used.len();
        for (name, value) in &decls {
            if used.contains(name) {
                if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &value.kind {
                    collect_idents(fields, tcx, &mut used);
                }
            }
        }
        used.sort();
        used.dedup();
        if used.len() == before {
            break;
        }
    }
    mark_decls(body, &used, false, out);
}

/// A declaration inside a loop body is left alone: hoisting would allocate in the caller's arena on every iteration and
/// never free it.
fn mark_decls(block: &HirBlock, used: &[String], in_loop: bool, out: &mut HashMap<NodeId, Sink>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::VarDecl { name, value, .. } => {
                let constructs = matches!(value.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::Call { .. } | HirExprKind::MakeClosure { .. });
                if constructs && !in_loop && used.contains(name) {
                    out.entry(value.id).or_insert(Sink::Result);
                }
            }
            HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } => mark_decls(b, used, in_loop, out),
            HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => mark_decls(b, used, true, out),
            HirStmt::Expr(e) => expr_blocks(e).into_iter().for_each(|b| mark_decls(b, used, in_loop, out)),
            _ => {}
        }
    }
}

pub(crate) fn nested_blocks(stmt: &HirStmt) -> Vec<&HirBlock> {
    match stmt {
        HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } | HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => vec![b],
        HirStmt::Expr(e) => expr_blocks(e),
        _ => vec![],
    }
}

pub(crate) fn expr_blocks(e: &crate::hir::HirExpr) -> Vec<&HirBlock> {
    match &e.kind {
        HirExprKind::If { then_block, else_block, .. } => std::iter::once(then_block).chain(else_block.iter()).collect(),
        HirExprKind::Match { arms, .. } => arms.iter().map(|arm| &arm.body).collect(),
        HirExprKind::PresenceMatch { some_block, none_block, .. } => std::iter::once(some_block).chain(none_block.iter()).collect(),
        _ => vec![],
    }
}

pub(crate) fn collect_returns<'a>(block: &'a HirBlock, out: &mut Vec<&'a crate::hir::HirExpr>) {
    for stmt in &block.stmts {
        if let HirStmt::Return { value: Some(v) } = stmt {
            out.push(v);
        }
        for inner in nested_blocks(stmt) {
            collect_returns(inner, out);
        }
    }
}

pub(crate) fn collect_decls<'a>(block: &'a HirBlock, out: &mut Vec<(String, &'a crate::hir::HirExpr)>) {
    for stmt in &block.stmts {
        if let HirStmt::VarDecl { name, value, .. } = stmt {
            out.push((name.clone(), value));
        }
        for inner in nested_blocks(stmt) {
            collect_decls(inner, out);
        }
    }
}

/// The expressions a returned value can actually be: itself, or for an `if`/`else` the tail
/// expression of each branch, recursively. `return if (f) { A(p) } else { A(q) }` stores `p` or `q`.
pub(crate) fn value_tails<'a>(expr: &'a crate::hir::HirExpr, out: &mut Vec<&'a crate::hir::HirExpr>) {
    match &expr.kind {
        HirExprKind::If { then_block, else_block, .. } => {
            for block in std::iter::once(then_block).chain(else_block.iter()) {
                if let Some(HirStmt::Expr(tail)) = block.stmts.last() {
                    value_tails(tail, out);
                }
            }
        }
        _ => out.push(expr),
    }
}

/// Names used directly as fields of a construct, looking through nested constructs.
pub(crate) fn collect_idents(fields: &[crate::hir::HirExpr], tcx: &TyCtx, out: &mut Vec<String>) {
    for field in fields {
        match &field.kind {
            HirExprKind::Ident { name, .. } => out.push(name.clone()),
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                collect_idents(fields, tcx, out)
            }
            // `local.n` stored in the result still points into `local`'s arena.
            // Only when the field is itself an object handle: a scalar read copies the value.
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. }
                if tcx.holds_object(field.ty) =>
            {
                collect_idents(std::slice::from_ref(receiver.as_ref()), tcx, out)
            }
            _ => {}
        }
    }
}

