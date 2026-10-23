//! Lowering for `T?`, `None`, `Some`, `?:`, and `!!`.

use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::codegen_gate::{nullable_repr, NullableRepr};
use crate::diag::Diagnostic;
use crate::hir::{HirExpr, HirExprKind, Prim};
use crate::ty::{TyData, TyId};

use super::context::Codegen;
use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'ctx> Codegen<'ctx> {
    pub fn const_null_value(&self, ty: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.tcx.is_managed_ref(ty) {
            return Ok(self.ref_handle_type().const_zero().into());
        }
        let llvm_ty = self
            .nullable_storage_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("nullable type `{}`", self.tcx.display(ty)), None))?;
        Ok(llvm_ty.const_zero())
    }
}

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    pub(super) fn emit_nullable_is_null(
        &mut self,
        ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(&self.cx.tcx, ty) {
            Some(NullableRepr::Buffer) => {
                let desc = value.into_struct_value();
                let ptr = cx
                    .builder
                    .build_extract_value(desc, 0, "opt.ptr")
                    .expect("descriptor field 0")
                    .into_pointer_value();
                let null = ptr.get_type().const_null();
                Ok(cx
                    .builder
                    .build_int_compare(IntPredicate::EQ, ptr, null, "opt.is_null")?)
            }
            Some(NullableRepr::TaggedScalar) => {
                let tag = cx
                    .builder
                    .build_extract_value(value.into_struct_value(), 0, "opt.tag")
                    .expect("option tag")
                    .into_int_value();
                let zero = cx.context.bool_type().const_int(0, false);
                Ok(cx
                    .builder
                    .build_int_compare(IntPredicate::EQ, tag, zero, "opt.is_null")?)
            }
            None => Err(not_yet_supported(
                &format!("nullable test for `{}`", self.cx.tcx.display(ty)),
                None,
            )),
        }
    }

    pub(super) fn emit_nullable_some(
        &mut self,
        nullable_ty: TyId,
        inner: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(&self.cx.tcx, nullable_ty) {
            Some(NullableRepr::Buffer) => Ok(inner),
            Some(NullableRepr::TaggedScalar) => {
                let storage = cx
                    .nullable_storage_type(nullable_ty)
                    .expect("tagged scalar nullable")
                    .into_struct_type();
                let tag = cx.context.bool_type().const_int(1, false);
                let empty = storage.const_named_struct(&[]);
                let with_tag = cx.builder.build_insert_value(empty, tag, 0, "opt.tag")?;
                Ok(cx
                    .builder
                    .build_insert_value(with_tag, inner, 1, "opt.val")?
                    .into_struct_value()
                    .into())
            }
            None => Err(not_yet_supported(
                &format!("Some for `{}`", self.cx.tcx.display(nullable_ty)),
                None,
            )),
        }
    }

    pub(super) fn emit_nullable_unwrap(
        &mut self,
        nullable_ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(&self.cx.tcx, nullable_ty) {
            Some(NullableRepr::Buffer) => Ok(value),
            Some(NullableRepr::TaggedScalar) => Ok(cx
                .builder
                .build_extract_value(value.into_struct_value(), 1, "opt.val")
                .expect("option payload")),
            None => Err(not_yet_supported(&format!("unwrap `{}`", self.cx.tcx.display(nullable_ty)), None)),
        }
    }

    pub(super) fn emit_nullable_expect_non_null(
        &mut self,
        nullable_ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let is_null = self.emit_nullable_is_null(nullable_ty, value)?;
        let ok_bb = cx.context.append_basic_block(self.llvm_fn, "nn.ok");
        let abort_bb = cx.context.append_basic_block(self.llvm_fn, "nn.abort");
        cx.builder
            .build_conditional_branch(is_null, abort_bb, ok_bb)?;
        cx.builder.position_at_end(abort_bb);
        cx.builder.build_call(cx.abort_function(), &[], "abort")?;
        cx.builder.build_unreachable()?;
        cx.builder.position_at_end(ok_bb);
        self.emit_nullable_unwrap(nullable_ty, value)
    }

    pub(super) fn emit_elvis(
        &mut self,
        lhs: &HirExpr,
        rhs: &HirExpr,
        lhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.cx.tcx.is_managed_ref(lhs.ty) {
            return self.emit_ref_elvis(lhs_val, rhs, expr);
        }
        let nullable_ty = lhs.ty;
        let result_ty = expr.ty;
        let is_null = self.emit_nullable_is_null(nullable_ty, lhs_val)?;
        let rhs_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.rhs");
        let lhs_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.lhs");
        let merge_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.end");
        self.cx
            .builder
            .build_conditional_branch(is_null, rhs_bb, lhs_bb)?;
        self.cx.builder.position_at_end(rhs_bb);
        let rhs_val = self.emit_skipped_as(rhs, result_ty)?;
        let rhs_end = self.cx.builder.get_insert_block().expect("rhs block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(lhs_bb);
        let lhs_unwrapped = self.emit_nullable_unwrap(nullable_ty, lhs_val)?;
        let lhs_end = self.cx.builder.get_insert_block().expect("lhs block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(merge_bb);
        let llvm_ty = self
            .cx
            .basic_type(result_ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{}`", self.cx.tcx.display(result_ty)), expr.span))?;
        let phi = self
            .cx
            .builder
            .build_phi(llvm_ty.as_basic_type_enum(), "elvis")?;
        phi.add_incoming(&[(&rhs_val, rhs_end), (&lhs_unwrapped, lhs_end)]);
        Ok(phi.as_basic_value())
    }

    pub(super) fn emit_nullable_eq(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let eq = *op == BinOp::Eq;
        let cx = self.cx;
        let i1 = cx.context.bool_type();

        let lhs_null = matches!(lhs.kind, HirExprKind::None);
        let rhs_null = matches!(rhs.kind, HirExprKind::None);

        let result = if lhs_null && rhs_null {
            i1.const_int(eq as u64, false)
        } else if lhs_null {
            let is_null = self.emit_nullable_is_null(rhs.ty, rhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if rhs_null {
            let is_null = self.emit_nullable_is_null(lhs.ty, lhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if self.cx.tcx.is_nullable(lhs.ty) || self.cx.tcx.is_nullable(rhs.ty) {
            let ty = if self.cx.tcx.is_nullable(lhs.ty) {
                lhs.ty
            } else {
                rhs.ty
            };
            let l_null = self.emit_nullable_is_null(ty, lhs_val)?;
            let r_null = self.emit_nullable_is_null(ty, rhs_val)?;
            let both_null = cx.builder.build_and(l_null, r_null, "both_null")?;
            let both_some = cx.builder.build_and(
                cx.builder.build_not(l_null, "l.some")?,
                cx.builder.build_not(r_null, "r.some")?,
                "both_some",
            )?;
            let payload_eq = match nullable_repr(&self.cx.tcx, ty) {
                Some(NullableRepr::Buffer) => self.emit_buffer_desc_eq(lhs_val, rhs_val)?,
                Some(NullableRepr::TaggedScalar) => {
                    self.emit_tagged_payload_eq(ty, lhs_val, rhs_val, expr.span)?
                }
                None => {
                    return Err(not_yet_supported(
                        &format!("nullable equality for `{}`", self.cx.tcx.display(ty)),
                        expr.span,
                    ));
                }
            };
            let value_eq = cx.builder.build_and(both_some, payload_eq, "val.eq")?;
            self.eq_or_ne(eq, cx.builder.build_or(both_null, value_eq, "nullable.eq")?)?
        } else {
            return Err(not_yet_supported("nullable equality mismatch", expr.span));
        };

        Ok(result.into())
    }

    fn eq_or_ne(&self, eq: bool, is_eq: IntValue<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        if eq {
            Ok(is_eq)
        } else {
            Ok(self.cx.builder.build_not(is_eq, "nullable.ne")?)
        }
    }

    pub(super) fn emit_buffer_desc_eq(
        &self,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        let l_desc = lhs_val.into_struct_value();
        let r_desc = rhs_val.into_struct_value();
        let l_ptr = cx
            .builder
            .build_extract_value(l_desc, 0, "l.ptr")?
            .into_pointer_value();
        let r_ptr = cx
            .builder
            .build_extract_value(r_desc, 0, "r.ptr")?
            .into_pointer_value();
        let l_len = cx.builder.build_extract_value(l_desc, 1, "l.len")?;
        let r_len = cx.builder.build_extract_value(r_desc, 1, "r.len")?;
        let ptr_eq = cx
            .builder
            .build_int_compare(IntPredicate::EQ, l_ptr, r_ptr, "ptr.eq")?;
        let len_eq = cx.builder.build_int_compare(
            IntPredicate::EQ,
            l_len.into_int_value(),
            r_len.into_int_value(),
            "len.eq",
        )?;
        Ok(cx.builder.build_and(ptr_eq, len_eq, "desc.eq")?)
    }

    fn emit_tagged_payload_eq(
        &mut self,
        ty: TyId,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let l_val = self
            .cx
            .builder
            .build_extract_value(lhs_val.into_struct_value(), 1, "l.v")?;
        let r_val = self
            .cx
            .builder
            .build_extract_value(rhs_val.into_struct_value(), 1, "r.v")?;
        let inner = self.cx.tcx.base(ty);
        if matches!(self.cx.tcx.data(inner), TyData::Prim(Prim::F32 | Prim::F64)) {
            let l = self.value_as_float(l_val, inner, span)?;
            let r = self.value_as_float(r_val, inner, span)?;
            return Ok(self.cx.builder.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                l,
                r,
                "feq",
            )?);
        }
        let l = self.value_as_int(l_val, inner, span)?;
        let r = self.value_as_int(r_val, inner, span)?;
        Ok(self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, l, r, "ieq")?)
    }

    pub(super) fn emit_safe_nullable_length(
        &mut self,
        receiver_ty: TyId,
        receiver_val: BasicValueEnum<'ctx>,
        result_ty: TyId,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let is_null = self.emit_nullable_is_null(receiver_ty, receiver_val)?;
        let null_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.null");
        let some_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.some");
        let merge_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.end");
        self.cx
            .builder
            .build_conditional_branch(is_null, null_bb, some_bb)?;
        self.cx.builder.position_at_end(null_bb);
        let none_val = self.cx.const_null_value(result_ty)?;
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(some_bb);
        let len = self.emit_buffer_length_value(receiver_val.into_struct_value())?;
        let some_val = self.emit_nullable_some(result_ty, len.into())?;
        let some_end = self.cx.builder.get_insert_block().expect("some block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(merge_bb);
        let llvm_ty = self
            .cx
            .basic_type(result_ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{}`", self.cx.tcx.display(result_ty)), span))?;
        let phi = self
            .cx
            .builder
            .build_phi(llvm_ty.as_basic_type_enum(), "safe.len")?;
        phi.add_incoming(&[(&none_val, null_bb), (&some_val, some_end)]);
        Ok(phi.as_basic_value())
    }
}

#[cfg(test)]
mod nullable_repr_tests {
    use crate::codegen_gate::{codegen_lowers_nullable, nullable_repr, NullableRepr};
    use crate::hir::Prim;
    use crate::ty::TyCtx;

    fn ctx() -> TyCtx {
        let program = crate::parse("class Point {\n    x: i32\n}\nfun main() {}\n").unwrap();
        TyCtx::new(&crate::defs::DefTable::build(&program))
    }

    #[test]
    fn string_optional_is_buffer() {
        let mut c = ctx();
        let ty = c.string(true);
        assert_eq!(nullable_repr(&c, ty), Some(NullableRepr::Buffer));
    }

    #[test]
    fn i32_optional_is_tagged() {
        let mut c = ctx();
        let (i, ty) = (c.i32(), c.i32());
        let ty = c.with_nullable(ty, true);
        let _ = i;
        assert_eq!(nullable_repr(&c, ty), Some(NullableRepr::TaggedScalar));
    }

    #[test]
    fn f32_optional_is_tagged() {
        let mut c = ctx();
        let f32_ty = c.prim(Prim::F32);
        let ty = c.with_nullable(f32_ty, true);
        assert_eq!(nullable_repr(&c, ty), Some(NullableRepr::TaggedScalar));
        assert!(codegen_lowers_nullable(&c, ty));
    }

    #[test]
    fn non_nullable_is_none() {
        let mut c = ctx();
        let ty = c.i32();
        assert_eq!(nullable_repr(&c, ty), None);
        assert!(codegen_lowers_nullable(&c, ty));
    }

    #[test]
    fn nullable_class_has_no_representation() {
        let mut c = ctx();
        let defs = crate::defs::DefTable::build(&crate::parse("class Point {\n    x: i32\n}\nfun main() {}\n").unwrap());
        let point = c.class(defs.lookup_class("Point").unwrap(), Vec::new());
        let class = c.with_nullable(point, true);
        assert_eq!(nullable_repr(&c, class), None);
        assert!(!codegen_lowers_nullable(&c, class));
    }
}
