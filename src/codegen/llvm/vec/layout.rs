//! Header layout `{ buf, len, cap, arena }` and the low-level helpers that read and write it.

use inkwell::types::StructType;
use inkwell::values::{IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::diag::Diagnostic;

use super::super::context::Codegen;
use super::super::emit_fn::FnEmitter;

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn vec_header_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        self.context
            .struct_type(&[ptr.into(), word.into(), word.into(), ptr.into()], false)
    }

    pub(in crate::codegen::llvm) fn vec_field(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        Ok(self
            .builder
            .build_struct_gep(self.vec_header_type(), header, field, "vec.field")?)
    }

    pub(in crate::codegen::llvm) fn vec_load_word(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let field = self.vec_field(header, field)?;
        Ok(self
            .builder
            .build_load(self.context.i64_type(), field, "vec.word")?
            .into_int_value())
    }

    pub(in crate::codegen::llvm) fn vec_load_ptr(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let field = self.vec_field(header, field)?;
        Ok(self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                field,
                "vec.ptr",
            )?
            .into_pointer_value())
    }

    pub(in crate::codegen::llvm) fn vec_store(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
        value: impl inkwell::values::BasicValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let field = self.vec_field(header, field)?;
        self.builder.build_store(field, value)?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn word(&self, value: u64) -> IntValue<'ctx> {
        self.context.i64_type().const_int(value, false)
    }

    pub(in crate::codegen::llvm) fn vec_slot(
        &self,
        buf: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let offset = self
            .builder
            .build_int_mul(index, self.word(stride as u64), "vec.off")?;
        // SAFETY: callers bounds-check `index` against `len` or `cap`.
        Ok(unsafe {
            self.builder
                .build_gep(self.context.i8_type(), buf, &[offset], "vec.slot")?
        })
    }
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// Run `body(i)` for `i` in `0..count` (`count` is an `i64`).
    pub(super) fn vec_counted_loop(
        &mut self,
        count: IntValue<'ctx>,
        mut body: impl FnMut(&mut Self, IntValue<'ctx>) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        let context = self.cx.context;
        let entry = self.cx.builder.get_insert_block().expect("positioned");
        let header = context.append_basic_block(self.llvm_fn, "vec.loop.hdr");
        let body_block = context.append_basic_block(self.llvm_fn, "vec.loop.body");
        let done = context.append_basic_block(self.llvm_fn, "vec.loop.done");
        self.cx.builder.build_unconditional_branch(header)?;
        self.cx.position_at_end(header);
        let index = self.cx.builder.build_phi(context.i64_type(), "vec.i")?;
        index.add_incoming(&[(&self.cx.word(0), entry)]);
        let i = index.as_basic_value().into_int_value();
        let more = self
            .cx
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "vec.more")?;
        self.cx
            .builder
            .build_conditional_branch(more, body_block, done)?;
        self.cx.position_at_end(body_block);
        body(self, i)?;
        let next = self.cx.builder.build_int_add(i, self.cx.word(1), "vec.next")?;
        let tail = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(header)?;
        index.add_incoming(&[(&next, tail)]);
        self.cx.position_at_end(done);
        Ok(())
    }
}
