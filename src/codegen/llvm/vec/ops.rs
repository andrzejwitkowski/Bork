//! The built-in methods (`push`, `pop`, `len`, `capacity`, `clear`, ...) and their dispatch.

use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::ty::TyId;
use crate::hir::{VecOp};

use super::super::emit_fn::FnEmitter;
use super::{ARENA, BUF, CAP, LEN};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// Built-in method on a vector. `ops[0]` is the borrowed header pointer.
    pub(in crate::codegen::llvm) fn emit_vec_op(
        &mut self,
        op: VecOp,
        elem_ty: TyId,
        result_ty: TyId,
        ops: &[BasicValueEnum<'ctx>],
        span: Option<crate::span::Span>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let header = ops[0].into_pointer_value();
        match op {
            VecOp::Len | VecOp::Capacity => {
                let field = if op == VecOp::Len { LEN } else { CAP };
                let count = self.vec_load_word(header, field)?;
                Ok(Some(
                    self.cx
                        .builder
                        .build_int_truncate(count, self.cx.context.i32_type(), "vec.count")?
                        .into(),
                ))
            }
            VecOp::Get => self.emit_vec_get(ops[0], ops[1], elem_ty, span).map(Some),
            VecOp::Set => {
                self.vec_set(header, ops[1], elem_ty, ops[2], span)?;
                Ok(None)
            }
            VecOp::Push => {
                self.vec_reserve_one(header, elem_ty)?;
                let (_, stride, _) = self.elem_storage(elem_ty)?;
                let len = self.vec_load_word(header, LEN)?;
                let buf = self.vec_load_ptr(header, BUF)?;
                let slot = self.vec_slot(buf, len, stride)?;
                let arena = self.vec_load_ptr(header, ARENA)?;
                self.vec_store_elem(slot, arena, elem_ty, ops[1], true)?;
                let next = self.cx.builder.build_int_add(len, self.word(1), "vec.len.new")?;
                self.vec_store(header, LEN, next)?;
                Ok(None)
            }
            VecOp::Clear => {
                if self.cx.tcx.is_managed_ref(elem_ty) {
                    let len = self.vec_load_word(header, LEN)?;
                    let buf = self.vec_load_ptr(header, BUF)?;
                    let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
                    self.vec_counted_loop(len, |this, i| {
                        let slot = this.vec_slot(buf, i, stride)?;
                        let handle = this.cx.builder.build_load(llvm_elem, slot, "vec.drop")?;
                        this.drop_ref_value(handle)
                    })?;
                }
                self.vec_store(header, LEN, self.word(0))?;
                Ok(None)
            }
            VecOp::Pop => self.emit_vec_pop(header, elem_ty, result_ty).map(Some),
        }
    }

    pub(super) fn emit_vec_pop(
        &mut self,
        header: PointerValue<'ctx>,
        elem_ty: TyId,
        result_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let context = self.cx.context;
        let len = self.vec_load_word(header, LEN)?;
        let empty = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, len, self.word(0), "vec.pop.empty")?;
        let present = context.append_basic_block(self.llvm_fn, "vec.pop.some");
        let absent = context.append_basic_block(self.llvm_fn, "vec.pop.none");
        let merge = context.append_basic_block(self.llvm_fn, "vec.pop.done");
        self.cx
            .builder
            .build_conditional_branch(empty, absent, present)?;

        self.cx.builder.position_at_end(present);
        let last = self.cx.builder.build_int_sub(len, self.word(1), "vec.last")?;
        self.vec_store(header, LEN, last)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, last, stride)?;
        let popped = if self.cx.tcx.is_managed_ref(elem_ty) {
            // The slot is past `len` now, so the handle moves out and the slot becomes `None`.
            self.move_ref(slot)?
        } else {
            let loaded = self.cx.builder.build_load(llvm_elem, slot, "vec.pop")?;
            self.emit_nullable_some(result_ty, loaded)?
        };
        let present_end = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(merge)?;

        self.cx.builder.position_at_end(absent);
        let none = self.emit_none(result_ty)?;
        let absent_end = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(merge)?;

        self.cx.builder.position_at_end(merge);
        let phi = self.cx.builder.build_phi(popped.get_type(), "vec.pop.value")?;
        phi.add_incoming(&[(&popped, present_end), (&none, absent_end)]);
        Ok(phi.as_basic_value())
    }
}
