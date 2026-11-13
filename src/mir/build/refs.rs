//! Ownership: moves and promotes, the `Ref` protocol, `if val` and drop registration.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Res};
use crate::hir::{HirBlock, HirConditionalBindings, HirExpr, UseKind};
use crate::mir::body::*;
use crate::mir::capability::Capability;
use crate::region_walk::RegionSite;
use crate::span::Span;
use crate::ty::TyId;

impl<'a> FnBuilder<'a> {
    /// Whether a value of `ty` is allocated in its consumer's arena (so branches must allocate in the outer one).
    pub(super) fn owning(&self, ty: TyId) -> bool {
        self.tcx.owns_result(ty) || self.tcx.uses_arena_storage(ty)
    }

    /// The arena a value must survive in when it is produced inside a region that is about to close.
    pub(super) fn outer_sink(&self) -> ArenaRef {
        if self.sink != ArenaRef::Current {
            return self.sink.clone();
        }
        self.innermost_pushed().map_or(ArenaRef::Current, ArenaRef::Region)
    }

    /// Local holding the value of `op`, for ops that name an arena through a local.
    pub(super) fn operand_local(&mut self, op: Operand, ty: TyId, span: Span) -> Local {
        let place = self.operand_place(op, ty, span);
        if place.is_bare() {
            return place.local;
        }
        let temp = self.temp(ty);
        self.stmt(StmtKind::Assign(Place::local(temp), Rvalue::Use(Operand::Copy(place))), span, None);
        temp
    }

    /// `move x` / `promote x` of values the gate cannot yet move (RFC 3.10 `move-aggregate`).
    fn move_unsupported(&self, ty: TyId) -> Option<&'static str> {
        let t = self.tcx;
        let payload = if t.is_nullable(ty) { t.enum_args(ty).and_then(|(_, a)| a.first().copied()).unwrap_or(ty) } else { ty };
        if t.enum_args(payload).is_some() {
            Some("moving enum values is not supported by codegen yet")
        } else if t.record_def(ty).is_some() && !t.is_managed_ref(ty) {
            Some("moving class objects is not supported by codegen yet")
        } else if t.array_elem(ty).is_some_and(|el| t.is_managed_ref(el)) {
            Some("moving an array of managed Ref is not supported by codegen yet")
        } else {
            None
        }
    }

    pub(super) fn ident_moved(&mut self, e: &HirExpr, place: Place, use_kind: UseKind) -> Res<Operand> {
        let span = span_of(e);
        match use_kind {
            UseKind::RefMove => {
                let dst = self.temp(e.ty);
                self.stmt(StmtKind::Ref(RefOp::MoveOut { dst: Place::local(dst), src: place }), span, Some(e.id));
                Ok(Operand::Copy(Place::local(dst)))
            }
            UseKind::RefCopy => {
                let dst = self.temp(e.ty);
                self.stmt(StmtKind::Ref(RefOp::Clone { dst: Place::local(dst), src: Operand::Copy(place) }), span, Some(e.id));
                Ok(Operand::Copy(Place::local(dst)))
            }
            _ => {
                if let Some(message) = self.move_unsupported(e.ty) {
                    return Err(BuildError::unsupported_with(Capability::MoveAggregate, span, message));
                }
                let t = self.tcx;
                if !(t.is_nullable(e.ty) || t.is_string(e.ty) || t.is_array(e.ty) || t.is_vec(e.ty)) {
                    return Ok(Operand::Move(place));
                }
                // These are copied into the sink arena; the source is marked moved by the `Move`.
                let moved = self.temp(e.ty);
                self.stmt(StmtKind::Move { dst: Place::local(moved), src: place }, span, Some(e.id));
                let arena = self.sink.clone();
                self.materialize(Operand::Move(Place::local(moved)), e.ty, &arena, span)
            }
        }
    }

    /// `RefCreate(object)`: a handle from the sink arena to the object's own arena.
    pub(super) fn ref_create(&mut self, e: &HirExpr, inner: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        let obj = self.expr(inner)?;
        let object = self.operand_local(obj, inner.ty, span);
        let dst = self.temp(e.ty);
        let source = self.sink.clone();
        self.stmt(
            StmtKind::Ref(RefOp::Create {
                dst: Place::local(dst),
                source,
                target: ArenaRef::ObjectArena(object),
                object: Operand::Copy(Place::local(object)),
            }),
            span,
            Some(e.id),
        );
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn ref_count(&mut self, e: &HirExpr, weak: bool, args: &[Operand]) -> Operand {
        let dst = self.temp(e.ty);
        let value = args.first().cloned().unwrap_or(Operand::Const(Const::Unit));
        self.stmt(StmtKind::Ref(RefOp::Count { dst: Place::local(dst), value, weak }), span_of(e), Some(e.id));
        Operand::Copy(Place::local(dst))
    }

    /// Registers the release of a `Ref` local with its home arena.
    pub(super) fn register_ref_local(&mut self, local: Local, ty: TyId, span: Span) {
        if self.tcx.is_managed_ref(ty) {
            self.stmt(StmtKind::RegisterDrop { arena: ArenaRef::Current, place: Place::local(local), glue: DropGlue::RefSlot }, span, None);
        }
    }

    /// `if val a = x, b = y { .. } else { .. }`: each source is observed; any absence takes the `else` path.
    pub(super) fn presence(
        &mut self,
        e: &HirExpr,
        bindings: &HirConditionalBindings,
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
    ) -> Res<Operand> {
        let span = span_of(e);
        let result = (!self.is_unit(e.ty)).then(|| self.temp(e.ty));
        let outer = self.mark();
        let sink = self.owning(e.ty).then(|| self.outer_sink());
        let (merge, none_b) = (self.new_block(), self.new_block());
        let some_b = self.new_block();
        let head = bindings.head.as_ref();
        let (obj, absent) = self.observe(&head.value, head.binding_ty, span, e)?;
        self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(absent)), cases: vec![(1, none_b)], otherwise: some_b }, span, Some(e.id));
        self.cur = some_b;
        let (peeled, _, _) = self.enter_region(RegionSite::PresenceSome, some_block, false, None, span)?;
        self.stmt(StmtKind::ObservationRegister { arena: ArenaRef::Current, obs: obj.0 }, span, None);
        self.declare(&head.name.name, obj.1);
        let cleanup = (!bindings.tail.is_empty()).then(|| self.new_block());
        for tail in &bindings.tail {
            let (o, a) = self.observe(&tail.value, tail.binding_ty, span, e)?;
            let next = self.new_block();
            let on_absent = cleanup.expect("created for a non-empty tail");
            self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(a)), cases: vec![(1, on_absent)], otherwise: next }, span, Some(e.id));
            self.cur = next;
            self.stmt(StmtKind::ObservationRegister { arena: ArenaRef::Current, obs: o.0 }, span, None);
            self.declare(&tail.name.name, o.1);
        }
        if let Some(cleanup) = cleanup {
            // A failed tail guard leaves the `Some` region and takes the absent path.
            let body_start = self.cur;
            self.cur = cleanup;
            self.exit_to(outer, span);
            self.goto(none_b, span);
            self.cur = body_start;
        }
        match &sink {
            Some(s) => self.with_sink(s.clone(), |b| b.block_value_in(peeled, result, Some(s)))?,
            None => self.block_value(peeled, result)?,
        }
        self.exit_region(span)?;
        self.goto(merge, span);
        self.cur = none_b;
        if let Some(block) = none_block {
            match &sink {
                Some(s) => self.with_sink(s.clone(), |b| b.presence_none(block, result, span, Some(s)))?,
                None => self.presence_none(block, result, span, None)?,
            }
        }
        self.goto(merge, span);
        self.cur = merge;
        Ok(result.map_or(Operand::Const(Const::Unit), |r| Operand::Copy(Place::local(r))))
    }

    fn presence_none(&mut self, block: &HirBlock, result: Option<Local>, span: Span, rehome: Option<&ArenaRef>) -> Res<()> {
        let (peeled, _, _) = self.enter_region(RegionSite::PresenceNone, block, false, None, span)?;
        self.block_value_in(peeled, result, rehome)?;
        self.exit_region(span)
    }

    /// Observes a `Ref` source: `(observation, object)` locals and the `absent` flag; the temporary
    /// `Ref` is released at once, as the observation pins the target.
    fn observe(&mut self, source: &HirExpr, binding_ty: TyId, span: Span, e: &HirExpr) -> Res<((Local, Local), Local)> {
        let value = self.expr(source)?;
        let obs = self.temp(self.unit);
        let object = self.new_local(binding_ty, LocalKind::Val, None);
        let absent = self.temp(self.bool_ty);
        self.stmt(StmtKind::Observe { obs, object, absent, value: as_copy(&value) }, span, Some(e.id));
        self.stmt(StmtKind::Ref(RefOp::Drop(value)), span, Some(e.id));
        Ok(((obs, object), absent))
    }
}

/// The same value, read without consuming it.
pub(super) fn as_copy(op: &Operand) -> Operand {
    match op {
        Operand::Move(p) => Operand::Copy(p.clone()),
        other => other.clone(),
    }
}
