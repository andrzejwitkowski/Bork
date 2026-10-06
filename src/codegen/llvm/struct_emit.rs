//! Struct value emission: `new`, field extract, structural equality.

use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::codegen_gate::{nullable_repr, NullableRepr};
use crate::diag::Diagnostic;
use crate::hir::{Prim, Ty, TyKind};

use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn emit_struct_new(
        &mut self,
        name: &str,
        field_vals: &[BasicValueEnum<'ctx>],
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let llvm_ty = self
            .cx
            .llvm_struct(name)
            .ok_or_else(|| not_yet_supported(&format!("struct `{name}`"), span))?;
        let mut agg = llvm_ty.get_undef();
        for (i, value) in field_vals.iter().enumerate() {
            agg = self
                .cx
                .builder
                .build_insert_value(agg, *value, i as u32, "struct.new")?
                .into_struct_value();
        }
        Ok(agg.into())
    }

    pub(super) fn emit_struct_field(
        &mut self,
        receiver_ty: &Ty,
        receiver_val: BasicValueEnum<'ctx>,
        name: &str,
        safe: bool,
        result_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let struct_name = receiver_ty
            .struct_name()
            .ok_or_else(|| not_yet_supported("struct field", span))?;
        let def = self
            .cx
            .struct_def(struct_name)
            .ok_or_else(|| not_yet_supported(&format!("struct `{struct_name}`"), span))?;
        let idx = def
            .field_index(name)
            .ok_or_else(|| not_yet_supported(&format!("field `{name}`"), span))?;
        let field_ty = def.field_ty(name).cloned().unwrap_or_else(Ty::unknown);
        if safe && receiver_ty.is_nullable() {
            return self.emit_safe_nullable_struct_field(
                receiver_ty,
                receiver_val,
                idx as u32,
                &field_ty,
                result_ty,
                span,
            );
        }
        Ok(self
            .cx
            .builder
            .build_extract_value(receiver_val.into_struct_value(), idx as u32, name)?)
    }

    pub(super) fn emit_struct_eq(
        &mut self,
        op: &BinOp,
        ty: &Ty,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let eq = self.emit_value_eq(ty, lhs_val, rhs_val, span)?;
        Ok(match op {
            BinOp::Eq => eq.into(),
            BinOp::Ne => self.cx.builder.build_not(eq, "struct.ne")?.into(),
            _ => return Err(not_yet_supported("struct comparison", span)),
        })
    }

    /// Recursive value equality for prims, structs, and nullable payloads.
    pub(super) fn emit_value_eq(
        &mut self,
        ty: &Ty,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        if ty.is_nullable() {
            return self.emit_nullable_value_eq(ty, lhs_val, rhs_val, span);
        }
        if let Some(name) = ty.struct_name() {
            let def = self
                .cx
                .struct_def(name)
                .ok_or_else(|| not_yet_supported(&format!("struct `{name}`"), span))?;
            let lhs = lhs_val.into_struct_value();
            let rhs = rhs_val.into_struct_value();
            let mut all_eq = self.cx.context.bool_type().const_int(1, false);
            for (i, field) in def.fields.iter().enumerate() {
                let l = self
                    .cx
                    .builder
                    .build_extract_value(lhs, i as u32, "eq.l")?;
                let r = self
                    .cx
                    .builder
                    .build_extract_value(rhs, i as u32, "eq.r")?;
                let field_eq = self.emit_value_eq(&field.ty, l, r, span)?;
                all_eq = self
                    .cx
                    .builder
                    .build_and(all_eq, field_eq, "struct.eq")?;
            }
            return Ok(all_eq);
        }
        if matches!(ty.kind, TyKind::Prim(Prim::F32 | Prim::F64)) {
            let lf = self.value_as_float(lhs_val, ty, span)?;
            let rf = self.value_as_float(rhs_val, ty, span)?;
            return Ok(self.cx.builder.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                lf,
                rf,
                "feq",
            )?);
        }
        let li = self.value_as_int(lhs_val, ty, span)?;
        let ri = self.value_as_int(rhs_val, ty, span)?;
        Ok(self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, li, ri, "ieq")?)
    }

    fn emit_nullable_value_eq(
        &mut self,
        ty: &Ty,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        match nullable_repr(ty) {
            Some(NullableRepr::Buffer) => self.emit_buffer_desc_eq(lhs_val, rhs_val),
            Some(NullableRepr::TaggedScalar) => {
                let l_null = self.emit_nullable_is_null(ty, lhs_val)?;
                let r_null = self.emit_nullable_is_null(ty, rhs_val)?;
                let both_null = self.cx.builder.build_and(l_null, r_null, "both_null")?;
                let both_some = self.cx.builder.build_and(
                    self.cx.builder.build_not(l_null, "l.some")?,
                    self.cx.builder.build_not(r_null, "r.some")?,
                    "both_some",
                )?;
                let l_pay = self.emit_nullable_unwrap(ty, lhs_val)?;
                let r_pay = self.emit_nullable_unwrap(ty, rhs_val)?;
                let payload_eq =
                    self.emit_value_eq(&ty.with_nullable(false), l_pay, r_pay, span)?;
                let value_eq = self
                    .cx
                    .builder
                    .build_and(both_some, payload_eq, "val.eq")?;
                Ok(self
                    .cx
                    .builder
                    .build_or(both_null, value_eq, "nullable.eq")?)
            }
            None => Err(not_yet_supported(
                &format!("nullable equality for `{ty}`"),
                span,
            )),
        }
    }
}
