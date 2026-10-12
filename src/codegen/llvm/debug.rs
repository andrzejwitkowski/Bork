//! Source metadata and labels for the optional runtime debug registry.
use super::{context::Codegen, emit_fn::FnEmitter};
use crate::{
    diag::Diagnostic,
    hir::{HirFunction, Prim, Ty, TyKind},
    span::Span,
};
use inkwell::{
    debug_info::{
        AsDIScope, DIFile, DIFlags, DIFlagsConstants, DIScope, DIType, DWARFEmissionKind,
        DWARFSourceLanguage, DebugInfoBuilder,
    },
    module::FlagBehavior,
    types::{BasicTypeEnum, StructType},
    values::{AsValueRef, IntValue, PointerValue},
    AddressSpace,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
};

pub(super) struct DebugInfo<'ctx> {
    pub builder: DebugInfoBuilder<'ctx>,
    file: DIFile<'ctx>,
    source: String,
    path: String,
    types: RefCell<HashMap<String, DIType<'ctx>>>,
    pub position: Cell<Span>,
}

impl<'ctx> DebugInfo<'ctx> {
    pub fn new(cx: &Codegen<'ctx>, source: &str, path: &Path) -> Self {
        cx.module.add_basic_value_flag(
            "Debug Info Version",
            FlagBehavior::Warning,
            cx.context.i32_type().const_int(3, false),
        );
        cx.module.add_basic_value_flag(
            "Dwarf Version",
            FlagBehavior::Warning,
            cx.context.i32_type().const_int(4, false),
        );
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        let (builder, unit) = cx.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C,
            &absolute.file_name().unwrap_or_default().to_string_lossy(),
            &absolute
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy(),
            "Bork",
            false,
            "-g",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        Self {
            builder,
            file: unit.get_file(),
            source: source.into(),
            path: absolute.to_string_lossy().into_owned(),
            types: RefCell::new(HashMap::new()),
            position: Cell::new(Span::default()),
        }
    }

    fn line_column(&self, span: Span) -> (u32, u32) {
        let prefix = &self.source.as_bytes()[..span.start.min(self.source.len())];
        let line = prefix.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(prefix.len() + 1, |p| prefix.len() - p);
        (line as u32, col as u32)
    }

    fn pointer(&self, cx: &Codegen<'ctx>, ty: DIType<'ctx>) -> DIType<'ctx> {
        let ptr = cx.context.ptr_type(AddressSpace::default());
        self.builder
            .create_pointer_type(
                "",
                ty,
                cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                cx.target_data.get_abi_alignment(&ptr) * 8,
                AddressSpace::default(),
            )
            .as_type()
    }

    fn structure(
        &self,
        cx: &Codegen<'ctx>,
        name: &str,
        llvm: StructType<'ctx>,
        fields: Vec<(&str, DIType<'ctx>)>,
    ) -> DIType<'ctx> {
        let scope = self.file.as_debug_info_scope();
        let members: Vec<_> = fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, ty))| {
                let llvm_field = llvm.get_field_type_at_index(i as u32).unwrap();
                self.builder
                    .create_member_type(
                        scope,
                        name,
                        self.file,
                        0,
                        cx.target_data.get_abi_size(&llvm_field) * 8,
                        cx.target_data.get_abi_alignment(&llvm_field) * 8,
                        cx.target_data.offset_of_element(&llvm, i as u32).unwrap() * 8,
                        DIFlags::PUBLIC,
                        ty,
                    )
                    .as_type()
            })
            .collect();
        self.builder
            .create_struct_type(
                scope,
                name,
                self.file,
                0,
                cx.target_data.get_abi_size(&llvm) * 8,
                cx.target_data.get_abi_alignment(&llvm) * 8,
                DIFlags::PUBLIC,
                None,
                &members,
                0,
                None,
                name,
            )
            .as_type()
    }

    pub fn ty(&self, cx: &Codegen<'ctx>, ty: &Ty) -> DIType<'ctx> {
        if let Some(inner) = ty.ref_inner() {
            return self.ty(cx, inner);
        }
        let key = ty.to_string();
        if let Some(found) = self.types.borrow().get(&key) {
            return *found;
        }
        // Recursive class graphs refer to this temporary typedef until their layout is complete.
        let placeholder = unsafe { self.builder.create_placeholder_derived_type(cx.context) };
        self.types
            .borrow_mut()
            .insert(key.clone(), placeholder.as_type());
        let u8_ty = || {
            self.builder
                .create_basic_type("u8", 8, 0x07, DIFlags::PUBLIC)
                .unwrap()
                .as_type()
        };
        let result = if ty.is_managed_ref() {
            let target = ty.managed_ref_inner().expect("Ref inner");
            let object = self.pointer(
                cx,
                self.payload(cx, target.record_name().expect("Ref record")),
            );
            let control = self
                .builder
                .create_basic_type(
                    "usize",
                    cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                    0x07,
                    DIFlags::PUBLIC,
                )
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &format!("bork::{key}"),
                cx.ref_handle_type(),
                vec![("control", control), ("object", object), ("kind", u8_ty())],
            )
        } else if ty.is_nullable() && !ty.with_nullable(false).is_array() && !ty.is_string() {
            let inner = ty.with_nullable(false);
            let llvm = cx.basic_type(ty).unwrap().into_struct_type();
            let bool_ty = self.ty(
                cx,
                &Ty {
                    kind: TyKind::Prim(Prim::Bool),
                    nullable: false,
                },
            );
            self.structure(
                cx,
                &key,
                llvm,
                vec![("present", bool_ty), ("value", self.ty(cx, &inner))],
            )
        } else if ty.is_string()
            || ty.is_array()
            || (ty.is_nullable()
                && (ty.with_nullable(false).is_array() || ty.with_nullable(false).is_string()))
        {
            let element = ty
                .array_elem()
                .map(|t| self.ty(cx, t))
                .unwrap_or_else(u8_ty);
            let data = self.pointer(cx, element);
            let length = self
                .builder
                .create_basic_type("usize", 64, 0x07, DIFlags::PUBLIC)
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &key,
                cx.buffer_descriptor_type(),
                vec![("data", data), ("length", length)],
            )
        } else if let TyKind::Named(name) = &ty.kind {
            let object = self.pointer(cx, self.payload(cx, name));
            let arena = self
                .builder
                .create_basic_type(
                    "usize",
                    cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                    0x07,
                    DIFlags::PUBLIC,
                )
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &format!("bork::Object<{name}>"),
                cx.object_handle_type(),
                vec![("arena", arena), ("object", object)],
            )
        } else {
            let llvm = cx.basic_type(ty);
            let bits = llvm.map_or(0, |t| match t {
                BasicTypeEnum::IntType(i) => i.get_bit_width() as u64,
                _ => cx.target_data.get_abi_size(&t) * 8,
            });
            let encoding = match ty.kind {
                TyKind::Prim(Prim::F32 | Prim::F64) => 0x04,
                TyKind::Prim(Prim::Bool) => 0x02,
                TyKind::Prim(Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64) => 0x07,
                _ => 0x05,
            };
            self.builder
                .create_basic_type(&key, bits, encoding, DIFlags::PUBLIC)
                .unwrap()
                .as_type()
        };
        let alias = self.builder.create_typedef(
            result,
            &key,
            self.file,
            0,
            self.file.as_debug_info_scope(),
            0,
        );
        unsafe {
            self.builder
                .replace_placeholder_derived_type(placeholder, alias);
        }
        self.types.borrow_mut().insert(key, result);
        result
    }

    fn payload(&self, cx: &Codegen<'ctx>, name: &str) -> DIType<'ctx> {
        let key = format!("bork::Payload<{name}>");
        if let Some(found) = self.types.borrow().get(&key) {
            return *found;
        }
        let placeholder = unsafe { self.builder.create_placeholder_derived_type(cx.context) };
        self.types
            .borrow_mut()
            .insert(key.clone(), placeholder.as_type());
        let class = cx.classes.iter().find(|c| c.name == name).unwrap();
        let fields = class
            .fields
            .iter()
            .map(|f| (f.name.as_str(), self.ty(cx, &f.ty)))
            .collect();
        let result = self.structure(cx, &key, cx.record_type(name).unwrap(), fields);
        let alias = self.builder.create_typedef(
            result,
            &key,
            self.file,
            0,
            self.file.as_debug_info_scope(),
            0,
        );
        unsafe {
            self.builder
                .replace_placeholder_derived_type(placeholder, alias);
        }
        self.types.borrow_mut().insert(key, result);
        result
    }

    pub fn function(
        &self,
        cx: &Codegen<'ctx>,
        function: &HirFunction,
        llvm: inkwell::values::FunctionValue<'ctx>,
    ) -> DIScope<'ctx> {
        let (line, _) = self.line_column(function.name_span);
        let params: Vec<_> = function.params.iter().map(|p| self.ty(cx, &p.ty)).collect();
        let result = (function.return_ty != Ty::unit()).then(|| self.ty(cx, &function.return_ty));
        let sig = self
            .builder
            .create_subroutine_type(self.file, result, &params, DIFlags::PUBLIC);
        let sub = self.builder.create_function(
            self.file.as_debug_info_scope(),
            &function.name,
            None,
            self.file,
            line,
            sig,
            function.name != "main",
            true,
            line,
            DIFlags::PUBLIC,
            false,
        );
        llvm.set_subprogram(sub);
        self.position.set(function.name_span);
        sub.as_debug_info_scope()
    }
}

impl<'ctx> Codegen<'ctx> {
    pub(super) fn debug_label_slot(
        &self,
        owner: PointerValue<'ctx>,
        slot: PointerValue<'ctx>,
        label: &str,
        ty: &Ty,
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
            .build_global_string_ptr(&ty.to_string(), "debug.type")?
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

    pub(super) fn debug_relabel_slot(
        &self,
        slot: PointerValue<'ctx>,
        label: &str,
        ty: &Ty,
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
            .build_global_string_ptr(&ty.to_string(), "debug.type")?
            .as_pointer_value();
        self.builder.build_call(
            function,
            &[slot.into(), label.into(), ty.into(), index.into()],
            "",
        )?;
        Ok(())
    }

    pub(super) fn debug_label_arena(
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

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn debug_location(&self, span: Option<Span>) {
        let Some(debug) = &self.cx.debug else {
            return;
        };
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

    pub(super) fn debug_enter_scope(
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
                .create_lexical_block(self.debug_scopes.last().unwrap().0, debug.file, line, col)
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

    pub(super) fn debug_unwind(&self, depth: usize) -> Result<(), Diagnostic> {
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

    pub(super) fn debug_exit_scope(&mut self) -> Result<(), Diagnostic> {
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

    pub(super) fn debug_local(
        &self,
        name: &str,
        ty: &Ty,
        ptr: PointerValue<'ctx>,
        home: PointerValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let Some(debug) = &self.cx.debug else {
            return Ok(());
        };
        let (scope, token) = *self.debug_scopes.last().unwrap();
        let (line, col) = debug.line_column(debug.position.get());
        let param = if self.scopes.len() == 1 {
            self.function.params.iter().position(|p| p.name == name)
        } else {
            None
        };
        let variable = match param {
            Some(i) => debug.builder.create_parameter_variable(
                scope,
                name,
                i as u32 + 1,
                debug.file,
                line,
                debug.ty(self.cx, ty),
                true,
                DIFlags::PUBLIC,
            ),
            None => debug.builder.create_auto_variable(
                scope,
                name,
                debug.file,
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
        if ty.is_managed_ref() {
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
