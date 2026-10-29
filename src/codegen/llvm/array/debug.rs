//! Debug-runtime labels for `[Ref<T>; N]` elements.

use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn debug_label_ref_array(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: TyId,
        label: &str,
    ) -> Result<(), Diagnostic> {
        let Some(elem) = self.cx.tcx.array_elem(ty).filter(|&elem| self.cx.tcx.is_managed_ref(elem)) else {
            return Ok(());
        };
        if self.cx.debug.is_none() || self.cx.tcx.is_ref(ty) {
            return Ok(());
        }
        let (base, len) = self.descriptor_parts(value.into_struct_value())?;
        let (_, stride, _) = self.elem_storage(elem)?;
        let cx = self.cx;
        let builder = &cx.builder;
        let word = cx.context.i64_type();
        let entry = builder
            .get_insert_block()
            .expect("array label insertion point");
        let header = cx
            .context
            .append_basic_block(self.llvm_fn, "debug.array.header");
        let body = cx
            .context
            .append_basic_block(self.llvm_fn, "debug.array.body");
        let done = cx
            .context
            .append_basic_block(self.llvm_fn, "debug.array.done");
        builder.build_unconditional_branch(header)?;
        cx.position_at_end(header);
        let index = builder.build_phi(word, "debug.index")?;
        index.add_incoming(&[(&word.const_zero(), entry)]);
        let current = index.as_basic_value().into_int_value();
        let more =
            builder.build_int_compare(IntPredicate::ULT, current, len, "debug.array.more")?;
        builder.build_conditional_branch(more, body, done)?;
        cx.position_at_end(body);
        let slot = self.elem_ptr_at(base, self.truncate_i32(current), stride)?;
        cx.debug_relabel_slot(slot, label, elem, current)?;
        let next = builder.build_int_add(current, word.const_int(1, false), "debug.index.next")?;
        builder.build_unconditional_branch(header)?;
        index.add_incoming(&[(&next, body)]);
        cx.position_at_end(done);
        Ok(())
    }
}
