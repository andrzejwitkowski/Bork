//! Runtime bounds check: abort when an index is outside `0..len`.

use inkwell::values::IntValue;
use inkwell::IntPredicate;

use crate::diag::Diagnostic;

use super::super::emit_fn::FnEmitter;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn guard_index(
        &mut self,
        index: IntValue<'ctx>,
        len: IntValue<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let idx = self
            .cx
            .builder
            .build_int_s_extend(index, len.get_type(), "idx")?;
        let zero = len.get_type().const_int(0, false);
        let neg = self
            .cx
            .builder
            .build_int_compare(IntPredicate::SLT, idx, zero, "neg")?;
        let ge = self
            .cx
            .builder
            .build_int_compare(IntPredicate::UGE, idx, len, "oob")?;
        let bad = self.cx.builder.build_or(neg, ge, "idx.bad")?;
        self.branch_abort_if(bad, span)?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn branch_abort_if(
        &mut self,
        cond: inkwell::values::IntValue<'ctx>,
        _span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let ok = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "bounds.ok");
        let bad = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "bounds.bad");
        self.cx.builder.build_conditional_branch(cond, bad, ok)?;
        self.cx.position_at_end(bad);
        self.cx
            .builder
            .build_call(self.cx.abort_function(), &[], "abort")?;
        self.cx.builder.build_unreachable()?;
        self.cx.position_at_end(ok);
        Ok(())
    }
}
