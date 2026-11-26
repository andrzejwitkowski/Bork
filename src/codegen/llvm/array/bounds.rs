//! Runtime bounds check: abort when an index is outside `0..len`.

use inkwell::values::{FunctionValue, IntValue};

use crate::diag::Diagnostic;

use super::super::context::Codegen;

impl<'ctx> Codegen<'ctx> {
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
