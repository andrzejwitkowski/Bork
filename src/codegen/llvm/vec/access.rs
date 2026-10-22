//! Element access: `v[i]`, `get`, `set`, index assignment and the shared slot store.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::AddressSpace;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;
use super::super::not_yet_supported;
use super::{ARENA, BUF, LEN};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// Store `value` into an uninitialised (`fresh`) or live slot of a vector owned by `arena`.
    pub(super) fn vec_store_elem(
        &mut self,
        slot: PointerValue<'ctx>,
        arena: PointerValue<'ctx>,
        elem_ty: TyId,
        value: BasicValueEnum<'ctx>,
        fresh: bool,
    ) -> Result<(), Diagnostic> {
        if self.cx.tcx.is_managed_ref(elem_ty) {
            if fresh {
                self.cx
                    .builder
                    .build_store(slot, self.cx.ref_handle_type().const_zero())?;
            }
            return self.store_ref(slot, arena, value);
        }
        // Strings are copied into the vector's arena; Copy values pass through.
        let stored = self.materialize_owned_value(value, elem_ty, arena)?;
        self.cx.builder.build_store(slot, stored)?;
        Ok(())
    }

    pub(super) fn vec_check_index(
        &mut self,
        header: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let idx = self.cx.builder.build_int_s_extend(
            index.into_int_value(),
            self.cx.context.i64_type(),
            "vec.idx",
        )?;
        let len = self.vec_load_word(header, LEN)?;
        self.guard_index(idx, len, span)?;
        Ok(idx)
    }

    /// `v[i]` / `v.get(i)`: Copy values load by value, `Ref` elements are cloned.
    pub(in crate::codegen::llvm) fn emit_vec_get(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: TyId,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let header = receiver.into_pointer_value();
        let idx = self.vec_check_index(header, index, span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, idx, stride)?;
        let loaded = self.cx.builder.build_load(llvm_elem, slot, "vec.get")?;
        if self.cx.tcx.is_managed_ref(elem_ty) {
            self.clone_ref(loaded)
        } else {
            Ok(loaded)
        }
    }

    /// `v[i] = value` where `slot_ptr` is the local holding the header pointer.
    pub(in crate::codegen::llvm) fn emit_vec_index_store(
        &mut self,
        slot_ptr: PointerValue<'ctx>,
        vec_ty: TyId,
        index: &crate::hir::HirExpr,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let elem_ty = self.cx.tcx.vec_elem(vec_ty)
            .ok_or_else(|| not_yet_supported("index assignment target", span))?;
        let header = self
            .cx
            .builder
            .build_load(
                self.cx.context.ptr_type(AddressSpace::default()),
                slot_ptr,
                "vec",
            )?
            .into_pointer_value();
        let index = self.emit_int(index, self.cx.i32_ty())?;
        self.vec_set(header, index.into(), elem_ty, value, span)
    }

    pub(super) fn vec_set(
        &mut self,
        header: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: TyId,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let idx = self.vec_check_index(header, index, span)?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, idx, stride)?;
        let arena = self.vec_load_ptr(header, ARENA)?;
        self.vec_store_elem(slot, arena, elem_ty, value, false)
    }
}
