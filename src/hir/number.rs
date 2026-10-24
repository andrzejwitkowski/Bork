//! Assigns every `HirExpr` a unique `NodeId`, in a deterministic pre-order walk.

use super::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt, NodeId};

/// Run after the last pass that rewrites the HIR (`hoist::annotate`), so ids cover every node.
pub fn number_nodes(program: &mut HirProgram) {
    let mut next = 0;
    for function in &mut program.functions {
        block(&mut function.body, &mut next);
    }
}

fn block(b: &mut HirBlock, next: &mut u32) {
    for stmt in &mut b.stmts {
        match stmt {
            HirStmt::Return { value } => value.iter_mut().for_each(|v| expr(v, next)),
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => block(inner, next),
            HirStmt::VarDecl { value, .. } | HirStmt::Expr(value) => expr(value, next),
            HirStmt::Assign { target, value } => {
                match target {
                    HirAssignTarget::Name { .. } => {}
                    HirAssignTarget::Index { index, .. } => expr(index, next),
                    HirAssignTarget::Field { receiver, .. } => expr(receiver, next),
                }
                expr(value, next);
            }
            HirStmt::For { iter, body, .. } => {
                expr(iter, next);
                block(body, next);
            }
            HirStmt::While { cond, body } => {
                expr(cond, next);
                block(body, next);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }
}

fn expr(e: &mut HirExpr, next: &mut u32) {
    e.id = NodeId(*next);
    *next += 1;
    match &mut e.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => {}
        HirExprKind::ArrayLit { elements } => elements.iter_mut().for_each(|x| expr(x, next)),
        HirExprKind::Index { receiver, index, .. } => {
            expr(receiver, next);
            expr(index, next);
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            expr(receiver, next);
            expr(lo, next);
            expr(hi, next);
        }
        HirExprKind::VecNew { cap, .. } => cap.iter_mut().for_each(|x| expr(x, next)),
        HirExprKind::VecOp { receiver, args, .. } => {
            expr(receiver, next);
            args.iter_mut().for_each(|x| expr(x, next));
        }
        HirExprKind::Some(inner) | HirExprKind::RefCreate(inner) => expr(inner, next),
        HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => fields.iter_mut().for_each(|x| expr(x, next)),
        HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => expr(receiver, next),
        HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
            expr(&mut bindings.head.value, next);
            bindings.tail.iter_mut().for_each(|b| expr(&mut b.value, next));
            block(some_block, next);
            none_block.iter_mut().for_each(|b| block(b, next));
        }
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs, next);
            expr(rhs, next);
        }
        HirExprKind::Unary { expr: inner, .. } => expr(inner, next),
        HirExprKind::Call { callee, args, .. } => {
            expr(callee, next);
            args.iter_mut().for_each(|x| expr(x, next));
        }
        HirExprKind::If { cond, then_block, else_block } => {
            expr(cond, next);
            block(then_block, next);
            else_block.iter_mut().for_each(|b| block(b, next));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn collect(e: &HirExpr, out: &mut Vec<NodeId>) {
        out.push(e.id);
    }

    #[test]
    fn ids_are_unique_numbered_and_deterministic() {
        let src = "class P {\n x: i32\n}\nfun f(a: i32): i32 {\n val p = P(1)\n if (a > 1) { return 2 }\n return a + p.x\n}\nfun main() {\n val a = 1\n val b = 1\n println(a + b)\n}\n";
        let number = || {
            let mut hir = crate::frontend::check(src).hir.expect("typed HIR");
            number_nodes(&mut hir);
            hir
        };
        let (one, two) = (number(), number());
        assert_eq!(one, two);
        let mut ids = Vec::new();
        for f in &one.functions {
            for stmt in &f.body.stmts {
                if let HirStmt::VarDecl { value, .. } | HirStmt::Expr(value) = stmt {
                    collect(value, &mut ids);
                }
            }
        }
        assert!(ids.len() >= 3);
        assert!(ids.iter().all(|id| *id != NodeId::DUMMY));
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    }
}
