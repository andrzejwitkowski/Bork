//! `Buf<T>` codegen (#285): a fat handle `{ptr, cap: i32}` with no arena header. MIR has already guarded every index
//! and size (`mir/build/buf.rs`), so these emit only the allocation, the slot access and the grow copy.

use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue, StructValue};
use inkwell::AddressSpace;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::context::Codegen;

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn buf_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), self.context.i32_type().into()], false)
    }

    /// `cap` (already known non-negative) uninitialised slots of `elem` in `arena`.
    pub(in crate::codegen::llvm) fn buf_alloc(&self, arena: PointerValue<'ctx>, elem: TyId, cap: IntValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let (_, stride, align) = self.elem_storage(elem)?;
        let wide = self.builder.build_int_s_extend(cap, self.context.i64_type(), "buf.cap")?;
        let bytes = self.builder.build_int_mul(wide, self.word(stride as u64), "buf.bytes")?;
        let ptr = self.arena_alloc(arena, bytes, align)?;
        self.buf_value(ptr, cap)
    }

    pub(in crate::codegen::llvm) fn buf_cap(&self, buf: StructValue<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        Ok(self.builder.build_extract_value(buf, 1, "buf.cap")?.into_int_value())
    }

    /// Address of slot `index` (already bounds-checked).
    pub(in crate::codegen::llvm) fn buf_slot(&self, buf: StructValue<'ctx>, index: IntValue<'ctx>, elem: TyId) -> Result<(PointerValue<'ctx>, inkwell::types::BasicTypeEnum<'ctx>), Diagnostic> {
        let (llvm, stride, _) = self.elem_storage(elem)?;
        let ptr = self.builder.build_extract_value(buf, 0, "buf.ptr")?.into_pointer_value();
        let wide = self.builder.build_int_s_extend(index, self.context.i64_type(), "buf.idx")?;
        Ok((self.vec_slot(ptr, wide, stride)?, llvm))
    }

    pub(in crate::codegen::llvm) fn buf_load(&self, buf: StructValue<'ctx>, index: IntValue<'ctx>, elem: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (slot, llvm) = self.buf_slot(buf, index, elem)?;
        Ok(self.builder.build_load(llvm, slot, "buf.load")?)
    }

    pub(in crate::codegen::llvm) fn buf_store(&self, buf: StructValue<'ctx>, index: IntValue<'ctx>, elem: TyId, value: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        let (slot, _) = self.buf_slot(buf, index, elem)?;
        self.builder.build_store(slot, value)?;
        Ok(())
    }

    /// A new buffer of `new_cap` (already known larger) in `arena` holding the old `cap` slots; the old one stays
    /// until its arena dies (no `bork_arena_grow`).
    pub(in crate::codegen::llvm) fn buf_grow(&self, arena: PointerValue<'ctx>, buf: StructValue<'ctx>, new_cap: IntValue<'ctx>, elem: TyId) -> Result<StructValue<'ctx>, Diagnostic> {
        let (_, stride, align) = self.elem_storage(elem)?;
        let grown = self.buf_alloc(arena, elem, new_cap)?;
        let old = self.builder.build_extract_value(buf, 0, "buf.old")?.into_pointer_value();
        let new = self.builder.build_extract_value(grown, 0, "buf.new")?.into_pointer_value();
        let cap = self.builder.build_int_s_extend(self.buf_cap(buf)?, self.context.i64_type(), "buf.cap.old")?;
        let bytes = self.builder.build_int_mul(cap, self.word(stride as u64), "buf.bytes.old")?;
        self.builder.build_memcpy(new, align as u32, old, align as u32, bytes)?;
        Ok(grown)
    }

    fn buf_value(&self, ptr: PointerValue<'ctx>, cap: IntValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let ty = self.buf_type();
        let with_ptr = self.builder.build_insert_value(ty.get_undef(), ptr, 0, "buf")?;
        Ok(self.builder.build_insert_value(with_ptr, cap, 1, "buf")?.into_struct_value())
    }
}
