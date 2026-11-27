//! The direct children of a HIR node, in evaluation order, so a pass that only recurses does not repeat the
//! shape of every variant.

use super::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirStmt};

pub enum ChildMut<'a> {
    Expr(&'a mut HirExpr),
    Block(&'a mut HirBlock),
}

impl HirStmt {
    pub fn children_mut(&mut self) -> Vec<ChildMut<'_>> {
        use ChildMut::{Block, Expr};
        match self {
            HirStmt::Return { value } => value.iter_mut().map(Expr).collect(),
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } => vec![Block(inner)],
            HirStmt::VarDecl { value, .. } | HirStmt::Expr(value) => vec![Expr(value)],
            HirStmt::Assign { target, value } => {
                let mut out = match target {
                    HirAssignTarget::Name { .. } => Vec::new(),
                    HirAssignTarget::Index { index, .. } => vec![Expr(index)],
                    HirAssignTarget::Field { receiver, .. } => vec![Expr(receiver)],
                };
                out.push(Expr(value));
                out
            }
            HirStmt::For { iter, body, .. } => vec![Expr(iter), Block(body)],
            HirStmt::While { cond, body } => vec![Expr(cond), Block(body)],
            HirStmt::Break { .. } | HirStmt::Continue { .. } => Vec::new(),
        }
    }
}

impl HirExpr {
    pub fn children_mut(&mut self) -> Vec<ChildMut<'_>> {
        use ChildMut::{Block, Expr};
        match &mut self.kind {
            HirExprKind::Int { .. } | HirExprKind::Float { .. } | HirExprKind::Str { .. } | HirExprKind::Bool { .. } | HirExprKind::Ident { .. } | HirExprKind::None => Vec::new(),
            HirExprKind::ArrayLit { elements } => elements.iter_mut().map(Expr).collect(),
            HirExprKind::Index { receiver, index, .. } => vec![Expr(receiver), Expr(index)],
            HirExprKind::Slice { receiver, lo, hi } => vec![Expr(receiver), Expr(lo), Expr(hi)],
            HirExprKind::Some(inner) | HirExprKind::RefCreate(inner) | HirExprKind::Unary { expr: inner, .. } => vec![Expr(inner)],
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => fields.iter_mut().map(Expr).collect(),
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. } => vec![Expr(receiver)],
            HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
                let values = std::iter::once(&mut *bindings.head).chain(&mut bindings.tail).map(|b| Expr(&mut b.value));
                values.chain(std::iter::once(Block(some_block))).chain(none_block.iter_mut().map(Block)).collect()
            }
            HirExprKind::Match { scrutinee, arms } => std::iter::once(Expr(scrutinee)).chain(arms.iter_mut().map(|arm| Block(&mut arm.body))).collect(),
            HirExprKind::Binary { lhs, rhs, .. } => vec![Expr(lhs), Expr(rhs)],
            HirExprKind::Call { callee, args } => std::iter::once(Expr(callee)).chain(args.iter_mut().map(Expr)).collect(),
            HirExprKind::Closure { body, .. } => vec![Block(body)],
            HirExprKind::MakeClosure { captures, .. } => captures.iter_mut().map(Expr).collect(),
            HirExprKind::If { cond, then_block, else_block } => std::iter::once(Expr(cond)).chain(std::iter::once(Block(then_block))).chain(else_block.iter_mut().map(Block)).collect(),
        }
    }
}
