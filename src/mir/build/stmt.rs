//! Statement lowering, loops and early exits.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Res};
use crate::ast::{BinOp, BindingKind};
use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirStmt};
use crate::mir::body::*;
use crate::region_walk::RegionSite;
use crate::span::Span;

impl<'a> FnBuilder<'a> {
    pub(super) fn stmts(&mut self, block: &HirBlock) -> Res<()> {
        for (i, s) in block.stmts.iter().enumerate() {
            self.stmt_of(s, block.statement_spans.get(i).copied().unwrap_or_default())?;
        }
        Ok(())
    }

    pub(super) fn stmt_of(&mut self, s: &HirStmt, span: Span) -> Res<()> {
        match s {
            HirStmt::Return { value } => {
                let v = value.as_ref().map(|v| self.expr(v)).transpose()?;
                if let Some(v) = v.filter(|_| !self.is_unit(self.function.return_ty)) {
                    self.stmt(StmtKind::Assign(Place::local(self.ret_local()), Rvalue::Use(v)), span, None);
                }
                self.exit_to(0, span);
                self.terminate(TermKind::Return, span, None);
                Ok(())
            }
            HirStmt::Block(b) => self.region_block(RegionSite::Block, b, span),
            HirStmt::MoveBlock { body, .. } => self.region_block(RegionSite::MoveBlock, body, span),
            HirStmt::VarDecl { kind, name, ty, value, alloc_in_binding } => {
                if alloc_in_binding.is_some() {
                    return Err(BuildError::not_yet("hoisted binding", span));
                }
                let v = self.expr(value)?;
                let local_kind = if matches!(kind, BindingKind::Val) { LocalKind::Val } else { LocalKind::Var };
                let local = self.new_local(*ty, local_kind, Some(name.clone()));
                let label = DebugLabel {
                    name: name.clone(),
                    ty: *ty,
                    span,
                    source: self.function.source,
                    declaration: true,
                    argument: None,
                    owner: None,
                    index: None,
                };
                self.stmt_with(StmtKind::Assign(Place::local(local), Rvalue::Use(v)), span, Some(value.id), Some(label));
                self.declare(name, local);
                Ok(())
            }
            HirStmt::Assign { target, value } => match target {
                HirAssignTarget::Name { name } => {
                    let local = self.lookup(name).ok_or_else(|| BuildError::not_yet(format!("name `{name}`"), span))?;
                    let v = self.with_sink(ArenaRef::Home(local), |b| b.expr(value))?;
                    self.stmt(StmtKind::Assign(Place::local(local), Rvalue::Use(v)), span, Some(value.id));
                    Ok(())
                }
                HirAssignTarget::Index { name, index } => self.assign_index(name, index, value, span),
                HirAssignTarget::Field { .. } => Err(BuildError::not_yet("field assignment", span)),
            },
            HirStmt::Expr(e) => self.expr(e).map(|_| ()),
            HirStmt::While { cond, body } => self.while_loop(cond, body, span),
            HirStmt::For { name, iter, body } => self.for_loop(name, iter, body, span),
            HirStmt::Break { span } => self.jump(true, *span),
            HirStmt::Continue { span } => self.jump(false, *span),
        }
    }

    fn ret_local(&self) -> Local {
        Local(0)
    }

    fn region_block(&mut self, site: RegionSite, b: &HirBlock, span: Span) -> Res<()> {
        let (peeled, _, _) = self.enter_region(site, b, false, None, span)?;
        self.stmts(peeled)?;
        self.exit_region(span)
    }

    /// `break` leaves the loop's inner regions and goes to its exit (which pops the loop region);
    /// `continue` goes to the latch, which resets the loop region.
    fn jump(&mut self, is_break: bool, span: Span) -> Res<()> {
        let (exit, latch, depth) = self.loop_ctx().ok_or_else(|| BuildError::Internal("break outside a loop".into()))?;
        self.exit_to(depth, span);
        self.terminate(TermKind::Goto(if is_break { exit } else { latch }), span, None);
        Ok(())
    }

    fn while_loop(&mut self, cond: &HirExpr, body: &HirBlock, span: Span) -> Res<()> {
        let pre = self.cur;
        let header = self.new_block();
        self.goto(header, span);
        self.cur = header;
        // The condition runs inside the loop region at run time, but its own report regions come
        // first in the walk, so the loop region is opened (in `pre`) only once it is lowered.
        let c = self.expr(cond)?;
        let (peeled, region, pushed) = self.enter_region(RegionSite::While, body, false, Some(pre), span)?;
        let (body_b, latch, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(TermKind::Switch { value: c, cases: vec![(1, body_b)], otherwise: exit }, span, Some(cond.id));
        self.loop_body(peeled, body_b, latch, exit, header, region, pushed, None, span)
    }

    fn for_loop(&mut self, name: &str, iter: &HirExpr, body: &HirBlock, span: Span) -> Res<()> {
        let HirExprKind::Binary { op: BinOp::RangeTo, lhs, rhs } = &iter.kind else {
            return Err(BuildError::not_yet("for over something other than a range", span_of(iter)));
        };
        let lo = self.expr(lhs)?;
        let hi = self.expr(rhs)?;
        self.push_scope();
        let hi_tmp = self.temp(rhs.ty);
        self.stmt(StmtKind::Assign(Place::local(hi_tmp), Rvalue::Use(hi)), span, None);
        let var = self.new_local(lhs.ty, LocalKind::Var, Some(name.to_string()));
        self.stmt(StmtKind::Assign(Place::local(var), Rvalue::Use(lo)), span, None);
        self.declare(name, var);
        let (peeled, region, pushed) = self.enter_region(RegionSite::For, body, false, None, span)?;
        let header = self.new_block();
        self.goto(header, span);
        self.cur = header;
        let lt = self.temp(self.bool_ty);
        self.stmt(
            StmtKind::Assign(Place::local(lt), Rvalue::Binary(BinOp::Lt, Operand::Copy(Place::local(var)), Operand::Copy(Place::local(hi_tmp)))),
            span,
            None,
        );
        let (body_b, latch, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(lt)), cases: vec![(1, body_b)], otherwise: exit }, span, None);
        let r = self.loop_body(peeled, body_b, latch, exit, header, region, pushed, Some(var), span);
        self.pop_scope();
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn loop_body(
        &mut self,
        body: &HirBlock,
        body_b: BlockId,
        latch: BlockId,
        exit: BlockId,
        header: BlockId,
        region: RegionId,
        pushed: bool,
        counter: Option<Local>,
        span: Span,
    ) -> Res<()> {
        let arena = pushed.then(|| self.new_local(self.unit, LocalKind::Hidden(HiddenKind::RegionArena), None));
        self.push_loop(exit, latch);
        self.cur = body_b;
        self.stmts(body)?;
        self.goto(latch, span);
        self.pop_loop();
        self.cur = latch;
        if let Some(var) = counter {
            let one = Operand::Const(Const::Int(1));
            self.stmt(StmtKind::Assign(Place::local(var), Rvalue::Binary(BinOp::Add, Operand::Copy(Place::local(var)), one)), span, None);
        }
        if let Some(dst) = arena {
            self.stmt(StmtKind::RegionReset { region, dst }, span, None);
        }
        self.goto(header, span);
        self.cur = exit;
        self.exit_region(span)
    }
}
