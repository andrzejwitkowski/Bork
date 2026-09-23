use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, IntType};
use inkwell::values::FunctionValue;
use inkwell::{AddressSpace, OptimizationLevel};

use crate::hir::{Prim, Ty, TyKind};

pub struct Codegen<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    pub builder: Builder<'ctx>,
}

impl<'ctx> Codegen<'ctx> {
    pub fn new(context: &'ctx Context, name: &str) -> Self {
        Self {
            context,
            module: context.create_module(name),
            builder: context.create_builder(),
        }
    }

    /// LLVM type for a value of `ty`; `None` for `unit` and types codegen cannot lower yet.
    pub fn basic_type(&self, ty: &Ty) -> Option<BasicTypeEnum<'ctx>> {
        self.int_type(ty).map(Into::into)
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

    pub fn arena_pop_fn(&self) -> FunctionValue<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let params: [BasicMetadataTypeEnum; 1] = [ptr.into()];
        self.runtime_fn("bork_arena_pop", || {
            self.context.void_type().fn_type(&params, false)
        })
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
