use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::hir::{HirExpr, Ty};

use super::emit_fn::FnEmitter;
use super::{codegen_error, not_yet_supported};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn emit_array_lit(
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

    pub(super) fn emit_index_load(
        &mut self,
        receiver: &HirExpr,
        index: &HirExpr,
        elem_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let desc = self
            .emit_value(receiver, receiver.ty)?
            .into_struct_value();
        let idx = self.emit_int(index, &Ty::i32())?;
        let (base, len) = self.descriptor_parts(desc)?;
        self.guard_index(idx, len, receiver.span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let ptr = self.elem_ptr_at(base, idx, llvm_elem, stride)?;
        builder_load(self, llvm_elem, ptr)
    }

    pub(super) fn emit_slice(
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

    pub(super) fn emit_buffer_length(
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

    pub(super) fn copy_array_into_arena(
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
            let builder = &self.cx.builder;
            builder.build_unconditional_branch(header)?;
            builder.position_at_end(header);
            let i_phi = builder.build_phi(self.cx.context.i64_type(), "i")?;
            i_phi.add_incoming(&[(&zero as &dyn inkwell::values::BasicValue<'_>, entry)]);
            let i = i_phi.as_basic_value().into_int_value();
            let done_cond = builder.build_int_compare(IntPredicate::UGE, i, count, "done")?;
            builder.build_conditional_branch(done_cond, done, body)?;
            builder.position_at_end(body);
            let src_slot = self.elem_ptr_at(base, self.truncate_i32(i), llvm_elem, stride)?;
            let dst_slot = self.elem_ptr_at(dst_base, self.truncate_i32(i), llvm_elem, stride)?;
            let loaded = builder_load(self, llvm_elem, src_slot)?.into_struct_value();
            let moved = self.copy_into_arena(loaded)?;
            builder.build_store(dst_slot, moved)?;
            let next = builder.build_int_add(i, one, "i.next")?;
            builder.build_unconditional_branch(header)?;
            i_phi.add_incoming(&[(&next as &dyn inkwell::values::BasicValue<'_>, body)]);
            builder.position_at_end(done);
        } else {
            self.cx
                .builder
                .build_memcpy(dst_base, align, base, align, byte_len)
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
        let size = llvm.size_of().unwrap_or(1) as usize;
        let align = llvm.get_alignment() as usize;
        Ok((llvm, size.max(1), align.max(1)))
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
        let cx = self.cx;
        let builder = &cx.builder;
        let idx = builder.build_int_s_extend(index, len.get_type(), "idx")?;
        let zero = len.get_type().const_int(0, false);
        let neg = builder.build_int_compare(IntPredicate::SLT, idx, zero, "neg")?;
        let ge = builder.build_int_compare(IntPredicate::UGE, idx, len, "oob")?;
        let bad = builder.build_or(neg, ge, "idx.bad")?;
        self.branch_abort_if(bad, span)?;
        Ok(())
    }

    fn branch_abort_if(
        &mut self,
        cond: inkwell::values::IntValue<'ctx>,
        span: Option<crate::span::Span>,
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

fn builder_load<'ctx>(
    emitter: &FnEmitter<'_, '_, '_, 'ctx>,
    ty: inkwell::types::BasicTypeEnum<'ctx>,
    ptr: PointerValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
    Ok(emitter.cx.builder.build_load(ty, ptr, "load")?)
}
