//! Growth: make room for one more element, doubling the buffer when `len == cap`.

use inkwell::values::PointerValue;
use inkwell::{AddressSpace, IntPredicate};

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;
use super::{ARENA, BUF, CAP, FIRST_CAPACITY, LEN};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// Make room for one more element, doubling the buffer when `len == cap`.
    pub(super) fn vec_reserve_one(
        &mut self,
        header: PointerValue<'ctx>,
        elem_ty: TyId,
    ) -> Result<(), Diagnostic> {
        let (_, stride, align) = self.elem_storage(elem_ty)?;
        let context = self.cx.context;
        let len = self.vec_load_word(header, LEN)?;
        let cap = self.vec_load_word(header, CAP)?;
        let full = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, len, cap, "vec.full")?;
        let grow = context.append_basic_block(self.llvm_fn, "vec.grow");
        let done = context.append_basic_block(self.llvm_fn, "vec.room");
        self.cx.builder.build_conditional_branch(full, grow, done)?;

        self.cx.builder.position_at_end(grow);
        let doubled = self
            .cx
            .builder
            .build_int_mul(cap, self.word(2), "vec.cap.x2")?;
        let empty = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, cap, self.word(0), "vec.empty")?;
        let new_cap = self
            .cx
            .builder
            .build_select(empty, self.word(FIRST_CAPACITY), doubled, "vec.cap.new")?
            .into_int_value();
        let new_bytes = self
            .cx
            .builder
            .build_int_mul(new_cap, self.word(stride as u64), "vec.bytes.new")?;
        let old_bytes = self
            .cx
            .builder
            .build_int_mul(cap, self.word(stride as u64), "vec.bytes.old")?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let arena = self.vec_load_ptr(header, ARENA)?;
        let fresh = context.append_basic_block(self.llvm_fn, "vec.grow.alloc");
        let moved = context.append_basic_block(self.llvm_fn, "vec.grow.grow");
        let joined = context.append_basic_block(self.llvm_fn, "vec.grow.join");
        self.cx.builder.build_conditional_branch(empty, fresh, moved)?;

        self.cx.builder.position_at_end(fresh);
        let first = self.arena_alloc(arena, new_bytes, align)?;
        self.cx.builder.build_unconditional_branch(joined)?;

        self.cx.builder.position_at_end(moved);
        let grown = self
            .cx
            .builder
            .build_call(
                self.cx.arena_grow_fn(),
                &[
                    arena.into(),
                    buf.into(),
                    old_bytes.into(),
                    new_bytes.into(),
                    self.word(align as u64).into(),
                ],
                "vec.grown",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_grow returns a pointer")
            .into_pointer_value();
        self.cx.builder.build_unconditional_branch(joined)?;

        self.cx.builder.position_at_end(joined);
        let phi = self.cx.builder.build_phi(
            context.ptr_type(AddressSpace::default()),
            "vec.buf.new",
        )?;
        phi.add_incoming(&[(&first, fresh), (&grown, moved)]);
        self.vec_store(header, BUF, phi.as_basic_value())?;
        self.vec_store(header, CAP, new_cap)?;
        self.cx.builder.build_unconditional_branch(done)?;
        self.cx.builder.position_at_end(done);
        Ok(())
    }
}
