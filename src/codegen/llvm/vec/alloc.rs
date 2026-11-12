//! Creating a vector: the header and, for `Vec<T>(n)`, the first buffer.

use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::context::Codegen;
use super::super::emit_fn::FnEmitter;
use super::{ARENA, BUF, CAP, HEADER_ALIGN, HEADER_SIZE, LEN};

impl<'ctx> Codegen<'ctx> {
    /// Allocate a header (and a `cap`-element buffer when `cap > 0`) in `arena`.
    pub(in crate::codegen::llvm) fn vec_alloc(
        &self,
        llvm_fn: FunctionValue<'ctx>,
        arena: PointerValue<'ctx>,
        elem_ty: TyId,
        cap: Option<IntValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let header = self.arena_alloc(arena, self.word(HEADER_SIZE), HEADER_ALIGN)?;
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.vec_store(header, BUF, null)?;
        self.vec_store(header, LEN, self.word(0))?;
        self.vec_store(header, CAP, self.word(0))?;
        self.vec_store(header, ARENA, arena)?;
        if let Some(cap) = cap {
            let (_, stride, align) = self.elem_storage(elem_ty)?;
            let positive = self.builder.build_int_compare(IntPredicate::SGT, cap, self.word(0), "vec.cap.pos")?;
            let alloc = self.context.append_basic_block(llvm_fn, "vec.cap.alloc");
            let done = self.context.append_basic_block(llvm_fn, "vec.cap.done");
            self.builder.build_conditional_branch(positive, alloc, done)?;
            self.position_at_end(alloc);
            let bytes = self.builder.build_int_mul(cap, self.word(stride as u64), "vec.cap.bytes")?;
            let buf = self.arena_alloc(arena, bytes, align)?;
            if self.debug.is_some() {
                self.debug_allocation(buf, "", &format!("Vec<{}>", self.tcx.display(elem_ty)), false)?;
            }
            self.vec_store(header, BUF, buf)?;
            self.vec_store(header, CAP, cap)?;
            self.builder.build_unconditional_branch(done)?;
            self.position_at_end(done);
        }
        if let Some(drop_fn) = self.vec_drop_glue(elem_ty) {
            self.builder.build_call(
                self.arena_register_drop_fn(),
                &[arena.into(), header.into(), drop_fn.as_global_value().as_pointer_value().into()],
                "",
            )?;
        }
        if self.debug.is_some() {
            self.debug_allocation(header, "", &format!("Vec<{}>", self.tcx.display(elem_ty)), false)?;
        }
        Ok(header)
    }

    /// `Vec<T>()` / `Vec<T>(cap)` in `arena`; `cap` is the `i32` value of the optional argument.
    pub(in crate::codegen::llvm) fn vec_new(
        &self,
        llvm_fn: FunctionValue<'ctx>,
        arena: PointerValue<'ctx>,
        elem_ty: TyId,
        cap: Option<BasicValueEnum<'ctx>>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let cap = match cap {
            Some(cap) => {
                let cap = cap.into_int_value();
                let negative = self.builder.build_int_compare(IntPredicate::SLT, cap, cap.get_type().const_zero(), "vec.cap.neg")?;
                self.branch_abort_if(llvm_fn, negative)?;
                Some(self.builder.build_int_s_extend(cap, self.context.i64_type(), "vec.cap")?)
            }
            None => None,
        };
        self.vec_alloc(llvm_fn, arena, elem_ty, cap)
    }
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// `Vec<T>()` / `Vec<T>(cap)`; `cap` is the `i32` value of the optional argument.
    pub(in crate::codegen::llvm) fn emit_vec_new(
        &mut self,
        elem_ty: TyId,
        cap: Option<BasicValueEnum<'ctx>>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let arena = self.sink_arena();
        Ok(self.cx.vec_new(self.llvm_fn, arena, elem_ty, cap)?.into())
    }
}
