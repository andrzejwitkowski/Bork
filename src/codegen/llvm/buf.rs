//! `Buf<T>` codegen (#285): a fat handle `{ptr, cap: i32}` with no arena header. MIR has already guarded every index
//! and size (`mir/build/buf.rs`), so these emit only the allocation, the slot access and the grow copy.

use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};
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
        if self.debug.is_some() {
            // A zero-byte allocation, still tracked for the debugger.
            self.debug_allocation(ptr, "", &format!("Buf<{}>", self.tcx.display(elem)), false)?;
        }
        if self.tcx.is_managed_ref(elem) {
            // Zeroed slots are `None` handles: a slot that was never stored releases nothing. One drop per buffer
            // (its header is `{ptr, cap}`), registered where the buffer lives.
            self.builder.build_memset(ptr, 1, self.context.i8_type().const_zero(), bytes)?;
            let header = self.arena_alloc(arena, self.word(16), 8)?;
            if self.debug.is_some() {
                self.debug_allocation(header, "", "BufHeader", false)?;
            }
            self.builder.build_store(header, ptr)?;
            let cap_slot = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), header, &[self.word(8)], "buf.hdr.cap")? };
            self.builder.build_store(cap_slot, wide)?;
            self.builder.build_call(self.arena_register_drop_fn(), &[arena.into(), header.into(), self.buf_drop_refs_fn().as_global_value().as_pointer_value().into()], "")?;
        }
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

    /// `buf_store` for an element that lives in `arena`: a `Ref` goes through `bork_ref_store_take` (releasing the slot's old
    /// value), a `String` is copied into `arena` first.
    pub(in crate::codegen::llvm) fn buf_store_in(&self, f: FunctionValue<'ctx>, arena: PointerValue<'ctx>, buf: StructValue<'ctx>, index: IntValue<'ctx>, elem: TyId, value: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        if self.tcx.is_managed_ref(elem) {
            let (slot, _) = self.buf_slot(buf, index, elem)?;
            return self.ref_store(f, slot, arena, value);
        }
        let copied = self.copy_string_into(arena, value.into_struct_value())?;
        self.buf_store(buf, index, elem, copied.into())
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
        // An empty old buffer (possibly `bufEmpty`'s null pointer) has nothing to copy or zero.
        let function = self.builder.get_insert_block().and_then(|b| b.get_parent()).ok_or_else(|| super::not_yet_supported("bufGrow outside a function", None))?;
        let copy = self.context.append_basic_block(function, "buf.grow.copy");
        let done = self.context.append_basic_block(function, "buf.grow.done");
        let nonempty = self.builder.build_int_compare(inkwell::IntPredicate::NE, cap, self.word(0), "buf.nonempty")?;
        self.builder.build_conditional_branch(nonempty, copy, done)?;
        self.builder.position_at_end(copy);
        self.builder.build_memcpy(new, align as u32, old, align as u32, bytes)?;
        if self.tcx.is_managed_ref(elem) {
            // The handles moved: zero the old slots so the old buffer's registered drop releases nothing twice.
            self.builder.build_memset(old, 1, self.context.i8_type().const_zero(), bytes)?;
        }
        self.builder.build_unconditional_branch(done)?;
        self.builder.position_at_end(done);
        Ok(grown)
    }

    /// `{null, 0}`: no arena, no allocation, no drop registration.
    pub(in crate::codegen::llvm) fn buf_empty(&self) -> Result<StructValue<'ctx>, Diagnostic> {
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.buf_value(null, self.context.i32_type().const_zero())
    }

    fn buf_value(&self, ptr: PointerValue<'ctx>, cap: IntValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let ty = self.buf_type();
        let with_ptr = self.builder.build_insert_value(ty.get_undef(), ptr, 0, "buf")?;
        Ok(self.builder.build_insert_value(with_ptr, cap, 1, "buf")?.into_struct_value())
    }
}
