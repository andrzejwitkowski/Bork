//! Inkwell (LLVM 23) emission for the codegen subset.

mod arena;
mod array;
mod call_abi;
mod context;
mod emit_fn;
mod binop;
mod expr;
mod managed_ref;
mod nullable;
mod presence;
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
use crate::memory::MemoryPlan;
use crate::sema::ArenaReport;
use crate::span::Span;

use arena::ArenaCalls;
use context::{native_target_machine, Codegen};
use emit_fn::Callees;

pub fn emit_module<'ctx>(
    context: &'ctx Context,
    hir: &HirProgram,
    report: &ArenaReport,
    memory_plan: MemoryPlan,
) -> Result<Module<'ctx>, Diagnostic> {
    if !hir.functions.iter().any(|function| function.name == "main") {
        return Err(codegen_error(
            "`fun main` is required to build an executable".into(),
            None,
        ));
    }

    let cx = Codegen::new(context, "bork", &hir.classes, memory_plan)
        .map_err(|message| codegen_error(message, None))?;
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

pub(super) fn not_yet_supported(what: &str, span: Option<Span>) -> Diagnostic {
    codegen_error(format!("{what} is not supported by codegen yet"), span)
}

fn schedule_error(err: ScheduleError) -> Diagnostic {
    codegen_error(format!("internal arena schedule mismatch: {err}"), None)
}

pub(super) fn region_walk_codegen_error(err: crate::region_walk::WalkError) -> Diagnostic {
    resolve_walk_failure(err)
}

pub(super) fn resolve_walk_failure(walk: crate::region_walk::WalkError) -> Diagnostic {
    let fallback = format!("internal arena schedule mismatch: {}", walk.as_str());
    walk.into_diagnostic()
        .unwrap_or_else(|| codegen_error(fallback, None))
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
            checked.memory_plan.unwrap(),
        )
        .expect("emit");
        module.print_to_string().to_string()
    }

    fn assert_presence_drops_temporary_after_observe(ir: &str) {
        let observe = ir
            .find("call void @bork_ref_observe_out(")
            .expect("presence observation");
        let drop = ir[observe..]
            .find("call void @bork_ref_drop_value_ptr(")
            .map(|offset| observe + offset)
            .expect("temporary Ref drop");
        let branch = ir[drop..]
            .find("br i1")
            .map(|offset| drop + offset)
            .expect("presence branch");
        assert!(observe < drop && drop < branch, "{ir}");
    }

    #[test]
    fn float_optional_none_lowers() {
        let _ = ir_of("fun main(): i32 {\n    val n: f32? = None\n    return 0\n}\n");
    }

    #[test]
    fn nullable_primitive_equality_lowers() {
        for (ty, comparison) in [
            ("i32", "icmp eq i32"),
            ("f32", "fcmp oeq float"),
            ("f64", "fcmp oeq double"),
        ] {
            let ir = ir_of(&format!(
                "fun equal(a: {ty}?, b: {ty}?): bool {{ return a == b }}\n\
                 fun main(): i32 {{ return 0 }}\n"
            ));
            assert!(ir.contains(comparison), "{ir}");
        }
    }

    #[test]
    fn walk_error_without_diagnostic_falls_back_to_schedule_mismatch() {
        let err = crate::region_walk::WalkError::message("cursor underflow");
        let diagnostic = resolve_walk_failure(err);
        assert!(
            diagnostic.message.contains("arena schedule"),
            "{diagnostic:?}"
        );
        assert!(
            diagnostic.message.contains("cursor underflow"),
            "{diagnostic:?}"
        );
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
        use crate::codegen::regions::{schedule, RegionEvent};
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
        assert_eq!(
            ir.matches("call ptr @bork_arena_push()").count()
                + ir.matches("call ptr @bork_arena_push_child(").count(),
            2,
            "{ir}"
        );
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
        assert_eq!(
            ir.matches("call ptr @bork_arena_push()").count()
                + ir.matches("call ptr @bork_arena_push_child(").count(),
            2,
            "{ir}"
        );
        let helper_body = ir
            .split("define internal i32 @bork.helper")
            .nth(1)
            .or_else(|| ir.split("define i32 @bork.helper").nth(1))
            .expect("helper IR")
            .split("\ndefine ")
            .next()
            .expect("helper IR only");
        let helper_pops = helper_body.matches("call void @bork_arena_pop(").count();
        assert_eq!(
            helper_pops, 2,
            "early return and final return each pop:\n{ir}"
        );
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
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 1, "{ir}");
        assert_eq!(
            ir.matches("call ptr @bork_arena_push_child(").count(),
            1,
            "{ir}"
        );
        assert_eq!(
            ir.matches("call ptr @bork_arena_reset(").count(),
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
            .filter(|line| {
                line.contains("call ptr @bork_arena_push()")
                    || line.contains("call ptr @bork_arena_push_child(")
            })
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

    fn add_uses_if_phi_and_forty(source: &str) {
        let ir = ir_of(source);
        let add = ir
            .lines()
            .find(|line| line.contains(" add "))
            .unwrap_or_else(|| panic!("expected an add:\n{ir}"));
        assert!(add.contains("%if"), "add must use the if phi:\n{add}\n{ir}");
        assert!(
            add.contains("40"),
            "add must use the sibling operand 40, not a branch literal:\n{add}\n{ir}"
        );
        assert!(
            !ir.contains("ret i32 42"),
            "must not fold else-literal 2 + 40 to ret i32 42:\n{ir}"
        );
    }

    #[test]
    fn value_if_as_left_operand_adds_phi_not_else_literal() {
        add_uses_if_phi_and_forty("fun main(): i32 { return (if (true) { 1 } else { 2 }) + 40 }\n");
    }

    #[test]
    fn value_if_as_right_operand_adds_phi_not_else_literal() {
        add_uses_if_phi_and_forty("fun main(): i32 { return 40 + (if (true) { 1 } else { 2 }) }\n");
    }

    #[test]
    fn record_field_and_managed_ref_lower_to_arena_handles() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main(): i32 {\n\
                 val item = Box(7)\n\
                 val reference: Ref<Box> = item\n\
                 return item.value\n\
             }\n",
        );
        assert!(
            ir.contains("%bork.Box = type { i32 }"),
            "record layout:\n{ir}"
        );
        assert!(
            ir.contains("call ptr @bork_arena_alloc("),
            "record payload:\n{ir}"
        );
        assert!(
            ir.contains("call void @bork_ref_create_out("),
            "managed ref:\n{ir}"
        );
        assert!(ir.contains("getelementptr"), "field load:\n{ir}");
    }

    #[test]
    fn if_val_observes_once_and_loads_the_live_field() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main(): i32 {\n\
                 val item = Box(7)\n\
                 val reference: Ref<Box> = item\n\
                 if val live = reference { return live.value }\n\
                 return 0\n\
             }\n",
        );
        assert_eq!(
            ir.matches("call void @bork_ref_observe_out(").count(),
            1,
            "{ir}"
        );
        assert!(
            ir.contains("@bork_arena_register_observation_drop("),
            "{ir}"
        );
        assert_presence_drops_temporary_after_observe(&ir);
        assert!(ir.contains("getelementptr"), "{ir}");
    }

    #[test]
    fn when_observes_and_drops_the_temporary_ref() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main(): i32 {\n\
                 val item = Box(7)\n\
                 val reference: Ref<Box> = item\n\
                 when reference {\n\
                     Some(live) => { return live.value }\n\
                     None => { return 0 }\n\
                 }\n\
                 return 0\n\
             }\n",
        );
        assert_presence_drops_temporary_after_observe(&ir);
    }

    #[test]
    fn ref_field_assignment_stores_through_the_handle_helper() {
        let ir = ir_of(
            "class Node { next: Ref<Node> }\n\
             fun main() {\n\
                 val a = Node(None)\n\
                 val b = Node(None)\n\
                 a.next = b\n\
             }\n",
        );
        assert!(ir.contains("call void @bork_ref_store_take_ptr("), "{ir}");
        assert!(!ir.contains("@llvm.memcpy"), "{ir}");
    }

    #[test]
    fn callee_lexical_arena_is_a_child_of_the_caller() {
        let ir = ir_of(
            "fun id(n: i32): i32 { return n }\n\
             fun main(): i32 { return id(7) }\n",
        );
        assert!(
            ir.contains("define internal i32 @bork.id(ptr"),
            "{ir}"
        );
        assert!(ir.contains("call ptr @bork_arena_push_child("), "{ir}");
        assert_eq!(ir.matches("call ptr @bork_arena_push()").count(), 1, "{ir}");
    }

    #[test]
    fn returned_string_is_allocated_in_the_caller_sink() {
        let ir = ir_of(
            "fun make(): String { return \"ab\" }\n\
             fun main() {\n\
                 val s = make()\n\
                 println(s)\n\
             }\n",
        );
        assert!(
            ir.contains("define internal { ptr, i64 } @bork.make(ptr"),
            "{ir}"
        );
        let make = ir.split("@bork.make").nth(1).expect("make");
        assert!(
            make.contains("@bork_arena_alloc(ptr %1,"),
            "result bytes use the caller sink:\n{ir}"
        );
    }

    #[test]
    fn safe_navigation_observes_and_releases_before_the_value_is_used() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main(): i32 {\n\
                 val item = Box(7)\n\
                 val reference: Ref<Box> = item\n\
                 val n = reference?.value\n\
                 return n ?: 0\n\
             }\n",
        );
        assert_eq!(
            ir.matches("call void @bork_ref_observe_out(").count(),
            1,
            "{ir}"
        );
        assert!(
            ir.contains("call void @bork_ref_release_observation_ptr("),
            "{ir}"
        );
        assert_presence_drops_temporary_after_observe(&ir);
    }

    #[test]
    fn ref_elvis_uses_presence_without_unwrapping_the_handle() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main() {\n\
                 val item = Box(7)\n\
                 val missing: Ref<Box> = None\n\
                 val present = missing ?: item\n\
             }\n",
        );
        assert!(ir.contains("call i1 @bork_ref_is_none_ptr("), "{ir}");
        assert!(
            block(&ir, "ref.elvis.none").contains("call void @bork_ref_drop_value_ptr("),
            "{ir}"
        );
        assert!(!ir.contains("extractvalue { ptr, ptr, i8 } %ref.elvis, 1"), "{ir}");
    }

    #[test]
    fn loop_local_ref_is_dropped_at_each_iteration() {
        let ir = ir_of(
            "class Node { value: i32 }\n\
             fun main() {\n\
                 val refs: [Ref<Node>; 2] = [Node(1), Node(2)]\n\
                 for (i in 0..2) { val candidate = refs[i] }\n\
             }\n",
        );
        assert!(ir.contains("call ptr @bork_arena_reset("), "{ir}");
    }

    #[test]
    fn presence_copies_buffer_results_before_closing_both_arms() {
        let ir = ir_of(
            "class Holder { values: [i32; 2] }\n\
             fun choose(reference: Ref<Holder>, fallback: [i32; 2]): [i32; 2] {\n\
                 return when reference {\n\
                     Some(holder) => { holder.values }\n\
                     None => { fallback }\n\
                 }\n\
             }\n\
             fun main() {}\n",
        );
        for label in ["ref.some", "ref.none"] {
            let arm = block(&ir, label);
            assert!(arm.contains("@llvm.memcpy"), "{ir}");
            if let Some(pop) = arm.find("call void @bork_arena_pop(") {
                assert!(arm.find("@llvm.memcpy").unwrap() < pop, "{ir}");
            }
        }
    }

    #[test]
    fn presence_copies_nullable_array_results() {
        let ir = ir_of(
            "class Holder { values: [i32; 2] }\n\
             class Root { holder: Ref<Holder> }\n\
             fun choose(reference: Ref<Root>, fallback: Ref<Holder>) {\n\
                 val values = when reference {\n\
                     Some(root) => { root.holder?.values }\n\
                     None => { fallback?.values }\n\
                 }\n\
                 println(values!!.length)\n\
             }\n\
             fun main() {}\n",
        );
        // Safe navigation copies once per arm, then each result must copy out of its arm.
        assert_eq!(ir.matches("call void @llvm.memcpy").count(), 4, "{ir}");
    }

    #[test]
    fn program_ref_roots_use_the_caller_result_sink() {
        let ir = ir_of(
            "class Node { value: i32 }\n\
             fun make_inner(): Ref<Node> {\n\
                 val node = Node(7)\n\
                 return node\n\
             }\n\
             fun make(): Ref<Node> {\n\
                 val result = make_inner()\n\
                 return result\n\
             }\n\
             fun main() {\n\
                 val result = make()\n\
             }\n",
        );
        let make = ir
            .split("define internal { ptr, ptr, i8 } @bork.make_inner")
            .nth(1)
            .expect("make IR")
            .split("\ndefine ")
            .next()
            .expect("make body");
        assert!(
            make.contains("call void @bork_arena_register_strong_root(ptr %1, ptr %arena.dynamic)"),
            "{ir}"
        );
    }

    #[test]
    fn break_pops_the_nested_arena_before_the_loop_exit() {
        let ir = ir_of(
            "fun main(): i32 {\n\
                 for (i in 0..3) {\n\
                     move {\n\
                         val bump = \".\"\n\
                         break\n\
                     }\n\
                 }\n\
                 return 0\n\
             }\n",
        );
        let body = block(&ir, "for.body");
        assert!(
            body.contains("call void @bork_arena_pop("),
            "break drops the nested arena:\n{body}"
        );
        assert!(
            block(&ir, "for.exit").contains("call void @bork_arena_pop(ptr %arena.gen)"),
            "loop exit drops the loop generation once:\n{ir}"
        );
    }

    #[test]
    fn ref_array_stores_use_element_glue() {
        let ir = ir_of(
            "class Box { value: i32 }\n\
             fun main() {\n\
                 val item = Box(7)\n\
                 val first: Ref<Box> = item\n\
                 val refs = [first, None]\n\
                 val taken = refs[0]\n\
             }\n",
        );
        assert!(ir.contains("call void @bork_ref_store_take_ptr("), "{ir}");
        assert!(ir.contains("call void @bork_ref_clone_same_owner_out("), "{ir}");
        assert!(!ir.contains("@llvm.memcpy"), "{ir}");
    }

    #[test]
    fn record_ref_array_field_copies_elements_into_the_owner_arena() {
        let ir = ir_of(
            "class Node { value: i32 }\n\
             class Holder { refs: [Ref<Node>; 2] }\n\
             fun main() {\n\
                 val item = Node(7)\n\
                 val reference: Ref<Node> = item\n\
                 val refs = [reference, None]\n\
                 val holder = Holder(refs)\n\
             }\n",
        );
        assert!(ir.matches("call void @bork_ref_clone_same_owner_out(").count() >= 2, "{ir}");
        assert!(ir.matches("call void @bork_ref_store_take_ptr(").count() >= 2, "{ir}");
    }

    #[test]
    fn record_string_field_copies_bytes_into_the_owner_arena() {
        let ir = ir_of(
            "class Box { text: String }\n\
             fun main() {\n\
                 val source = \"source\"\n\
                 val box = Box(source)\n\
             }\n",
        );
        assert!(ir.contains("call ptr @bork_arena_alloc("), "{ir}");
        assert!(ir.contains("@llvm.memcpy"), "{ir}");
    }

    #[test]
    fn elvis_as_right_operand_adds_phi_not_skipped_rhs() {
        let ir = ir_of("fun main(): i32 {\n    val a: i32? = None\n    return 40 + (a ?: 2)\n}\n");
        let add = ir
            .lines()
            .find(|line| line.contains(" add "))
            .unwrap_or_else(|| panic!("expected an add:\n{ir}"));
        assert!(
            add.contains("40"),
            "add must use the sibling operand 40, not the skipped elvis rhs:\n{add}\n{ir}"
        );
        assert!(
            add.contains("elvis"),
            "add must use the elvis phi:\n{add}\n{ir}"
        );
    }
}
