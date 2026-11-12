//! Array value layout: the `{ ptr, len }` descriptor, element size/stride and slot addressing.


use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::super::context::Codegen;
use super::super::not_yet_supported;

fn array_elem_layout(
    cx: &super::super::context::Codegen<'_>,
    elem_ty: TyId,
) -> Result<(usize, usize), Diagnostic> {
    if cx.tcx.is_string(elem_ty) {
        return Ok((16, 8));
    }
    if cx.tcx.is_managed_ref(elem_ty) {
        return Ok(cx.ref_handle_layout());
    }
    use crate::hir::Prim;
    use crate::ty::TyData;
    match cx.tcx.data(elem_ty) {
        TyData::Prim(prim) => match prim {
            Prim::I8 | Prim::U8 | Prim::Bool => Ok((1, 1)),
            Prim::I16 | Prim::U16 => Ok((2, 2)),
            Prim::I32 | Prim::U32 | Prim::F32 => Ok((4, 4)),
            Prim::I64 | Prim::U64 | Prim::F64 => Ok((8, 8)),
            Prim::Unit => Err(not_yet_supported("array element `unit`", None)),
        },
        _ => Err(not_yet_supported(
            &format!("array element `{}`", cx.tcx.display(elem_ty)),
            None,
        )),
    }
}

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn elem_storage(
        &self,
        elem_ty: TyId,
    ) -> Result<(inkwell::types::BasicTypeEnum<'ctx>, usize, usize), Diagnostic> {
        let llvm = self
            .basic_type(elem_ty)
            .ok_or_else(|| not_yet_supported(&format!("array element `{}`", self.tcx.display(elem_ty)), None))?;
        let (size, align) = array_elem_layout(self, elem_ty)?;
        Ok((llvm, size, align))
    }
}
