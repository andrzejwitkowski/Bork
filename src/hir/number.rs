//! Assigns every `HirExpr` a unique `NodeId`, in a deterministic pre-order walk.

use super::{ChildMut, HirBlock, HirExpr, HirProgram, NodeId};

/// Run after the last pass that rewrites the HIR, so ids cover every node.
pub fn number_nodes(program: &mut HirProgram) {
    let mut next = 0;
    for function in &mut program.functions {
        block(&mut function.body, &mut next);
    }
}

fn block(b: &mut HirBlock, next: &mut u32) {
    for stmt in &mut b.stmts {
        stmt.children_mut().into_iter().for_each(|c| child(c, next));
    }
}

fn child(c: ChildMut<'_>, next: &mut u32) {
    match c {
        ChildMut::Expr(e) => expr(e, next),
        ChildMut::Block(b) => block(b, next),
    }
}

fn expr(e: &mut HirExpr, next: &mut u32) {
    e.id = NodeId(*next);
    *next += 1;
    e.children_mut().into_iter().for_each(|c| child(c, next));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::HirStmt;
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
