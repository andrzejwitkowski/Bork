//! Calls that label persistent Ref slots and arenas in the debug runtime.
use crate::ty::TyId;
use crate::{codegen::llvm::context::Codegen, diag::Diagnostic};
use inkwell::{
    values::{IntValue, PointerValue},
    AddressSpace,
};

impl<'ctx> Codegen<'ctx> {
    pub(in crate::codegen::llvm) fn debug_source(
        &self,
        span: crate::span::Span,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let debug = self.debug.as_ref().expect("debug source");
        let key = (debug.active_source.get(), span.start, span.end);
        if let Some(pointer) = debug.locations.borrow().get(&key) {
            return Ok(*pointer);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let path = self
            .builder
            .build_global_string_ptr(debug.source_parts().0, "debug.path")?
            .as_pointer_value();
        let (line, col) = debug.line_column(span);
        let ty = self.context.struct_type(
            &[
                ptr.into(),
                word.into(),
                word.into(),
                word.into(),
                word.into(),
            ],
            false,
        );
        let location = ty.const_named_struct(&[
            path.into(),
            word.const_int(span.start as u64, false).into(),
            word.const_int(span.end as u64, false).into(),
            word.const_int(line as u64, false).into(),
            word.const_int(col as u64, false).into(),
        ]);
        let global = self.module.add_global(ty, None, "debug.location");
        global.set_initializer(&location);
        global.set_constant(true);
        global.set_linkage(inkwell::module::Linkage::Private);
        let pointer = global.as_pointer_value();
        debug.locations.borrow_mut().insert(key, pointer);
        Ok(pointer)
    }

    pub(in crate::codegen::llvm) fn debug_allocation(
        &self,
        address: PointerValue<'ctx>,
        name: &str,
        type_name: &str,
        declaration: bool,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.debug else {
            return Ok(());
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.runtime_fn("bork_debug_label_allocation", || {
            self.context.void_type().fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            )
        });
        let name = self
            .builder
            .build_global_string_ptr(name, "debug.name")?
            .as_pointer_value();
        let ty = self
            .builder
            .build_global_string_ptr(type_name, "debug.concrete_type")?
            .as_pointer_value();
        let site = self.debug_source(debug.position.get())?;
        self.builder.build_call(
            function,
            &[
                address.into(),
                name.into(),
                ty.into(),
                site.into(),
                self.context
                    .i8_type()
                    .const_int(declaration as u64, false)
                    .into(),
            ],
            "",
        )?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_value(
        &self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        ty: TyId,
        name: &str,
        declaration: bool,
    ) -> Result<(), Diagnostic> {
        if self.debug.is_none() || self.tcx.is_managed_ref(ty) {
            return Ok(());
        }
        let pointer = if self.tcx.is_vec(ty) {
            value.into_pointer_value()
        } else if self.tcx.class_def(ty).is_some()
            || matches!(self.tcx.data(ty), crate::ty::TyData::Enum(..))
        {
            self.builder
                .build_extract_value(value.into_struct_value(), 1, "debug.payload")?
                .into_pointer_value()
        } else if self.tcx.is_string(ty) || self.tcx.is_array(ty) {
            self.builder
                .build_extract_value(value.into_struct_value(), 0, "debug.bytes")?
                .into_pointer_value()
        } else {
            return Ok(());
        };
        self.debug_allocation(
            pointer,
            name,
            &self.tcx.display(ty).to_string(),
            declaration,
        )?;
        if self.tcx.is_vec(ty) {
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let buffer = self
                .builder
                .build_load(ptr_ty, pointer, "debug.vec.buffer")?
                .into_pointer_value();
            self.debug_allocation(buffer, name, &self.tcx.display(ty).to_string(), declaration)?;
        }
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_copy_origin(
        &self,
        source: PointerValue<'ctx>,
        destination: PointerValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        if self.debug.is_none() {
            return Ok(());
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.runtime_fn("bork_debug_copy_origin", || {
            self.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into()], false)
        });
        self.builder
            .build_call(function, &[source.into(), destination.into()], "")?;
        Ok(())
    }

    pub(in crate::codegen::llvm) fn debug_declaration(
        &self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        ty: TyId,
        name: &str,
        span: crate::span::Span,
        source: Option<&'static str>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.debug else {
            return Ok(());
        };
        let previous = debug.position.replace(span);
        let previous_source = debug.active_source.replace(source);
        let result = self.debug_value(value, ty, name, true);
        debug.position.set(previous);
        debug.active_source.set(previous_source);
        result
    }

    pub(in crate::codegen::llvm) fn debug_check_handle(
        &self,
        owner: PointerValue<'ctx>,
        address: PointerValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.debug else {
            return Ok(());
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let site = self.debug_source(debug.position.get())?;
        let function = self.runtime_fn("bork_debug_check_handle", || {
            self.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into(), ptr.into()], false)
        });
        self.builder
            .build_call(function, &[owner.into(), address.into(), site.into()], "")?;
        Ok(())
    }

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
            .build_global_string_ptr(debug.source_parts().0, "debug.source")?
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
