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
            HirExprKind::Str { value } => Ok(Some(self.emit_str_literal(value)?.into())),
            HirExprKind::Ident { name, use_kind } => {
                let slot = self
                    .lookup(name)
                    .ok_or_else(|| not_yet_supported(&format!("`{name}` as a value"), expr.span))?;
                let ty = self
                    .cx
                    .basic_type(&slot.ty)
                    .expect("locals only hold lowerable types");
                let value = self.cx.builder.build_load(ty, slot.ptr, name)?;
                if *use_kind == UseKind::Move && value.is_struct_value() {
                    Ok(Some(self.copy_into_arena(value.into_struct_value())?.into()))
                } else {
                    // Shared/Local/Copy: load the slot (string descriptors copy by value).
                    Ok(Some(value))
                }
            }
            HirExprKind::Binary { op, lhs, rhs } => self
                .emit_binary(op, lhs, rhs, expr)
                .map(|value| Some(value.into())),
            HirExprKind::Call { callee, args, .. } => self.emit_call(callee, args, expr),
            HirExprKind::If {
                cond,
                then_block,
                else_block,
            } => self.emit_if(cond, then_block, else_block.as_ref(), &expr.ty),
            _ => Err(not_yet_supported("this expression", expr.span)),
        }
    }

    pub fn emit_value(
        &mut self,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if ty.is_string() {
            return self
                .emit_expr(expr)?
                .ok_or_else(|| not_yet_supported("a `unit` value here", expr.span));
        }
        self.emit_int(expr, ty).map(Into::into)
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

    /// `move` of a string copies its bytes into the innermost open arena.
    fn copy_into_arena(
        &mut self,
        source: StructValue<'ctx>,
    ) -> Result<StructValue<'ctx>, Diagnostic> {
        let arena = self.current_arena();
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
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        if let Some(predicate) = comparison(op) {
            let operand_ty = self.wider(&lhs.ty, &rhs.ty).clone();
            let unsigned = is_unsigned(&operand_ty);
            let l = self.emit_int(lhs, &operand_ty)?;
            let r = self.emit_int(rhs, &operand_ty)?;
            let predicate = if unsigned { predicate.1 } else { predicate.0 };
            return Ok(builder.build_int_compare(predicate, l, r, "cmp")?);
        }

        let l = self.emit_int(lhs, &expr.ty)?;
        let r = self.emit_int(rhs, &expr.ty)?;
        let value = match op {
            BinOp::Add => builder.build_int_add(l, r, "add")?,
            BinOp::Sub => builder.build_int_sub(l, r, "sub")?,
            BinOp::Mul => builder.build_int_mul(l, r, "mul")?,
            BinOp::Div if is_unsigned(&expr.ty) => builder.build_int_unsigned_div(l, r, "div")?,
            BinOp::Div => builder.build_int_signed_div(l, r, "div")?,
            _ => return Err(not_yet_supported("this operator", expr.span)),
        };
        Ok(value)
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
            if let ([arg], true) = (args, builtins::is_print(name)) {
                return self.emit_print(arg, name == "println").map(|()| None);
            }
            return Err(not_yet_supported(
                &format!("calls to `{name}`"),
                expr.span.or(callee.span),
            ));
        };
        let args = args
            .iter()
            .zip(&function.params)
            .map(|(arg, param)| {
                self.emit_value(arg, &param.ty)
                    .map(BasicMetadataValueEnum::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let call = self.cx.builder.build_call(target, &args, "call")?;
        Ok(call.try_as_basic_value().basic())
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

    fn emit_if(
        &mut self,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        ty: &Ty,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let result_ty = self.cx.basic_type(ty);
        let value_ty = result_ty.is_some().then_some(ty);
        let cond = self.emit_int(cond, &Ty::bool())?;

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
            let value = self.emit_region(site, block, value_ty)?;
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

        let Some(result_ty) = result_ty else {
            return Ok(None);
        };
        if incoming.is_empty() {
            return Ok(Some(result_ty.const_zero()));
        }
        let phi = self
            .cx
            .builder
            .build_phi(result_ty.as_basic_type_enum(), "if")?;
        let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
            .iter()
            .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
            .collect();
        phi.add_incoming(&incoming);
        Ok(Some(phi.as_basic_value()))
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
