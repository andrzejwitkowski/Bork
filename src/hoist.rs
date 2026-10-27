//! Hoist owned initializer allocation into a definite outer binding's arena.

use std::collections::HashSet;

use crate::ty::{TyCtx, TyData, TyId};
use crate::hir::{HirAssignTarget, HirBlock, HirExprKind, HirProgram, HirStmt, UseKind};

pub fn annotate(program: &mut HirProgram, tcx: &TyCtx) {
    for function in &mut program.functions {
        let visible: HashSet<String> = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        annotate_block(&mut function.body, &visible);
        hoist_into_result(&mut function.body, tcx);
    }
}

fn annotate_block(block: &mut HirBlock, outer: &HashSet<String>) {
    let mut local = HashSet::new();
    let len = block.stmts.len();
    for index in 0..len {
        try_hoist(&mut block.stmts, index, outer, &local);
        match &mut block.stmts[index] {
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => {
                let mut nested = outer.clone();
                nested.extend(local.iter().cloned());
                annotate_block(inner, &nested);
            }
            HirStmt::For { body, .. } | HirStmt::While { body, .. } => {
                let mut nested = outer.clone();
                nested.extend(local.iter().cloned());
                annotate_block(body, &nested);
            }
            HirStmt::VarDecl { name, .. } => {
                local.insert(name.clone());
            }
            _ => {}
        }
    }
}

fn try_hoist(
    stmts: &mut [HirStmt],
    index: usize,
    outer: &HashSet<String>,
    local: &HashSet<String>,
) {
    let Some(outer_name) = hoist_target(&stmts[index], stmts.get(index + 1), outer, local) else {
        return;
    };
    if let HirStmt::VarDecl {
        alloc_in_binding, ..
    } = &mut stmts[index]
    {
        *alloc_in_binding = Some(outer_name);
    }
}

fn hoist_target(
    decl: &HirStmt,
    next: Option<&HirStmt>,
    outer: &HashSet<String>,
    local: &HashSet<String>,
) -> Option<String> {
    let HirStmt::VarDecl {
        name: inner,
        value,
        alloc_in_binding,
        ..
    } = decl
    else {
        return None;
    };
    let HirStmt::Assign {
        target,
        value: rhs,
    } = next?
    else {
        return None;
    };
    let HirAssignTarget::Name { name: outer_name } = target else {
        return None;
    };
    if alloc_in_binding.is_some() || !value_may_hoist(value) || !assign_moves_ident(rhs, inner) {
        return None;
    }
    if !outer.contains(outer_name) && !local.contains(outer_name) {
        return None;
    }
    Some(outer_name.clone())
}

fn value_may_hoist(value: &crate::hir::HirExpr) -> bool {
    matches!(
        value.kind,
        HirExprKind::Str { .. } | HirExprKind::Call { .. }
    )
}

fn assign_moves_ident(value: &crate::hir::HirExpr, name: &str) -> bool {
    matches!(
        &value.kind,
        HirExprKind::Ident {
            name: rhs,
            use_kind: UseKind::Move,
        } if rhs == name
    )
}

/// A `val` stored in (or returned as) any `return` of the function, at any depth, is allocated in the
/// result sink, so it outlives the function with its owner. Declarations inside loop bodies are skipped.
pub const RESULT_SINK: &str = "$result";

fn hoist_into_result(body: &mut HirBlock, tcx: &TyCtx) {
    // EXPERIMENT: every `return` at any depth feeds one set of names to hoist.
    let mut returns = Vec::new();
    collect_returns(body, &mut returns);
    let mut used = Vec::new();
    let tails: Vec<&crate::hir::HirExpr> = {
        let mut out = Vec::new();
        for ret in &returns {
            value_tails(ret, &mut out);
        }
        out
    };
    for ret in &tails {
        match &ret.kind {
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                collect_idents(fields, tcx, &mut used)
            }
            _ => {}
        }
    }
    let mut decls = Vec::new();
    collect_decls(body, &mut decls);
    for ret in &tails {
        // Only a construct: a call's result may own bytes (a `String?`) that this does not move.
        if let HirExprKind::Ident { name, .. } = &ret.kind {
            let is_construct = decls.iter().any(|(n, v)| n == name && matches!(v.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. }));
            if is_construct {
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
    mark_decls(body, &used, false);
}

fn nested_blocks(stmt: &HirStmt) -> Vec<&HirBlock> {
    match stmt {
        HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } | HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => vec![b],
        HirStmt::Expr(e) => expr_blocks(e),
        _ => vec![],
    }
}

fn expr_blocks(e: &crate::hir::HirExpr) -> Vec<&HirBlock> {
    match &e.kind {
        HirExprKind::If { then_block, else_block, .. } => std::iter::once(then_block).chain(else_block.iter()).collect(),
        HirExprKind::Match { arms, .. } => arms.iter().map(|arm| &arm.body).collect(),
        HirExprKind::PresenceMatch { some_block, none_block, .. } => std::iter::once(some_block).chain(none_block.iter()).collect(),
        _ => vec![],
    }
}

fn collect_returns<'a>(block: &'a HirBlock, out: &mut Vec<&'a crate::hir::HirExpr>) {
    for stmt in &block.stmts {
        if let HirStmt::Return { value: Some(v) } = stmt {
            out.push(v);
        }
        for inner in nested_blocks(stmt) {
            collect_returns(inner, out);
        }
    }
}

fn collect_decls<'a>(block: &'a HirBlock, out: &mut Vec<(String, &'a crate::hir::HirExpr)>) {
    for stmt in &block.stmts {
        if let HirStmt::VarDecl { name, value, .. } = stmt {
            out.push((name.clone(), value));
        }
        for inner in nested_blocks(stmt) {
            collect_decls(inner, out);
        }
    }
}

/// Marks `val`s for the result sink. A declaration inside a loop body is left alone: hoisting it would
/// allocate in the caller's arena on every iteration and never free it. `escape` rejects it instead.
fn mark_decls(block: &mut HirBlock, used: &[String], in_loop: bool) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::VarDecl { name, value, alloc_in_binding, .. } => {
                let constructs = matches!(
                    value.kind,
                    HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::Call { .. }
                );
                if constructs && !in_loop && used.contains(name) && alloc_in_binding.is_none() {
                    *alloc_in_binding = Some(RESULT_SINK.to_string());
                }
            }
            HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } => mark_decls(b, used, in_loop),
            HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => mark_decls(b, used, true),
            HirStmt::Expr(e) => match &mut e.kind {
                HirExprKind::If { then_block, else_block, .. } => {
                    mark_decls(then_block, used, in_loop);
                    if let Some(b) = else_block { mark_decls(b, used, in_loop); }
                }
                HirExprKind::Match { arms, .. } => arms.iter_mut().for_each(|arm| mark_decls(&mut arm.body, used, in_loop)),
                HirExprKind::PresenceMatch { some_block, none_block, .. } => {
                    mark_decls(some_block, used, in_loop);
                    if let Some(b) = none_block { mark_decls(b, used, in_loop); }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// The expressions a returned value can actually be: itself, or for an `if`/`else` the tail
/// expression of each branch, recursively. `return if (f) { A(p) } else { A(q) }` stores `p` or `q`.
pub fn value_tails<'a>(expr: &'a crate::hir::HirExpr, out: &mut Vec<&'a crate::hir::HirExpr>) {
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
pub fn collect_idents(fields: &[crate::hir::HirExpr], tcx: &TyCtx, out: &mut Vec<String>) {
    for field in fields {
        match &field.kind {
            HirExprKind::Ident { name, .. } => out.push(name.clone()),
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                collect_idents(fields, tcx, out)
            }
            // `local.n` stored in the result still points into `local`'s arena.
            // Only when the field is itself an object handle: a scalar read copies the value.
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. }
                if holds_object(tcx, field.ty) =>
            {
                collect_idents(std::slice::from_ref(receiver.as_ref()), tcx, out)
            }
            _ => {}
        }
    }
}

/// A class or enum value, nullable or not: a handle into some arena.
pub fn holds_object(tcx: &TyCtx, ty: TyId) -> bool {
    let ty = tcx.base(ty);
    tcx.class_def(ty).is_some() || matches!(tcx.data(ty), TyData::Enum(..))
}
