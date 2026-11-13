use super::*;
use inkwell::{
    memory_buffer::MemoryBuffer,
    values::{CallSiteValue, InstructionOpcode, InstructionValue},
};

fn calls_function(instruction: InstructionValue<'_>, name: &str) -> bool {
    CallSiteValue::try_from(instruction)
        .ok()
        .and_then(|call| call.get_called_fn_value())
        .is_some_and(|function| function.get_name().to_bytes() == name.as_bytes())
}

fn debug_module<'ctx>(
    context: &'ctx inkwell::context::Context,
    source: &str,
) -> inkwell::module::Module<'ctx> {
    let checked = crate::frontend::check(source);
    assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    let mut tcx = checked.tcx.clone();
    let mono = crate::mono::monomorphize(
        checked.hir.as_ref().unwrap(),
        &mut tcx,
        checked.report.as_ref().unwrap(),
    )
    .unwrap();
    crate::codegen::llvm::emit_module_with_debug(context, &mono, &tcx, source,
        Path::new("fixture.bork"),
    )
    .unwrap()
}

fn instrumented_ir<'ctx>(context: &'ctx inkwell::context::Context, ir: &[u8]) -> Codegen<'ctx> {
    let checked = crate::frontend::check("fun main() {}\n");
    let mut cx = Codegen::new(
        context,
        "test",
        &[],
        &[],
        &checked.tcx,
    )
    .unwrap();
    cx.module = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(ir, "test"))
        .unwrap();
    cx.debug = Some(DebugInfo::new(
        &cx,
        "fun main() {}\n",
        Path::new("fixture.bork"),
    ));
    cx.debug.as_ref().unwrap().builder.finalize();
    super::checks::instrument(&cx).unwrap();
    cx.module.verify().unwrap();
    cx
}

#[test]
fn known_stack_and_global_addresses_skip_checks_but_loaded_pointers_do_not() {
    let context = inkwell::context::Context::create();
    let cx = instrumented_ir(
        &context,
        br#"
        @data = global [2 x i32] zeroinitializer
        @heap = global ptr null
        declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)
        declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)
        define void @access(ptr %argument, i64 %index, i1 %condition) {
          %stack = alloca [2 x i32]
          %local = getelementptr [2 x i32], ptr %stack, i64 0, i64 %index
          %global = getelementptr [2 x i32], ptr @data, i64 0, i64 %index
          store i32 1, ptr %local
          %a = load i32, ptr %global
          store i32 %a, ptr getelementptr ([2 x i32], ptr @data, i64 0, i64 1)
          %pointer = load ptr, ptr @heap
          %b = load i32, ptr %pointer
          store i32 %b, ptr %argument
          %mixed = select i1 %condition, ptr %stack, ptr %argument
          %c = load i32, ptr %mixed
          %d = load i32, ptr null
          call void @llvm.memcpy.p0.p0.i64(ptr %local, ptr %argument, i64 4, i1 false)
          call void @llvm.memset.p0.i64(ptr %local, i8 0, i64 4, i1 false)
          ret void
        }
    "#,
    );
    let checks = cx
        .module
        .get_function("access")
        .unwrap()
        .get_first_basic_block()
        .unwrap()
        .get_instructions()
        .filter(|i| calls_function(*i, "bork_debug_check"))
        .count();
    assert_eq!(checks, 5);
}

#[test]
fn indirect_calls_set_their_own_source_site() {
    let context = inkwell::context::Context::create();
    let cx = instrumented_ir(
        &context,
        br#"
        define void @caller(ptr %callback) {
          call void %callback(), !bork.span !0
          ret void
        }
        !0 = !{i64 4, i64 8}
    "#,
    );
    let call = cx
        .module
        .get_function("caller")
        .unwrap()
        .get_first_basic_block()
        .unwrap()
        .get_instructions()
        .find(|i| {
            i.get_opcode() == InstructionOpcode::Call
                && CallSiteValue::try_from(*i)
                    .unwrap()
                    .get_called_fn_value()
                    .is_none()
        })
        .unwrap();
    let site = call.get_previous_instruction().unwrap();
    assert!(calls_function(site, "bork_debug_site"));
    assert!(!site
        .get_operand(0)
        .unwrap()
        .unwrap_value()
        .into_pointer_value()
        .is_null());
}

#[cfg(unix)]
fn run_module(module: &inkwell::module::Module<'_>) -> std::process::Output {
    module.verify().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let object = dir.path().join("test.o");
    let binary = dir.path().join("test");
    crate::codegen::llvm::write_object_with_debug(module, &object, true).unwrap();
    crate::codegen::link::link_executable_with_debug(&object, &binary, true).unwrap();
    std::process::Command::new(&binary).output().unwrap()
}

#[test]
fn reads_do_not_relabel_the_input_allocation() {
    let source = "fun read(items: [i32;2]): i32 { return items[0] }\nfun main(): i32 { val items = [1,2]\n return read(move items) }\n";
    let context = inkwell::context::Context::create();
    let module = debug_module(&context, source);
    let function = module.get_function("bork.read").unwrap();
    assert!(
        !function
            .get_basic_blocks()
            .iter()
            .flat_map(|b| b.get_instructions())
            .any(|i| { calls_function(i, "bork_debug_label_allocation") }),
        "reading an array parameter must not relabel its allocation"
    );
}

#[test]
fn field_declaration_spans_survive_specialization() {
    let source = "class Holder<T> { values: T }\nfun main() { val h = Holder([1, 2]) }\n";
    let checked = crate::frontend::check(source);
    assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    let mut tcx = checked.tcx.clone();
    let mono = crate::mono::monomorphize(
        checked.hir.as_ref().unwrap(),
        &mut tcx,
        checked.report.as_ref().unwrap(),
    )
    .unwrap();
    let field = &mono
        .hir
        .classes
        .iter()
        .find(|c| c.generics.is_empty())
        .unwrap()
        .fields[0];
    assert_eq!(&source[field.span.start..field.span.end], "values");
}

#[test]
fn class_enum_and_option_unpacking_check_arena_identity() {
    let source = "class Box { value: i32 }\nenum Slot { Empty, Held(value: i32) }\n\
        fun classRead(b: &Box): i32 { return b.value }\n\
        fun enumRead(e: &Slot): i32 { return when e { Slot.Empty => { 0 }\n Slot.Held(v) => { v } } }\n\
        fun optionRead(n: i32?): i32 { return n ?: 0 }\n\
        fun main(): i32 { val b = Box(1)\n val e = Slot.Held(2)\n return classRead(&b) + enumRead(&e) + optionRead(Some(3)) }\n";
    let checked = crate::frontend::check(source);
    assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    let mut tcx = checked.tcx.clone();
    let mono = crate::mono::monomorphize(
        checked.hir.as_ref().unwrap(),
        &mut tcx,
        checked.report.as_ref().unwrap(),
    )
    .unwrap();
    let context = inkwell::context::Context::create();
    let module = crate::codegen::llvm::emit_module_with_debug(&context, &mono, &tcx, source,
        Path::new("fixture.bork"),
    )
    .unwrap();
    for name in ["classRead", "enumRead", "optionRead"] {
        let function = module.get_function(&format!("bork.{name}")).unwrap();
        assert!(
            function
                .get_basic_blocks()
                .iter()
                .flat_map(|b| b.get_instructions())
                .any(|i| { calls_function(i, "bork_debug_check_handle") }),
            "{name} lacks handle identity validation"
        );
    }
    let ordinary = crate::codegen::llvm::emit_module(&context, &mono, &tcx)
    .unwrap();
    assert!(!ordinary
        .print_to_string()
        .to_string()
        .contains("bork_debug_"));
}

#[test]
#[cfg(unix)]
fn faulty_ir_aborts_for_each_memory_operation_with_a_recursive_stack() {
    use std::os::unix::process::ExitStatusExt;
    for (operation, expected) in [
        ("%x = load i32, ptr %p", Some("read")),
        ("store i32 1, ptr %p", Some("write")),
        (
            "call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %p, i64 4, i1 false)",
            Some("read"),
        ),
        (
            "call void @llvm.memmove.p0.p0.i64(ptr %p, ptr %q, i64 4, i1 false)",
            Some("write"),
        ),
        (
            "call void @llvm.memset.p0.i64(ptr %p, i8 0, i64 4, i1 false)",
            Some("write"),
        ),
        (
            "call void @llvm.memcpy.p0.p0.i64(ptr %p, ptr %p, i64 0, i1 false)",
            None,
        ),
    ] {
        let ir = format!(
            r#"
            @path = private constant [13 x i8] c"fixture.bork\00"
            @name = private constant [5 x i8] c"item\00"
            @type = private constant [10 x i8] c"Node<i32>\00"
            @site = private constant {{ ptr, i64, i64, i64, i64 }} {{ ptr @path, i64 12, i64 27, i64 2, i64 5 }}
            declare ptr @bork_arena_push()
            declare ptr @bork_arena_alloc(ptr, i64, i64)
            declare void @bork_arena_free(ptr, ptr, i64)
            declare void @bork_debug_site(ptr)
            declare void @bork_debug_label_allocation(ptr, ptr, ptr, ptr, i8)
            declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)
            declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1)
            declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)
            define i32 @main() {{
                call void @bork_debug_site(ptr @site)
                %a = call ptr @bork_arena_push()
                %p = call ptr @bork_arena_alloc(ptr %a, i64 64, i64 8)
                %q = call ptr @bork_arena_alloc(ptr %a, i64 64, i64 8)
                call void @bork_debug_label_allocation(ptr %p, ptr @name, ptr @type, ptr @site, i8 1)
                call void @recursive(ptr %a, ptr %p, ptr %q, i32 2)
                ret i32 0
            }}
            define void @recursive(ptr %a, ptr %p, ptr %q, i32 %n) {{
                %recurse = icmp ne i32 %n, 0
                br i1 %recurse, label %again, label %access
            again:
                %next = sub i32 %n, 1
                call void @recursive(ptr %a, ptr %p, ptr %q, i32 %next)
                ret void
            access:
                call void @bork_arena_free(ptr %a, ptr %p, i64 64)
                {operation}
                ret void
            }}
        "#
        );
        let context = inkwell::context::Context::create();
        let cx = instrumented_ir(&context, ir.as_bytes());
        let output = run_module(&cx.module);
        if let Some(expected) = expected {
            assert_eq!(output.status.signal(), Some(6), "{operation}: {output:?}");
            let report = String::from_utf8_lossy(&output.stderr);
            for text in [
                expected,
                "Freed",
                "item",
                "Node<i32>",
                "fixture.bork:2:5 [bytes 12..27]",
            ] {
                assert!(
                    report.contains(text),
                    "{operation}: missing {text}: {report}"
                );
            }
            assert_eq!(report.matches("recursive at").count(), 3, "{report}");
        } else {
            assert!(output.status.success(), "{output:?}");
        }
    }
}

#[test]
fn every_memory_access_including_glue_is_checked_before_execution() {
    let context = inkwell::context::Context::create();
    let ir = br#"
        declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)
        declare void @llvm.memmove.p0.p0.i64(ptr, ptr, i64, i1)
        declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)
        define void @bork.drop.test(ptr %p, ptr %q, i64 %n) {
          %v = load i32, ptr %p
          store i32 %v, ptr %q
          call void @llvm.memcpy.p0.p0.i64(ptr %q, ptr %p, i64 %n, i1 false)
          call void @llvm.memmove.p0.p0.i64(ptr %q, ptr %p, i64 %n, i1 false)
          call void @llvm.memset.p0.i64(ptr %q, i8 0, i64 %n, i1 false)
          ret void
        }
    "#;
    let cx = instrumented_ir(&context, ir);
    let function = cx.module.get_function("bork.drop.test").unwrap();
    let calls: Vec<_> = function
        .get_basic_blocks()
        .iter()
        .flat_map(|b| b.get_instructions())
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|i| calls_function(**i, "bork_debug_check"))
            .count(),
        7
    );
    for (index, instruction) in calls.iter().enumerate() {
        let memory = matches!(
            instruction.get_opcode(),
            InstructionOpcode::Load | InstructionOpcode::Store
        ) || (instruction.get_opcode() == InstructionOpcode::Call
            && CallSiteValue::try_from(*instruction)
                .unwrap()
                .get_called_fn_value()
                .is_some_and(|f| f.get_name().to_bytes().starts_with(b"llvm.mem")));
        if memory {
            let before = CallSiteValue::try_from(calls[index - 1])
                .unwrap()
                .get_called_fn_value()
                .unwrap();
            assert_eq!(before.get_name().to_bytes(), b"bork_debug_check");
        }
    }
}
