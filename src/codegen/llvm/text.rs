//! Content equality for `String` / `&String` operands.

use inkwell::values::BasicValueEnum;
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::hir::HirExpr;

use super::codegen_error;
use super::emit_fn::FnEmitter;

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    /// `left == right` (or `!=`) over descriptor values, by bytes and length.
    pub(super) fn emit_string_eq(
        &mut self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        negate: bool,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        let size = cx.size_type();
        let mut args = Vec::new();
        for desc in [left, right] {
            let desc = desc.into_struct_value();
            args.push(builder.build_extract_value(desc, 0, "text.ptr")?.into());
            let len = builder
                .build_extract_value(desc, 1, "text.len")?
                .into_int_value();
            args.push(builder.build_int_cast(len, size, "text.len.usize")?.into());
        }
        let call = builder
            .build_call(cx.string_equal_fn(), &args, "text.eq")?
            .try_as_basic_value()
            .unwrap_basic()
            .into_int_value();
        let wanted = if negate {
            IntPredicate::EQ
        } else {
            IntPredicate::NE
        };
        let zero = call.get_type().const_zero();
        Ok(builder
            .build_int_compare(wanted, call, zero, "text.cmp")?
            .into())
    }

    /// `stringFromUtf8(&bytes, start, count)`. A bad range aborts; invalid UTF-8 yields `None`
    /// before any allocation; a valid range is copied into the sink.
    pub(super) fn emit_string_from_utf8(
        &mut self,
        buffer: BasicValueEnum<'ctx>,
        start: BasicValueEnum<'ctx>,
        count: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        let i64_ty = cx.context.i64_type();
        let i8_ty = cx.context.i8_type();
        let desc = buffer.into_struct_value();
        let base = builder
            .build_extract_value(desc, 0, "utf8.base")?
            .into_pointer_value();
        let len = builder
            .build_extract_value(desc, 1, "utf8.len")?
            .into_int_value();
        let start = builder.build_int_s_extend(start.into_int_value(), i64_ty, "utf8.start")?;
        let count = builder.build_int_s_extend(count.into_int_value(), i64_ty, "utf8.count")?;

        // `len - start` may wrap when `start > len`, but that case is already `bad`.
        let rest = builder.build_int_sub(len, start, "utf8.rest")?;
        let zero = i64_ty.const_zero();
        let neg_start =
            builder.build_int_compare(IntPredicate::SLT, start, zero, "utf8.neg.start")?;
        let neg_count =
            builder.build_int_compare(IntPredicate::SLT, count, zero, "utf8.neg.count")?;
        let start_oob =
            builder.build_int_compare(IntPredicate::SGT, start, len, "utf8.start.oob")?;
        let count_oob =
            builder.build_int_compare(IntPredicate::SGT, count, rest, "utf8.count.oob")?;
        let bad_start = builder.build_or(neg_start, start_oob, "utf8.bad.start")?;
        let bad_count = builder.build_or(neg_count, count_oob, "utf8.bad.count")?;
        let bad = builder.build_or(bad_start, bad_count, "utf8.bad")?;
        self.branch_abort_if(bad, expr.span)?;

        let src = unsafe { builder.build_gep(i8_ty, base, &[start], "utf8.src")? };
        let count_usize = builder.build_int_cast(count, cx.size_type(), "utf8.count.usize")?;
        let valid = builder
            .build_call(
                cx.utf8_valid_fn(),
                &[src.into(), count_usize.into()],
                "utf8.ok",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_utf8_valid returns a byte")
            .into_int_value();
        let is_valid =
            builder.build_int_compare(IntPredicate::NE, valid, i8_ty.const_zero(), "utf8.valid")?;

        let copy = cx.context.append_basic_block(self.llvm_fn, "utf8.copy");
        let absent = cx.context.append_basic_block(self.llvm_fn, "utf8.none");
        let done = cx.context.append_basic_block(self.llvm_fn, "utf8.done");
        builder.build_conditional_branch(is_valid, copy, absent)?;

        builder.position_at_end(copy);
        let arena = self.sink_arena();
        let dst = builder
            .build_call(
                cx.arena_alloc_fn(),
                &[
                    arena.into(),
                    count.into(),
                    i64_ty.const_int(1, false).into(),
                ],
                "utf8.dst",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value();
        builder
            .build_memcpy(dst, 1, src, 1, count)
            .map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let copied = cx.string_type().const_named_struct(&[]);
        let copied = builder.build_insert_value(copied, dst, 0, "utf8.out")?;
        let copied = builder
            .build_insert_value(copied, count, 1, "utf8.out")?
            .into_struct_value();
        let some = self.emit_nullable_some(expr.ty, copied.into())?;
        let copy_end = self.cx.builder.get_insert_block().expect("positioned in a block");
        self.cx.builder.build_unconditional_branch(done)?;

        self.cx.builder.position_at_end(absent);
        let none = self.emit_nullable_none(expr.ty)?;
        let none_end = self.cx.builder.get_insert_block().expect("positioned in a block");
        self.cx.builder.build_unconditional_branch(done)?;

        self.cx.builder.position_at_end(done);
        let phi = self.cx.builder.build_phi(some.get_type(), "utf8.result")?;
        phi.add_incoming(&[(&none, none_end), (&some, copy_end)]);
        Ok(phi.as_basic_value())
    }
}
