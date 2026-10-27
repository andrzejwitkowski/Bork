//! Instrument the complete module, including glue emitted before DebugInfo existed.
use super::*;
use crate::diag::Diagnostic;
use inkwell::values::{BasicValueEnum, CallSiteValue, InstructionOpcode};

pub(in crate::codegen::llvm) fn instrument(cx: &Codegen<'_>) -> Result<(), Diagnostic> {
    let debug = cx.debug.as_ref().expect("debug instrumentation");
    let functions: Vec<_> = cx
        .module
        .get_functions()
        .filter(|f| f.count_basic_blocks() != 0)
        .collect();
    for function in functions {
        let instructions: Vec<_> = function
            .get_basic_blocks()
            .iter()
            .flat_map(|b| b.get_instructions())
            .collect();
        let first = instructions
            .iter()
            .find(|i| i.get_opcode() != InstructionOpcode::Phi)
            .unwrap();
        cx.builder.position_before(first);
        cx.builder.unset_current_debug_location();
        let name = function.get_name().to_string_lossy();
        let site = debug.function_spans.borrow().get(name.as_ref()).copied();
        debug.active_source.set(site.and_then(|(_, source)| source));
        let site = match site {
            Some((span, _)) => cx.debug_source(span)?,
            None => cx
                .context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null(),
        };
        let label = cx
            .builder
            .build_global_string_ptr(&name, "debug.function")?
            .as_pointer_value();
        let ptr = cx.context.ptr_type(inkwell::AddressSpace::default());
        let enter = cx.runtime_fn("bork_debug_frame_enter", || {
            cx.context
                .void_type()
                .fn_type(&[ptr.into(), ptr.into()], false)
        });
        cx.builder
            .build_call(enter, &[label.into(), site.into()], "")?;
        for instruction in instructions {
            cx.builder.position_before(&instruction);
            let loc = instruction.get_debug_location();
            match loc {
                Some(loc) => cx.builder.set_current_debug_location(loc),
                None => cx.builder.unset_current_debug_location(),
            }
            let site = match instruction.get_metadata(cx.context.get_kind_id("bork.span")) {
                Some(metadata) => {
                    let values = metadata.get_node_values().expect("span metadata");
                    let span = Span {
                        start: values[0]
                            .into_int_value()
                            .get_zero_extended_constant()
                            .unwrap() as usize,
                        end: values[1]
                            .into_int_value()
                            .get_zero_extended_constant()
                            .unwrap() as usize,
                    };
                    cx.debug_source(span)?
                }
                None => ptr.const_null(),
            };
            let operand = |i: u32| instruction.get_operand(i).unwrap().unwrap_value();
            match instruction.get_opcode() {
                InstructionOpcode::Load => {
                    let size = cx.target_data.get_store_size(&instruction.get_type());
                    check(
                        cx,
                        operand(0),
                        cx.context.i64_type().const_int(size, false).into(),
                        false,
                        site,
                    )?;
                }
                InstructionOpcode::Store => {
                    let size = cx.target_data.get_store_size(&operand(0).get_type());
                    check(
                        cx,
                        operand(1),
                        cx.context.i64_type().const_int(size, false).into(),
                        true,
                        site,
                    )?;
                }
                InstructionOpcode::Call => {
                    let call = CallSiteValue::try_from(instruction).unwrap();
                    let Some(callee) = call.get_called_fn_value() else {
                        continue;
                    };
                    let name = callee.get_name().to_string_lossy();
                    if name.starts_with("llvm.memcpy.") || name.starts_with("llvm.memmove.") {
                        check(cx, operand(1), operand(2), false, site)?;
                        check(cx, operand(0), operand(2), true, site)?;
                    } else if name.starts_with("llvm.memset.") {
                        check(cx, operand(0), operand(2), true, site)?;
                    } else if !name.starts_with("bork_debug_") && !name.starts_with("llvm.") {
                        let set_site = cx.runtime_fn("bork_debug_site", || {
                            cx.context.void_type().fn_type(&[ptr.into()], false)
                        });
                        cx.builder.build_call(set_site, &[site.into()], "")?;
                    }
                }
                InstructionOpcode::Return => {
                    let exit = cx.runtime_fn("bork_debug_frame_exit", || {
                        cx.context.void_type().fn_type(&[], false)
                    });
                    cx.builder.build_call(exit, &[], "")?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn check<'ctx>(
    cx: &Codegen<'ctx>,
    address: BasicValueEnum<'ctx>,
    size: BasicValueEnum<'ctx>,
    write: bool,
    site: inkwell::values::PointerValue<'ctx>,
) -> Result<(), Diagnostic> {
    let ptr = cx.context.ptr_type(inkwell::AddressSpace::default());
    let word = cx.context.i64_type();
    let size =
        cx.builder
            .build_int_z_extend_or_bit_cast(size.into_int_value(), word, "debug.size")?;
    let function = cx.runtime_fn("bork_debug_check", || {
        cx.context.void_type().fn_type(
            &[
                ptr.into(),
                word.into(),
                cx.context.i8_type().into(),
                ptr.into(),
            ],
            false,
        )
    });
    cx.builder.build_call(
        function,
        &[
            address.into(),
            size.into(),
            cx.context.i8_type().const_int(write as u64, false).into(),
            site.into(),
        ],
        "",
    )?;
    Ok(())
}
