use inkwell::module::Linkage;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, IntValue, StructValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::builtins;
use crate::diag::Diagnostic;
use crate::hir::{HirBlock, HirExpr, HirExprKind, Prim, Ty, UseKind};

use super::context::is_unsigned;
use super::emit_fn::FnEmitter;
use super::region_emit::IfKind;
use super::{codegen_error, not_yet_supported};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub fn emit_expr(
        &mut self,
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        match child_plan(expr) {
            ChildPlan::If {
                cond,
                then_block,
                else_block,
            } => {
                self.emit_if_with_driver(cond, then_block, else_block, IfKind::from_ty(&expr.ty))?;
                Ok(self.walk.trailing.take())
            }
            plan => {
                let ops = if call_gathers_without_sink(expr) {
                    self.without_alloc_sink(|e| e.emit_plan(plan))?
                } else {
                    self.emit_plan(plan)?
                };
                self.combine_expr(expr, &ops)
            }
        }
    }

    pub fn emit_value(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if ty.is_nullable() || ty.uses_arena_storage() {
            return self.require_value(expr);
        }
        if ty == &Ty::bool() {
            return self.emit_bool(expr).map(Into::into);
        }
        if matches!(ty.kind, crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)) {
            return self.emit_float(expr, ty).map(Into::into);
        }
        self.emit_int(expr, ty).map(Into::into)
    }

    pub fn emit_bool(&mut self, expr: &HirExpr) -> Result<IntValue<'ctx>, Diagnostic> {
        let value = self.emit_skipped(expr)?;
        self.value_as_bool(value, expr.span)
    }

    fn require_value(&mut self, expr: &HirExpr) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        self.emit_expr(expr)?
            .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span))
    }

    fn emit_skipped(&mut self, expr: &HirExpr) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.walk_driver_active() {
            let ptr = self.codegen_driver_ptr();
            super::emit_fn::FnEmitter::codegen_driver_mut(ptr)
                .walk_expr(self, expr)
                .map_err(super::region_walk_codegen_error)?;
            self.walk
                .trailing
                .take()
                .ok_or_else(|| not_yet_supported("skipped operand missing value", expr.span))
        } else {
            self.require_value(expr)
        }
    }

    pub(super) fn emit_skipped_as(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.walk_driver_active() {
            self.emit_skipped(expr)
        } else {
            self.emit_value(expr, ty)
        }
    }

    pub fn emit_float(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<inkwell::values::FloatValue<'ctx>, Diagnostic> {
        let value = self.require_value(expr)?;
        self.value_as_float(value, ty, expr.span)
    }

    pub fn emit_int(&mut self, expr: &HirExpr, ty: &Ty) -> Result<IntValue<'ctx>, Diagnostic> {
        let value = self.require_value(expr)?;
        if !value.is_int_value() {
            return Err(not_yet_supported(
                &format!("`{}` in this position", expr.ty),
                expr.span,
            ));
        }
        self.cast(value.into_int_value(), &expr.ty, ty, expr)
    }

    /// Literal bytes live in a private constant; the descriptor borrows them.
    fn emit_str_literal(&self, value: &str) -> Result<StructValue<'ctx>, Diagnostic> {
        let context = self.cx.context;
        let bytes = context.const_string(value.as_bytes(), false);
        let global = self.cx.module.add_global(bytes.get_type(), None, "str");
        global.set_initializer(&bytes);
        global.set_constant(true);
        global.set_linkage(Linkage::Private);
        global.set_unnamed_addr(true);
        let len = context.i64_type().const_int(value.len() as u64, false);
        Ok(self
            .cx
            .string_type()
            .const_named_struct(&[global.as_pointer_value().into(), len.into()]))
    }

    fn emit_plan(&mut self, plan: ChildPlan<'_>) -> Result<Vec<BasicValueEnum<'ctx>>, Diagnostic> {
        Ok(match plan {
            ChildPlan::If { .. } => unreachable!("If is not combined"),
            ChildPlan::Zero => Vec::new(),
            ChildPlan::One(a) => vec![self.require_value(a)?],
            ChildPlan::Two(a, b) => vec![self.require_value(a)?, self.require_value(b)?],
            ChildPlan::Three(a, b, c) => vec![
                self.require_value(a)?,
                self.require_value(b)?,
                self.require_value(c)?,
            ],
            ChildPlan::Many(children) => {
                let mut ops = Vec::with_capacity(children.len());
                for child in children {
                    ops.push(self.require_value(child)?);
                }
                ops
            }
        })
    }

    fn combine_expr(
        &mut self,
        expr: &HirExpr,
        ops: &[BasicValueEnum<'ctx>],
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        debug_assert_eq!(ops.len(), child_plan(expr).len());
        match &expr.kind {
            HirExprKind::Int { value } => {
                let ty = self
                    .cx
                    .int_type(&expr.ty)
                    .ok_or_else(|| not_yet_supported(&format!("type `{}`", expr.ty), expr.span))?;
                Ok(Some(ty.const_int(*value as u64, true).into()))
            }
            HirExprKind::Bool { value } => Ok(Some(
                self.cx
                    .context
                    .bool_type()
                    .const_int(*value as u64, false)
                    .into(),
            )),
            HirExprKind::Float { value } => {
                let ty = self
                    .cx
                    .basic_type(&expr.ty)
                    .ok_or_else(|| not_yet_supported(&format!("type `{}`", expr.ty), expr.span))?
                    .into_float_type();
                Ok(Some(ty.const_float(*value).into()))
            }
            HirExprKind::Str { value } => Ok(Some(self.emit_str_literal(value)?.into())),
            HirExprKind::None => self.cx.const_null_value(&expr.ty).map(Some),
            HirExprKind::Ident { name, use_kind } => {
                let slot = self
                    .lookup(name)
                    .ok_or_else(|| not_yet_supported(&format!("`{name}` as a value"), expr.span))?;
                let slot_ty = slot.ty.clone();
                let slot_ptr = slot.ptr;
                let ty = self
                    .cx
                    .basic_type(&slot_ty)
                    .expect("locals only hold lowerable types");
                let value = self.cx.builder.build_load(ty, slot_ptr, name)?;
                if matches!(*use_kind, UseKind::Move | UseKind::Promote) && value.is_struct_value()
                {
                    let copied = if slot_ty.is_array() {
                        self.copy_array_into_arena(value.into_struct_value(), &slot_ty)?
                    } else {
                        self.copy_into_arena(value.into_struct_value())?
                    };
                    Ok(Some(copied.into()))
                } else {
                    Ok(Some(value))
                }
            }
            HirExprKind::Some(_) => self.emit_nullable_some(&expr.ty, ops[0]).map(Some),
            HirExprKind::Unary { op, expr: inner } => match op {
                crate::ast::UnaryOp::Borrow => Ok(Some(ops[0])),
                crate::ast::UnaryOp::Not => {
                    let value = self.value_as_bool(ops[0], expr.span)?;
                    let one = self.cx.context.bool_type().const_int(1, false);
                    Ok(Some(self.cx.builder.build_xor(value, one, "not")?.into()))
                }
                crate::ast::UnaryOp::NotNullAssert => self
                    .emit_nullable_expect_non_null(&inner.ty, ops[0])
                    .map(Some),
            },
            HirExprKind::Binary { op, lhs, rhs } => match op {
                BinOp::Elvis => self.emit_elvis(lhs, rhs, ops[0], expr).map(Some),
                BinOp::And | BinOp::Or => {
                    let lhs_bool = self.value_as_bool(ops[0], expr.span)?;
                    self.emit_logical_from_lhs(op, lhs_bool, rhs, expr)
                        .map(Some)
                }
                BinOp::Eq | BinOp::Ne if is_nullable_equality(op, lhs, rhs) => self
                    .emit_nullable_eq(op, lhs, rhs, ops[0], ops[1], expr)
                    .map(Some),
                _ => self
                    .combine_binary_values(op, lhs, rhs, ops[0], ops[1], expr)
                    .map(Some),
            },
            HirExprKind::Call { callee, .. } => self.emit_call_with_values(callee, ops, expr),
            HirExprKind::ArrayLit { .. } => {
                let elem_ty = expr
                    .ty
                    .array_elem()
                    .ok_or_else(|| not_yet_supported("array literal type", expr.span))?;
                Ok(Some(
                    self.emit_array_lit_values(ops, elem_ty, &expr.ty)?.into(),
                ))
            }
            HirExprKind::Index { .. } => self
                .emit_index_load_values(ops[0], ops[1], &expr.ty, expr.span)
                .map(Some),
            HirExprKind::Slice { .. } => Ok(Some(
                self.emit_slice_values(ops[0], ops[1], &expr.ty, expr.span)?
                    .into(),
            )),
            HirExprKind::Field {
                receiver,
                name,
                safe,
                ..
            } => {
                if name != "length" {
                    return Err(not_yet_supported("field access", expr.span));
                }
                if *safe && receiver.ty.is_nullable() {
                    return self
                        .emit_safe_nullable_length(&receiver.ty, ops[0], &expr.ty, expr.span)
                        .map(Some);
                }
                Ok(Some(
                    self.emit_buffer_length_value(ops[0].into_struct_value())?
                        .into(),
                ))
            }
            HirExprKind::If { .. } => unreachable!("If is ChildPlan::If"),
        }
    }

    /// `move` of a string copies its bytes into the current sink arena.
    pub(super) fn copy_into_arena(
        &mut self,
        source: StructValue<'ctx>,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let arena = self.sink_arena();
        let cx = self.cx;
        let builder = &cx.builder;
        let src = builder
            .build_extract_value(source, 0, "move.src")?
            .into_pointer_value();
        let len = builder
            .build_extract_value(source, 1, "move.len")?
            .into_int_value();
        let one = cx.context.i64_type().const_int(1, false);
        let dst = builder
            .build_call(
                cx.arena_alloc_fn(),
                &[arena.into(), len.into(), one.into()],
                "move.dst",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value();
        builder
            .build_memcpy(dst, 1, src, 1, len)
            .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let moved = builder.build_insert_value(source, dst, 0, "moved")?;
        Ok(moved.into_struct_value())
    }

    fn cast(
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

    pub(in crate::codegen::llvm) fn emit_call_with_values(
        &mut self,
        callee: &HirExpr,
        arg_values: &[BasicValueEnum<'ctx>],
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let HirExprKind::Ident { name, .. } = &callee.kind else {
            return Err(not_yet_supported("indirect calls", callee.span));
        };
        let callees = self.callees;
        let Some(&(target, function)) = callees.get(name.as_str()) else {
            match (builtins::resolve(name), arg_values) {
                (Some(builtins::Builtin::Print), [arg]) => {
                    return self.emit_print_value(*arg, false).map(|()| None);
                }
                (Some(builtins::Builtin::Println), [arg]) => {
                    return self.emit_print_value(*arg, true).map(|()| None);
                }
                (Some(builtins::Builtin::Concat), [left, right]) => {
                    return self.emit_concat_values(*left, *right, expr).map(Some);
                }
                _ => {}
            }
            return Err(not_yet_supported(
                &format!("calls to `{name}`"),
                expr.span.or(callee.span),
            ));
        };
        let args = arg_values
            .iter()
            .zip(&function.params)
            .map(|(value, param)| {
                self.coerce_value_to_ty(*value, &param.ty, expr.span)
                    .map(BasicMetadataValueEnum::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let call = self.cx.builder.build_call(target, &args, "call")?;
        Ok(call.try_as_basic_value().basic())
    }

    fn coerce_value_to_ty(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if ty == &Ty::bool() {
            return self.value_as_bool(value, span).map(Into::into);
        }
        if matches!(ty.kind, crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)) {
            return Err(not_yet_supported("float call argument after walk", span));
        }
        if ty.uses_arena_storage() {
            return if value.is_struct_value() {
                Ok(value)
            } else {
                Err(not_yet_supported("non-descriptor buffer argument", span))
            };
        }
        self.value_as_int(value, ty, span).map(Into::into)
    }

    fn emit_concat_values(
        &mut self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        _expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let left_val = left.into_struct_value();
        let right_val = right.into_struct_value();
        let cx = self.cx;
        let builder = &cx.builder;
        let left_ptr = builder
            .build_extract_value(left_val, 0, "concat.l.ptr")?
            .into_pointer_value();
        let left_len = builder
            .build_extract_value(left_val, 1, "concat.l.len")?
            .into_int_value();
        let right_ptr = builder
            .build_extract_value(right_val, 0, "concat.r.ptr")?
            .into_pointer_value();
        let right_len = builder
            .build_extract_value(right_val, 1, "concat.r.len")?
            .into_int_value();
        let total = builder.build_int_add(left_len, right_len, "concat.len")?;
        let arena = self.sink_arena();
        let one = cx.context.i64_type().const_int(1, false);
        let dst = builder
            .build_call(
                cx.arena_alloc_fn(),
                &[arena.into(), total.into(), one.into()],
                "concat.dst",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value();
        builder
            .build_memcpy(dst, 1, left_ptr, 1, left_len)
            .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let offset =
            unsafe { builder.build_gep(cx.context.i8_type(), dst, &[left_len], "concat.r.off")? };
        builder
            .build_memcpy(offset, 1, right_ptr, 1, right_len)
            .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let out = cx.string_type().const_named_struct(&[]);
        let out = builder.build_insert_value(out, dst, 0, "concat.out")?;
        let out = builder.build_insert_value(out, total, 1, "concat.out")?;
        Ok(out.into_struct_value().into())
    }

    fn emit_print_value(
        &mut self,
        arg: BasicValueEnum<'ctx>,
        newline: bool,
    ) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        if arg.is_struct_value() {
            let desc = arg.into_struct_value();
            let bytes = builder.build_extract_value(desc, 0, "print.ptr")?;
            let len = builder.build_extract_value(desc, 1, "print.len")?;
            builder.build_call(cx.print_str_fn(newline), &[bytes.into(), len.into()], "")?;
        } else {
            let int = arg.into_int_value();
            let i64_ty = cx.context.i64_type();
            let wide = if int.get_type().get_bit_width() < 64 {
                builder.build_int_s_extend(int, i64_ty, "print.wide")?
            } else {
                builder.build_int_truncate(int, i64_ty, "print.wide")?
            };
            builder.build_call(cx.print_i64_fn(newline), &[wide.into()], "")?;
        }
        Ok(())
    }

    pub(super) fn emit_after_walk(
        &mut self,
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        if !self.walk_driver_active() {
            return self.emit_expr(expr);
        }
        let n = child_plan(expr).len();
        let mut ops = Vec::with_capacity(n);
        for _ in 0..n {
            ops.push(self.pop_walk_operand(expr.span)?);
        }
        ops.reverse();
        self.combine_expr(expr, &ops)
    }

    fn pop_walk_operand(
        &mut self,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        self.walk
            .eval_stack
            .pop()
            .ok_or_else(|| not_yet_supported("region walk eval stack underflow", span))
    }

    fn value_as_bool(
        &mut self,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        if !value.is_int_value() {
            return Err(not_yet_supported("bool context", span));
        }
        let int = value.into_int_value();
        if int.get_type().get_bit_width() == 1 {
            return Ok(int);
        }
        let zero = self.cx.context.bool_type().const_int(0, false);
        Ok(self
            .cx
            .builder
            .build_int_compare(IntPredicate::NE, int, zero, "bool")?)
    }

    fn combine_binary_values(
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

    fn emit_logical_from_lhs(
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

fn is_nullable_equality(op: &BinOp, lhs: &HirExpr, rhs: &HirExpr) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne)
        && (lhs.ty.is_nullable()
            || rhs.ty.is_nullable()
            || matches!(lhs.kind, HirExprKind::None)
            || matches!(rhs.kind, HirExprKind::None))
}

#[derive(Clone, Copy, Debug)]
enum ChildPlan<'a> {
    Zero,
    If {
        cond: &'a HirExpr,
        then_block: &'a HirBlock,
        else_block: Option<&'a HirBlock>,
    },
    One(&'a HirExpr),
    Two(&'a HirExpr, &'a HirExpr),
    Three(&'a HirExpr, &'a HirExpr, &'a HirExpr),
    Many(&'a [HirExpr]),
}

impl ChildPlan<'_> {
    fn len(self) -> usize {
        match self {
            ChildPlan::If { .. } => unreachable!("If is not stacked"),
            ChildPlan::Zero => 0,
            ChildPlan::One(_) => 1,
            ChildPlan::Two(_, _) => 2,
            ChildPlan::Three(_, _, _) => 3,
            ChildPlan::Many(children) => children.len(),
        }
    }
}

fn child_plan(expr: &HirExpr) -> ChildPlan<'_> {
    match &expr.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => ChildPlan::Zero,
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => ChildPlan::If {
            cond,
            then_block,
            else_block: else_block.as_ref(),
        },
        HirExprKind::Some(inner)
        | HirExprKind::Unary { expr: inner, .. }
        | HirExprKind::Field {
            receiver: inner, ..
        } => ChildPlan::One(inner),
        HirExprKind::Binary { op, lhs, rhs } => {
            if matches!(op, BinOp::And | BinOp::Or | BinOp::Elvis) {
                ChildPlan::One(lhs)
            } else {
                ChildPlan::Two(lhs, rhs)
            }
        }
        HirExprKind::Index {
            receiver, index, ..
        } => ChildPlan::Two(receiver, index),
        HirExprKind::Slice { receiver, lo, hi } => ChildPlan::Three(receiver, lo, hi),
        HirExprKind::Call { args, .. } => ChildPlan::Many(args),
        HirExprKind::ArrayLit { elements } => ChildPlan::Many(elements),
    }
}

fn call_gathers_without_sink(expr: &HirExpr) -> bool {
    let HirExprKind::Call { callee, .. } = &expr.kind else {
        return false;
    };
    let HirExprKind::Ident { name, .. } = &callee.kind else {
        return false;
    };
    !builtins::is_print(name)
}

#[cfg(test)]
mod child_plan_tests {
    use super::{child_plan, ChildPlan};
    use crate::ast::BinOp;
    use crate::hir::{HirExprKind, HirStmt};

    fn return_expr(source: &str) -> crate::hir::HirProgram {
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        result.hir.expect("ok check has HIR")
    }

    fn main_return(program: &crate::hir::HirProgram) -> &crate::hir::HirExpr {
        match program.functions[0].body.stmts.last() {
            Some(HirStmt::Return { value: Some(expr) }) => expr,
            other => panic!("expected return value, got {other:?}"),
        }
    }

    #[test]
    fn child_plan_none_is_zero() {
        let program = return_expr("fun main(): String? { return None }\n");
        assert_eq!(child_plan(main_return(&program)).len(), 0);
    }

    #[test]
    fn child_plan_elvis_and_or_are_one() {
        let elvis = return_expr("fun main(s: String?): String { return s ?: \"x\" }\n");
        assert!(matches!(
            main_return(&elvis).kind,
            HirExprKind::Binary {
                op: BinOp::Elvis,
                ..
            }
        ));
        assert_eq!(child_plan(main_return(&elvis)).len(), 1);

        let and = return_expr("fun main(a: bool, b: bool): bool { return a && b }\n");
        assert_eq!(child_plan(main_return(&and)).len(), 1);

        let or = return_expr("fun main(a: bool, b: bool): bool { return a || b }\n");
        assert_eq!(child_plan(main_return(&or)).len(), 1);
    }

    #[test]
    fn child_plan_nullable_eq_is_two() {
        let program = return_expr("fun main(s: String?): bool { return s == None }\n");
        assert_eq!(child_plan(main_return(&program)).len(), 2);
    }

    #[test]
    fn child_plan_value_if_carries_blocks() {
        let program = return_expr("fun main(c: bool): i32 { return if (c) { 1 } else { 2 } }\n");
        let expr = main_return(&program);
        let plan = child_plan(expr);
        let ChildPlan::If {
            cond,
            then_block,
            else_block,
        } = plan
        else {
            panic!("expected ChildPlan::If, got {plan:?}");
        };
        assert!(matches!(cond.kind, HirExprKind::Ident { .. }));
        assert!(else_block.is_some());
        assert!(!then_block.stmts.is_empty());
    }
}
