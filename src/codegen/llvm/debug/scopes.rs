//! Source locations, lexical scopes and local variable declarations.
use crate::ty::TyId;
use crate::{codegen::llvm::emit_fn::FnEmitter, diag::Diagnostic, span::Span};
use inkwell::{
    debug_info::{AsDIScope, DIFlags, DIFlagsConstants, DIScope},
    values::{AsValueRef, PointerValue},
};

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(in crate::codegen::llvm) fn debug_location(&self, span: Option<Span>) {
        let Some(debug) = &self.cx.debug else {
            return;
        };
        // DWARF's line/column cannot distinguish nested spans with the same starting byte.
        if let Some(function) = self
            .cx
            .builder
            .get_insert_block()
            .and_then(|b| b.get_parent())
        {
            let old = debug.position.get();
            let word = self.cx.context.i64_type();
            let metadata = self.cx.context.metadata_node(&[
                word.const_int(old.start as u64, false).into(),
                word.const_int(old.end as u64, false).into(),
            ]);
            let kind = self.cx.context.get_kind_id("bork.span");
            let mut stamped = debug.last_stamped.borrow_mut();
            for block in function.get_basic_blocks() {
                let mut next = match stamped.get(&block) {
                    Some(last) => last.get_next_instruction(),
                    None => block.get_first_instruction(),
                };
                while let Some(instruction) = next {
                    instruction
                        .set_metadata(metadata, kind)
                        .expect("span metadata node");
                    stamped.insert(block, instruction);
                    next = instruction.get_next_instruction();
                }
            }
        }
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
