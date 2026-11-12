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
        let scope = function.or_else(|| self.cx.debug_lexical_block(self.debug_scopes.last().unwrap().0)).expect("debug is on");
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
        if self.cx.debug.is_none() {
            return Ok(());
        }
        let (scope, token) = *self.debug_scopes.last().unwrap();
        self.cx.debug_local_in(scope, Some(token), name, ty, ptr, Some(home), argument)
    }
}

impl<'ctx> Codegen<'ctx> {
    /// Lexical block under `parent` at the current position.
    pub(in crate::codegen::llvm) fn debug_lexical_block(&self, parent: DIScope<'ctx>) -> Option<DIScope<'ctx>> {
        let debug = self.debug.as_ref()?;
        let (line, col) = debug.line_column(debug.position.get());
        Some(debug.builder.create_lexical_block(parent, debug.source_file(), line, col).as_debug_info_scope())
    }

    /// Sets the statement position and the current DWARF location inside `scope` (MIR lowerer).
    pub(in crate::codegen::llvm) fn debug_at(&self, scope: DIScope<'ctx>, span: Span) {
        let Some(debug) = &self.debug else {
            return;
        };
        self.debug_stamp_current_block();
        debug.position.set(span);
        let (line, col) = debug.line_column(span);
        let location = debug.builder.create_debug_location(self.context, line, col, scope, None);
        self.builder.set_current_debug_location(location);
    }

    /// DILocalVariable plus `dbg.declare`; managed `Ref` locals also register their slot, which needs
    /// the runtime scope `token` and the owning arena `home`.
    pub(in crate::codegen::llvm) fn debug_local_in(
        &self,
        scope: DIScope<'ctx>,
        token: Option<inkwell::values::IntValue<'ctx>>,
        name: &str,
        ty: TyId,
        ptr: PointerValue<'ctx>,
        home: Option<PointerValue<'ctx>>,
        argument: Option<u32>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.debug else {
            return Ok(());
        };
        let (line, col) = debug.line_column(debug.position.get());
        let variable = match argument {
            Some(i) => debug.builder.create_parameter_variable(scope, name, i, debug.source_file(), line, debug.ty(self, ty), true, DIFlags::PUBLIC),
            None => debug.builder.create_auto_variable(scope, name, debug.source_file(), line, debug.ty(self, ty), true, DIFlags::PUBLIC, 0),
        };
        let loc = debug.builder.create_debug_location(self.context, line, col, scope, None);
        // LLVM 23 returns a DbgRecord; Inkwell incorrectly wraps it as an InstructionValue.
        unsafe {
            inkwell::llvm_sys::debuginfo::LLVMDIBuilderInsertDeclareRecordAtEnd(
                debug.builder.as_mut_ptr(),
                ptr.as_value_ref(),
                variable.as_mut_ptr(),
                debug.builder.create_expression(vec![]).as_mut_ptr(),
                loc.as_mut_ptr(),
                self.builder.get_insert_block().unwrap().as_mut_ptr(),
            );
        }
        if self.tcx.is_managed_ref(ty) {
            let (Some(token), Some(home)) = (token, home) else {
                panic!("managed Ref local `{name}` needs a scope token and a home arena");
            };
            self.debug_label_slot(home, ptr, name, ty, token, self.context.i64_type().const_all_ones())?;
        }
        Ok(())
    }
}
