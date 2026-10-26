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

use crate::hir::{HirClass, HirEnum, Prim};
use crate::ty::{TyCtx, TyData, TyId};
use crate::memory::MemoryPlan;

use crate::codegen_gate::{nullable_repr, NullableRepr};

/// How one enum instance sits in its arena slot: `{ i32 tag, payload.. }` per variant, the slot
/// being as large and as aligned as the biggest variant (no niche optimization).
pub(super) struct EnumLayout<'ctx> {
    /// `{ i32 tag, field.. }` per variant, in tag order; the tag is element 0.
    pub variants: Vec<StructType<'ctx>>,
    pub size: u64,
    pub align: u64,
}

pub struct Codegen<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
    record_types: HashMap<String, StructType<'ctx>>,
    record_fields: HashMap<String, Vec<TyId>>,
    pub(super) target_data: TargetData,
    pub(super) debug: Option<super::debug::DebugInfo<'ctx>>,
    pub(super) classes: Vec<HirClass>,
    pub(super) enums: Vec<HirEnum>,
    pub(super) enum_layouts: HashMap<String, EnumLayout<'ctx>>,
    /// `void drop(ptr slot)` per enum with a `Ref` payload, registered once per constructed value.
    pub(super) enum_drops: HashMap<String, FunctionValue<'ctx>>,
    /// The checked program's types; ids in the HIR resolve here.
    pub(super) tcx: TyCtx,
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
        enums: &[HirEnum],
        tcx: &TyCtx,
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
                    class.fields.iter().map(|field| field.ty).collect(),
                )
            })
            .collect();
        let machine = native_target_machine()?;
        let mut codegen = Self {
            context,
            module: context.create_module(name),
            builder: context.create_builder(),
            record_types,
            record_fields,
            target_data: machine.get_target_data(),
            memory_plan,
            debug: None,
            classes: classes.to_vec(),
            enums: enums.to_vec(),
            enum_layouts: HashMap::new(),
            enum_drops: HashMap::new(),
            tcx: tcx.clone(),
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
                    codegen.basic_type(field.ty).ok_or_else(|| {
                        format!(
                            "record field `{}.{}` has unsupported type `{}`",
                            class.name, field.name, codegen.tcx.display(field.ty)
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            codegen
                .record_type(&class.name)
                .expect("record shell was declared")
                .set_body(&fields, false);
        }
        for e in enums {
            let layout = codegen.enum_layout_of(e)?;
            codegen.enum_layouts.insert(e.name.clone(), layout);
        }
        for e in enums {
            if let Some(drop_fn) = codegen.emit_enum_drop(e)? {
                codegen.enum_drops.insert(e.name.clone(), drop_fn);
            }
        }
        Ok(codegen)
    }

    fn enum_layout_of(&self, e: &HirEnum) -> Result<EnumLayout<'ctx>, String> {
        let mut variants = Vec::new();
        let (mut size, mut align) = (0, 1);
        for variant in &e.variants {
            let mut elements = vec![self.context.i32_type().into()];
            for field in &variant.fields {
                elements.push(self.basic_type(field.ty).ok_or_else(|| {
                    format!(
                        "enum payload `{}.{}` has unsupported type `{}`",
                        e.name, field.name, self.tcx.display(field.ty)
                    )
                })?);
            }
            let ty = self.context.struct_type(&elements, false);
            size = size.max(self.target_data.get_abi_size(&ty));
            align = align.max(u64::from(self.target_data.get_abi_alignment(&ty)));
            variants.push(ty);
        }
        Ok(EnumLayout { variants, size: size.next_multiple_of(align), align })
    }

    pub(super) fn enum_layout(&self, name: &str) -> Option<&EnumLayout<'ctx>> {
        self.enum_layouts.get(name)
    }

    /// The `HirEnum` laid out under `name`, with the tag of `variant`.
    pub(super) fn enum_variant(&self, name: &str, variant: &str) -> Option<(&crate::hir::HirVariant, u32)> {
        let e = self.enums.iter().find(|e| e.name == name)?;
        let tag = e.variants.iter().position(|v| v.name == variant)?;
        Some((&e.variants[tag], tag as u32))
    }

    /// LLVM type for a value of `ty`; `None` for `unit` and types codegen cannot lower yet.
    pub fn basic_type(&self, ty: TyId) -> Option<BasicTypeEnum<'ctx>> {
        if let Some(inner) = self.tcx.ref_inner(ty) {
            return self.basic_type(inner);
        }
        if self.tcx.is_managed_ref(ty) {
            return Some(self.ref_handle_type().into());
        }
        if self.tcx.is_nullable(ty) {
            return self.nullable_storage_type(ty);
        }
        if self.tcx.is_string(ty) {
            return Some(self.buffer_descriptor_type().into());
        }
        if self.tcx.is_array(ty) {
            return Some(self.buffer_descriptor_type().into());
        }
        if self.tcx.is_vec(ty) {
            // Pointer to the arena-resident header `{ buf, len, cap, arena }`.
            return Some(self.context.ptr_type(AddressSpace::default()).into());
        }
        if self.tcx.class_def(ty).is_some() || matches!(self.tcx.data(ty), TyData::Enum(..)) {
            return Some(self.object_handle_type().into());
        }
        if let TyData::Prim(prim) = *self.tcx.data(ty) {
            return match prim {
                Prim::F32 => Some(self.context.f32_type().into()),
                Prim::F64 => Some(self.context.f64_type().into()),
                _ => self.int_type(ty).map(Into::into),
            };
        }
        None
    }

    pub(super) fn unit_ty(&self) -> TyId {
        self.tcx.prim_id(Prim::Unit)
    }

    pub(super) fn i32_ty(&self) -> TyId {
        self.tcx.prim_id(Prim::I32)
    }

    pub(super) fn bool_ty(&self) -> TyId {
        self.tcx.prim_id(Prim::Bool)
    }

    pub fn record_type(&self, name: &str) -> Option<StructType<'ctx>> {
        self.record_types.get(name).copied()
    }

    /// The `HirClass` laid out under `name` (a plain class name or a mono instance name).
    pub fn class_by_name(&self, name: &str) -> Option<&HirClass> {
        self.classes.iter().find(|class| class.name == name)
    }

    /// Name of field `index` of class `name`; a missing class is a diagnostic, not a panic.
    pub fn class_field_name(&self, name: &str, index: usize) -> Result<&str, crate::diag::Diagnostic> {
        self.class_by_name(name)
            .and_then(|class| class.fields.get(index))
            .map(|field| field.name.as_str())
            .ok_or_else(|| super::not_yet_supported(&format!("record `{name}`"), None))
    }

    pub fn record_field_type(&self, name: &str, index: usize) -> Option<TyId> {
        self.record_fields.get(name)?.get(index).copied()
    }

    /// Internal owning/borrowed object descriptor `{ arena_control, object }`.
    pub fn object_handle_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    /// Runtime `RefHandle`: `{ target_control, object, RefKind:u8 }`.
    pub fn ref_handle_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[ptr.into(), ptr.into(), self.context.i8_type().into()],
            false,
        )
    }

    pub fn ref_handle_layout(&self) -> (usize, usize) {
        let ty = self.ref_handle_type();
        (
            self.target_data.get_abi_size(&ty) as usize,
            self.target_data.get_abi_alignment(&ty) as usize,
        )
    }

    #[cfg(test)]
    pub fn ref_handle_offsets(&self) -> [usize; 3] {
        let ty = self.ref_handle_type();
        // Trailing fields can hide in padding, so the count is checked too.
        assert_eq!(ty.count_fields(), 3, "Ref handle must be {{control, object, kind}}");
        std::array::from_fn(|i| self.target_data.offset_of_element(&ty, i as u32).unwrap() as usize)
    }

    pub fn ref_observation_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(&[ptr.into(), ptr.into()], false)
    }

    /// In-memory shape for `T?` (not `&T`).
    pub fn nullable_storage_type(&self, ty: TyId) -> Option<BasicTypeEnum<'ctx>> {
        match nullable_repr(&self.tcx, ty)? {
            NullableRepr::Buffer => Some(self.buffer_descriptor_type().into()),
            NullableRepr::TaggedScalar => {
                let inner = self.tcx.base(ty);
                let value_ty = self.basic_type(inner)?;
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

    pub fn int_type(&self, ty: TyId) -> Option<IntType<'ctx>> {
        let TyData::Prim(prim) = *self.tcx.data(ty) else {
            return None;
        };
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
        let params: [BasicMetadataTypeEnum; 4] = [ptr.into(), ptr.into(), ptr.into(), ptr.into()];
        self.runtime_fn("bork_ref_create_out", || {
            self.context.void_type().fn_type(&params, false)
        })
    }

    pub fn ref_store_take_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 3] = [ptr.into(), ptr.into(), ptr.into()];
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

    /// `ptr bork_arena_grow(ptr arena, ptr block, i64 old_size, i64 new_size, i64 align)`.
    pub fn arena_grow_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let params: [BasicMetadataTypeEnum; 5] = [
            ptr.into(),
            ptr.into(),
            i64_type.into(),
            i64_type.into(),
            i64_type.into(),
        ];
        self.runtime_fn("bork_arena_grow", || ptr.fn_type(&params, false))
    }

    /// `void bork_arena_register_drop(ptr arena, ptr value, ptr drop_fn)`.
    pub fn arena_register_drop_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 3] = [ptr.into(), ptr.into(), ptr.into()];
        self.runtime_fn("bork_arena_register_drop", || {
            self.context.void_type().fn_type(&params, false)
        })
    }

    /// `void bork_vec_drop_refs(ptr header)`: the drop for a `Vec<Ref<T>>` header.
    pub fn vec_drop_refs_fn(&self) -> FunctionValue<'ctx> {
        self.arena_handle_fn("bork_vec_drop_refs")
    }

    /// `i1 bork_arena_count_lease_retain(ptr arena)`.
    pub fn arena_count_lease_retain_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn("bork_arena_count_lease_retain", || {
            self.context.bool_type().fn_type(&params, false)
        })
    }

    /// `void bork_arena_count_lease_release(ptr arena)`.
    pub fn arena_count_lease_release_fn(&self) -> FunctionValue<'ctx> {
        self.arena_handle_fn("bork_arena_count_lease_release")
    }

    /// `i64 bork_arena_{strong,weak}_count(ptr arena)`.
    pub fn arena_count_fn(&self, weak: bool) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        let name = if weak { "bork_arena_weak_count" } else { "bork_arena_strong_count" };
        self.runtime_fn(name, || self.size_type().fn_type(&params, false))
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

    /// Integer type of the runtime's `usize` on the target.
    pub fn size_type(&self) -> inkwell::types::IntType<'ctx> {
        self.context.ptr_sized_int_type(&self.target_data, None)
    }

    /// `i8 bork_string_equal(ptr, usize, ptr, usize)`.
    pub fn string_equal_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let len = self.size_type();
        let params: [BasicMetadataTypeEnum; 4] =
            [ptr.into(), len.into(), ptr.into(), len.into()];
        self.runtime_fn("bork_string_equal", || {
            self.context.i8_type().fn_type(&params, false)
        })
    }

    /// `i8 bork_utf8_valid(ptr, usize)`.
    pub fn utf8_valid_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 2] = [ptr.into(), self.size_type().into()];
        self.runtime_fn("bork_utf8_valid", || {
            self.context.i8_type().fn_type(&params, false)
        })
    }

    /// `void name(ptr arena)`.
    pub(super) fn arena_handle_fn(&self, name: &str) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    fn two_pointer_void_fn(&self, name: &str) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 2] = [ptr.into(), ptr.into()];
        self.runtime_fn(name, || self.context.void_type().fn_type(&params, false))
    }

    pub(super) fn runtime_fn(
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
pub fn is_unsigned(tcx: &TyCtx, ty: TyId) -> bool {
    matches!(
        tcx.data(ty),
        TyData::Prim(Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64 | Prim::Bool)
    )
}

pub fn native_target_machine() -> Result<TargetMachine, String> {
    native_target_machine_with_debug(false)
}

pub fn native_target_machine_with_debug(debug: bool) -> Result<TargetMachine, String> {
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
            if debug {
                OptimizationLevel::None
            } else {
                OptimizationLevel::Default
            },
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| format!("LLVM cannot create a target machine for {triple}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INV-9: the hand-written LLVM `Ref` struct must match the runtime `#[repr(C)] RefHandle`.
    #[test]
    fn ref_handle_layout_matches_runtime() {
        let context = Context::create();
        let cg = Codegen::new(&context, "layout", &[], &[], &TyCtx::new(&Default::default()), MemoryPlan::default()).unwrap();
        let (size, align, offsets) = bork_runtime::REF_HANDLE_LAYOUT;
        assert_eq!(
            (cg.ref_handle_layout(), cg.ref_handle_offsets()),
            ((size, align), offsets),
            "LLVM Ref handle != runtime RefHandle (size/align, field offsets); update both definitions"
        );
    }
}
