use std::iter;

use inkwell::basic_block::BasicBlock;
use inkwell::module::Linkage;
use inkwell::types::BasicType;
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, IntValue, StructValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::builtins;
use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::hir::{HirBlock, HirExpr, HirExprKind, Prim, Ty, UseKind};

use super::context::is_unsigned;
use super::emit_fn::FnEmitter;
use super::{codegen_error, not_yet_supported};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub fn emit_expr(
        &mut self,
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
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
            HirExprKind::ArrayLit { elements } => Ok(Some(
                self.emit_array_lit(elements, &expr.ty)?.into(),
            )),
            HirExprKind::Index { receiver, index, .. } => {
                let elem = expr.ty.clone();
                self.emit_index_load(receiver, index, &elem).map(Some)
            }
            HirExprKind::Slice { receiver, lo, .. } => {
                Ok(Some(self.emit_slice(receiver, lo, &expr.ty)?.into()))
            }
            HirExprKind::Field { receiver, name, .. } if name == "length" => self
                .emit_buffer_length(receiver)
                .map(|v| Some(v.into())),
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
                if matches!(*use_kind, UseKind::Move | UseKind::Promote) && value.is_struct_value() {
                    let copied = if slot_ty.is_array() {
                        self.copy_array_into_arena(value.into_struct_value(), &slot_ty)?
                    } else {
                        self.copy_into_arena(value.into_struct_value())?
                    };
                    Ok(Some(copied.into()))
                } else {
                    // Shared/Local/Copy: load the slot (string descriptors copy by value).
                    Ok(Some(value))
                }
            }
            HirExprKind::Unary { op, expr: inner } => match op {
                crate::ast::UnaryOp::Borrow => self.emit_expr(inner),
                crate::ast::UnaryOp::Not => {
                    let value = self.emit_bool(inner)?;
                    let one = self.cx.context.bool_type().const_int(1, false);
                    Ok(Some(
                        self.cx
                            .builder
                            .build_xor(value, one, "not")?
                            .into(),
                    ))
                }
                crate::ast::UnaryOp::NotNullAssert => {
                    Err(not_yet_supported("`!!`", expr.span))
                }
            },
            HirExprKind::Binary { op, lhs, rhs } => self
                .emit_binary(op, lhs, rhs, expr)
                .map(Some),
            HirExprKind::Call { callee, args, .. } => self.emit_call(callee, args, expr),
            HirExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.emit_if_with_driver(cond, then_block, else_block.as_ref(), Some(&expr.ty))?;
                Ok(self.walk.trailing.take())
            }
            _ => Err(not_yet_supported("this expression", expr.span)),
        }
    }

    pub fn emit_value(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if ty.uses_arena_storage() {
            return self
                .emit_expr(expr)?
                .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span));
        }
        if ty == &Ty::bool() {
            return self.emit_bool(expr).map(Into::into);
        }
        if matches!(ty.kind, crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)) {
            return self.emit_float(expr, ty).map(Into::into);
        }
        self.emit_int(expr, ty).map(Into::into)
    }

    /// LLVM `i1` condition or value for a `bool` expression.
    pub fn emit_bool(&mut self, expr: &HirExpr) -> Result<IntValue<'ctx>, Diagnostic> {
        let value = if self.walk_driver_active() {
            let ptr = self.codegen_driver_ptr();
            super::emit_fn::FnEmitter::codegen_driver_mut(ptr)
                .walk_expr(self, expr)
                .map_err(super::region_walk_codegen_error)?;
            self.walk.trailing
                .take()
                .ok_or_else(|| not_yet_supported("bool condition missing value", expr.span))?
        } else {
            self.emit_expr(expr)?
                .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span))?
        };
        if !value.is_int_value() {
            return Err(not_yet_supported(
                &format!("`{}` in bool context", expr.ty),
                expr.span,
            ));
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

    pub fn emit_float(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<inkwell::values::FloatValue<'ctx>, Diagnostic> {
        let value = self
            .emit_expr(expr)?
            .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span))?;
        let target = self
            .cx
            .basic_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{ty}`"), expr.span))?
            .into_float_type();
        if value.is_float_value() {
            let float = value.into_float_value();
            if float.get_type() == target {
                return Ok(float);
            }
            return Ok(self
                .cx
                .builder
                .build_float_cast(float, target, "cast")?);
        }
        Err(not_yet_supported(
            &format!("`{}` in this position", expr.ty),
            expr.span,
        ))
    }

    pub fn emit_int(&mut self, expr: &HirExpr, ty: &Ty) -> Result<IntValue<'ctx>, Diagnostic> {
        let value = self
            .emit_expr(expr)?
            .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span))?;
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

    fn emit_binary(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if matches!(op, BinOp::And | BinOp::Or) {
            return self.emit_logical(op, lhs, rhs, expr);
        }
        if matches!(expr.ty.kind, crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)) {
            return self.emit_float_binary(op, lhs, rhs, expr);
        }
        let cx = self.cx;
        let builder = &cx.builder;
        if let Some(predicate) = comparison(op) {
            let operand_ty = self.wider(&lhs.ty, &rhs.ty).clone();
            let unsigned = is_unsigned(&operand_ty);
            let l = self.emit_int(lhs, &operand_ty)?;
            let r = self.emit_int(rhs, &operand_ty)?;
            let predicate = if unsigned { predicate.1 } else { predicate.0 };
            return Ok(builder.build_int_compare(predicate, l, r, "cmp")?.into());
        }

        let l = self.emit_int(lhs, &expr.ty)?;
        let r = self.emit_int(rhs, &expr.ty)?;
        let value = match op {
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
            _ => return Err(not_yet_supported("this operator", expr.span)),
        };
        Ok(value.into())
    }

    fn emit_float_binary(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        let l = self.emit_float(lhs, &expr.ty)?;
        let r = self.emit_float(rhs, &expr.ty)?;
        let value = match op {
            BinOp::Add => builder.build_float_add(l, r, "fadd")?,
            BinOp::Sub => builder.build_float_sub(l, r, "fsub")?,
            BinOp::Mul => builder.build_float_mul(l, r, "fmul")?,
            BinOp::Div => builder.build_float_div(l, r, "fdiv")?,
            BinOp::Gt => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::OGT, l, r, "fcmp")?
                .into()),
            BinOp::Lt => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::OLT, l, r, "fcmp")?
                .into()),
            BinOp::Ge => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::OGE, l, r, "fcmp")?
                .into()),
            BinOp::Le => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::OLE, l, r, "fcmp")?
                .into()),
            BinOp::Eq => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "fcmp")?
                .into()),
            BinOp::Ne => return Ok(builder
                .build_float_compare(inkwell::FloatPredicate::ONE, l, r, "fcmp")?
                .into()),
            _ => return Err(not_yet_supported("this operator", expr.span)),
        };
        Ok(value.into())
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

    fn emit_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let HirExprKind::Ident { name, .. } = &callee.kind else {
            return Err(not_yet_supported("indirect calls", callee.span));
        };
        let callees = self.callees;
        let Some(&(target, function)) = callees.get(name.as_str()) else {
            match (builtins::resolve(name), args) {
                (Some(builtins::Builtin::Print), [arg]) => {
                    return self.emit_print(arg, false).map(|()| None);
                }
                (Some(builtins::Builtin::Println), [arg]) => {
                    return self.emit_print(arg, true).map(|()| None);
                }
                (Some(builtins::Builtin::Concat), _) => {
                    return self.emit_concat(args, expr).map(Some);
                }
                _ => {}
            }
            return Err(not_yet_supported(
                &format!("calls to `{name}`"),
                expr.span.or(callee.span),
            ));
        };
        let args = self.without_alloc_sink(|emitter| {
            args.iter()
                .zip(&function.params)
                .map(|(arg, param)| {
                    emitter
                        .emit_value(arg, &param.ty)
                        .map(BasicMetadataValueEnum::from)
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        let call = self.cx.builder.build_call(target, &args, "call")?;
        Ok(call.try_as_basic_value().basic())
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
                    return self
                        .emit_concat_values(*left, *right, expr)
                        .map(Some);
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
        let offset = unsafe {
            builder.build_gep(
                cx.context.i8_type(),
                dst,
                &[left_len],
                "concat.r.off",
            )?
        };
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

    fn emit_concat(
        &mut self,
        args: &[HirExpr],
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (left, right) = match args {
            [left, right] => (left, right),
            _ => {
                return Err(not_yet_supported(
                    "`concat` expects two `String` arguments",
                    expr.span,
                ));
            }
        };
        let (left_val, right_val) = self.without_alloc_sink(|emitter| {
            Ok((
                emitter
                    .emit_value(left, &left.ty)?
                    .into_struct_value(),
                emitter
                    .emit_value(right, &right.ty)?
                    .into_struct_value(),
            ))
        })?;
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
        let offset = unsafe {
            builder.build_gep(
                cx.context.i8_type(),
                dst,
                &[left_len],
                "concat.r.off",
            )?
        };
        builder
            .build_memcpy(offset, 1, right_ptr, 1, right_len)
            .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let out = cx.string_type().const_named_struct(&[]);
        let out = builder.build_insert_value(out, dst, 0, "concat.out")?;
        let out = builder.build_insert_value(out, total, 1, "concat.out")?;
        Ok(out.into_struct_value().into())
    }

    /// Strings go to `bork_print*_str` as bytes + length; integers are widened to `i64`.
    fn emit_print(&mut self, arg: &HirExpr, newline: bool) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        if arg.ty.is_string() {
            let value = self.emit_value(arg, &arg.ty)?.into_struct_value();
            let bytes = builder.build_extract_value(value, 0, "print.ptr")?;
            let len = builder.build_extract_value(value, 1, "print.len")?;
            builder.build_call(cx.print_str_fn(newline), &[bytes.into(), len.into()], "")?;
        } else {
            let value = self.emit_int(arg, &Ty::prim(Prim::I64))?;
            builder.build_call(cx.print_i64_fn(newline), &[value.into()], "")?;
        }
        Ok(())
    }

    pub(super) fn emit_if_with_driver(
        &mut self,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        ty: Option<&Ty>,
    ) -> Result<(), Diagnostic> {
        let result_ty = ty.and_then(|ty| self.cx.basic_type(ty));
        let value_ty = result_ty.is_some().then(|| ty.unwrap());
        let cond = self.emit_bool(cond)?;

        let context = self.cx.context;
        let then_bb = context.append_basic_block(self.llvm_fn, "then");
        let else_bb = else_block.map(|_| context.append_basic_block(self.llvm_fn, "else"));
        let merge_bb = context.append_basic_block(self.llvm_fn, "endif");
        self.cx
            .builder
            .build_conditional_branch(cond, then_bb, else_bb.unwrap_or(merge_bb))?;

        let branches = iter::once((RegionSite::IfThen, then_block, then_bb)).chain(
            else_block
                .zip(else_bb)
                .map(|(block, bb)| (RegionSite::IfElse, block, bb)),
        );
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (site, block, bb) in branches {
            self.cx.builder.position_at_end(bb);
            let value = self
                .emit_region_with_driver(site, block, value_ty)
                .map_err(super::region_walk_codegen_error)?;
            if self.cx.current_block_terminated() {
                continue;
            }
            let end = self
                .cx
                .builder
                .get_insert_block()
                .expect("builder is positioned");
            self.cx.builder.build_unconditional_branch(merge_bb)?;
            if value_ty.is_some() {
                let value = value.ok_or_else(|| {
                    not_yet_supported("an `if` branch without a trailing value", None)
                })?;
                incoming.push((value, end));
            }
        }
        self.cx.builder.position_at_end(merge_bb);

        if let Some(result_ty) = result_ty {
            if !incoming.is_empty() {
                let phi = self
                    .cx
                    .builder
                    .build_phi(result_ty.as_basic_type_enum(), "if")?;
                let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
                    .collect();
                phi.add_incoming(&incoming);
                self.walk.trailing = Some(phi.as_basic_value());
            } else if ty.is_some() {
                self.walk.trailing = Some(result_ty.const_zero());
            }
        }
        Ok(())
    }

    pub(super) fn emit_after_walk(
        &mut self,
        expr: &HirExpr,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        if !self.walk_driver_active() {
            return Ok(self.emit_expr(expr)?);
        }
        match &expr.kind {
            HirExprKind::Binary { op, rhs, .. }
                if matches!(op, BinOp::And | BinOp::Or) =>
            {
                let lhs_val = self.pop_walk_operand(expr.span)?;
                let lhs_bool = self.value_as_bool(lhs_val, expr.span)?;
                return self
                    .emit_logical_with_lhs(op, lhs_bool, rhs, expr)
                    .map(Some);
            }
            HirExprKind::Binary { op, lhs, rhs, .. } => {
                let rhs_val = self.pop_walk_operand(expr.span)?;
                let lhs_val = self.pop_walk_operand(expr.span)?;
                return self
                    .combine_binary_values(op, lhs, rhs, lhs_val, rhs_val, expr)
                    .map(Some);
            }
            HirExprKind::Unary {
                op: crate::ast::UnaryOp::Borrow,
                ..
            } => {
                return Ok(Some(self.pop_walk_operand(expr.span)?));
            }
            HirExprKind::Unary {
                op: crate::ast::UnaryOp::Not,
                ..
            } => {
                let inner = self.pop_walk_operand(expr.span)?;
                let value = self.value_as_bool(inner, expr.span)?;
                let one = self.cx.context.bool_type().const_int(1, false);
                return Ok(Some(
                    self.cx
                        .builder
                        .build_xor(value, one, "not")?
                        .into(),
                ));
            }
            HirExprKind::Call { callee, args, .. } => {
                let mut arg_values = Vec::with_capacity(args.len());
                for _ in args {
                    arg_values.push(self.pop_walk_operand(expr.span)?);
                }
                arg_values.reverse();
                return self.emit_call_with_values(callee, &arg_values, expr);
            }
            HirExprKind::ArrayLit { elements, .. } => {
                let mut values = Vec::with_capacity(elements.len());
                for _ in elements {
                    values.push(self.pop_walk_operand(expr.span)?);
                }
                values.reverse();
                let elem_ty = expr
                    .ty
                    .array_elem()
                    .ok_or_else(|| not_yet_supported("array literal type", expr.span))?;
                return Ok(Some(
                    self.emit_array_lit_values(&values, elem_ty, &expr.ty)?
                        .into(),
                ));
            }
            HirExprKind::Index { .. } => {
                let index = self.pop_walk_operand(expr.span)?;
                let receiver = self.pop_walk_operand(expr.span)?;
                let elem = expr.ty.clone();
                return self
                    .emit_index_load_values(receiver, index, &elem, expr.span)
                    .map(Some);
            }
            HirExprKind::Slice { .. } => {
                let _hi = self.pop_walk_operand(expr.span)?;
                let lo = self.pop_walk_operand(expr.span)?;
                let receiver = self.pop_walk_operand(expr.span)?;
                return Ok(Some(
                    self.emit_slice_values(receiver, lo, &expr.ty, expr.span)?
                        .into(),
                ));
            }
            HirExprKind::Field { name, .. } if name == "length" => {
                let receiver = self.pop_walk_operand(expr.span)?;
                return Ok(Some(
                    self.emit_buffer_length_value(receiver.into_struct_value())?
                        .into(),
                ));
            }
            _ => Ok(self.emit_expr(expr)?),
        }
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
        if matches!(expr.ty.kind, crate::hir::TyKind::Prim(Prim::F32 | Prim::F64)) {
            return Err(not_yet_supported("float binary after walk", expr.span));
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

    fn value_as_int(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let int = value
            .into_int_value();
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

    fn emit_logical_with_lhs(
        &mut self,
        op: &BinOp,
        lhs_val: IntValue<'ctx>,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        self.emit_logical_from_lhs(op, lhs_val, rhs, expr)
    }

    fn emit_logical(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let lhs_val = self.emit_bool(lhs)?;
        self.emit_logical_from_lhs(op, lhs_val, rhs, expr)
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
