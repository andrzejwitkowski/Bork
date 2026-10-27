//! Creating arrays: arena allocation, array literals and copying into another arena.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;
use super::super::codegen_error;
use super::layout::builder_load;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn arena_alloc(
        &self,
        arena: PointerValue<'ctx>,
        size: IntValue<'ctx>,
        align: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        Ok(cx
            .builder
            .build_call(
                cx.arena_alloc_fn(),
                &[
                    arena.into(),
                    size.into(),
                    cx.context.i64_type().const_int(align as u64, false).into(),
                ],
                "arr.alloc",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value())
    }

    pub(in crate::codegen::llvm) fn emit_array_lit_values(
        &mut self,
        values: &[BasicValueEnum<'ctx>],
        elem_ty: TyId,
        array_ty: TyId,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let count = values.len();
        let (_, stride, align) = self.elem_storage(elem_ty)?;
        let cx = self.cx;
        let builder = &cx.builder;
        let len = cx.context.i64_type().const_int(count as u64, false);
        let byte_len = builder.build_int_mul(
            len,
            cx.context.i64_type().const_int(stride as u64, false),
            "arr.bytes",
        )?;
        let arena = self.sink_arena();
        let base = self.arena_alloc(arena, byte_len, align)?;
        cx.debug_allocation(base, "", &cx.tcx.display(array_ty).to_string(), false)?;
        for (index, value) in values.iter().enumerate() {
            let slot = self.elem_ptr(base, index, stride)?;
            if self.cx.tcx.is_managed_ref(elem_ty) {
                self.store_fresh_ref(slot, arena, *value)?;
                self.cx.debug_label_slot(
                    arena,
                    slot,
                    "[]",
                    elem_ty,
                    self.cx.context.i64_type().const_zero(),
                    self.cx.context.i64_type().const_int(index as u64, false),
                )?;
            } else {
                builder.build_store(slot, *value)?;
            }
        }
        Ok(self.descriptor(base, len))
    }

    pub(in crate::codegen::llvm) fn copy_array_into_arena(
        &mut self,
        source: StructValue<'ctx>,
        array_ty: TyId,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = self.cx.tcx.array_elem(array_ty).expect("array type");
        let (base, len) = self.descriptor_parts(source)?;
        let count = len;
        let (llvm_elem, stride, align) = self.elem_storage(elem_ty)?;
        let byte_len = self.cx.builder.build_int_mul(
            count,
            self.cx.context.i64_type().const_int(stride as u64, false),
            "copy.bytes",
        )?;
        let arena = self.sink_arena();
        let dst_base = self.arena_alloc(arena, byte_len, align)?;
        self.cx.debug_allocation(dst_base, "", &self.cx.tcx.display(array_ty).to_string(), false)?;
        self.cx.debug_copy_origin(base, dst_base)?;
        if self.cx.tcx.is_string(elem_ty) || self.cx.tcx.is_managed_ref(elem_ty) {
            let zero = self.cx.context.i64_type().const_int(0, false);
            let one = self.cx.context.i64_type().const_int(1, false);
            let entry = self.cx.builder.get_insert_block().expect("positioned");
            let header = self
                .cx
                .context
                .append_basic_block(self.llvm_fn, "arr.copy.hdr");
            let body = self
                .cx
                .context
                .append_basic_block(self.llvm_fn, "arr.copy.body");
            let done = self
                .cx
                .context
                .append_basic_block(self.llvm_fn, "arr.copy.done");
            self.cx.builder.build_unconditional_branch(header)?;
            self.cx.builder.position_at_end(header);
            let i_phi = self.cx.builder.build_phi(self.cx.context.i64_type(), "i")?;
            i_phi.add_incoming(&[(&zero as &dyn inkwell::values::BasicValue<'_>, entry)]);
            let i = i_phi.as_basic_value().into_int_value();
            let done_cond =
                self.cx
                    .builder
                    .build_int_compare(IntPredicate::UGE, i, count, "done")?;
            self.cx
                .builder
                .build_conditional_branch(done_cond, done, body)?;
            self.cx.builder.position_at_end(body);
            let src_slot = self.elem_ptr_at(base, self.truncate_i32(i), stride)?;
            let dst_slot = self.elem_ptr_at(dst_base, self.truncate_i32(i), stride)?;
            let loaded = builder_load(self, llvm_elem, src_slot)?;
            if self.cx.tcx.is_string(elem_ty) {
                let copied = self.copy_into_arena(loaded.into_struct_value())?;
                self.cx.builder.build_store(dst_slot, copied)?;
            } else {
                let cloned = self.clone_ref(loaded)?;
                self.store_fresh_ref(dst_slot, arena, cloned)?;
                self.cx.debug_label_slot(
                    arena,
                    dst_slot,
                    "[]",
                    elem_ty,
                    self.cx.context.i64_type().const_zero(),
                    i,
                )?;
            }
            let next = self.cx.builder.build_int_add(i, one, "i.next")?;
            self.cx.builder.build_unconditional_branch(header)?;
            i_phi.add_incoming(&[(&next as &dyn inkwell::values::BasicValue<'_>, body)]);
            self.cx.builder.position_at_end(done);
        } else {
            self.cx
                .builder
                .build_memcpy(dst_base, align as u32, base, align as u32, byte_len)
                .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        }
        Ok(self.descriptor(dst_base, count))
    }
}
