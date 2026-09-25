use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::hir::{HirExpr, Ty};

use super::super::emit_fn::FnEmitter;
use super::super::{codegen_error, not_yet_supported};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn emit_array_lit(
        &mut self,
        elements: &[HirExpr],
        array_ty: &Ty,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = array_ty
            .array_elem()
            .ok_or_else(|| not_yet_supported("array literal type", None))?;
        let count = elements.len();
        let (llvm_elem, stride, align) = self.elem_storage(elem_ty)?;
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
        for (index, element) in elements.iter().enumerate() {
            let slot = self.elem_ptr(base, index, llvm_elem, stride)?;
            let value = self.emit_value(element, elem_ty)?;
            builder.build_store(slot, value)?;
        }
        Ok(self.descriptor(base, len))
    }

    pub(in crate::codegen::llvm) fn emit_index_load(
        &mut self,
        receiver: &HirExpr,
        index: &HirExpr,
        elem_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let desc = self
            .emit_value(receiver, &receiver.ty)?
            .into_struct_value();
        let idx = self.emit_int(index, &Ty::i32())?;
        let (base, len) = self.descriptor_parts(desc)?;
        self.guard_index(idx, len, receiver.span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let ptr = self.elem_ptr_at(base, idx, llvm_elem, stride)?;
        builder_load(self, llvm_elem, ptr)
    }

    pub(in crate::codegen::llvm) fn emit_index_store(
        &mut self,
        array_ptr: PointerValue<'ctx>,
        array_ty: &Ty,
        index: &HirExpr,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
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
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let elem_ptr = self.elem_ptr_at(base, idx, llvm_elem, stride)?;
        self.cx.builder.build_store(elem_ptr, value)?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn emit_slice(
        &mut self,
        receiver: &HirExpr,
        lo: &HirExpr,
        result_ty: &Ty,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let desc = self
            .emit_value(receiver, &receiver.ty)?
            .into_struct_value();
        let (base, _) = self.descriptor_parts(desc)?;
        let lo_i = self.emit_int(lo, &Ty::i32())?;
        let elem_ty = result_ty
            .array_elem()
            .ok_or_else(|| not_yet_supported("slice result type", receiver.span))?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let lo64 = self.cx.builder.build_int_s_extend(
            lo_i,
            self.cx.context.i64_type(),
            "lo",
        )?;
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
        let new_len = result_ty.array_len().ok_or_else(|| {
            not_yet_supported("slice length is not part of the type", receiver.span)
        })?;
        Ok(self.descriptor(
            new_base,
            self.cx.context.i64_type().const_int(new_len as u64, false),
        ))
    }

    pub(in crate::codegen::llvm) fn emit_buffer_length(
        &mut self,
        receiver: &HirExpr,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let desc = self
            .emit_value(receiver, &receiver.ty)?
            .into_struct_value();
        let len = self
            .cx
            .builder
            .build_extract_value(desc, 1, "len")?
            .into_int_value();
        Ok(self.cx.builder.build_int_truncate(
            len,
            self.cx.context.i32_type(),
            "len.i32",
        )?)
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
        Ok(self.cx.builder.build_int_truncate(
            len,
            self.cx.context.i32_type(),
            "len.i32",
        )?)
    }

    pub(in crate::codegen::llvm) fn emit_index_load_values(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let desc = receiver.into_struct_value();
        let idx = self
            .cx
            .builder
            .build_int_truncate(
                index.into_int_value(),
                self.cx.context.i32_type(),
                "idx",
            )?;
        let (base, len) = self.descriptor_parts(desc)?;
        self.guard_index(idx, len, span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let ptr = self.elem_ptr_at(base, idx, llvm_elem, stride)?;
        builder_load(self, llvm_elem, ptr)
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
        let lo_i = self
            .cx
            .builder
            .build_int_truncate(lo.into_int_value(), self.cx.context.i32_type(), "lo")?;
        let elem_ty = result_ty
            .array_elem()
            .ok_or_else(|| not_yet_supported("slice result type", span))?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let lo64 = self.cx.builder.build_int_s_extend(
            lo_i,
            self.cx.context.i64_type(),
            "lo",
        )?;
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

    pub(in crate::codegen::llvm) fn emit_array_lit_values(
        &mut self,
        values: &[BasicValueEnum<'ctx>],
        elem_ty: &Ty,
        _array_ty: &Ty,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let count = values.len();
        let (llvm_elem, stride, align) = self.elem_storage(elem_ty)?;
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
        for (index, value) in values.iter().enumerate() {
            let slot = self.elem_ptr(base, index, llvm_elem, stride)?;
            builder.build_store(slot, *value)?;
        }
        Ok(self.descriptor(base, len))
    }

    pub(in crate::codegen::llvm) fn copy_array_into_arena(
        &mut self,
        source: StructValue<'ctx>,
        array_ty: &Ty,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = array_ty.array_elem().expect("array type");
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
        if elem_ty.is_string() {
            let zero = self.cx.context.i64_type().const_int(0, false);
            let one = self.cx.context.i64_type().const_int(1, false);
            let entry = self
                .cx
                .builder
                .get_insert_block()
                .expect("positioned");
            let header = self.cx.context.append_basic_block(self.llvm_fn, "arr.copy.hdr");
            let body = self.cx.context.append_basic_block(self.llvm_fn, "arr.copy.body");
            let done = self.cx.context.append_basic_block(self.llvm_fn, "arr.copy.done");
            self.cx.builder.build_unconditional_branch(header)?;
            self.cx.builder.position_at_end(header);
            let i_phi = self.cx.builder.build_phi(self.cx.context.i64_type(), "i")?;
            i_phi.add_incoming(&[(&zero as &dyn inkwell::values::BasicValue<'_>, entry)]);
            let i = i_phi.as_basic_value().into_int_value();
            let done_cond = self
                .cx
                .builder
                .build_int_compare(IntPredicate::UGE, i, count, "done")?;
            self.cx.builder.build_conditional_branch(done_cond, done, body)?;
            self.cx.builder.position_at_end(body);
            let src_slot = self.elem_ptr_at(base, self.truncate_i32(i), llvm_elem, stride)?;
            let dst_slot = self.elem_ptr_at(dst_base, self.truncate_i32(i), llvm_elem, stride)?;
            let loaded = builder_load(self, llvm_elem, src_slot)?.into_struct_value();
            let moved = self.copy_into_arena(loaded)?;
            self.cx.builder.build_store(dst_slot, moved)?;
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

    fn descriptor(&self, ptr: PointerValue<'ctx>, len: IntValue<'ctx>) -> StructValue<'ctx> {
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

    fn descriptor_parts(
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

    fn arena_alloc(
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
                    cx.context
                        .i64_type()
                        .const_int(align as u64, false)
                        .into(),
                ],
                "arr.alloc",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value())
    }

    fn elem_storage(
        &self,
        elem_ty: &Ty,
    ) -> Result<(inkwell::types::BasicTypeEnum<'ctx>, usize, usize), Diagnostic> {
        let llvm = self
            .cx
            .basic_type(elem_ty)
            .ok_or_else(|| not_yet_supported(&format!("array element `{elem_ty}`"), None))?;
        let (size, align) = array_elem_layout(elem_ty)?;
        Ok((llvm, size, align))
    }

    fn elem_ptr(
        &self,
        base: PointerValue<'ctx>,
        index: usize,
        llvm_elem: inkwell::types::BasicTypeEnum<'ctx>,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let idx = self
            .cx
            .context
            .i32_type()
            .const_int(index as u64, false);
        self.elem_ptr_at(base, idx, llvm_elem, stride)
    }

    fn elem_ptr_at(
        &self,
        base: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        llvm_elem: inkwell::types::BasicTypeEnum<'ctx>,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let offset = self.cx.builder.build_int_mul(
            self.cx
                .builder
                .build_int_s_extend(index, self.cx.context.i64_type(), "idx")?,
            self.cx
                .context
                .i64_type()
                .const_int(stride as u64, false),
            "slot.off",
        )?;
        let bytes = unsafe {
            self.cx.builder.build_gep(
                self.cx.context.i8_type(),
                base,
                &[offset],
                "slot.bytes",
            )?
        };
        Ok(self.cx.builder.build_pointer_cast(
            bytes,
            llvm_elem.ptr_type(inkwell::AddressSpace::default()),
            "slot",
        )?)
    }

    fn truncate_i32(&self, value: IntValue<'ctx>) -> IntValue<'ctx> {
        if value.get_type().get_bit_width() == 32 {
            value
        } else {
            self.cx
                .builder
                .build_int_truncate(value, self.cx.context.i32_type(), "i32")
                .expect("truncate in loop")
        }
    }

    fn guard_index(
        &mut self,
        index: IntValue<'ctx>,
        len: IntValue<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let idx = self
            .cx
            .builder
            .build_int_s_extend(index, len.get_type(), "idx")?;
        let zero = len.get_type().const_int(0, false);
        let neg = self
            .cx
            .builder
            .build_int_compare(IntPredicate::SLT, idx, zero, "neg")?;
        let ge = self
            .cx
            .builder
            .build_int_compare(IntPredicate::UGE, idx, len, "oob")?;
        let bad = self.cx.builder.build_or(neg, ge, "idx.bad")?;
        self.branch_abort_if(bad, span)?;
        Ok(())
    }

    fn branch_abort_if(
        &mut self,
        cond: inkwell::values::IntValue<'ctx>,
        _span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let ok = self.cx.context.append_basic_block(self.llvm_fn, "bounds.ok");
        let bad = self.cx.context.append_basic_block(self.llvm_fn, "bounds.bad");
        self.cx.builder.build_conditional_branch(cond, bad, ok)?;
        self.cx.builder.position_at_end(bad);
        self.cx.builder.build_call(self.cx.abort_function(), &[], "abort")?;
        self.cx.builder.build_unreachable()?;
        self.cx.builder.position_at_end(ok);
        Ok(())
    }
}

fn array_elem_layout(elem_ty: &Ty) -> Result<(usize, usize), Diagnostic> {
    if elem_ty.is_string() {
        return Ok((16, 8));
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
        _ => Err(not_yet_supported(&format!("array element `{elem_ty}`"), None)),
    }
}

fn builder_load<'ctx>(
    emitter: &FnEmitter<'_, '_, '_, 'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    ptr: PointerValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
    Ok(emitter.cx.builder.build_load(ty, ptr, "load")?)
}
