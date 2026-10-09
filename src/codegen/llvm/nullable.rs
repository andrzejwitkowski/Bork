//! Lowering for `T?`, `None`, `Some`, `?:`, and `!!`.

use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::codegen_gate::{nullable_repr, NullableRepr};
use crate::diag::Diagnostic;
use crate::hir::{HirExpr, HirExprKind, Prim, Ty, TyKind};

use super::context::Codegen;
use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'ctx> Codegen<'ctx> {
    pub fn const_null_value(&self, ty: &Ty) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if ty.is_managed_ref() {
            return Ok(self.ref_handle_type().const_zero().into());
        }
        let llvm_ty = self
            .nullable_storage_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("nullable type `{ty}`"), None))?;
        Ok(llvm_ty.const_zero())
    }
}

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    pub(super) fn emit_nullable_is_null(
        &mut self,
        ty: &Ty,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(ty) {
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
                &format!("nullable test for `{ty}`"),
                None,
            )),
        }
    }

    pub(super) fn emit_nullable_some(
        &mut self,
        nullable_ty: &Ty,
        inner: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(nullable_ty) {
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
                &format!("Some for `{nullable_ty}`"),
                None,
            )),
        }
    }

    pub(super) fn emit_nullable_unwrap(
        &mut self,
        nullable_ty: &Ty,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        match nullable_repr(nullable_ty) {
            Some(NullableRepr::Buffer) => Ok(value),
            Some(NullableRepr::TaggedScalar) => Ok(cx
                .builder
                .build_extract_value(value.into_struct_value(), 1, "opt.val")
                .expect("option payload")),
            None => Err(not_yet_supported(&format!("unwrap `{nullable_ty}`"), None)),
        }
    }

    pub(super) fn emit_nullable_expect_non_null(
        &mut self,
        nullable_ty: &Ty,
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
        if lhs.ty.is_managed_ref() {
            return self.emit_ref_elvis(lhs_val, rhs, expr);
        }
        let nullable_ty = &lhs.ty;
        let result_ty = &expr.ty;
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
            .ok_or_else(|| not_yet_supported(&format!("type `{result_ty}`"), expr.span))?;
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
            let is_null = self.emit_nullable_is_null(&rhs.ty, rhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if rhs_null {
            let is_null = self.emit_nullable_is_null(&lhs.ty, lhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if lhs.ty.is_nullable() || rhs.ty.is_nullable() {
            let ty = if lhs.ty.is_nullable() {
                &lhs.ty
            } else {
                &rhs.ty
            };
            let l_null = self.emit_nullable_is_null(ty, lhs_val)?;
            let r_null = self.emit_nullable_is_null(ty, rhs_val)?;
            let both_null = cx.builder.build_and(l_null, r_null, "both_null")?;
            let both_some = cx.builder.build_and(
                cx.builder.build_not(l_null, "l.some")?,
                cx.builder.build_not(r_null, "r.some")?,
                "both_some",
            )?;
            let payload_eq = match nullable_repr(ty) {
                Some(NullableRepr::Buffer) => self.emit_buffer_desc_eq(lhs_val, rhs_val)?,
                Some(NullableRepr::TaggedScalar) => {
                    self.emit_tagged_payload_eq(ty, lhs_val, rhs_val, expr.span)?
                }
                None => {
                    return Err(not_yet_supported(
                        &format!("nullable equality for `{ty}`"),
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
        ty: &Ty,
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
        let inner = ty.with_nullable(false);
        if matches!(inner.kind, TyKind::Prim(Prim::F32 | Prim::F64)) {
            let l = self.value_as_float(l_val, &inner, span)?;
            let r = self.value_as_float(r_val, &inner, span)?;
            return Ok(self.cx.builder.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                l,
                r,
                "feq",
            )?);
        }
        let l = self.value_as_int(l_val, &inner, span)?;
        let r = self.value_as_int(r_val, &inner, span)?;
        Ok(self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, l, r, "ieq")?)
    }

    pub(super) fn emit_safe_nullable_length(
        &mut self,
        receiver_ty: &Ty,
        receiver_val: BasicValueEnum<'ctx>,
        result_ty: &Ty,
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
            .ok_or_else(|| not_yet_supported(&format!("type `{result_ty}`"), span))?;
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
    use crate::hir::{Prim, Ty, TyKind};

    #[test]
    fn string_optional_is_buffer() {
        assert_eq!(nullable_repr(&Ty::string(true)), Some(NullableRepr::Buffer));
    }

    #[test]
    fn i32_optional_is_tagged() {
        assert_eq!(
            nullable_repr(&Ty::i32().with_nullable(true)),
            Some(NullableRepr::TaggedScalar)
        );
    }

    #[test]
    fn f32_optional_is_tagged() {
        let ty = Ty::new(TyKind::Prim(Prim::F32), true);
        assert_eq!(nullable_repr(&ty), Some(NullableRepr::TaggedScalar));
        assert!(codegen_lowers_nullable(&ty));
    }

    #[test]
    fn non_nullable_is_none() {
        assert_eq!(nullable_repr(&Ty::i32()), None);
        assert!(codegen_lowers_nullable(&Ty::i32()));
    }

    #[test]
    fn nullable_class_has_no_representation() {
        let ty = Ty::new(TyKind::Named("Point".into()), true);
        assert_eq!(nullable_repr(&ty), None);
        assert!(!codegen_lowers_nullable(&ty));
    }
}
