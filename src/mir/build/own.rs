//! Objects, enums, nullables and `when`: allocation sites, arena choice and `Match`.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Res};
use crate::ast::BinOp;
use crate::hir::{HirArm, HirExpr, HirExprKind, HirPat, HirPatKind};
use crate::memory::{ArenaClass, LifetimeDomain};
use crate::mir::body::*;
use crate::mir::capability::Capability;
use crate::region_walk::{pat_binds_owned_ref, RegionSite};
use crate::span::Span;
use crate::ty::TyId;

struct Bound {
    name: String,
    local: Local,
    place: Place,
    ty: TyId,
}

impl<'a> FnBuilder<'a> {
    fn enum_name(&self, ty: TyId, span: Span) -> Res<String> {
        crate::mono::enum_instance_name(self.tcx, ty).ok_or_else(|| BuildError::Internal(format!("no enum instance for type at {span:?}")))
    }

    fn variant_index(&self, enum_name: &str, variant: &str) -> Res<u32> {
        let e = self.mono.hir.enums.iter().find(|e| e.name == enum_name);
        e.and_then(|e| e.variants.iter().position(|v| v.name == variant))
            .map(|i| i as u32)
            .ok_or_else(|| BuildError::Internal(format!("enum `{enum_name}` has no variant `{variant}`")))
    }

    /// Payload type `T` of a nullable `T?`.
    fn nullable_payload(&self, ty: TyId) -> TyId {
        self.tcx.enum_args(ty).and_then(|(_, a)| a.first().copied()).expect("nullable has a payload")
    }

    pub(super) fn make_none(&mut self, ty: TyId, span: Span) -> Res<Operand> {
        if self.tcx.is_managed_ref(ty) {
            return Ok(Operand::Const(Const::None));
        }
        let name = self.enum_name(ty, span)?;
        let variant = self.variant_index(&name, "None")?;
        let dst = self.temp(ty);
        let arena = self.sink.clone();
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::EnumSlot { enum_name: name, variant }, arena }, span, None);
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn make_some(&mut self, ty: TyId, inner: Operand, inner_ty: TyId, span: Span) -> Res<Operand> {
        if self.tcx.is_managed_ref(ty) {
            return Ok(inner);
        }
        let name = self.enum_name(ty, span)?;
        let variant = self.variant_index(&name, "Some")?;
        let dst = self.temp(ty);
        let arena = self.sink.clone();
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::EnumSlot { enum_name: name, variant }, arena: arena.clone() }, span, None);
        let payload = self.materialize(inner, inner_ty, &arena, span)?;
        let slot = Place { local: dst, proj: vec![Proj::Some] };
        self.stmt(StmtKind::Assign(slot, Rvalue::Use(payload)), span, None);
        Ok(Operand::Copy(Place::local(dst)))
    }

    /// The arena a `Class(..)` / `Enum.V(..)` slot goes into, creating a dynamic one first when the plan says so.
    fn construct_arena(&mut self, e: &HirExpr) -> ArenaRef {
        let span = span_of(e);
        if self.neutral {
            return self.sink.clone();
        }
        let Some(plan) = self.mono.memory_plan.allocation_at(e.id) else { return self.sink.clone() };
        let sink_or_result = if self.abi.result_sink { ArenaRef::ResultSink } else { self.sink.clone() };
        match plan.class {
            ArenaClass::Lexical => {
                if self.abi.result_sink && e.ty == self.function.return_ty {
                    ArenaRef::ResultSink
                } else {
                    self.sink.clone()
                }
            }
            ArenaClass::Dynamic => {
                let (owner, parent, scoped) = match plan.lifetime_domain {
                    LifetimeDomain::Program => (sink_or_result, ArenaRef::Root, false),
                    LifetimeDomain::LexicalScope(_) => (ArenaRef::Current, ArenaRef::Current, true),
                };
                let dst = self.new_local(self.unit, LocalKind::Hidden(HiddenKind::DynamicArena), None);
                self.stmt(StmtKind::ArenaCreateDynamic { dst, parent, scoped }, span, Some(e.id));
                self.stmt(StmtKind::RegisterStrongRoot { owner, arena: dst }, span, Some(e.id));
                ArenaRef::Fresh(dst.0)
            }
        }
    }

    pub(super) fn object_construct(&mut self, e: &HirExpr, class_name: &str, fields: &[HirExpr]) -> Res<Operand> {
        let span = span_of(e);
        let arena = self.construct_arena(e);
        let ops = self.with_sink(arena.clone(), |b| b.exprs(fields))?;
        let class = self.mono.hir.classes.iter().find(|c| c.name == class_name).ok_or_else(|| BuildError::Internal(format!("no class `{class_name}`")))?;
        let dst = self.temp(e.ty);
        self.stmt(StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::Object { class: class_name.to_string() }, arena: arena.clone() }, span, Some(e.id));
        for (i, (op, field)) in ops.into_iter().zip(&class.fields).enumerate() {
            let slot = Place { local: dst, proj: vec![Proj::Field(i as u32)] };
            self.store_owned(slot, op, field.ty, &arena, true, span)?;
        }
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn enum_construct(&mut self, e: &HirExpr, enum_name: &str, variant: &str, fields: &[HirExpr]) -> Res<Operand> {
        let span = span_of(e);
        let arena = self.construct_arena(e);
        let ops = self.with_sink(arena.clone(), |b| b.exprs(fields))?;
        let index = self.variant_index(enum_name, variant)?;
        let decl = self.mono.hir.enums.iter().find(|x| x.name == enum_name).expect("variant_index found it");
        let dst = self.temp(e.ty);
        self.stmt(
            StmtKind::Alloc { dst: Place::local(dst), kind: AllocKind::EnumSlot { enum_name: enum_name.to_string(), variant: index }, arena: arena.clone() },
            span,
            Some(e.id),
        );
        for (i, (op, field)) in ops.into_iter().zip(&decl.variants[index as usize].fields).enumerate() {
            if field.ty == self.unit {
                continue;
            }
            let slot = Place { local: dst, proj: vec![Proj::Variant(index), Proj::Field(i as u32)] };
            self.store_owned(slot, op, field.ty, &arena, false, span)?;
        }
        if self.enum_has_ref(enum_name) {
            self.stmt(StmtKind::RegisterDrop { arena, place: Place::local(dst), glue: DropGlue::Enum(enum_name.to_string()) }, span, Some(e.id));
        }
        Ok(Operand::Copy(Place::local(dst)))
    }

    pub(super) fn enum_has_ref(&self, enum_name: &str) -> bool {
        self.mono
            .hir
            .enums
            .iter()
            .find(|e| e.name == enum_name)
            .is_some_and(|e| e.variants.iter().any(|v| v.fields.iter().any(|f| self.tcx.is_managed_ref(f.ty))))
    }

    pub(super) fn exprs(&mut self, es: &[HirExpr]) -> Res<Vec<Operand>> {
        es.iter().map(|x| self.expr(x)).collect()
    }

    /// Stores `op` into a field slot of `arena`: `Ref`s are stored through the runtime protocol (and, for
    /// object fields, registered for release), other owned values are copied in.
    pub(super) fn store_owned(&mut self, slot: Place, op: Operand, ty: TyId, arena: &ArenaRef, register: bool, span: Span) -> Res<()> {
        if self.tcx.is_managed_ref(ty) {
            // Object fields (registered) and enum payloads are fresh memory; assignments overwrite a live handle.
            let fresh = register || slot.proj.iter().any(|p| matches!(p, Proj::Variant(_)));
            self.stmt(StmtKind::Ref(RefOp::Store { slot: slot.clone(), owner: arena.clone(), value: op, fresh }), span, None);
            if register {
                self.stmt(StmtKind::RegisterDrop { arena: arena.clone(), place: slot, glue: DropGlue::RefSlot }, span, None);
            }
            return Ok(());
        }
        let v = self.materialize(op, ty, arena, span)?;
        self.stmt(StmtKind::Assign(slot, Rvalue::Use(v)), span, None);
        Ok(())
    }

    pub(super) fn object_field(&mut self, e: &HirExpr, receiver: &HirExpr, field_index: usize, safe: bool) -> Res<Operand> {
        let span = span_of(e);
        if safe {
            if self.tcx.is_nullable(e.ty) && self.is_enum_ty(self.nullable_payload(e.ty)) {
                return Err(BuildError::unsupported(Capability::SafeFieldEnum, span));
            }
            if self.tcx.is_managed_ref(receiver.ty) {
                return self.safe_ref_field(e, receiver, field_index);
            }
            return self.safe_navigate(e, receiver, |_, mut base| {
                base.proj.push(Proj::Field(field_index as u32));
                Ok(Operand::Copy(base))
            });
        }
        let mut place = self.place_of(receiver)?;
        if self.tcx.is_managed_ref(self.tcx.deref_ty(receiver.ty)) {
            place.proj.push(Proj::Deref);
        }
        place.proj.push(Proj::Field(field_index as u32));
        if self.tcx.is_managed_ref(e.ty) {
            let dst = self.temp(e.ty);
            self.stmt(StmtKind::Ref(RefOp::Clone { dst: Place::local(dst), src: Operand::Copy(place) }), span, Some(e.id));
            return Ok(Operand::Copy(Place::local(dst)));
        }
        Ok(Operand::Copy(place))
    }

    /// `r?.f` on a `Ref`: `None` when the target is gone, otherwise `Some(f)` read through an observation.
    fn safe_ref_field(&mut self, e: &HirExpr, receiver: &HirExpr, field_index: usize) -> Res<Operand> {
        let span = span_of(e);
        let target = self.tcx.managed_ref_inner(receiver.ty).ok_or_else(|| BuildError::Internal("`?.` Ref without a target".into()))?;
        let r = self.expr(receiver)?;
        let obs = self.temp(self.unit);
        let object = self.new_local(target, LocalKind::Val, None);
        let absent = self.temp(self.bool_ty);
        self.stmt(StmtKind::Observe { obs, object, absent, value: super::refs::as_copy(&r) }, span, Some(e.id));
        self.stmt(StmtKind::Ref(RefOp::Drop(r)), span, Some(e.id));
        let result = self.temp(e.ty);
        let (some_b, none_b, end) = (self.new_block(), self.new_block(), self.new_block());
        self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(absent)), cases: vec![(1, none_b)], otherwise: some_b }, span, Some(e.id));
        self.cur = some_b;
        let field_ty = self.nullable_payload(e.ty);
        let field = Place { local: object, proj: vec![Proj::Deref, Proj::Field(field_index as u32)] };
        let value = if self.tcx.is_managed_ref(field_ty) {
            let dst = self.temp(field_ty);
            self.stmt(StmtKind::Ref(RefOp::Clone { dst: Place::local(dst), src: Operand::Copy(field) }), span, Some(e.id));
            Operand::Move(Place::local(dst))
        } else {
            Operand::Copy(field)
        };
        let wrapped = self.make_some(e.ty, value, field_ty, span)?;
        self.stmt(StmtKind::ObservationRelease(obs), span, Some(e.id));
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(wrapped)), span, Some(e.id));
        self.goto(end, span);
        self.cur = none_b;
        let none = self.make_none(e.ty, span)?;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(none)), span, Some(e.id));
        self.goto(end, span);
        self.cur = end;
        Ok(Operand::Copy(Place::local(result)))
    }

    fn is_enum_ty(&self, ty: TyId) -> bool {
        self.tcx.enum_args(ty).is_some()
    }

    /// `recv?.f`: `None` when `recv` is `None`, otherwise `Some(project(recv.some))`.
    pub(super) fn safe_navigate(&mut self, e: &HirExpr, receiver: &HirExpr, project: impl FnOnce(&mut Self, Place) -> Res<Operand>) -> Res<Operand> {
        let span = span_of(e);
        let place = self.place_of(receiver)?;
        let payload_ty = self.nullable_payload(receiver.ty);
        let inner = self.temp(payload_ty);
        let (some_b, none_b, end) = (self.new_block(), self.new_block(), self.new_block());
        let result = self.temp(e.ty);
        self.nullable_match(&place, inner, some_b, none_b, span, e);
        self.cur = some_b;
        let value = project(self, Place::local(inner))?;
        let inner_ty = self.nullable_payload(e.ty);
        let wrapped = self.make_some(e.ty, value, inner_ty, span)?;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(wrapped)), span, Some(e.id));
        self.goto(end, span);
        self.cur = none_b;
        let none = self.make_none(e.ty, span)?;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(none)), span, Some(e.id));
        self.goto(end, span);
        self.cur = end;
        Ok(Operand::Copy(Place::local(result)))
    }

    /// `Match` of a nullable into its `Some` (binding `inner` to the payload) and `None` arms.
    fn nullable_match(&mut self, place: &Place, inner: Local, some_b: BlockId, none_b: BlockId, span: Span, e: &HirExpr) {
        let mut payload = place.clone();
        payload.proj.push(Proj::Some);
        let arms = vec![
            MirArm { pat: MirPat::Some(Box::new(MirPat::Bind)), bindings: vec![(inner, payload)], target: some_b },
            MirArm { pat: MirPat::None, bindings: Vec::new(), target: none_b },
        ];
        self.terminate(TermKind::Match { scrutinee: place.clone(), arms }, span, Some(e.id));
    }

    /// `x!!`
    pub(super) fn not_null_assert(&mut self, e: &HirExpr, inner: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        let place = self.place_of(inner)?;
        if self.tcx.is_managed_ref(inner.ty) {
            return Err(BuildError::unlowered("`!!` on a `Ref`", span));
        }
        let value = self.temp(e.ty);
        let (some_b, none_b) = (self.new_block(), self.new_block());
        self.nullable_match(&place, value, some_b, none_b, span, e);
        self.cur = none_b;
        self.terminate(TermKind::Abort(AbortReason::NullUnwrap), span, Some(e.id));
        self.cur = some_b;
        Ok(Operand::Copy(Place::local(value)))
    }

    /// `a ?: b`: `b` only runs when `a` is `None`.
    pub(super) fn elvis(&mut self, e: &HirExpr, lhs: &HirExpr, rhs: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        let result = self.temp(e.ty);
        let (some_b, none_b, end) = (self.new_block(), self.new_block(), self.new_block());
        if self.tcx.is_managed_ref(lhs.ty) {
            let l = self.expr(lhs)?;
            let is_none = self.temp(self.bool_ty);
            self.stmt(StmtKind::Ref(RefOp::IsNone { dst: Place::local(is_none), value: super::refs::as_copy(&l) }), span, Some(e.id));
            self.terminate(TermKind::Switch { value: Operand::Copy(Place::local(is_none)), cases: vec![(1, none_b)], otherwise: some_b }, span, Some(e.id));
            self.cur = some_b;
            self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(l)), span, Some(e.id));
        } else {
            let place = self.place_of(lhs)?;
            let value = self.temp(e.ty);
            self.nullable_match(&place, value, some_b, none_b, span, e);
            self.cur = some_b;
            self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(Operand::Copy(Place::local(value)))), span, Some(e.id));
        }
        self.goto(end, span);
        self.cur = none_b;
        let r = self.expr(rhs)?;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(r)), span, Some(e.id));
        self.goto(end, span);
        self.cur = end;
        Ok(Operand::Copy(Place::local(result)))
    }

    /// `==` / `!=` where a side is a `Ref`, text or nullable.
    pub(super) fn equality(&mut self, e: &HirExpr, is_ne: bool, lhs: &HirExpr, rhs: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        let t = self.tcx;
        let result = self.temp(self.bool_ty);
        if t.is_managed_ref(lhs.ty) || t.is_managed_ref(rhs.ty) {
            let (l, r) = (self.expr(lhs)?, self.expr(rhs)?);
            self.stmt(StmtKind::Ref(RefOp::Eq { dst: Place::local(result), lhs: l, rhs: r }), span, Some(e.id));
        } else if t.is_nullable(lhs.ty) || t.is_nullable(rhs.ty) || matches!(lhs.kind, HirExprKind::None) || matches!(rhs.kind, HirExprKind::None) {
            let (lp, rp) = (self.place_of(lhs)?, self.place_of(rhs)?);
            self.nullable_eq(result, &lp, lhs.ty, &rp, rhs.ty, span, e)?;
        } else {
            let (l, r) = (self.expr(lhs)?, self.expr(rhs)?);
            self.stmt(StmtKind::Intrinsic { dst: Some(Place::local(result)), op: IntrinsicOp::StringEq, args: vec![l, r], arena: None }, span, Some(e.id));
        }
        if is_ne {
            return Ok(self.assign_temp(e, Rvalue::Unary(MirUnOp::Not, Operand::Copy(Place::local(result)))));
        }
        Ok(Operand::Copy(Place::local(result)))
    }

    /// Equal when both are `None`, or both `Some` with equal payloads.
    #[allow(clippy::too_many_arguments)]
    fn nullable_eq(&mut self, result: Local, l: &Place, lt: TyId, r: &Place, rt: TyId, span: Span, e: &HirExpr) -> Res<()> {
        let set = |b: &mut Self, v: bool| b.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(Operand::Const(Const::Bool(v)))), span, Some(e.id));
        let end = self.new_block();
        let (l_some, l_none) = (self.new_block(), self.new_block());
        // Bound by `Match`; `payload_eq` gives them the payload type once it is known.
        let (lx, rx) = (self.temp(self.unit), self.temp(self.unit));
        // The `None` literal has no payload: it equals only a `None`.
        let l_is_nullable = self.tcx.is_nullable(lt);
        let r_is_nullable = self.tcx.is_nullable(rt);
        match (l_is_nullable, r_is_nullable) {
            (true, true) => {
                self.nullable_match(l, lx, l_some, l_none, span, e);
                self.cur = l_none;
                let (n_some, n_none) = (self.new_block(), self.new_block());
                self.nullable_match(r, rx, n_some, n_none, span, e);
                self.cur = n_some;
                set(self, false);
                self.goto(end, span);
                self.cur = n_none;
                set(self, true);
                self.goto(end, span);
                self.cur = l_some;
                let (s_some, s_none) = (self.new_block(), self.new_block());
                self.nullable_match(r, rx, s_some, s_none, span, e);
                self.cur = s_none;
                set(self, false);
                self.goto(end, span);
                self.cur = s_some;
                let payload = self.nullable_payload(lt);
                self.payload_eq(result, lx, rx, payload, span, e);
                self.goto(end, span);
            }
            (true, false) | (false, true) => {
                let nullable = if l_is_nullable { l } else { r };
                self.nullable_match(nullable, lx, l_some, l_none, span, e);
                self.cur = l_some;
                set(self, false);
                self.goto(end, span);
                self.cur = l_none;
                set(self, true);
                self.goto(end, span);
            }
            (false, false) => return Err(BuildError::Internal("nullable equality without a nullable".into())),
        }
        self.cur = end;
        Ok(())
    }

    fn payload_eq(&mut self, result: Local, l: Local, r: Local, payload: TyId, span: Span, e: &HirExpr) {
        // `l`/`r` were bound with a placeholder type; retype them to the payload.
        self.locals[l.0 as usize].ty = payload;
        self.locals[r.0 as usize].ty = payload;
        let (lo, ro) = (Operand::Copy(Place::local(l)), Operand::Copy(Place::local(r)));
        if self.tcx.is_text(payload) {
            self.stmt(StmtKind::Intrinsic { dst: Some(Place::local(result)), op: IntrinsicOp::StringEq, args: vec![lo, ro], arena: None }, span, Some(e.id));
        } else {
            self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Binary(BinOp::Eq, lo, ro)), span, Some(e.id));
        }
    }

    // --- `when` ---

    pub(super) fn match_expr(&mut self, e: &HirExpr, scrutinee: &HirExpr, arms: &[HirArm]) -> Res<Operand> {
        let span = span_of(e);
        if self.tcx.is_managed_ref(self.tcx.deref_ty(scrutinee.ty)) {
            return Err(BuildError::unsupported(Capability::MatchRef, span));
        }
        let place = self.place_of(scrutinee)?;
        let result = (!self.is_unit(e.ty)).then(|| self.temp(e.ty));
        let merge = self.new_block();
        let mut lowered = Vec::new();
        for arm in arms {
            let mut bound = Vec::new();
            let pat = self.lower_pat(&arm.pat, place.clone(), &mut bound)?;
            lowered.push((pat, bound, self.new_block()));
        }
        let mir_arms = lowered
            .iter()
            .map(|(pat, bound, target)| MirArm {
                pat: pat.clone(),
                bindings: bound.iter().filter(|b| !self.tcx.is_managed_ref(b.ty)).map(|b| (b.local, b.place.clone())).collect(),
                target: *target,
            })
            .collect();
        self.terminate(TermKind::Match { scrutinee: place, arms: mir_arms }, span, Some(e.id));
        let sink = result.map(|r| self.locals[r.0 as usize].ty).filter(|&ty| self.owning(ty)).map(|_| self.outer_sink());
        for (arm, (_, bound, target)) in arms.iter().zip(lowered) {
            self.cur = target;
            let force = pat_binds_owned_ref(self.tcx, &arm.pat);
            let (peeled, _, _) = self.enter_region(RegionSite::MatchArm, &arm.body, force, None, span)?;
            for b in &bound {
                if self.tcx.is_managed_ref(b.ty) {
                    self.stmt(StmtKind::Ref(RefOp::Clone { dst: Place::local(b.local), src: Operand::Copy(b.place.clone()) }), span, None);
                    self.register_ref_local(b.local, b.ty, span);
                }
                self.declare(&b.name, b.local);
            }
            match &sink {
                Some(s) => self.with_sink(s.clone(), |b| b.block_value(peeled, result))?,
                None => self.block_value(peeled, result)?,
            }
            self.exit_region(span)?;
            self.goto(merge, span);
        }
        self.cur = merge;
        Ok(result.map_or(Operand::Const(Const::Unit), |r| Operand::Copy(Place::local(r))))
    }

    fn lower_pat(&mut self, pat: &HirPat, place: Place, bound: &mut Vec<Bound>) -> Res<MirPat> {
        match &pat.kind {
            HirPatKind::Wildcard => Ok(MirPat::Wildcard),
            HirPatKind::Lit(l) => Ok(MirPat::Lit(l.clone())),
            HirPatKind::Bind { name, binding_ty } => {
                // A bound `unit` payload declares no local (RFC 3.2.1).
                // As `bind_pat` does: a payload of `unit` (or a borrow of it) declares no local.
                if self.tcx.deref_ty(*binding_ty) == self.unit {
                    self.unit_binds.insert(name.name.clone());
                } else {
                    let local = self.new_local(*binding_ty, LocalKind::Val, Some(name.name.clone()));
                    bound.push(Bound { name: name.name.clone(), local, place, ty: *binding_ty });
                }
                Ok(MirPat::Bind)
            }
            HirPatKind::Variant { enum_name, index, fields, .. } => {
                let mut subs = Vec::with_capacity(fields.len());
                for (i, f) in fields.iter().enumerate() {
                    let mut p = place.clone();
                    p.proj.push(Proj::Variant(*index as u32));
                    p.proj.push(Proj::Field(i as u32));
                    subs.push(self.lower_pat(f, p, bound)?);
                }
                Ok(MirPat::Variant { enum_name: enum_name.clone(), index: *index as u32, fields: subs })
            }
            HirPatKind::Some(_) | HirPatKind::None => Err(BuildError::unsupported(Capability::PatPresence, pat.span)),
        }
    }
}
