//! Creating a vector: the header and, for `Vec<T>(n)`, the first buffer.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;
use super::{ARENA, BUF, CAP, HEADER_ALIGN, HEADER_SIZE, LEN};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// Allocate a header (and a `cap`-element buffer when `cap > 0`) in `arena`.
    pub(super) fn vec_alloc(
        &mut self,
        arena: PointerValue<'ctx>,
        elem_ty: TyId,
        cap: Option<IntValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let header = self.arena_alloc(arena, self.word(HEADER_SIZE), HEADER_ALIGN)?;
        let null = self.cx.context.ptr_type(AddressSpace::default()).const_null();
        self.vec_store(header, BUF, null)?;
        self.vec_store(header, LEN, self.word(0))?;
        self.vec_store(header, CAP, self.word(0))?;
        self.vec_store(header, ARENA, arena)?;
        if let Some(cap) = cap {
            let (_, stride, align) = self.elem_storage(elem_ty)?;
            let positive = self.cx.builder.build_int_compare(
                IntPredicate::SGT,
                cap,
                self.word(0),
                "vec.cap.pos",
            )?;
            let alloc = self.cx.context.append_basic_block(self.llvm_fn, "vec.cap.alloc");
            let done = self.cx.context.append_basic_block(self.llvm_fn, "vec.cap.done");
            self.cx
                .builder
                .build_conditional_branch(positive, alloc, done)?;
            self.cx.position_at_end(alloc);
            let bytes = self
                .cx
                .builder
                .build_int_mul(cap, self.word(stride as u64), "vec.cap.bytes")?;
            let buf = self.arena_alloc(arena, bytes, align)?;
            if self.cx.debug.is_some() {
                self.cx.debug_allocation(buf, "", &format!("Vec<{}>", self.cx.tcx.display(elem_ty)), false)?;
            }
            self.vec_store(header, BUF, buf)?;
            self.vec_store(header, CAP, cap)?;
            self.cx.builder.build_unconditional_branch(done)?;
            self.cx.position_at_end(done);
        }
        if self.cx.tcx.is_managed_ref(elem_ty) {
            // One drop per vector: the buffer moves on growth, so slot addresses cannot be registered.
            let drop_fn = self
                .cx
                .vec_drop_refs_fn()
                .as_global_value()
                .as_pointer_value();
            self.cx.builder.build_call(
                self.cx.arena_register_drop_fn(),
                &[arena.into(), header.into(), drop_fn.into()],
                "",
            )?;
        }
        if self.cx.debug.is_some() {
            self.cx.debug_allocation(header, "", &format!("Vec<{}>", self.cx.tcx.display(elem_ty)), false)?;
        }
        Ok(header)
    }

    /// `Vec<T>()` / `Vec<T>(cap)`; `cap` is the `i32` value of the optional argument.
    pub(in crate::codegen::llvm) fn emit_vec_new(
        &mut self,
        elem_ty: TyId,
        cap: Option<BasicValueEnum<'ctx>>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let arena = self.sink_arena();
        let cap = match cap {
            Some(cap) => {
                let cap = cap.into_int_value();
                let negative = self.cx.builder.build_int_compare(
                    IntPredicate::SLT,
                    cap,
                    cap.get_type().const_zero(),
                    "vec.cap.neg",
                )?;
                self.branch_abort_if(negative, None)?;
                Some(self.cx.builder.build_int_s_extend(
                    cap,
                    self.cx.context.i64_type(),
                    "vec.cap",
                )?)
            }
            None => None,
        };
        Ok(self.vec_alloc(arena, elem_ty, cap)?.into())
    }
}
