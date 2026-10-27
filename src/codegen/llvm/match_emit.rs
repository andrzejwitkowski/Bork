//! `when` over patterns (#53): the scrutinee is evaluated once, then each arm is a test chain
//! followed by a `MatchArm` region. The first arm whose pattern matches runs; coverage is checked
//! by typeck, so the block after the last arm's failed test is unreachable.
//!
//! Every binding borrows from the matched value, so an arm never owns a payload: the enum's own
//! drop glue (#52) stays the only releaser. The one exception is a `Ref` bound by value (a `Ref`
//! payload over a temporary), which is cloned into the arm's arena so the local owns a count.
//!
//! shortcut: arms are tested in order (no shared decision tree), and `Some`/`None` patterns are
//! not lowered here (`if val` covers `Ref`); upgrade when a ticket needs either.

use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValue, BasicValueEnum, IntValue, PointerValue};
use inkwell::IntPredicate;

use crate::ast::PatLit;
use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::hir::{HirArm, HirExpr, HirPat, HirPatKind};
use crate::region_walk::{pat_binds_owned_ref, region_enter_forced, ArenaCursor, WalkDriver};
use crate::span::Span;
use crate::ty::TyId;

use super::emit_fn::FnEmitter;
use super::{not_yet_supported, region_walk_codegen_error};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn emit_match_with_driver<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        scrutinee: &HirExpr,
        arms: &[HirArm],
        result_ty: TyId,
    ) -> Result<(), Diagnostic> {
        let stack_depth = self.walk.eval_stack.len();
        let value_ty = (result_ty != self.cx.unit_ty()).then_some(result_ty);
        let llvm_ty = match value_ty {
            Some(ty) => Some(self.cx.basic_type(ty).ok_or_else(|| {
                not_yet_supported(&format!("a `when` of type `{}`", self.cx.tcx.display(ty)), scrutinee.span)
            })?),
            None => None,
        };
        let result_sink = self.sink_arena();
        let previous = self.alloc_sink;

        driver.walk_expr(self, scrutinee).map_err(region_walk_codegen_error)?;
        let value = self
            .walk
            .trailing
            .take()
            .ok_or_else(|| not_yet_supported("a `when` scrutinee without a value", scrutinee.span))?;
        self.walk.eval_stack.truncate(stack_depth);

        let context = self.cx.context;
        let end_bb = context.append_basic_block(self.llvm_fn, "match.end");
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for arm in arms {
            let arm_bb = context.append_basic_block(self.llvm_fn, "match.arm");
            let next_bb = context.append_basic_block(self.llvm_fn, "match.next");
            self.emit_pat_test(&arm.pat, value, next_bb)?;
            self.cx.builder.build_unconditional_branch(arm_bb)?;
            self.cx.builder.position_at_end(arm_bb);

            let force = pat_binds_owned_ref(&self.cx.tcx, &arm.pat);
            region_enter_forced(driver, self, RegionSite::MatchArm, &arm.body, force)
                .map_err(region_walk_codegen_error)?;
            self.bind_pat(&arm.pat, value)?;
            if self.cx.tcx.owns_result(result_ty) {
                self.alloc_sink = Some(result_sink);
            }
            let arm_value =
                self.emit_presence_arm(driver, RegionSite::MatchArm, &arm.body, value_ty, result_ty, result_sink)?;
            self.alloc_sink = previous;

            if !self.cx.current_block_terminated() {
                let end = self.cx.builder.get_insert_block().expect("builder is positioned");
                self.cx.builder.build_unconditional_branch(end_bb)?;
                if value_ty.is_some() {
                    let arm_value = arm_value
                        .ok_or_else(|| not_yet_supported("a `when` arm without a trailing value", arm.pat.span.into()))?;
                    incoming.push((arm_value, end));
                }
            }
            self.cx.builder.position_at_end(next_bb);
        }
        // Typeck proved the arms cover every value, so no pattern fails past the last arm.
        self.cx.builder.build_unreachable()?;
        self.cx.builder.position_at_end(end_bb);

        if let Some(llvm_ty) = llvm_ty {
            let result = if incoming.is_empty() {
                llvm_ty.const_zero().as_basic_value_enum()
            } else {
                let phi = self.cx.builder.build_phi(llvm_ty, "match")?;
                let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> =
                    incoming.iter().map(|(v, b)| (v as &dyn BasicValue<'ctx>, *b)).collect();
                phi.add_incoming(&incoming);
                phi.as_basic_value()
            };
            self.walk.eval_stack.push(result);
            self.walk.trailing = Some(result);
        }
        Ok(())
    }

    /// Branches to `fail` unless `value` matches `pat`; otherwise falls through in a fresh block.
    fn emit_pat_test(
        &mut self,
        pat: &HirPat,
        value: BasicValueEnum<'ctx>,
        fail: BasicBlock<'ctx>,
    ) -> Result<(), Diagnostic> {
        match &pat.kind {
            HirPatKind::Wildcard | HirPatKind::Bind { .. } => Ok(()),
            HirPatKind::Lit(lit) => {
                let hit = self.emit_lit_test(lit, value, pat.span)?;
                self.branch_unless(hit, fail)
            }
            HirPatKind::Variant { index, fields, .. } => {
                let (record, object) = self.variant_slot(pat, *index, value)?;
                let i32_ty = self.cx.context.i32_type();
                let tag_ptr = self.cx.builder.build_struct_gep(record, object, 0, "tag.ptr")?;
                let tag = self.cx.builder.build_load(i32_ty, tag_ptr, "tag")?.into_int_value();
                let wanted = i32_ty.const_int(*index as u64, false);
                let hit = self.cx.builder.build_int_compare(IntPredicate::EQ, tag, wanted, "is.variant")?;
                self.branch_unless(hit, fail)?;
                for (field_index, field) in fields.iter().enumerate() {
                    if matches!(field.kind, HirPatKind::Wildcard | HirPatKind::Bind { .. }) {
                        continue;
                    }
                    let payload = self.load_payload(record, object, field_index, field.ty)?;
                    self.emit_pat_test(field, payload, fail)?;
                }
                Ok(())
            }
            HirPatKind::Some(_) | HirPatKind::None => {
                Err(not_yet_supported("`Some`/`None` patterns in `when` (use `if val`)", Some(pat.span)))
            }
        }
    }

    /// Declares every name `pat` binds, in the arm's region. A test already succeeded.
    fn bind_pat(&mut self, pat: &HirPat, value: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        match &pat.kind {
            HirPatKind::Wildcard | HirPatKind::Lit(_) | HirPatKind::None => Ok(()),
            // shortcut: a bound `unit` payload declares no local, so reading it in the arm is not supported yet.
            HirPatKind::Bind { binding_ty, .. } if self.cx.tcx.deref_ty(*binding_ty) == self.cx.unit_ty() => Ok(()),
            HirPatKind::Bind { name, binding_ty } => {
                let value = if self.cx.tcx.is_managed_ref(*binding_ty) { self.clone_ref(value)? } else { value };
                self.declare_local(&name.name, *binding_ty, value, None)
            }
            HirPatKind::Variant { index, fields, .. } => {
                let (record, object) = self.variant_slot(pat, *index, value)?;
                for (field_index, field) in fields.iter().enumerate() {
                    if matches!(field.kind, HirPatKind::Wildcard | HirPatKind::Lit(_)) {
                        continue;
                    }
                    let payload = self.load_payload(record, object, field_index, field.ty)?;
                    self.bind_pat(field, payload)?;
                }
                Ok(())
            }
            HirPatKind::Some(inner) => self.bind_pat(inner, value),
        }
    }

    /// The variant's struct type and the slot pointer of the enum value `value` of type `pat.ty`.
    fn variant_slot(
        &mut self,
        pat: &HirPat,
        index: usize,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(inkwell::types::StructType<'ctx>, PointerValue<'ctx>), Diagnostic> {
        let name = crate::mono::enum_instance_name(&self.cx.tcx, pat.ty)
            .ok_or_else(|| not_yet_supported("a variant pattern over a non-enum", Some(pat.span)))?;
        let record = self
            .cx
            .enum_layout(&name)
            .and_then(|layout| layout.variants.get(index).copied())
            .ok_or_else(|| not_yet_supported(&format!("enum `{name}`"), Some(pat.span)))?;
        let object = self
            .cx
            .builder
            .build_extract_value(value.into_struct_value(), 1, "enum.ptr")?
            .into_pointer_value();
        Ok((record, object))
    }

    /// Reads payload field `field_index` of the active variant. Not retained: a bound `Ref` is
    /// cloned by [`Self::bind_pat`], a tested one is only compared.
    fn load_payload(
        &mut self,
        record: inkwell::types::StructType<'ctx>,
        object: PointerValue<'ctx>,
        field_index: usize,
        field_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if field_ty == self.cx.unit_ty() {
            return Ok(self.cx.context.struct_type(&[], false).const_zero().into());
        }
        let ty = self.cx.basic_type(field_ty).ok_or_else(|| {
            not_yet_supported(&format!("an enum payload of type `{}`", self.cx.tcx.display(field_ty)), None)
        })?;
        let slot = self.cx.builder.build_struct_gep(record, object, field_index as u32 + 1, "payload")?;
        Ok(self.cx.builder.build_load(ty, slot, "payload")?)
    }

    fn emit_lit_test(
        &mut self,
        lit: &PatLit,
        value: BasicValueEnum<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match lit {
            PatLit::Int(n) => {
                if !value.is_int_value() {
                    return Err(not_yet_supported("an integer pattern over a non-integer", Some(span)));
                }
                let int = value.into_int_value();
                let wanted = int.get_type().const_int(*n as u64, true);
                Ok(self.cx.builder.build_int_compare(IntPredicate::EQ, int, wanted, "lit.eq")?)
            }
            PatLit::Bool(b) => {
                let flag = self.value_as_bool(value, Some(span))?;
                let wanted = flag.get_type().const_int(u64::from(*b), false);
                Ok(self.cx.builder.build_int_compare(IntPredicate::EQ, flag, wanted, "lit.eq")?)
            }
            PatLit::Str(text) => {
                let literal = self.emit_str_literal(text)?;
                Ok(self.emit_string_eq(value, literal.into(), false)?.into_int_value())
            }
        }
    }

    fn branch_unless(&mut self, hit: IntValue<'ctx>, fail: BasicBlock<'ctx>) -> Result<(), Diagnostic> {
        let ok = self.cx.context.append_basic_block(self.llvm_fn, "match.ok");
        self.cx.builder.build_conditional_branch(hit, ok, fail)?;
        self.cx.builder.position_at_end(ok);
        Ok(())
    }
}
