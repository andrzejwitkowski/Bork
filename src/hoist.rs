//! Hoist owned initializer allocation into a definite outer binding's arena.

use crate::hir::{HirBlock, HirExprKind, HirFunction, HirProgram, HirStmt, UseKind};

pub fn annotate(program: &mut HirProgram) {
    for function in &mut program.functions {
        annotate_function(function);
    }
}

fn annotate_function(function: &mut HirFunction) {
    annotate_block(&mut function.body);
}

fn annotate_block(block: &mut HirBlock) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Block(inner) => annotate_block(inner),
            HirStmt::MoveBlock { body, .. } => annotate_block(body),
            HirStmt::For { body, .. } => annotate_block(body),
            _ => {}
        }
    }
    annotate_siblings(&mut block.stmts);
}

fn annotate_siblings(stmts: &mut [HirStmt]) {
    let len = stmts.len();
    for index in 0..len.saturating_sub(1) {
        let (_, outer_name) = match (&stmts[index], &stmts[index + 1]) {
            (
                HirStmt::VarDecl {
                    name: inner,
                    value,
                    alloc_in_binding,
                    ..
                },
                HirStmt::Assign { name: outer, value: rhs },
            ) => {
                if alloc_in_binding.is_some() || !value_may_hoist(value) {
                    continue;
                }
                if !assign_moves_ident(rhs, inner) {
                    continue;
                }
                (inner.clone(), outer.clone())
            }
            _ => continue,
        };
        if let HirStmt::VarDecl {
            alloc_in_binding, ..
        } = &mut stmts[index]
        {
            *alloc_in_binding = Some(outer_name);
        }
    }
}

fn value_may_hoist(value: &crate::hir::HirExpr) -> bool {
    matches!(
        value.kind,
        HirExprKind::Str { .. } | HirExprKind::Call { .. } | HirExprKind::Some(_)
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
