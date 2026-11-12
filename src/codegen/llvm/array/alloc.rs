//! Creating arrays: arena allocation, array literals and copying into another arena.

use inkwell::values::{IntValue, PointerValue};

use crate::diag::Diagnostic;

use super::super::context::Codegen;

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn arena_alloc(
        &self,
        arena: PointerValue<'ctx>,
        size: IntValue<'ctx>,
        align: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let cx = self;
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
}
