use super::*;
use inkwell::{
    memory_buffer::MemoryBuffer,
    values::{CallSiteValue, InstructionOpcode},
};

#[cfg(unix)]
fn invalidated_array_report(source: &str) -> String {
    use std::{os::unix::process::ExitStatusExt, process::Command};
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
    let module = crate::codegen::llvm::emit_module_with_debug(
        &context,
        &mono.hir,
        &tcx,
        &mono.report,
        mono.memory_plan,
        source,
        Path::new("fixture.bork"),
    )
    .unwrap();
    let instructions: Vec<_> = module
        .get_function("main")
        .unwrap()
        .get_basic_blocks()
        .iter()
        .flat_map(|b| b.get_instructions())
        .collect();
    let allocation = instructions
        .iter()
        .find(|i| {
            i.get_opcode() == InstructionOpcode::Call
                && CallSiteValue::try_from(**i)
                    .unwrap()
                    .get_called_fn_value()
                    .is_some_and(|f| f.get_name().to_bytes() == b"bork_arena_alloc")
        })
        .unwrap();
    let load = instructions
        .iter()
        .find(|i| {
            i.get_opcode() == InstructionOpcode::Load
                && i.get_name().is_some_and(|n| n.to_bytes() == b"load")
        })
        .unwrap();
    let builder = context.create_builder();
    builder.position_before(&load.get_previous_instruction().unwrap());
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let free = module.add_function(
        "bork_arena_free",
        context
            .void_type()
            .fn_type(&[ptr.into(), ptr.into(), context.i64_type().into()], false),
        None,
    );
    builder
        .build_call(
            free,
            &[
                allocation.get_operand(0).unwrap().unwrap_value().into(),
                load.get_operand(0).unwrap().unwrap_value().into(),
                context.i64_type().const_int(8, false).into(),
            ],
            "",
        )
        .unwrap();
    module.verify().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let object = dir.path().join("test.o");
    let binary = dir.path().join("test");
    crate::codegen::llvm::write_object_with_debug(&module, &object, true).unwrap();
    crate::codegen::link::link_executable_with_debug(&object, &binary, true).unwrap();
    let output = Command::new(&binary).output().unwrap();
    assert_eq!(output.status.signal(), Some(6), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
#[cfg(unix)]
fn reported_replacements_keep_binding_names_and_original_declarations() {
    for (source, name, declaration) in [
        ("fun main(): i32 {\n var items = [1,2]\n items = [3,4]\n return items[0]\n}\n", "items", "var items = [1,2]"),
        ("class Holder { values: [i32;2] }\nfun main(): i32 {\n val box = Holder([1,2])\n box.values = [3,4]\n return box.values[0]\n}\n", "Holder.values", "values"),
    ] {
        let report = invalidated_array_report(source);
        assert!(report.contains(&format!("name: {name}\n")), "{report}");
        let start = source.find(declaration).unwrap();
        let expected = format!("[bytes {start}..{}]", start + declaration.len());
        assert!(report.lines().any(|l| l.contains("declared:") && l.contains(&expected)), "{report}");
    }
}

#[test]
#[cfg(unix)]
fn reported_prelude_payload_declarations_use_their_own_source_files() {
    for (body, prelude) in [
        ("val option = Some([1,2])\n val items = option!!\n return items[0]", "option.bork"),
        ("val result: Result<[i32;2], i32> = Result.Ok([1,2])\n return when result { Ok(items) => { items[0] }\n Err(error) => { error } }", "result.bork"),
    ] {
        let report = invalidated_array_report(&format!("fun main(): i32 {{\n {body}\n}}\n"));
        assert!(report.lines().any(|l| l.contains("declared:") && l.contains(&format!("/src/prelude/{prelude}:"))), "{report}");
        let text = if prelude == "option.bork" { include_str!("../../../prelude/option.bork") } else { include_str!("../../../prelude/result.bork") };
        let start = text.find("value").unwrap();
        assert!(report.lines().any(|l| l.contains("declared:") && l.contains(&format!("[bytes {start}..{}]", start + 5))), "{report}");
    }
    let source = "enum Option<T> { Some(value: T), None }\nfun main(): i32 { val option = Some([1,2])\n val items = option!!\n return items[0] }\n";
    let report = invalidated_array_report(source);
    let start = source.find("value").unwrap();
    assert!(
        report.lines().any(|l| l.contains("declared:")
            && l.contains("fixture.bork:")
            && l.contains(&format!("[bytes {start}..{}]", start + 5))),
        "{report}"
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
    let module = crate::codegen::llvm::emit_module_with_debug(
        &context,
        &mono.hir,
        &tcx,
        &mono.report,
        mono.memory_plan.clone(),
        source,
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
                .any(|i| {
                    i.get_opcode() == InstructionOpcode::Call
                        && CallSiteValue::try_from(i)
                            .unwrap()
                            .get_called_fn_value()
                            .is_some_and(|f| f.get_name().to_bytes() == b"bork_debug_check_handle")
                }),
            "{name} lacks handle identity validation"
        );
    }
    let ordinary = crate::codegen::llvm::emit_module(
        &context,
        &mono.hir,
        &tcx,
        &mono.report,
        mono.memory_plan,
    )
    .unwrap();
    assert!(!ordinary
        .print_to_string()
        .to_string()
        .contains("bork_debug_"));
}

#[test]
fn nested_expressions_keep_their_distinct_original_byte_spans() {
    let source =
        "class Box { value: i32 }\nfun main(): i32 { val item = Box(7)\n return item.value }\n";
    let checked = crate::frontend::check(source);
    let context = inkwell::context::Context::create();
    let module = crate::codegen::llvm::emit_module_with_debug(
        &context,
        checked.hir.as_ref().unwrap(),
        &checked.tcx,
        checked.report.as_ref().unwrap(),
        checked.memory_plan.unwrap(),
        source,
        Path::new("fixture.bork"),
    )
    .unwrap();
    let loads: Vec<_> = module
        .get_function("main")
        .unwrap()
        .get_basic_blocks()
        .iter()
        .flat_map(|b| b.get_instructions())
        .filter(|i| i.get_opcode() == InstructionOpcode::Load)
        .collect();
    let spans: Vec<_> = loads
        .iter()
        .map(|i| {
            let metadata = i
                .get_metadata(context.get_kind_id("bork.span"))
                .expect("original span metadata");
            let values = metadata.get_node_values().unwrap();
            let a = values[0]
                .into_int_value()
                .get_zero_extended_constant()
                .unwrap() as usize;
            let b = values[1]
                .into_int_value()
                .get_zero_extended_constant()
                .unwrap() as usize;
            &source[a..b]
        })
        .collect();
    assert!(spans.contains(&"item"), "{spans:?}");
    assert!(spans.contains(&"item.value"), "{spans:?}");
}

#[test]
#[cfg(unix)]
fn hir_provenance_reaches_reports_after_returning_and_copying_fields() {
    use std::{os::unix::process::ExitStatusExt, process::Command};
    let source = "class Holder<T> { values: T }\nfun make(): Holder<[i32; 2]> { return Holder([1,2]) }\nfun main(): i32 { val item = make()\n return item.values[0] }\n";
    for field in [false, true] {
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
        let module = crate::codegen::llvm::emit_module_with_debug(
            &context,
            &mono.hir,
            &tcx,
            &mono.report,
            mono.memory_plan,
            source,
            Path::new("fixture.bork"),
        )
        .unwrap();
        let instructions: Vec<_> = module
            .get_function("main")
            .unwrap()
            .get_basic_blocks()
            .iter()
            .flat_map(|b| b.get_instructions())
            .collect();
        let guard = instructions
            .iter()
            .find(|i| {
                i.get_opcode() == InstructionOpcode::Call
                    && CallSiteValue::try_from(**i)
                        .unwrap()
                        .get_called_fn_value()
                        .is_some_and(|f| f.get_name().to_bytes() == b"bork_debug_check_handle")
            })
            .unwrap();
        let owner = guard.get_operand(0).unwrap().unwrap_value();
        let builder = context.create_builder();
        if field {
            let load = instructions
                .iter()
                .rev()
                .find(|i| {
                    i.get_opcode() == InstructionOpcode::Load
                        && i.get_type().is_int_type()
                        && i.get_type().into_int_type().get_bit_width() == 32
                })
                .unwrap();
            builder.position_before(&load.get_previous_instruction().unwrap());
            builder
                .build_call(
                    module.get_function("bork_arena_free").unwrap_or_else(|| {
                        module.add_function(
                            "bork_arena_free",
                            context.void_type().fn_type(
                                &[
                                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                                    context.ptr_type(inkwell::AddressSpace::default()).into(),
                                    context.i64_type().into(),
                                ],
                                false,
                            ),
                            None,
                        )
                    }),
                    &[
                        owner.into(),
                        load.get_operand(0).unwrap().unwrap_value().into(),
                        context.i64_type().const_int(8, false).into(),
                    ],
                    "",
                )
                .unwrap();
        } else {
            builder.position_before(guard);
            builder
                .build_call(
                    module.get_function("bork_arena_pop").unwrap(),
                    &[owner.into()],
                    "",
                )
                .unwrap();
        }
        module.verify().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let object = dir.path().join("test.o");
        let binary = dir.path().join("test");
        crate::codegen::llvm::write_object_with_debug(&module, &object, true).unwrap();
        crate::codegen::link::link_executable_with_debug(&object, &binary, true).unwrap();
        let output = Command::new(&binary).output().unwrap();
        assert_eq!(output.status.signal(), Some(6), "{output:?}");
        let report = String::from_utf8_lossy(&output.stderr);
        let origin = if field { "[1,2]" } else { "Holder([1,2])" };
        let start = source.find(origin).unwrap();
        // Constructor HIR retains the callee name's original span.
        let end = start + if field { origin.len() } else { "Holder".len() };
        assert!(
            report.contains(&format!("[bytes {start}..{end}]")),
            "{report}"
        );
        assert!(report.contains("i32"), "{report}");
        if field {
            assert!(report.contains(".values"), "{report}");
            let start = source.find("values").unwrap();
            assert!(
                report.contains("declared: ")
                    && report.contains(&format!("[bytes {start}..{}]", start + 6)),
                "{report}"
            );
        } else {
            assert!(
                report.contains("name: item") && report.contains("type: Holder<"),
                "{report}"
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn faulty_ir_aborts_for_each_memory_operation_with_a_recursive_stack() {
    use std::{os::unix::process::ExitStatusExt, process::Command};
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
        let checked = crate::frontend::check("fun main() {}\n");
        let mut cx = Codegen::new(
            &context,
            "test",
            &[],
            &[],
            &checked.tcx,
            checked.memory_plan.unwrap(),
        )
        .unwrap();
        cx.module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "test",
            ))
            .unwrap();
        cx.debug = Some(DebugInfo::new(
            &cx,
            "fun main() {}\n",
            Path::new("fixture.bork"),
        ));
        cx.debug.as_ref().unwrap().builder.finalize();
        super::checks::instrument(&cx).unwrap();
        cx.module.verify().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let object = directory.path().join("test.o");
        let binary = directory.path().join("test");
        crate::codegen::llvm::write_object_with_debug(&cx.module, &object, true).unwrap();
        crate::codegen::link::link_executable_with_debug(&object, &binary, true).unwrap();
        let output = Command::new(&binary).output().unwrap();
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
    let checked = crate::frontend::check("fun main() {}\n");
    let mut cx = Codegen::new(
        &context,
        "test",
        &[],
        &[],
        &checked.tcx,
        checked.memory_plan.clone().unwrap(),
    )
    .unwrap();
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
    let function = cx.module.get_function("bork.drop.test").unwrap();
    let calls: Vec<_> = function
        .get_basic_blocks()
        .iter()
        .flat_map(|b| b.get_instructions())
        .collect();
    assert_eq!(
        calls
            .iter()
            .filter(|i| i.get_opcode() == InstructionOpcode::Call
                && CallSiteValue::try_from(**i)
                    .unwrap()
                    .get_called_fn_value()
                    .is_some_and(|f| f.get_name().to_bytes() == b"bork_debug_check"))
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
