//! Hoist owned initializer allocation into a definite outer binding's arena.

use std::collections::HashSet;

use crate::hir::{HirAssignTarget, HirBlock, HirExprKind, HirProgram, HirStmt, UseKind};

pub fn annotate(program: &mut HirProgram) {
    for function in &mut program.functions {
        let visible: HashSet<String> = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        annotate_block(&mut function.body, &visible);
        hoist_into_result(&mut function.body);
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

fn hoist_into_result(block: &mut HirBlock) {
    let Some(HirStmt::Return { value: Some(ret) }) = block.stmts.last() else { return };
    let (HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. }) = &ret.kind else { return };
    let mut used = Vec::new();
    collect_idents(fields, &mut used);
    // A hoisted local's own constructor fields outlive it too (`a` in `val b = B(a)`).
    loop {
        let before = used.len();
        for stmt in block.stmts.iter() {
            if let HirStmt::VarDecl { name, value, .. } = stmt {
                if used.contains(name) {
                    if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &value.kind {
                        collect_idents(fields, &mut used);
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
pub fn collect_idents(fields: &[crate::hir::HirExpr], out: &mut Vec<String>) {
    for field in fields {
        match &field.kind {
            HirExprKind::Ident { name, .. } => out.push(name.clone()),
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                collect_idents(fields, out)
            }
            // `local.n` stored in the result still points into `local`'s arena.
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => {
                collect_idents(std::slice::from_ref(receiver.as_ref()), out)
            }
            _ => {}
        }
    }
}
