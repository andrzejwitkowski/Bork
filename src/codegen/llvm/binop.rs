use inkwell::values::{BasicValueEnum, IntValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::diag::Diagnostic;
use crate::hir::{HirExpr, Prim, Ty};

use super::context::is_unsigned;
use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn cast(
        &self,
        value: IntValue<'ctx>,
        from: &Ty,
        to: &Ty,
        expr: &HirExpr,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let target = self
            .cx
            .int_type(to)
            .ok_or_else(|| not_yet_supported(&format!("type `{to}`"), expr.span))?;
        if value.get_type() == target {
            return Ok(value);
        }
        Ok(self
            .cx
            .builder
            .build_int_cast_sign_flag(value, target, !is_unsigned(from), "cast")?)
    }

    fn guard_int_div(
        &mut self,
        l: IntValue<'ctx>,
        r: IntValue<'ctx>,
        ty: &Ty,
    ) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        let int_ty = l.get_type();
        let zero = int_ty.const_int(0, false);
        let div_by_zero = builder.build_int_compare(IntPredicate::EQ, r, zero, "div0")?;
        let illegal = if is_unsigned(ty) {
            div_by_zero
        } else {
            let bits = int_ty.get_bit_width();
            let min = int_ty.const_int(1u64 << (bits - 1), true);
            let neg_one = int_ty.const_all_ones();
            let is_min = builder.build_int_compare(IntPredicate::EQ, l, min, "lmin")?;
            let is_neg1 = builder.build_int_compare(IntPredicate::EQ, r, neg_one, "rneg1")?;
            let min_neg1 = builder.build_and(is_min, is_neg1, "min_neg1")?;
            builder.build_or(div_by_zero, min_neg1, "div_bad")?
        };
        let ok_bb = cx.context.append_basic_block(self.llvm_fn, "div.ok");
        let bad_bb = cx.context.append_basic_block(self.llvm_fn, "div.bad");
        builder.build_conditional_branch(illegal, bad_bb, ok_bb)?;
        builder.position_at_end(bad_bb);
        builder.build_call(cx.abort_function(), &[], "abort")?;
        builder.build_unreachable()?;
        builder.position_at_end(ok_bb);
        Ok(())
    }

    /// Comparison operands are converted to whichever side has more bits.
    fn wider<'t>(&self, lhs: &'t Ty, rhs: &'t Ty) -> &'t Ty {
        let width = |ty: &Ty| self.cx.int_type(ty).map_or(0, |ty| ty.get_bit_width());
        if width(rhs) > width(lhs) {
            rhs
        } else {
            lhs
        }
    }

    pub(super) fn combine_binary_values(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if matches!(
            expr.ty.kind,
            crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)
        ) {
            return self.combine_float_binary_values(op, lhs_val, rhs_val, expr);
        }
        if let Some(predicate) = comparison(op) {
            let operand_ty = self.wider(&lhs.ty, &rhs.ty);
            let l = self.value_as_int(lhs_val, &operand_ty, expr.span)?;
            let r = self.value_as_int(rhs_val, &operand_ty, expr.span)?;
            let unsigned = is_unsigned(&operand_ty);
            let predicate = if unsigned { predicate.1 } else { predicate.0 };
            return Ok(self
                .cx
                .builder
                .build_int_compare(predicate, l, r, "cmp")?
                .into());
        }
        let l = self.value_as_int(lhs_val, &expr.ty, expr.span)?;
        let r = self.value_as_int(rhs_val, &expr.ty, expr.span)?;
        let builder = &self.cx.builder;
        Ok(match op {
            BinOp::Add => builder.build_int_add(l, r, "add")?,
            BinOp::Sub => builder.build_int_sub(l, r, "sub")?,
            BinOp::Mul => builder.build_int_mul(l, r, "mul")?,
            BinOp::Div if is_unsigned(&expr.ty) => {
                self.guard_int_div(l, r, &expr.ty)?;
                builder.build_int_unsigned_div(l, r, "div")?
            }
            BinOp::Div => {
                self.guard_int_div(l, r, &expr.ty)?;
                builder.build_int_signed_div(l, r, "div")?
            }
            _ => return Err(not_yet_supported("this binary operator", expr.span)),
        }
        .into())
    }

    pub(super) fn value_as_int(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        if !value.is_int_value() {
            return Err(not_yet_supported("integer operand", span));
        }
        let int = value.into_int_value();
        let target = self
            .cx
            .int_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{ty}`"), span))?;
        if int.get_type() == target {
            return Ok(int);
        }
        Ok(self
            .cx
            .builder
            .build_int_cast_sign_flag(int, target, !is_unsigned(ty), "cast")?)
    }

    fn combine_float_binary_values(
        &mut self,
        op: &BinOp,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let l = self.value_as_float(lhs_val, &expr.ty, expr.span)?;
        let r = self.value_as_float(rhs_val, &expr.ty, expr.span)?;
        let builder = &self.cx.builder;
        Ok(match op {
            BinOp::Add => builder.build_float_add(l, r, "fadd")?.into(),
            BinOp::Sub => builder.build_float_sub(l, r, "fsub")?.into(),
            BinOp::Mul => builder.build_float_mul(l, r, "fmul")?.into(),
            BinOp::Div => builder.build_float_div(l, r, "fdiv")?.into(),
            BinOp::Gt => builder
                .build_float_compare(inkwell::FloatPredicate::OGT, l, r, "fcmp")?
                .into(),
            BinOp::Lt => builder
                .build_float_compare(inkwell::FloatPredicate::OLT, l, r, "fcmp")?
                .into(),
            BinOp::Ge => builder
                .build_float_compare(inkwell::FloatPredicate::OGE, l, r, "fcmp")?
                .into(),
            BinOp::Le => builder
                .build_float_compare(inkwell::FloatPredicate::OLE, l, r, "fcmp")?
                .into(),
            BinOp::Eq => builder
                .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "fcmp")?
                .into(),
            BinOp::Ne => builder
                .build_float_compare(inkwell::FloatPredicate::ONE, l, r, "fcmp")?
                .into(),
            _ => return Err(not_yet_supported("this operator", expr.span)),
        })
    }

    pub(super) fn value_as_float(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<inkwell::values::FloatValue<'ctx>, Diagnostic> {
        let target = self
            .cx
            .basic_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{ty}`"), span))?
            .into_float_type();
        if !value.is_float_value() {
            return Err(not_yet_supported("float operand", span));
        }
        let float = value.into_float_value();
        if float.get_type() == target {
            return Ok(float);
        }
        Ok(self.cx.builder.build_float_cast(float, target, "cast")?)
    }

    pub(super) fn emit_logical_from_lhs(
        &mut self,
        op: &BinOp,
        lhs_val: IntValue<'ctx>,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let context = cx.context;
        let builder = &cx.builder;
        let i1 = context.bool_type();
        let rhs_bb = context.append_basic_block(self.llvm_fn, "log.rhs");
        let merge_bb = context.append_basic_block(self.llvm_fn, "log.end");
        let short_bb = context.append_basic_block(self.llvm_fn, "log.short");
        let zero = i1.const_int(0, false);
        let one = i1.const_int(1, false);
        let lhs_true = builder.build_int_compare(IntPredicate::NE, lhs_val, zero, "lhs.t")?;
        match op {
            BinOp::And => {
                builder.build_conditional_branch(lhs_true, rhs_bb, short_bb)?;
                builder.position_at_end(rhs_bb);
                let rhs_val = self.emit_bool(rhs)?;
                let rhs_end = builder
                    .get_insert_block()
                    .expect("rhs emission positions builder");
                builder.build_unconditional_branch(merge_bb)?;
                builder.position_at_end(short_bb);
                builder.build_unconditional_branch(merge_bb)?;
                builder.position_at_end(merge_bb);
                let phi = builder.build_phi(i1, "log")?;
                phi.add_incoming(&[
                    (&rhs_val as &dyn inkwell::values::BasicValue, rhs_end),
                    (&zero as &dyn inkwell::values::BasicValue, short_bb),
                ]);
                Ok(phi.as_basic_value())
            }
            BinOp::Or => {
                builder.build_conditional_branch(lhs_true, short_bb, rhs_bb)?;
                builder.position_at_end(short_bb);
                builder.build_unconditional_branch(merge_bb)?;
                builder.position_at_end(rhs_bb);
                let rhs_val = self.emit_bool(rhs)?;
                let rhs_end = builder
                    .get_insert_block()
                    .expect("rhs emission positions builder");
                builder.build_unconditional_branch(merge_bb)?;
                builder.position_at_end(merge_bb);
                let phi = builder.build_phi(i1, "log")?;
                phi.add_incoming(&[
                    (&one as &dyn inkwell::values::BasicValue, short_bb),
                    (&rhs_val as &dyn inkwell::values::BasicValue, rhs_end),
                ]);
                Ok(phi.as_basic_value())
            }
            _ => Err(not_yet_supported("logical operator", expr.span)),
        }
    }
}

/// Signed and unsigned predicates for a comparison operator.
fn comparison(op: &BinOp) -> Option<(IntPredicate, IntPredicate)> {
    Some(match op {
        BinOp::Gt => (IntPredicate::SGT, IntPredicate::UGT),
        BinOp::Lt => (IntPredicate::SLT, IntPredicate::ULT),
        BinOp::Ge => (IntPredicate::SGE, IntPredicate::UGE),
        BinOp::Le => (IntPredicate::SLE, IntPredicate::ULE),
        BinOp::Eq => (IntPredicate::EQ, IntPredicate::EQ),
        BinOp::Ne => (IntPredicate::NE, IntPredicate::NE),
        _ => return None,
    })
}
