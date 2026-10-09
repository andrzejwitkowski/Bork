use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, IntType, StructType};
use inkwell::values::FunctionValue;
use inkwell::{AddressSpace, OptimizationLevel};

use crate::hir::{HirClass, Prim, Ty, TyKind};
use crate::memory::MemoryPlan;

use crate::codegen_gate::{nullable_repr, NullableRepr};

pub struct Codegen<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    record_types: HashMap<String, StructType<'ctx>>,
    record_fields: HashMap<String, Vec<Ty>>,
    target_data: TargetData,
    pub memory_plan: MemoryPlan,
}

impl<'ctx> Codegen<'ctx> {
    pub fn abort_function(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("abort") {
            return f;
        }
        let fn_type = self.context.void_type().fn_type(&[], false);
        self.module
            .add_function("abort", fn_type, Some(Linkage::External))
    }

    pub fn new(
        context: &'ctx Context,
        name: &str,
        classes: &[HirClass],
        memory_plan: MemoryPlan,
    ) -> Result<Self, String> {
        let record_types = classes
            .iter()
            .map(|class| {
                (
                    class.name.clone(),
                    context.opaque_struct_type(&format!("bork.{}", class.name)),
                )
            })
            .collect();
        let record_fields = classes
            .iter()
            .map(|class| {
                (
                    class.name.clone(),
                    class.fields.iter().map(|field| field.ty.clone()).collect(),
                )
            })
            .collect();
        let machine = native_target_machine()?;
        let codegen = Self {
            context,
            module: context.create_module(name),
            builder: context.create_builder(),
            record_types,
            record_fields,
            target_data: machine.get_target_data(),
            memory_plan,
        };
        codegen.module.set_triple(&machine.get_triple());
        codegen
            .module
            .set_data_layout(&codegen.target_data.get_data_layout());
        for class in classes {
            let fields = class
                .fields
                .iter()
                .map(|field| {
                    codegen.basic_type(&field.ty).ok_or_else(|| {
                        format!(
                            "record field `{}.{}` has unsupported type `{}`",
                            class.name, field.name, field.ty
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            codegen
                .record_type(&class.name)
                .expect("record shell was declared")
                .set_body(&fields, false);
        }
        Ok(codegen)
    }

    /// LLVM type for a value of `ty`; `None` for `unit` and types codegen cannot lower yet.
    pub fn basic_type(&self, ty: &Ty) -> Option<BasicTypeEnum<'ctx>> {
        if let Some(inner) = ty.ref_inner() {
            return self.basic_type(inner);
        }
        if ty.is_managed_ref() {
            return Some(self.ref_handle_type().into());
        }
        if ty.is_nullable() {
            return self.nullable_storage_type(ty);
        }
        if ty.is_string() {
            return Some(self.buffer_descriptor_type().into());
        }
        if ty.is_array() {
            return Some(self.buffer_descriptor_type().into());
        }
        if matches!(ty.kind, TyKind::Named(_)) {
            return Some(self.object_handle_type().into());
        }
        if let TyKind::Prim(prim) = ty.kind {
            return match prim {
                Prim::F32 => Some(self.context.f32_type().into()),
                Prim::F64 => Some(self.context.f64_type().into()),
                _ => self.int_type(ty).map(Into::into),
            };
        }
        None
    }

    pub fn record_type(&self, name: &str) -> Option<StructType<'ctx>> {
        self.record_types.get(name).copied()
    }

    pub fn record_field_type(&self, name: &str, index: usize) -> Option<&Ty> {
        self.record_fields.get(name)?.get(index)
    }

    /// Internal owning/borrowed object descriptor `{ arena_control, object }`.
    pub fn object_handle_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    /// Runtime `RefHandle`: `{ target_control, object, RefKind:u8 }`.
    pub fn ref_handle_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context
            .struct_type(&[ptr.into(), ptr.into(), self.context.i8_type().into()], false)
    }

    pub fn ref_handle_layout(&self) -> (usize, usize) {
        let ty = self.ref_handle_type();
        (
            self.target_data.get_abi_size(&ty) as usize,
            self.target_data.get_abi_alignment(&ty) as usize,
        )
    }

    pub fn ref_observation_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    /// In-memory shape for `T?` (not `&T`).
    pub fn nullable_storage_type(&self, ty: &Ty) -> Option<BasicTypeEnum<'ctx>> {
        match nullable_repr(ty)? {
            NullableRepr::Buffer => Some(self.buffer_descriptor_type().into()),
            NullableRepr::TaggedScalar => {
                let inner = ty.with_nullable(false);
                let value_ty = self.basic_type(&inner)?;
                let tag = self.context.bool_type();
                Some(
                    self.context
                        .struct_type(&[tag.into(), value_ty], false)
                        .into(),
                )
            }
        }
    }

    /// Arena-backed buffer descriptor `{ ptr, i64 }` (`String`, `[T]`).
    pub fn buffer_descriptor_type(&self) -> StructType<'ctx> {
        self.string_type()
    }

    /// String descriptor `{ ptr, i64 }`: borrowed bytes and their length, no terminator.
    pub fn string_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context
            .struct_type(&[ptr.into(), self.context.i64_type().into()], false)
    }

    pub fn int_type(&self, ty: &Ty) -> Option<IntType<'ctx>> {
        let TyKind::Prim(prim) = ty.kind else {
            return None;
        };
        if ty.nullable {
            return None;
        }
        Some(match prim {
            Prim::I8 | Prim::U8 => self.context.i8_type(),
            Prim::I16 | Prim::U16 => self.context.i16_type(),
            Prim::I32 | Prim::U32 => self.context.i32_type(),
            Prim::I64 | Prim::U64 => self.context.i64_type(),
            Prim::Bool => self.context.bool_type(),
            Prim::F32 | Prim::F64 | Prim::Unit => return None,
        })
    }

    /// True when the insertion block already ends in a terminator, so anything emitted is dead.
    pub fn current_block_terminated(&self) -> bool {
        self.builder
            .get_insert_block()
            .and_then(|block| block.get_terminator())
            .is_some()
    }

    /// True when code at the insertion point can never run: the block is terminated, or it is
    /// not the entry block and nothing branches to it.
    pub fn insertion_is_dead(&self) -> bool {
        let Some(block) = self.builder.get_insert_block() else {
            return true;
        };
        let is_entry = block
            .get_parent()
            .and_then(|function| function.get_first_basic_block())
            == Some(block);
        block.get_terminator().is_some() || (!is_entry && block.get_first_use().is_none())
    }

    pub fn arena_push_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.runtime_fn("bork_arena_push", || ptr.fn_type(&[], false))
    }

    pub fn arena_push_child_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn("bork_arena_push_child", || ptr.fn_type(&params, false))
    }

    pub fn arena_root_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.runtime_fn("bork_arena_root", || ptr.fn_type(&[], false))
    }

    pub fn arena_create_dynamic_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 2] = [ptr.into(), self.context.i8_type().into()];
        self.runtime_fn("bork_arena_create_dynamic", || ptr.fn_type(&params, false))
    }

    pub fn arena_register_strong_root_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_arena_register_strong_root")
    }

    pub fn arena_register_ref_drop_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_arena_register_ref_drop")
    }

    pub fn arena_register_observation_drop_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_arena_register_observation_drop")
    }

    pub fn ref_create_out_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 4] =
            [ptr.into(), ptr.into(), ptr.into(), ptr.into()];
        self.runtime_fn("bork_ref_create_out", || {
            self.context.void_type().fn_type(&params, false)
        })
    }

    pub fn ref_store_take_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 3] = [
            ptr.into(),
            ptr.into(),
            ptr.into(),
        ];
        self.runtime_fn("bork_ref_store_take_ptr", || {
            self.context.void_type().fn_type(&params, false)
        })
    }

    pub fn ref_clone_out_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_ref_clone_same_owner_out")
    }

    pub fn ref_move_out_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_ref_move_out")
    }

    pub fn ref_observe_fn(&self) -> FunctionValue<'ctx> {
        self.two_pointer_void_fn("bork_ref_observe_out")
    }

    pub fn ref_release_observation_fn(&self) -> FunctionValue<'ctx> {
        self.arena_handle_fn("bork_ref_release_observation_ptr")
    }

    pub fn ref_drop_value_fn(&self) -> FunctionValue<'ctx> {
        self.arena_handle_fn("bork_ref_drop_value_ptr")
    }

    pub fn ref_rehome_out_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 3] = [ptr.into(), ptr.into(), ptr.into()];
        self.runtime_fn("bork_ref_rehome_out", || {
            self.context.void_type().fn_type(&params, false)
        })
    }

    pub fn ref_is_none_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn("bork_ref_is_none_ptr", || {
            self.context.bool_type().fn_type(&params, false)
        })
    }

    pub fn ref_equal_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.runtime_fn("bork_ref_equal_ptr", || {
            self.context
                .bool_type()
                .fn_type(&[ptr.into(), ptr.into()], false)
        })
    }

    pub fn arena_pop_fn(&self) -> FunctionValue<'ctx> {
        self.arena_handle_fn("bork_arena_pop")
    }

    pub fn arena_reset_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn("bork_arena_reset", || ptr.fn_type(&params, false))
    }

    /// `ptr bork_arena_alloc(ptr arena, i64 size, i64 align)`.
    pub fn arena_alloc_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let params: [BasicMetadataTypeEnum; 3] = [ptr.into(), i64_type.into(), i64_type.into()];
        self.runtime_fn("bork_arena_alloc", || ptr.fn_type(&params, false))
    }

    /// `void bork_{print,println}_i64(i64)`.
    pub fn print_i64_fn(&self, newline: bool) -> FunctionValue<'ctx> {
        let name = if newline {
            "bork_println_i64"
        } else {
            "bork_print_i64"
        };
        let params: [BasicMetadataTypeEnum; 1] = [self.context.i64_type().into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    /// `void bork_{print,println}_str(ptr bytes, i64 len)`.
    pub fn print_str_fn(&self, newline: bool) -> FunctionValue<'ctx> {
        let name = if newline {
            "bork_println_str"
        } else {
            "bork_print_str"
        };
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 2] = [ptr.into(), self.context.i64_type().into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    /// `void name(ptr arena)`.
    fn arena_handle_fn(&self, name: &str) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    fn two_pointer_void_fn(&self, name: &str) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 2] = [ptr.into(), ptr.into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    fn runtime_fn(
        &self,
        name: &str,
        signature: impl FnOnce() -> inkwell::types::FunctionType<'ctx>,
    ) -> FunctionValue<'ctx> {
        self.module
            .get_function(name)
            .unwrap_or_else(|| self.module.add_function(name, signature(), None))
    }
}

/// Whether integer ops on `ty` use unsigned semantics (`udiv`, `ult`, zero-extension).
pub fn is_unsigned(ty: &Ty) -> bool {
    matches!(
        ty.kind,
        TyKind::Prim(Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64 | Prim::Bool)
    )
}

pub fn native_target_machine() -> Result<TargetMachine, String> {
    Target::initialize_native(&InitializationConfig::default())?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|err| err.to_string())?;
    let cpu = TargetMachine::get_host_cpu_name();
    let features = TargetMachine::get_host_cpu_features();
    target
        .create_target_machine(
            &triple,
            &cpu.to_string_lossy(),
            &features.to_string_lossy(),
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| format!("LLVM cannot create a target machine for {triple}"))
}
