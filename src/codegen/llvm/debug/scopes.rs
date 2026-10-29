//! Source locations, lexical scopes and local variable declarations.
use crate::ty::TyId;
use crate::{
    codegen::llvm::{context::Codegen, emit_fn::FnEmitter},
    diag::Diagnostic,
    span::Span,
};
use inkwell::{
    basic_block::BasicBlock,
    debug_info::{AsDIScope, DIFlags, DIFlagsConstants, DIScope},
    values::{AsValueRef, PointerValue},
};

#[cfg(test)]
thread_local! { pub(super) static STAMPED_BLOCK_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn position_at_end(&self, block: BasicBlock<'ctx>) {
        self.debug_stamp_current_block();
        self.builder.position_at_end(block);
    }

    pub(in crate::codegen::llvm) fn debug_stamp_current_block(&self) {
        let Some(debug) = &self.debug else {
            return;
        };
        let Some(block) = self.builder.get_insert_block() else {
            return;
        };
        #[cfg(test)]
        STAMPED_BLOCK_VISITS.with(|visits| visits.set(visits.get() + 1));
        let mut stamped = debug.last_stamped.borrow_mut();
        let mut next = match stamped.get(&block) {
            Some(last) => last.get_next_instruction(),
            None => block.get_first_instruction(),
        };
        if next.is_none() {
            return;
        }
        // DWARF's line/column cannot distinguish nested spans with the same starting byte.
        let old = debug.position.get();
        let word = self.context.i64_type();
        let metadata = self.context.metadata_node(&[
            word.const_int(old.start as u64, false).into(),
            word.const_int(old.end as u64, false).into(),
        ]);
        let kind = self.context.get_kind_id("bork.span");
        let mut last = None;
        while let Some(instruction) = next {
            instruction
                .set_metadata(metadata, kind)
                .expect("span metadata node");
            last = Some(instruction);
            next = instruction.get_next_instruction();
        }
        stamped.insert(block, last.unwrap());
    }
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn debug_location(&self, span: Option<Span>) {
        let Some(debug) = &self.cx.debug else {
            return;
        };
        self.cx.debug_stamp_current_block();
        if let Some(span) = span {
            debug.position.set(span);
        }
        if let Some((scope, _)) = self.debug_scopes.last() {
            let (line, col) = debug.line_column(debug.position.get());
            let location =
                debug
                    .builder
                    .create_debug_location(self.cx.context, line, col, *scope, None);
            self.cx.builder.set_current_debug_location(location);
        }
    }

    pub(in crate::codegen::llvm) fn debug_enter_scope(
        &mut self,
        function: Option<DIScope<'ctx>>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.cx.debug else {
            return Ok(());
        };
        let is_function = function.is_some();
        let scope = function.unwrap_or_else(|| {
            let (line, col) = debug.line_column(debug.position.get());
            debug
                .builder
                .create_lexical_block(
                    self.debug_scopes.last().unwrap().0,
                    debug.source_file(),
                    line,
                    col,
                )
                .as_debug_info_scope()
        });
        let f = self.cx.runtime_fn("bork_debug_scope_enter", || {
            self.cx.context.i64_type().fn_type(&[], false)
        });
        let token = self
            .cx
            .builder
            .build_call(f, &[], "debug.scope")?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        self.debug_scopes.push((scope, token));
        if !is_function {
            self.debug_location(None);
        }
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_unwind(&self, depth: usize) -> Result<(), Diagnostic> {
        if self.cx.debug.is_none() {
            return Ok(());
        }
        let f = self.cx.runtime_fn("bork_debug_scope_exit", || {
            self.cx
                .context
                .void_type()
                .fn_type(&[self.cx.context.i64_type().into()], false)
        });
        for (_, token) in self.debug_scopes.iter().skip(depth).rev() {
            self.cx.builder.build_call(f, &[(*token).into()], "")?;
        }
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_exit_scope(&mut self) -> Result<(), Diagnostic> {
        if self.cx.debug.is_none() {
            return Ok(());
        }
        if !self.cx.current_block_terminated() {
            self.debug_unwind(self.debug_scopes.len() - 1)?;
        }
        self.debug_scopes.pop();
        self.debug_location(None);
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_local(
        &self,
        name: &str,
        ty: TyId,
        ptr: PointerValue<'ctx>,
        home: PointerValue<'ctx>,
        argument: Option<u32>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.cx.debug else {
            return Ok(());
        };
        let (scope, token) = *self.debug_scopes.last().unwrap();
        let (line, col) = debug.line_column(debug.position.get());
        let variable = match argument {
            Some(i) => debug.builder.create_parameter_variable(
                scope,
                name,
                i,
                debug.source_file(),
                line,
                debug.ty(self.cx, ty),
                true,
                DIFlags::PUBLIC,
            ),
            None => debug.builder.create_auto_variable(
                scope,
                name,
                debug.source_file(),
                line,
                debug.ty(self.cx, ty),
                true,
                DIFlags::PUBLIC,
                0,
            ),
        };
        let loc = debug
            .builder
            .create_debug_location(self.cx.context, line, col, scope, None);
        // LLVM 23 returns a DbgRecord; Inkwell incorrectly wraps it as an InstructionValue.
        unsafe {
            inkwell::llvm_sys::debuginfo::LLVMDIBuilderInsertDeclareRecordAtEnd(
                debug.builder.as_mut_ptr(),
                ptr.as_value_ref(),
                variable.as_mut_ptr(),
                debug.builder.create_expression(vec![]).as_mut_ptr(),
                loc.as_mut_ptr(),
                self.cx.builder.get_insert_block().unwrap().as_mut_ptr(),
            );
        }
        if self.cx.tcx.is_managed_ref(ty) {
            self.cx.debug_label_slot(
                home,
                ptr,
                name,
                ty,
                token,
                self.cx.context.i64_type().const_all_ones(),
            )?;
        }
        Ok(())
    }
}
