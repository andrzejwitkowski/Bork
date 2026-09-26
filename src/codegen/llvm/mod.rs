//! Inkwell (LLVM 23) emission for the codegen subset.

mod arena;
mod array;
mod context;
mod emit_fn;
mod expr;
mod region_emit;

use std::path::Path;

use inkwell::builder::BuilderError;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::FileType;

use crate::codegen::regions::{RegionEmitter, ScheduleError};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::HirProgram;
use crate::sema::ArenaReport;
use crate::span::Span;

use arena::ArenaCalls;
use context::{native_target_machine, Codegen};
use emit_fn::Callees;

pub fn emit_module<'ctx>(
    context: &'ctx Context,
    hir: &HirProgram,
    report: &ArenaReport,
) -> Result<Module<'ctx>, Diagnostic> {
    if !hir.functions.iter().any(|function| function.name == "main") {
        return Err(codegen_error(
            "`fun main` is required to build an executable".into(),
            None,
        ));
    }

    let cx = Codegen::new(context, "bork");
    let callees = hir
        .functions
        .iter()
        .map(|function| {
            let value = emit_fn::declare_function(&cx, function)?;
            Ok((function.name.as_str(), (value, function)))
        })
        .collect::<Result<Callees, Diagnostic>>()?;
    let mut regions = RegionEmitter::new(report, ArenaCalls::new(&cx));
    for function in &hir.functions {
        emit_fn::emit_function(&cx, &callees, &mut regions, &report.roots, function)?;
    }
    regions.finish().map_err(schedule_error)?;
    cx.module
        .verify()
        .map_err(|err| codegen_error(format!("LLVM rejected the module: {err}"), None))?;
    Ok(cx.module)
}

/// Failures here are toolchain problems, not user errors.
pub fn write_object(module: &Module<'_>, path: &Path) -> Result<(), String> {
    let machine = native_target_machine()?;
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());

    if let Some(file) = std::env::var_os("BORK_DUMP_IR") {
        std::fs::write(
            std::path::PathBuf::from(file),
            module.print_to_string().to_string(),
        )
        .map_err(|err| format!("failed to dump IR: {err}"))?;
    }

    let options = PassBuilderOptions::create();
    options.set_loop_vectorization(true);
    options.set_loop_slp_vectorization(true);
    options.set_loop_unrolling(true);
    options.set_loop_interleaving(true);
    options.set_verify_each(false);
    module
        .run_passes(
            "default<O3>,mergefunc,function-attrs,globaldce",
            &machine,
            options,
        )
        .map_err(|err| format!("LLVM O3 pass pipeline failed: {err}"))?;
    module
        .verify()
        .map_err(|err| format!("LLVM rejected the optimized module: {err}"))?;

    machine
        .write_to_file(module, FileType::Object, path)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn codegen_error(message: String, span: Option<Span>) -> Diagnostic {
    Diagnostic {
        phase: Phase::Codegen,
        severity: Severity::Error,
        message,
        span,
    }
}

fn not_yet_supported(what: &str, span: Option<Span>) -> Diagnostic {
    codegen_error(format!("{what} is not supported by codegen yet"), span)
}

fn schedule_error(err: ScheduleError) -> Diagnostic {
    codegen_error(format!("internal arena schedule mismatch: {err}"), None)
}

pub(super) fn region_walk_codegen_error(err: crate::region_walk::WalkError) -> Diagnostic {
    schedule_error(ScheduleError {
        message: err.as_str().into(),
    })
}

pub(super) fn resolve_walk_failure(walk: crate::region_walk::WalkError) -> Diagnostic {
    walk.diagnostic()
        .cloned()
        .unwrap_or_else(|| region_walk_codegen_error(walk))
}

impl From<BuilderError> for Diagnostic {
    fn from(err: BuilderError) -> Self {
        codegen_error(format!("LLVM builder error: {err}"), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::check;

    fn ir_of(source: &str) -> String {
        let checked = check(source);
        assert!(checked.is_ok(), "{:?}", checked.diagnostics);
        let context = Context::create();
        let module = emit_module(
            &context,
            checked.hir.as_ref().unwrap(),
            checked.report.as_ref().unwrap(),
        )
        .expect("emit");
        module.print_to_string().to_string()
    }

    fn ret_blocks_contain_arena_pop(ir: &str) {
        for block in ir.split("\n\n") {
            if block.contains(" ret ") {
                assert!(
                    block.contains("call void @bork_arena_pop("),
                    "return block must pop open arenas:\n{block}"
                );
            }
        }
    }

    #[test]
    fn codegen_arena_push_pop_counts_match_schedule() {
        use crate::codegen::regions::{RegionEvent, schedule};
        use crate::region_walk::stamp_codegen_push;

        let sources = [
            "fun main(): i32 {\n    var total = 0\n    for (i in 0..5) { total = total + i }\n    return total\n}\n",
            "fun main() {\n    val s = \"x\"\n    { println(s) }\n}\n",
            "fun main(): i32 {\n    val x = 1\n    if (x > 0) { return 1 } else { return 0 }\n}\n",
        ];
        for source in sources {
            let checked = check(source);
            assert!(checked.is_ok(), "{source:?}");
            let mut report = checked.report.unwrap();
            stamp_codegen_push(checked.hir.as_ref().unwrap(), &mut report);
            let events = schedule(checked.hir.as_ref().unwrap(), &report).expect("schedule");
            let scheduled_pushes = events
                .iter()
                .filter(|e| matches!(e, RegionEvent::Push { .. }))
                .count();
            let scheduled_pops = events
                .iter()
                .filter(|e| matches!(e, RegionEvent::Pop { .. }))
                .count();
            let ir = ir_of(source);
            assert_eq!(
                ir.matches("call ptr @bork_arena_push()").count(),
                scheduled_pushes,
                "{source}"
            );
            let ir_pops = ir.matches("call void @bork_arena_pop(").count();
            assert!(
                ir_pops >= scheduled_pops,
                "expected at least {scheduled_pops} pops, got {ir_pops} for {source}"
            );
            ret_blocks_contain_arena_pop(&ir);
        }
    }

    #[test]
    fn early_returns_pop_every_open_arena() {
        let ir = ir_of(
            "fun add(a: i32, b: i32): i32 { return a + b }\n\
             fun main(): i32 {\n\
                 val x = add(40, 2)\n\
                 if (x > 40) { return x } else { return 0 }\n\
             }\n",
        );
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 2, "{ir}");
        assert_eq!(ir.matches("call void @bork_arena_pop(").count(), 3, "{ir}");
        ret_blocks_contain_arena_pop(&ir);
    }

    #[test]
    fn early_return_and_fallthrough_both_pop_function_arena() {
        let ir = ir_of(
            "fun helper(n: i32): i32 {\n\
                 if (n <= 0) { return 0 }\n\
                 val s = \".\"\n\
                 return n\n\
             }\n\
             fun main(): i32 { return helper(1) }\n",
        );
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 2, "{ir}");
        let helper_body = ir
            .split("define internal i32 @bork.helper")
            .nth(1)
            .or_else(|| ir.split("define i32 @bork.helper").nth(1))
            .expect("helper IR")
            .split("\ndefine ")
            .next()
            .expect("helper IR only");
        let helper_pops = helper_body.matches("call void @bork_arena_pop(").count();
        assert_eq!(helper_pops, 2, "early return and final return each pop:\n{ir}");
        ret_blocks_contain_arena_pop(helper_body);
    }

    #[test]
    fn fallthrough_pops_nested_then_function_arena() {
        let ir = ir_of(
            "fun main() {\n    val a = 1\n    {\n        val b = 2\n    }\n    val c = 3\n}\n",
        );
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 1, "{ir}");
        assert_eq!(ir.matches("call void @bork_arena_pop(").count(), 1, "{ir}");
    }

    fn block<'s>(ir: &'s str, label: &str) -> &'s str {
        let start = ir
            .find(&format!("\n{label}:"))
            .unwrap_or_else(|| panic!("no block {label}: {ir}"));
        let rest = &ir[start + 1..];
        &rest[..rest.find("\n\n").unwrap_or(rest.len())]
    }

    #[test]
    fn for_loop_pushes_once_resets_at_latch_and_pops_on_exit() {
        let ir = ir_of(
            "fun main(): i32 {\n    var total = 0\n    for (i in 0..5) {\n        val _bump = \".\"\n        total = total + i\n    }\n    return total\n}\n",
        );
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 2, "{ir}");
        assert_eq!(
            ir.matches("call void @bork_arena_reset(").count(),
            1,
            "{ir}"
        );
        assert_eq!(ir.matches("call void @bork_arena_pop(").count(), 2, "{ir}");
        assert!(!block(&ir, "for.header").contains("@bork_arena"), "{ir}");
        assert!(!block(&ir, "for.body").contains("@bork_arena"), "{ir}");
        assert!(
            block(&ir, "for.latch").contains("@bork_arena_reset"),
            "{ir}"
        );
        assert!(block(&ir, "for.exit").contains("@bork_arena_pop"), "{ir}");
    }

    #[test]
    fn move_copies_into_innermost_arena_and_shared_does_not() {
        let ir = ir_of(
            "fun main() {\n    val s = \"x\"\n    {\n        val _pad = \"q\"\n        println(s)\n    }\n    move {\n        val _pad = \"m\"\n        val t = move s\n        println(t)\n    }\n}\n",
        );
        assert_eq!(ir.matches("call ptr @bork_arena_alloc(").count(), 1, "{ir}");
        assert_eq!(ir.matches("@llvm.memcpy").count(), 2, "{ir}");
        assert!(ir.contains("c\"x\""), "{ir}");
        let handles: Vec<&str> = ir
            .lines()
            .filter(|line| line.contains("call ptr @bork_arena_push()"))
            .filter_map(|line| line.trim().split(' ').next())
            .collect();
        let move_arena = handles.get(2).expect("third push opens the move block");
        assert!(
            ir.contains(&format!("@bork_arena_alloc(ptr {move_arena},")),
            "{ir}"
        );
    }

    #[test]
    fn return_inside_loop_pops_loop_arena_and_skips_dead_latch_reset() {
        let ir = ir_of(
            "fun main(): i32 {\n    for (i in 0..5) {\n        val _bump = \".\"\n        return i\n    }\n    return 0\n}\n",
        );
        assert!(!block(&ir, "for.latch").contains("@bork_arena"), "{ir}");
        assert_eq!(
            block(&ir, "for.body")
                .matches("call void @bork_arena_pop(")
                .count(),
            2,
            "{ir}"
        );
    }
}
