//! Array value layout: the `{ ptr, len }` descriptor, element size/stride and slot addressing.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};

use crate::diag::Diagnostic;
use crate::hir::Ty;

use super::super::emit_fn::FnEmitter;
use super::super::not_yet_supported;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn descriptor(&self, ptr: PointerValue<'ctx>, len: IntValue<'ctx>) -> StructValue<'ctx> {
        let builder = &self.cx.builder;
        let empty = self.cx.buffer_descriptor_type().const_named_struct(&[]);
        let value = builder
            .build_insert_value(empty, ptr, 0, "desc.ptr")
            .expect("descriptor ptr");
        builder
            .build_insert_value(value, len, 1, "desc.len")
            .expect("descriptor len")
            .into_struct_value()
    }

    pub(super) fn descriptor_parts(
        &self,
        desc: StructValue<'ctx>,
    ) -> Result<(PointerValue<'ctx>, IntValue<'ctx>), Diagnostic> {
        let builder = &self.cx.builder;
        let base = builder
            .build_extract_value(desc, 0, "arr.base")?
            .into_pointer_value();
        let len = builder
            .build_extract_value(desc, 1, "arr.len")?
            .into_int_value();
        Ok((base, len))
    }

    pub(in crate::codegen::llvm) fn elem_storage(
        &self,
        elem_ty: &Ty,
    ) -> Result<(inkwell::types::BasicTypeEnum<'ctx>, usize, usize), Diagnostic> {
        let llvm = self
            .cx
            .basic_type(elem_ty)
            .ok_or_else(|| not_yet_supported(&format!("array element `{elem_ty}`"), None))?;
        let (size, align) = array_elem_layout(self.cx, elem_ty)?;
        Ok((llvm, size, align))
    }

    pub(super) fn elem_ptr(
        &self,
        base: PointerValue<'ctx>,
        index: usize,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let idx = self.cx.context.i32_type().const_int(index as u64, false);
        self.elem_ptr_at(base, idx, stride)
    }

    pub(super) fn elem_ptr_at(
        &self,
        base: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let offset = self.cx.builder.build_int_mul(
            self.cx
                .builder
                .build_int_s_extend(index, self.cx.context.i64_type(), "idx")?,
            self.cx.context.i64_type().const_int(stride as u64, false),
            "slot.off",
        )?;
        let bytes = unsafe {
            self.cx
                .builder
                .build_gep(self.cx.context.i8_type(), base, &[offset], "slot.bytes")?
        };
        Ok(bytes)
    }

    pub(in crate::codegen::llvm) fn truncate_i32(&self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        if value.get_type().get_bit_width() == 32 {
            value
        } else {
            self.cx
                .builder
                .build_int_truncate(value, self.cx.context.i32_type(), "i32")
                .expect("truncate in loop")
        }
    }
}

fn array_elem_layout(
    cx: &super::super::context::Codegen<'_>,
    elem_ty: &Ty,
) -> Result<(usize, usize), Diagnostic> {
    if elem_ty.is_string() {
        return Ok((16, 8));
    }
    if elem_ty.is_managed_ref() {
        return Ok(cx.ref_handle_layout());
    }
    use crate::hir::{Prim, TyKind};
    match &elem_ty.kind {
        TyKind::Prim(prim) if !elem_ty.nullable => match prim {
            Prim::I8 | Prim::U8 | Prim::Bool => Ok((1, 1)),
            Prim::I16 | Prim::U16 => Ok((2, 2)),
            Prim::I32 | Prim::U32 | Prim::F32 => Ok((4, 4)),
            Prim::I64 | Prim::U64 | Prim::F64 => Ok((8, 8)),
            Prim::Unit => Err(not_yet_supported("array element `unit`", None)),
        },
        _ => Err(not_yet_supported(
            &format!("array element `{elem_ty}`"),
            None,
        )),
    }
}

pub(super) fn builder_load<'ctx>(
    emitter: &FnEmitter<'_, '_, '_, 'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    ptr: PointerValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
    Ok(emitter.cx.builder.build_load(ty, ptr, "load")?)
}
