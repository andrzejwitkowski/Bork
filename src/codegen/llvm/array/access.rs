//! Reading and writing elements: `a[i]`, `a[i] = v`, `.length` and slices.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};

use crate::diag::Diagnostic;
use crate::hir::{HirExpr, Ty};

use super::super::emit_fn::FnEmitter;
use super::super::not_yet_supported;
use super::layout::builder_load;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn emit_index_load_values(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let desc = receiver.into_struct_value();
        let idx = self.cx.builder.build_int_truncate(
            index.into_int_value(),
            self.cx.context.i32_type(),
            "idx",
        )?;
        let (base, len) = self.descriptor_parts(desc)?;
        self.guard_index(idx, len, span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let ptr = self.elem_ptr_at(base, idx, stride)?;
        let loaded = builder_load(self, llvm_elem, ptr)?;
        if elem_ty.is_managed_ref() {
            self.clone_ref(loaded)
        } else {
            Ok(loaded)
        }
    }

    pub(in crate::codegen::llvm) fn emit_index_store(
        &mut self,
        array_ptr: PointerValue<'ctx>,
        array_ty: &Ty,
        index: &HirExpr,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        if array_ty.vec_elem().is_some() {
            return self.emit_vec_index_store(array_ptr, array_ty, index, value, span);
        }
        let elem_ty = array_ty
            .array_elem()
            .ok_or_else(|| not_yet_supported("index assignment target", span))?;
        let llvm_ty = self
            .cx
            .basic_type(array_ty)
            .ok_or_else(|| not_yet_supported(&format!("array `{array_ty}`"), span))?;
        let desc = self
            .cx
            .builder
            .build_load(llvm_ty, array_ptr, "arr")?
            .into_struct_value();
        let idx = self.emit_int(index, &Ty::i32())?;
        let (base, len) = self.descriptor_parts(desc)?;
        self.guard_index(idx, len, span)?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let elem_ptr = self.elem_ptr_at(base, idx, stride)?;
        if elem_ty.is_managed_ref() {
            let arena = self.sink_arena();
            self.store_ref(elem_ptr, arena, value)?;
        } else {
            self.cx.builder.build_store(elem_ptr, value)?;
        }
        Ok(())
    }

    pub(in crate::codegen::llvm) fn emit_buffer_length_value(
        &mut self,
        desc: StructValue<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let len = self
            .cx
            .builder
            .build_extract_value(desc, 1, "len")?
            .into_int_value();
        Ok(self
            .cx
            .builder
            .build_int_truncate(len, self.cx.context.i32_type(), "len.i32")?)
    }

    pub(in crate::codegen::llvm) fn emit_slice_values(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        lo: BasicValueEnum<'ctx>,
        result_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let desc = receiver.into_struct_value();
        let (base, _) = self.descriptor_parts(desc)?;
        let lo_i = self.cx.builder.build_int_truncate(
            lo.into_int_value(),
            self.cx.context.i32_type(),
            "lo",
        )?;
        let elem_ty = result_ty
            .array_elem()
            .ok_or_else(|| not_yet_supported("slice result type", span))?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let lo64 = self
            .cx
            .builder
            .build_int_s_extend(lo_i, self.cx.context.i64_type(), "lo")?;
        let offset = self.cx.builder.build_int_mul(
            lo64,
            self.cx.context.i64_type().const_int(stride as u64, false),
            "off",
        )?;
        let new_base = unsafe {
            self.cx
                .builder
                .build_gep(self.cx.context.i8_type(), base, &[offset], "slice.base")?
        };
        let new_len = result_ty
            .array_len()
            .ok_or_else(|| not_yet_supported("slice length is not part of the type", span))?;
        Ok(self.descriptor(
            new_base,
            self.cx.context.i64_type().const_int(new_len as u64, false),
        ))
    }
}
