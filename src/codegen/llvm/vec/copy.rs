//! `move` / `promote`: copy a vector into the sink arena, rehoming every `Ref`.

use inkwell::values::PointerValue;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::emit_fn::FnEmitter;
use super::super::not_yet_supported;
use super::{BUF, LEN};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    /// `move` / `promote` / owned result: a new vector in the sink arena holding copies of the
    /// elements. Refs are cloned and rehomed (their edge kinds are recomputed for the new owner);
    /// the source stays valid, like `copy_array_into_arena`.
    pub(in crate::codegen::llvm) fn copy_vec_into_arena(
        &mut self,
        source: PointerValue<'ctx>,
        vec_ty: TyId,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let elem_ty = self.cx.tcx.vec_elem(vec_ty)
            .ok_or_else(|| not_yet_supported("vector copy type", None))?
            .clone();
        let arena = self.sink_arena();
        let len = self.vec_load_word(source, LEN)?;
        let header = self.vec_alloc(arena, elem_ty, Some(len))?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let from = self.vec_load_ptr(source, BUF)?;
        let to = self.vec_load_ptr(header, BUF)?;
        // `vec_alloc` left `to` null for an empty source; the loop below never runs then.
        self.vec_counted_loop(len, |this, i| {
            let src_slot = this.vec_slot(from, i, stride)?;
            let dst_slot = this.vec_slot(to, i, stride)?;
            let loaded = this.cx.builder.build_load(llvm_elem, src_slot, "vec.copy")?;
            let value = if self.cx.tcx.is_managed_ref(elem_ty) {
                this.clone_ref(loaded)?
            } else {
                loaded
            };
            this.vec_store_elem(dst_slot, arena, elem_ty, value, true)
        })?;
        self.vec_store(header, LEN, len)?;
        Ok(header)
    }
}
