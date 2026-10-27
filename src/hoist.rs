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

/// `val n = <construct>` whose only use is a field of the returned construct is
/// allocated in the result sink, so it outlives the function with its owner.
pub const RESULT_SINK: &str = "$result";

fn hoist_into_result(block: &mut HirBlock, tcx: &TyCtx) {
    let Some(HirStmt::Return { value: Some(ret) }) = block.stmts.last() else { return };
    let mut used = Vec::new();
    match &ret.kind {
        HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
            collect_idents(fields, tcx, &mut used)
        }
        // `val a = A(..); return a` is `return A(..)`: `a` itself is born in the result sink.
        // Only a construct: a call's result may own bytes (a `String?`) that this does not move.
        HirExprKind::Ident { name, .. } if is_construct_local(block, name) => used.push(name.clone()),
        _ => return,
    }
    // A hoisted local's own constructor fields outlive it too (`a` in `val b = B(a)`).
    loop {
        let before = used.len();
        for stmt in block.stmts.iter() {
            if let HirStmt::VarDecl { name, value, .. } = stmt {
                if used.contains(name) {
                    if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &value.kind {
                        collect_idents(fields, tcx, &mut used);
                    }
                }
            }
        }
        used.sort();
        used.dedup();
        if used.len() == before {
            break;
        }
    }
    for stmt in &mut block.stmts {
        if let HirStmt::VarDecl { name, value, alloc_in_binding, .. } = stmt {
            // A call initializer is hoisted but its arguments are not traced: the planner makes every
            // object passed to a call program-lived (`memory.rs`, `Call`), so they outlive this frame.
            let constructs = matches!(
                value.kind,
                HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::Call { .. }
            );
            if constructs && used.contains(name) && alloc_in_binding.is_none() {
                *alloc_in_binding = Some(RESULT_SINK.to_string());
            }
        }
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

/// `name` is a `val` of this block initialised by a class or enum construct.
fn is_construct_local(block: &HirBlock, name: &str) -> bool {
    block.stmts.iter().any(|stmt| {
        matches!(stmt, HirStmt::VarDecl { kind: crate::ast::BindingKind::Val, name: n, value, .. }
            if n == name && matches!(value.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. }))
    })
}
