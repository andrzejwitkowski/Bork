use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValue, BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::hir::{HirExpr, Ty};

use super::emit_fn::FnEmitter;
use super::not_yet_supported;

pub(super) struct PresenceSplit<'ctx> {
    label: String,
    none_bb: BasicBlock<'ctx>,
    merge_bb: BasicBlock<'ctx>,
    incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)>,
    result_ty: Option<BasicTypeEnum<'ctx>>,
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn observe_ref(
        &mut self,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<(StructValue<'ctx>, PointerValue<'ctx>, IntValue<'ctx>), Diagnostic> {
        let input = self.entry_alloca(self.cx.ref_handle_type(), "ref.observe.in")?;
        let output = self.entry_alloca(self.cx.ref_observation_type(), "ref.observe.out")?;
        self.cx.builder.build_store(input, reference)?;
        self.cx
            .builder
            .build_call(self.cx.ref_observe_fn(), &[output.into(), input.into()], "")?;
        let observation = self.cx.builder.build_load(
            self.cx.ref_observation_type(),
            output,
            "ref.observe",
        )?
            .into_struct_value();
        let object = self
            .cx
            .builder
            .build_extract_value(observation, 0, "ref.object")?
            .into_pointer_value();
        let is_absent = self.cx.builder.build_int_compare(
            IntPredicate::EQ,
            object,
            object.get_type().const_null(),
            "ref.absent",
        )?;
        Ok((observation, object, is_absent))
    }

    pub(super) fn open_presence(
        &mut self,
        label: &str,
        is_absent: IntValue<'ctx>,
        result_ty: Option<&Ty>,
    ) -> Result<PresenceSplit<'ctx>, Diagnostic> {
        let some_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, &format!("{label}.some"));
        let none_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, &format!("{label}.none"));
        let merge_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, &format!("{label}.end"));
        self.cx
            .builder
            .build_conditional_branch(is_absent, none_bb, some_bb)?;
        self.cx.builder.position_at_end(some_bb);
        Ok(PresenceSplit {
            label: label.to_owned(),
            none_bb,
            merge_bb,
            incoming: Vec::new(),
            result_ty: result_ty.and_then(|ty| self.cx.basic_type(ty)),
        })
    }

    pub(super) fn end_present_arm(
        &mut self,
        split: &mut PresenceSplit<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<(), Diagnostic> {
        self.finish_presence_branch(split.merge_bb, value, &mut split.incoming)?;
        self.cx.builder.position_at_end(split.none_bb);
        Ok(())
    }

    pub(super) fn end_absent_arm(
        &mut self,
        mut split: PresenceSplit<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        self.finish_presence_branch(split.merge_bb, value, &mut split.incoming)?;
        self.cx.builder.position_at_end(split.merge_bb);
        let Some(result_ty) = split.result_ty else {
            return Ok(None);
        };
        if split.incoming.is_empty() {
            return Ok(Some(result_ty.const_zero()));
        }
        let phi = self.cx.builder.build_phi(result_ty, &split.label)?;
        let incoming_refs: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = split
            .incoming
            .iter()
            .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
            .collect();
        phi.add_incoming(&incoming_refs);
        Ok(Some(phi.as_basic_value()))
    }

    fn finish_presence_branch(
        &mut self,
        merge_bb: BasicBlock<'ctx>,
        value: Option<BasicValueEnum<'ctx>>,
        incoming: &mut Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)>,
    ) -> Result<(), Diagnostic> {
        if self.cx.current_block_terminated() {
            return Ok(());
        }
        let end = self
            .cx
            .builder
            .get_insert_block()
            .expect("presence branch");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        if let Some(value) = value {
            incoming.push((value, end));
        }
        Ok(())
    }

    pub(super) fn emit_safe_ref_field(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        class_name: &str,
        field_index: usize,
        result_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (observation, object, is_absent) = self.observe_ref(receiver)?;
        self.drop_ref_value(receiver)?;
        let field_ty = self
            .cx
            .record_field_type(class_name, field_index)
            .cloned()
            .ok_or_else(|| not_yet_supported("record field", span))?;
        let mut split = self.open_presence("nav", is_absent, Some(result_ty))?;
        let loaded = self.load_object_field(object, class_name, field_index, &field_ty, span)?;
        let present = self.materialize_navigated_field(loaded, &field_ty, result_ty)?;
        self.release_observation(observation)?;
        self.end_present_arm(&mut split, Some(present))?;
        self.release_observation(observation)?;
        let absent = self.cx.const_null_value(result_ty)?;
        self.end_absent_arm(split, Some(absent))?
            .ok_or_else(|| not_yet_supported(&format!("navigated type `{result_ty}`"), span))
    }

    fn materialize_navigated_field(
        &mut self,
        loaded: BasicValueEnum<'ctx>,
        field_ty: &Ty,
        result_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if field_ty.is_managed_ref() || field_ty.is_array() || field_ty.is_string() {
            let sink = self.sink_arena();
            return self.materialize_owned_value(loaded, field_ty, sink);
        }
        if field_ty.is_nullable() && field_ty.with_nullable(false).is_copy() {
            return Ok(loaded);
        }
        if field_ty.is_copy() {
            return self.emit_nullable_some(result_ty, loaded);
        }
        Err(not_yet_supported(
            &format!("safe navigation of `{field_ty}`"),
            None,
        ))
    }


    fn release_observation(&mut self, observation: StructValue<'ctx>) -> Result<(), Diagnostic> {
        let input = self
            .entry_alloca(self.cx.ref_observation_type(), "observation")?;
        self.cx.builder.build_store(input, observation)?;
        self.cx.builder.build_call(
            self.cx.ref_release_observation_fn(),
            &[input.into()],
            "",
        )?;
        Ok(())
    }

    pub(super) fn emit_ref_elvis(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let input = self.entry_alloca(self.cx.ref_handle_type(), "ref.none.in")?;
        self.cx.builder.build_store(input, lhs)?;
        let is_absent = self
            .cx
            .builder
            .build_call(self.cx.ref_is_none_fn(), &[input.into()], "ref.none")?
            .try_as_basic_value()
            .basic()
            .expect("bork_ref_is_none returns bool")
            .into_int_value();
        let mut split = self.open_presence("ref.elvis", is_absent, Some(&expr.ty))?;
        self.end_present_arm(&mut split, Some(lhs))?;
        self.drop_ref_value(lhs)?;
        let rhs_val = self.emit_skipped_as(rhs, &expr.ty)?;
        self.end_absent_arm(split, Some(rhs_val))?
            .ok_or_else(|| not_yet_supported(&format!("type `{}`", expr.ty), expr.span))
    }

}
