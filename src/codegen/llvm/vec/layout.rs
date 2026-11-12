//! Header layout `{ buf, len, cap, arena }` and the low-level helpers that read and write it.

use inkwell::types::StructType;
use inkwell::values::{IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::diag::Diagnostic;

use super::super::context::Codegen;

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
