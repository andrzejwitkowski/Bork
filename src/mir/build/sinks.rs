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
}

impl Sink {
    /// The string `hoist::annotate` stores in `HirStmt::VarDecl::alloc_in_binding` for the same decision.
    pub(crate) fn annotation(&self) -> &str {
        match self {
            Sink::Result => crate::hoist::RESULT_SINK,
            Sink::Outer(name) => name,
        }
    }
}

pub(crate) fn decl_sinks(function: &HirFunction, tcx: &TyCtx) -> HashMap<NodeId, Sink> {
    let mut sinks = HashMap::new();
    let visible: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
    outer_hoists(&function.body, &visible, &mut sinks);
    result_hoists(&function.body, tcx, &mut sinks);
    sinks
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
            if decls.iter().any(|(n, v)| n == name && matches!(v.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. })) {
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
                let constructs = matches!(value.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::Call { .. });
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

