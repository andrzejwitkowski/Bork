//! Type- and form-driven `return` rules of the escape check (RFC 0006, #269): they judge the
//! returned expression's type and syntax, not arena depth, so they live apart from `escape.rs`.

use crate::ast::BinOp;
use crate::builtins;
use crate::diag::Diagnostic;
use crate::hir::{peel_to_body, HirBlock, HirExpr, HirExprKind, HirStmt, UseKind};
use crate::ty::TyCtx;

/// The "returning a reference" rule: a pure type check.
pub(crate) fn returns_reference(tcx: &TyCtx, value: &HirExpr) -> bool {
    tcx.is_ref(value.ty)
}

/// Rejects a returned `String` that is a fresh allocation or a moved/promoted binding.
pub(crate) fn check_return_string_form(tcx: &TyCtx, diagnostics: &mut Vec<Diagnostic>, expr: &HirExpr) {
    ReturnForm { tcx, diagnostics }.check_return_string_form(expr);
}

struct ReturnForm<'d> {
    tcx: &'d TyCtx,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl ReturnForm<'_> {
    fn check_return_string_form(&mut self, expr: &HirExpr) {
        if self.tcx.is_string(expr.ty) && self.is_fresh_string(expr) {
            crate::escape::reject(
                self.diagnostics,
                "returning the result of `concat` or `stringFromUtf8` is not supported yet: \
                 returned `String` bytes must outlive the callee arena",
                expr.span,
            );
            return;
        }
        if self.is_moved_or_promoted_string(expr) {
            let note = crate::escape::param_note(self.tcx, expr.ty);
            crate::escape::reject(
                self.diagnostics,
                &format!(
                    "returning a moved or promoted `String` is not supported yet: use \
                     `return s` for parameters and locals, or return a string literal{note}"
                ),
                expr.span,
            );
            return;
        }
        match &expr.kind {
            HirExprKind::If {
                then_block,
                else_block,
                ..
            }
            | HirExprKind::PresenceMatch {
                some_block: then_block,
                none_block: else_block,
                ..
            } => {
                self.check_return_string_in_block(then_block);
                if let Some(else_block) = else_block {
                    self.check_return_string_in_block(else_block);
                }
            }
            _ => {}
        }
    }

    fn check_return_string_in_block(&mut self, block: &HirBlock) {
        let block = peel_to_body(block);
        if let Some(HirStmt::Expr(expr)) = block.stmts.last() {
            self.check_return_string_form(expr);
        }
    }

    /// A value whose bytes are allocated by an allocating builtin, possibly wrapped in
    /// `Some`, `!!`, `?:`, `if`, or `when` whose branch ends in one.
    fn is_fresh_string(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Call { callee, .. } => matches!(
                &callee.kind,
                HirExprKind::Ident { name, .. } if builtins::is_string_alloc(name)
            ),
            HirExprKind::Some(inner)
            | HirExprKind::Unary { expr: inner, .. } => self.is_fresh_string(inner),
            HirExprKind::Binary { op: BinOp::Elvis, lhs, rhs } => {
                self.is_fresh_string(lhs) || self.is_fresh_string(rhs)
            }
            HirExprKind::If { then_block, else_block, .. } => {
                self.block_ends_fresh(then_block)
                    || else_block.as_ref().is_some_and(|block| self.block_ends_fresh(block))
            }
            HirExprKind::PresenceMatch {
                some_block,
                none_block,
                ..
            } => {
                self.block_ends_fresh(some_block)
                    || none_block.as_ref().is_some_and(|block| self.block_ends_fresh(block))
            }
            _ => false,
        }
    }

    fn block_ends_fresh(&self, block: &HirBlock) -> bool {
        matches!(peel_to_body(block).stmts.last(), Some(HirStmt::Expr(expr)) if self.is_fresh_string(expr))
    }

    fn is_moved_or_promoted_string(&self, expr: &HirExpr) -> bool {
        matches!(
            &expr.kind,
            HirExprKind::Ident {
                use_kind: UseKind::Move | UseKind::Promote,
                ..
            } if self.tcx.is_string(expr.ty) || self.tcx.is_param(expr.ty)
        )
    }
}
