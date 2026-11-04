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
                let local = self.lookup(name).ok_or_else(|| BuildError::not_yet(format!("name `{name}`"), span))?;
                Ok(Place::local(local))
            }
            HirExprKind::ObjectField { receiver, field_index, safe: false, .. } => {
                let mut place = self.place_of(receiver)?;
                place.proj.push(Proj::Field(*field_index as u32));
                Ok(place)
            }
            HirExprKind::Index { receiver, index, .. } if !self.is_vec_ty(receiver.ty) => {
                let mut place = self.place_of(receiver)?;
                let idx = self.expr(index)?;
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

    /// Aborts unless `0 <= idx < len(base)`.
    pub(super) fn bounds_guard(&mut self, base: &Place, idx: &Operand, span: Span, e: &HirExpr) {
        let i32_ty = self.tcx.prim_id(crate::hir::Prim::I32);
        let len = self.temp(i32_ty);
        self.stmt(StmtKind::Assign(Place::local(len), Rvalue::Len(base.clone())), span, Some(e.id));
        let inb = self.temp(self.bool_ty);
        // shortcut: one signed `idx < len`; the backend lowers it as the unsigned compare that also rejects negatives.
        self.stmt(
            StmtKind::Assign(Place::local(inb), Rvalue::Binary(BinOp::Lt, idx.clone(), Operand::Copy(Place::local(len)))),
            span,
            Some(e.id),
        );
        let (ok, abort) = (self.new_block(), self.new_block());
        self.terminate(
            TermKind::Switch { value: Operand::Copy(Place::local(inb)), cases: vec![(1, ok)], otherwise: abort },
            span,
            Some(e.id),
        );
        self.cur = abort;
        self.terminate(TermKind::Abort(AbortReason::Bounds), span, Some(e.id));
        self.cur = ok;
    }

    pub(super) fn index(&mut self, e: &HirExpr, receiver: &HirExpr, index: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        if self.is_vec_ty(receiver.ty) {
            let recv = self.expr(receiver)?;
            let idx = self.expr(index)?;
            return Ok(self.vec_op(e, VecOp::Get, recv, vec![idx]));
        }
        let place = self.place_of(e)?;
        let _ = span;
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
            return self.safe_navigate(e, receiver, |b, base| {
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
        self.stmt(StmtKind::VecOp { dst: dst.map(Place::local), op, receiver, args }, span_of(e), Some(e.id));
        dst.map_or(Operand::Const(Const::Unit), |d| Operand::Copy(Place::local(d)))
    }

    pub(super) fn vec_new(&mut self, e: &HirExpr, cap: Option<&HirExpr>) -> Res<Operand> {
        let cap = cap.map(|c| self.expr(c)).transpose()?;
        let dst = self.temp(e.ty);
        let arena = self.sink.clone();
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::Vec { cap }, arena }, span_of(e), Some(e.id));
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn vec_op_expr(&mut self, e: &HirExpr, op: VecOp, receiver: &HirExpr, args: &[HirExpr]) -> Res<Operand> {
        let recv = self.expr(receiver)?;
        let mut ops = Vec::with_capacity(args.len());
        for a in args {
            ops.push(self.expr(a)?);
        }
        Ok(self.vec_op(e, op, recv, ops))
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
            let v = self.materialize(op, el.ty, &arena, span_of(el))?;
            let slot = Place { local: dst, proj: vec![Proj::Index(Box::new(Operand::Const(Const::Int(i as i64))))] };
            self.stmt(StmtKind::Assign(slot, Rvalue::Use(v)), span_of(el), Some(el.id));
        }
        Ok(Operand::Copy(Place::local(dst)))
    }

    /// Stores into `place[idx]` / `vec[idx]`.
    pub(super) fn assign_index(&mut self, name: &str, index: &HirExpr, value: &HirExpr, span: Span) -> Res<()> {
        let local = self.lookup(name).ok_or_else(|| BuildError::not_yet(format!("name `{name}`"), span))?;
        let v = self.with_sink(ArenaRef::Home(local), |b| b.expr(value))?;
        let idx = self.expr(index)?;
        let ty = self.locals[local.0 as usize].ty;
        if self.is_vec_ty(ty) {
            self.stmt(
                StmtKind::VecOp { dst: None, op: VecOp::Set, receiver: Operand::Reborrow(Place::local(local)), args: vec![idx, v] },
                span,
                Some(value.id),
            );
            return Ok(());
        }
        let base = Place::local(local);
        self.bounds_guard(&base, &idx, span, value);
        let slot = Place { local, proj: vec![Proj::Index(Box::new(idx))] };
        self.stmt(StmtKind::Assign(slot, Rvalue::Use(v)), span, Some(value.id));
        Ok(())
    }

    /// Copies arena-resident values (`String`, arrays, `Vec`, nullables) into `arena`; handles and scalars pass through.
    pub(super) fn materialize(&mut self, op: Operand, ty: TyId, arena: &ArenaRef, span: Span) -> Res<Operand> {
        let t = self.tcx;
        if t.is_managed_ref(ty) {
            return Err(BuildError::not_yet("materialising a Ref", span));
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
