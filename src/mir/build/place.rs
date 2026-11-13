//! Places, arrays, slices, `Vec` operations and bounds checks.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Res};
use crate::ast::BinOp;
use crate::hir::{HirExpr, HirExprKind, VecOp};
use crate::mir::body::*;
use crate::mir::capability::Capability;
use crate::span::Span;
use crate::ty::TyId;

impl<'a> FnBuilder<'a> {
    pub(super) fn operand_place(&mut self, op: Operand, ty: TyId, span: Span) -> Place {
        match op {
            Operand::Copy(p) | Operand::Move(p) | Operand::Reborrow(p) => p,
            Operand::Const(_) => {
                let temp = self.temp(ty);
                self.stmt(StmtKind::Assign(Place::local(temp), Rvalue::Use(op)), span, None);
                Place::local(temp)
            }
        }
    }

    pub(super) fn is_vec_ty(&self, ty: TyId) -> bool {
        self.tcx.is_vec(self.tcx.deref_ty(ty))
    }

    /// The storage `e` names (a binding, a field, an array element); anything else is evaluated into a temporary.
    pub(super) fn place_of(&mut self, e: &HirExpr) -> Res<Place> {
        let span = span_of(e);
        match &e.kind {
            HirExprKind::Ident { name, .. } => {
                let local = self.lookup(name, span)?;
                Ok(Place::local(local))
            }
            HirExprKind::ObjectField { receiver, field_index, safe: false, .. } => {
                let mut place = self.place_of(receiver)?;
                place.proj.push(Proj::Field(*field_index as u32));
                Ok(place)
            }
            HirExprKind::Index { receiver, index, .. } if !self.is_vec_ty(receiver.ty) => {
                let mut place = self.place_of(receiver)?;
                let idx = self.index_operand(index)?;
                self.bounds_guard(&place, &idx, span, e);
                place.proj.push(Proj::Index(Box::new(idx)));
                Ok(place)
            }
            _ => {
                let op = self.expr(e)?;
                Ok(self.operand_place(op, e.ty, span))
            }
        }
    }

    /// An index is a constant or a bare local (LLVM lowering reads nothing else); `a[b[1]]` copies `b[1]` into a temporary.
    fn index_operand(&mut self, index: &HirExpr) -> Res<Operand> {
        let idx = self.expr(index)?;
        match &idx {
            Operand::Copy(p) | Operand::Move(p) if !p.is_bare() => {
                let local = self.operand_local(idx.clone(), index.ty, span_of(index));
                Ok(Operand::Copy(Place::local(local)))
            }
            _ => Ok(idx),
        }
    }

    /// Aborts unless `0 <= idx < len(base)`.
    pub(super) fn bounds_guard(&mut self, base: &Place, idx: &Operand, span: Span, e: &HirExpr) {
        let i32_ty = self.tcx.prim_id(crate::hir::Prim::I32);
        let len = self.temp(i32_ty);
        self.stmt(StmtKind::Assign(Place::local(len), Rvalue::Len(base.clone())), span, Some(e.id));
        let abort = self.new_block();
        // Two checks, because MIR comparisons are signed: a negative index is below `len` too.
        for (op, bound) in [(BinOp::Ge, Operand::Const(Const::Int(0))), (BinOp::Lt, Operand::Copy(Place::local(len)))] {
            let inb = self.temp(self.bool_ty);
            self.stmt(StmtKind::Assign(Place::local(inb), Rvalue::Binary(op, idx.clone(), bound)), span, Some(e.id));
            let ok = self.new_block();
            self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(inb)), cases: vec![(1, ok)], otherwise: abort }, span, Some(e.id));
            self.cur = ok;
        }
        let ok = self.cur;
        self.cur = abort;
        self.terminate(TermKind::Abort(AbortReason::Bounds), span, Some(e.id));
        self.cur = ok;
    }

    pub(super) fn index(&mut self, e: &HirExpr, receiver: &HirExpr, index: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        if self.is_vec_ty(receiver.ty) {
            let recv = self.expr(receiver)?;
            let idx = self.index_operand(index)?;
            return Ok(self.vec_op(e, VecOp::Get, recv, vec![idx]));
        }
        let place = self.place_of(e)?;
        if self.tcx.is_managed_ref(e.ty) {
            let dst = self.temp(e.ty);
            self.stmt(StmtKind::Ref(RefOp::Clone { dst: Place::local(dst), src: Operand::Copy(place) }), span, Some(e.id));
            return Ok(Operand::Copy(Place::local(dst)));
        }
        Ok(Operand::Copy(place))
    }

    pub(super) fn slice(&mut self, e: &HirExpr, receiver: &HirExpr, lo: &HirExpr, hi: &HirExpr) -> Res<Operand> {
        let base = self.expr(receiver)?;
        let (lo, hi) = (self.expr(lo)?, self.expr(hi)?);
        Ok(self.assign_temp(e, Rvalue::Slice { base, lo, hi }))
    }

    pub(super) fn field(&mut self, e: &HirExpr, receiver: &HirExpr, name: &str, safe: bool) -> Res<Operand> {
        let span = span_of(e);
        if name != "length" {
            return Err(BuildError::unsupported(Capability::FieldAccess, span));
        }
        if safe {
            return self.safe_navigate(e, receiver, None, |b, base| {
                let len = b.temp(b.tcx.prim_id(crate::hir::Prim::I32));
                b.stmt(StmtKind::Assign(Place::local(len), Rvalue::Len(base)), span, Some(e.id));
                Ok(Operand::Copy(Place::local(len)))
            });
        }
        let place = self.place_of(receiver)?;
        Ok(self.assign_temp(e, Rvalue::Len(place)))
    }

    pub(super) fn vec_op(&mut self, e: &HirExpr, op: VecOp, receiver: Operand, args: Vec<Operand>) -> Operand {
        let dst = (!self.is_unit(e.ty)).then(|| self.temp(e.ty));
        // The `Option` slot of a `pop` that is not a `Ref` lives in the sink arena.
        let arena = (op == VecOp::Pop && !self.tcx.is_managed_ref(e.ty)).then(|| self.sink.clone());
        self.stmt(StmtKind::VecOp { dst: dst.map(Place::local), op, receiver, args, arena }, span_of(e), Some(e.id));
        dst.map_or(Operand::Const(Const::Unit), |d| Operand::Copy(Place::local(d)))
    }

    pub(super) fn vec_new(&mut self, e: &HirExpr, cap: Option<&HirExpr>) -> Res<Operand> {
        let cap = cap.map(|c| self.expr(c)).transpose()?;
        let dst = self.temp(e.ty);
        let arena = self.sink.clone();
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::Vec { cap }, arena: arena.clone() }, span_of(e), Some(e.id));
        // Only a `Vec` of `Ref`s has a release to run when its arena goes.
        if let Some(elem) = self.tcx.vec_elem(e.ty).filter(|&el| self.tcx.is_managed_ref(el)) {
            self.stmt(StmtKind::RegisterDrop { arena, place: Place::local(dst), glue: DropGlue::Vec(elem) }, span_of(e), Some(e.id));
        }
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn vec_op_expr(&mut self, e: &HirExpr, op: VecOp, receiver: &HirExpr, args: &[HirExpr]) -> Res<Operand> {
        let recv = self.expr(receiver)?;
        let mut ops = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let is_index = i == 0 && matches!(op, VecOp::Get | VecOp::Set);
            ops.push(if is_index { self.index_operand(a)? } else { self.expr(a)? });
        }
        Ok(self.vec_op(e, op, recv, ops))
    }

    /// Stores a `Ref` into an array element through the runtime protocol, labelled `[]` with its index for the debug
    /// registry. The runtime store does not register the slot for release: `array_lit` does that for its fresh slots.
    fn ref_element_store(&mut self, slot: Place, value: Operand, el: &HirExpr, arena: &ArenaRef, fresh: bool, index: Operand) {
        let label = DebugLabel { name: "[]".into(), ty: el.ty, span: span_of(el), source: None, declaration: false, argument: None, owner: Some(arena.clone()), index: Some(index) };
        self.stmt_with(StmtKind::Ref(RefOp::Store { slot, owner: arena.clone(), value, fresh }), span_of(el), Some(el.id), Some(label));
    }

    pub(super) fn array_lit(&mut self, e: &HirExpr, elements: &[HirExpr]) -> Res<Operand> {
        let span = span_of(e);
        let mut ops = Vec::with_capacity(elements.len());
        for el in elements {
            ops.push(self.expr(el)?);
        }
        let dst = self.temp(e.ty);
        let arena = self.sink.clone();
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::Array, arena: arena.clone() }, span, Some(e.id));
        for (i, (op, el)) in ops.into_iter().zip(elements).enumerate() {
            let index = Operand::Const(Const::Int(i as i64));
            let slot = Place { local: dst, proj: vec![Proj::Index(Box::new(index.clone()))] };
            if self.tcx.is_managed_ref(el.ty) {
                self.ref_element_store(slot.clone(), op, el, &arena, true, index);
                self.stmt(StmtKind::RegisterDrop { arena: arena.clone(), place: slot, glue: DropGlue::RefSlot }, span_of(el), None);
                continue;
            }
            let v = self.materialize(op, el.ty, &arena, span_of(el))?;
            self.stmt(StmtKind::Assign(slot, Rvalue::Use(v)), span_of(el), Some(el.id));
        }
        Ok(Operand::Copy(Place::local(dst)))
    }

    /// Stores into `place[idx]` / `vec[idx]`.
    pub(super) fn assign_index(&mut self, name: &str, index: &HirExpr, value: &HirExpr, span: Span) -> Res<()> {
        let local = self.lookup(name, span)?;
        let v = self.with_sink(ArenaRef::Home(local), |b| b.expr(value))?;
        let idx = self.index_operand(index)?;
        let ty = self.locals[local.0 as usize].ty;
        if self.is_vec_ty(ty) {
            self.stmt(
                StmtKind::VecOp { dst: None, op: VecOp::Set, receiver: Operand::Reborrow(Place::local(local)), args: vec![idx, v], arena: None },
                span,
                Some(value.id),
            );
            return Ok(());
        }
        let base = Place::local(local);
        self.bounds_guard(&base, &idx, span, value);
        let slot = Place { local, proj: vec![Proj::Index(Box::new(idx.clone()))] };
        if self.tcx.is_managed_ref(value.ty) {
            self.ref_element_store(slot, v, value, &ArenaRef::Home(local), false, idx);
            return Ok(());
        }
        self.stmt(StmtKind::Assign(slot, Rvalue::Use(v)), span, Some(value.id));
        Ok(())
    }

    /// Copies arena-resident values (`String`, arrays, `Vec`, nullables) into `arena`; handles and scalars pass through.
    pub(super) fn materialize(&mut self, op: Operand, ty: TyId, arena: &ArenaRef, span: Span) -> Res<Operand> {
        let t = self.tcx;
        if t.is_managed_ref(ty) {
            let dst = self.temp(ty);
            self.stmt(StmtKind::Ref(RefOp::Rehome { dst: Place::local(dst), owner: arena.clone(), value: op }), span, None);
            return Ok(Operand::Move(Place::local(dst)));
        }
        if !(t.is_nullable(ty) || t.is_string(ty) || t.is_array(ty) || t.is_vec(ty)) {
            return Ok(op);
        }
        let src = self.operand_place(op, ty, span);
        let dst = self.temp(ty);
        self.stmt(StmtKind::CopyInto { dst: Place::local(dst), src, arena: arena.clone() }, span, None);
        Ok(Operand::Copy(Place::local(dst)))
    }
}
