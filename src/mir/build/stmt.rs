//! Statement lowering, loops and early exits.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Mark, Res};
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
                // `return move a` hands an object to the caller's result arena.
                let to_result = |b: &Self, v: &HirExpr| b.abi.result_sink && matches!(v.kind, HirExprKind::Ident { .. }) && b.moves_handle(v.ty);
                let v = match value.as_ref() {
                    Some(v) if to_result(self, v) => Some(self.with_sink(ArenaRef::ResultSink, |b| b.expr(v))?),
                    Some(v) => Some(self.expr(v)?),
                    None => None,
                };
                if let Some(mut v) = v.filter(|_| !self.is_unit(self.function.return_ty)) {
                    if self.abi.result_sink {
                        v = self.materialize(v, self.function.return_ty, &ArenaRef::ResultSink, span)?;
                    }
                    self.stmt(StmtKind::Assign(Place::local(self.ret_local()), Rvalue::Use(v)), span, None);
                }
                self.exit_to(Mark::default(), span);
                self.terminate(TermKind::Return, span, None);
                Ok(())
            }
            HirStmt::Block(b) => self.region_block(RegionSite::Block, b, span),
            HirStmt::MoveBlock { body, .. } => self.region_block(RegionSite::MoveBlock, body, span),
            HirStmt::VarDecl { kind, name, ty, value, alloc_in_binding } => {
                let (sink, v) = match alloc_in_binding.as_deref() {
                    None => (self.sink.clone(), self.expr(value)?),
                    Some(crate::hoist::RESULT_SINK) => (ArenaRef::ResultSink, self.with_sink(ArenaRef::ResultSink, |b| b.expr(value))?),
                    Some(target) => {
                        let home = self.lookup(target, span)?;
                        (ArenaRef::Home(home), self.with_sink(ArenaRef::Home(home), |b| b.expr(value))?)
                    }
                };
                let key = self.sink_key(&sink);
                let local_kind = if matches!(kind, BindingKind::Val) { LocalKind::Val } else { LocalKind::Var };
                let local = self.new_local(*ty, local_kind, Some(name.clone()));
                self.decl_sinks.insert(local, key);
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
                self.register_ref_local(local, *ty, span);
                self.declare(name, local);
                Ok(())
            }
            HirStmt::Assign { target, value } => match target {
                HirAssignTarget::Name { name } => {
                    let local = self.lookup(name, span)?;
                    let v = self.with_sink(ArenaRef::Home(local), |b| b.expr(value))?;
                    if self.tcx.is_managed_ref(self.locals[local.0 as usize].ty) {
                        let store = RefOp::Store { slot: Place::local(local), owner: ArenaRef::Home(local), value: v, fresh: false };
                        self.stmt(StmtKind::Ref(store), span, Some(value.id));
                        return Ok(());
                    }
                    self.stmt(StmtKind::Assign(Place::local(local), Rvalue::Use(v)), span, Some(value.id));
                    Ok(())
                }
                HirAssignTarget::Index { name, index } => self.assign_index(name, index, value, span),
                HirAssignTarget::Field { receiver, field_index, .. } => self.assign_field(receiver, *field_index, value, span),
            },
            HirStmt::Expr(e) => {
                // A discarded owned `Ref` is released.
                if let Operand::Move(p) = self.expr(e)? {
                    if p.is_bare() && self.tcx.is_managed_ref(e.ty) {
                        self.stmt(StmtKind::Ref(RefOp::Drop(Operand::Move(p))), span, Some(e.id));
                    }
                }
                Ok(())
            }
            HirStmt::While { cond, body } => self.while_loop(cond, body, span),
            HirStmt::For { name, iter, body } => self.for_loop(name, iter, body, span),
            HirStmt::Break { span } => self.jump(true, *span),
            HirStmt::Continue { span } => self.jump(false, *span),
        }
    }

    fn assign_field(&mut self, receiver: &HirExpr, field_index: usize, value: &HirExpr, span: Span) -> Res<()> {
        let recv = self.expr(receiver)?;
        let object = self.operand_local(recv, receiver.ty, span);
        let arena = ArenaRef::ObjectArena(object);
        let v = self.with_sink(arena.clone(), |b| b.expr(value))?;
        let class = crate::mono::class_instance_name(self.tcx, receiver.ty)
            .and_then(|n| self.mono.hir.classes.iter().find(|c| c.name == n))
            .ok_or_else(|| BuildError::Internal("field assignment on a non-class".into()))?;
        let ty = class.fields[field_index].ty;
        let mut slot = Place::local(object);
        if self.tcx.is_managed_ref(self.tcx.deref_ty(receiver.ty)) {
            slot.proj.push(Proj::Deref);
        }
        slot.proj.push(Proj::Field(field_index as u32));
        // A plain field being overwritten is labelled, a `Ref` slot is not.
        let named = (!self.tcx.is_managed_ref(ty)).then(|| (format!("{}.{}", class.name, class.fields[field_index].name), &class.fields[field_index]));
        self.store_owned(slot, v, ty, &arena, false, span, named)
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
        let (exit, latch, mark) = self.loop_ctx().ok_or_else(|| BuildError::Internal("break outside a loop".into()))?;
        self.exit_to(mark, span);
        self.terminate(TermKind::Goto(if is_break { exit } else { latch }), span, None);
        Ok(())
    }

    fn while_loop(&mut self, cond: &HirExpr, body: &HirBlock, span: Span) -> Res<()> {
        // After a `return`/`break` the loop is dead code: its enter must not join the exit path's block.
        let pre = (!self.terminated()).then_some(self.cur);
        let header = self.new_block();
        self.goto(header, span);
        self.cur = header;
        // The condition runs inside the loop region at run time, but its own report regions come
        // first in the walk, so the loop region is opened (in `pre`) only once it is lowered.
        let c = self.expr(cond)?;
        let (peeled, region, pushed) = self.enter_region(RegionSite::While, body, false, pre, span)?;
        let (body_b, latch, exit) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(TermKind::Switch { value: c, cases: vec![(1, body_b)], otherwise: exit }, span, Some(cond.id));
        self.loop_body(peeled, body_b, latch, exit, header, region, pushed, None, span)
    }

    fn for_loop(&mut self, name: &str, iter: &HirExpr, body: &HirBlock, span: Span) -> Res<()> {
        let HirExprKind::Binary { op: BinOp::RangeTo, lhs, rhs } = &iter.kind else {
            return Err(BuildError::unlowered("`for` over something other than a range", span_of(iter)));
        };
        let lo = self.expr(lhs)?;
        let hi = self.expr(rhs)?;
        self.push_scope();
        let hi_tmp = self.temp(rhs.ty);
        self.stmt(StmtKind::Assign(Place::local(hi_tmp), Rvalue::Use(hi)), span, None);
        let var = self.new_local(lhs.ty, LocalKind::Var, Some(name.to_string()));
        // The counter is declared like any other local.
        let label = DebugLabel { name: name.to_string(), ty: lhs.ty, span, source: self.function.source, declaration: true, argument: None, owner: None, index: None };
        self.stmt_with(StmtKind::Assign(Place::local(var), Rvalue::Use(lo)), span, None, Some(label));
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
        // The per-iteration scope of a `for`/`while` body.
        self.scope_enter(span, None);
        self.stmts(body)?;
        self.scope_exit(span);
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
