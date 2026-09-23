use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::types::BasicTypeEnum;
use inkwell::OptimizationLevel;

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

    pub fn basic_type(&self, ty: &Ty) -> Option<BasicTypeEnum<'ctx>> {
        match ty.kind {
            TyKind::Prim(Prim::I32) if !ty.nullable => Some(self.context.i32_type().into()),
            _ => None,
        }
    }
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
