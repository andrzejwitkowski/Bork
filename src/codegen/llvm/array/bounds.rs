//! Runtime bounds check: abort when an index is outside `0..len`.

use inkwell::values::{FunctionValue, IntValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;

use super::super::context::Codegen;
use super::super::emit_fn::FnEmitter;

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn guard_index(
        &self,
        llvm_fn: FunctionValue<'ctx>,
        index: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let idx = self.builder.build_int_s_extend(index, len.get_type(), "idx")?;
        let zero = len.get_type().const_int(0, false);
        let neg = self.builder.build_int_compare(IntPredicate::SLT, idx, zero, "neg")?;
        let ge = self.builder.build_int_compare(IntPredicate::UGE, idx, len, "oob")?;
        let bad = self.builder.build_or(neg, ge, "idx.bad")?;
        self.branch_abort_if(llvm_fn, bad)
    }

    pub(in crate::codegen::llvm) fn branch_abort_if(
        &self,
        llvm_fn: FunctionValue<'ctx>,
        cond: IntValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let ok = self.context.append_basic_block(llvm_fn, "bounds.ok");
        let bad = self.context.append_basic_block(llvm_fn, "bounds.bad");
        self.builder.build_conditional_branch(cond, bad, ok)?;
        self.position_at_end(bad);
        self.builder.build_call(self.abort_function(), &[], "abort")?;
        self.builder.build_unreachable()?;
        self.position_at_end(ok);
        Ok(())
    }
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn guard_index(
        &mut self,
        index: IntValue<'ctx>,
        len: IntValue<'ctx>,
        _span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        self.cx.guard_index(self.llvm_fn, index, len)
    }
}
