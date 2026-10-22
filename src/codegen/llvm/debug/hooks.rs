//! Calls that label persistent Ref slots and arenas in the debug runtime.
use crate::ty::TyId;
use crate::{codegen::llvm::context::Codegen, diag::Diagnostic};
use inkwell::{
    values::{IntValue, PointerValue},
    AddressSpace,
};

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn debug_label_slot(
        &self,
        owner: PointerValue<'ctx>,
        slot: PointerValue<'ctx>,
        label: &str,
        ty: TyId,
        scope: IntValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        if self.debug.is_none() {
            return Ok(());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let function = self.runtime_fn("bork_debug_register_slot", || {
            self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    word.into(),
                    word.into(),
                ],
                false,
            )
        });
        let label = self
            .builder
            .build_global_string_ptr(label, "debug.label")?
            .as_pointer_value();
        let ty = self
            .builder
            .build_global_string_ptr(&self.tcx.display(ty).to_string(), "debug.type")?
            .as_pointer_value();
        self.builder.build_call(
            function,
            &[
                owner.into(),
                slot.into(),
                label.into(),
                ty.into(),
                scope.into(),
                index.into(),
            ],
            "",
        )?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_relabel_slot(
        &self,
        slot: PointerValue<'ctx>,
        label: &str,
        ty: TyId,
        index: IntValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        if self.debug.is_none() {
            return Ok(());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.runtime_fn("bork_debug_label_slot", || {
            self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    self.context.i64_type().into(),
                ],
                false,
            )
        });
        let label = self
            .builder
            .build_global_string_ptr(label, "debug.label")?
            .as_pointer_value();
        let ty = self
            .builder
            .build_global_string_ptr(&self.tcx.display(ty).to_string(), "debug.type")?
            .as_pointer_value();
        self.builder.build_call(
            function,
            &[slot.into(), label.into(), ty.into(), index.into()],
            "",
        )?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_label_arena(
        &self,
        arena: PointerValue<'ctx>,
    ) -> Result<(), inkwell::builder::BuilderError> {
        let Some(debug) = &self.debug else {
            return Ok(());
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let function = self.runtime_fn("bork_debug_label_arena", || {
            self.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into(), word.into()], false)
        });
        let path = self
            .builder
            .build_global_string_ptr(&debug.path, "debug.source")?
            .as_pointer_value();
        let (line, _) = debug.line_column(debug.position.get());
        self.builder.build_call(
            function,
            &[
                arena.into(),
                path.into(),
                word.const_int(line as u64, false).into(),
            ],
            "",
        )?;
        Ok(())
    }
}
