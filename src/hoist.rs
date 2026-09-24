//! Hoist owned initializer allocation into a definite outer binding's arena.

use std::collections::HashSet;

use crate::hir::{HirBlock, HirExprKind, HirFunction, HirProgram, HirStmt, UseKind};

pub fn annotate(program: &mut HirProgram) {
    for function in &mut program.functions {
        let visible: HashSet<String> = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect();
        annotate_block(&mut function.body, &visible);
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
            HirStmt::For { body, .. } => {
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
        name: outer_name,
        value: rhs,
    } = next?
    else {
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
