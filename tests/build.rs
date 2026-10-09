#![cfg(feature = "codegen")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn bork_build(dir: &Path, source: &str) -> (std::process::Output, PathBuf) {
    let input = dir.join("main.bork");
    let output = dir.join("main");
    fs::write(&input, source).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_bork"))
        .arg("build")
        .arg("-o")
        .arg(&output)
        .arg(&input)
        .output()
        .unwrap();
    (result, output)
}

#[test]
fn builds_return_constant() {
    let dir = scratch_dir("builds_return_constant");
    let (build, binary) = bork_build(&dir, "fun main(): i32 {\n    return 7\n}\n");
    assert_eq!(
        build.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&binary).output().unwrap();
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_reject_program_without_main() {
    let dir = scratch_dir("builds_reject_program_without_main");
    let (build, binary) = bork_build(&dir, "fun helper(): i32 { return 7 }\n");
    let stderr = String::from_utf8_lossy(&build.stderr);

    assert_eq!(build.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("codegen"), "stderr: {stderr}");
    assert!(stderr.contains("main"), "stderr: {stderr}");
    assert!(!binary.exists());
}

#[test]
fn gate_rejection_exits_one_without_binary() {
    let dir = scratch_dir("gate_rejection_exits_one_without_binary");
    let (build, binary) = bork_build(
        &dir,
        "fun apply(block: (i32) -> i32): i32 { return block(1) }\n\
         fun main(): i32 { return apply() { x -> x } }\n",
    );
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("codegen"));
    assert!(!binary.exists());
}

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let dir = scratch_dir(name);
    let (build, binary) = bork_build(&dir, source);
    assert_eq!(
        build.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    Command::new(&binary).output().unwrap()
}

#[test]
fn builds_calls_locals_and_if() {
    let run = build_and_run(
        "builds_calls_locals_and_if",
        "fun add(a: i32, b: i32): i32 { return a + b }\n\
         fun main(): i32 {\n\
             val x = add(40, 2)\n\
             if (x > 40) { return x } else { return 0 }\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn builds_assignment_nested_blocks_and_i64() {
    let run = build_and_run(
        "builds_assignment_nested_blocks_and_i64",
        "fun sub(a: i64, b: i64): i64 { return a - b }\n\
         fun main(): i32 {\n\
             var total = 1\n\
             {\n\
                 val big: i64 = 100\n\
                 total = total + 6\n\
                 if (sub(big, 50) > 40) { return total * 6 }\n\
             }\n\
             return 1\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn builds_value_if_and_unit_functions() {
    let run = build_and_run(
        "builds_value_if_and_unit_functions",
        "fun noop(n: i32) { val m = n / 2 }\n\
         fun pick(flag: i32): i32 {\n\
             val v = if (flag == 1) { 40 } else { 7 }\n\
             return v + 2\n\
         }\n\
         fun main(): i32 {\n\
             noop(4)\n\
             return pick(1)\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn builds_unit_main_exits_zero() {
    let run = build_and_run(
        "builds_unit_main_exits_zero",
        "fun main() {\n    val x = 3\n}\n",
    );
    assert_eq!(run.status.code(), Some(0));
}

#[test]
fn builds_nullable_ref_navigation_and_return() {
    let run = build_and_run(
        "builds_nullable_ref_navigation_and_return",
        "class Holder {\n text: String?\n count: i32?\n }\n\
         fun missing(): String? { return None }\n\
         fun main(): i32 {\n\
             val reference: Ref<Holder> = Holder(None, None)\n\
             if (missing() == None && reference?.text == None && reference?.count == None) { return 7 }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_nullable_array_navigation_length() {
    let run = build_and_run(
        "builds_nullable_array_navigation_length",
        r#"
class Holder { values: [i32; 2] }
fun main(): i32 {
    val present: Ref<Holder> = Holder([11, 22])
    val missing: Ref<Holder> = None
    if (missing?.values?.length == None) { return present?.values?.length ?: 0 }
    return 0
}
"#,
    );
    assert_eq!(run.status.code(), Some(2));
}

#[test]
fn builds_ref_equality() {
    let run = build_and_run(
        "builds_ref_equality",
        "class Box { value: i32 }\n\
         fun main(): i32 {\n\
             val first: Ref<Box> = Box(1)\n\
             val same = first\n\
             val other: Ref<Box> = Box(1)\n\
             if (first == same && first != other && first != None) { return 7 }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_presence_result_buffers() {
    let run = build_and_run(
        "builds_presence_result_buffers",
        r#"
class Holder { values: [i32; 2] }
fun make(): Ref<Holder> { return Holder([11, 22]) }
fun main() {
    val reference = make()
    val fallback = [0, 0]
    val values = when reference {
        Some(holder) => {
            val other = make()
            when other {
                Some(inner) => { inner.values }
                None => { fallback }
            }
        }
        None => { fallback }
    }
    {
        val overwrite = [99, 99]
        println(overwrite[0])
    }
    println(values[1])
}
"#,
    );
    assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(stdout_of(&run), "99\n22\n");
}

#[test]
fn builds_ref_returned_from_if() {
    let run = build_and_run(
        "builds_ref_returned_from_if",
        "class Box { value: i32 }\n\
         fun make(flag: bool): Ref<Box> {\n\
             return if (flag) { Box(7) } else { Box(9) }\n\
         }\n\
         fun main(): i32 {\n\
             val local: Ref<Box> = if (true) { Box(5) } else { Box(6) }\n\
             val reference = make(true)\n\
             if val first = local {\n\
                 if val live = reference { return first.value + live.value }\n\
             }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(12), "{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn builds_ref_returned_after_conditional_assignment() {
    let run = build_and_run(
        "builds_ref_returned_after_conditional_assignment",
        r#"
class Box { value: i32 }
fun make(flag: bool): Ref<Box> {
    var reference: Ref<Box> = Box(7)
    if (flag) { reference = Box(9) }
    return reference
}
fun main(): i32 {
    val reference = make(false)
    if val live = reference { return live.value }
    return 0
}
"#,
    );
    assert_eq!(run.status.code(), Some(7), "{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn builds_managed_ref_copies_to_var_and_child_scope() {
    let run = build_and_run(
        "builds_managed_ref_copies_to_var_and_child_scope",
        r#"
class Box { value: i32 }
fun main(): i32 {
    val reference: Ref<Box> = Box(7)
    var copy: Ref<Box> = reference
    if (true) {
        val child = copy
        if val live = child { return live.value }
    }
    return 0
}
"#,
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_record_length_field() {
    let run = build_and_run(
        "builds_record_length_field",
        "class Count { length: i32 }\nfun main(): i32 { return Count(7).length }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_ref_assigned_out_of_loop() {
    let run = build_and_run(
        "builds_ref_assigned_out_of_loop",
        "class Box { value: i32 }\n\
         class Holder { target: Ref<Box> }\n\
         fun make(value: i32): Ref<Box> { return Box(value) }\n\
         fun main(): i32 {\n\
             var saved: Ref<Holder> = None\n\
             for (i in 0..3) {\n\
                 val reference = make(i)\n\
                 saved = Holder(reference)\n\
             }\n\
             if val holder = saved {\n\
                 if val live = holder.target { return 1 }\n\
             }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0), "{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn builds_confined_ref_cleanup() {
    let run = build_and_run(
        "builds_confined_ref_cleanup",
        "class Box { value: i32 }\n\
         fun main(): i32 {\n\
             val item = Box(7)\n\
             val reference: Ref<Box> = item\n\
             if val live = reference { return live.value }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_scoped_ref_cleanup_across_loop_reset() {
    let run = build_and_run(
        "builds_scoped_ref_cleanup_across_loop_reset",
        "class Box { value: i32 }\n\
         fun main(): i32 {\n\
             var total = 0\n\
             for (i in 0..2) {\n\
                 val item = Box(7)\n\
                 val reference: Ref<Box> = item\n\
                 if val live = reference { total = total + live.value }\n\
             }\n\
             return total\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(14));
}

#[test]
fn builds_returned_ref_to_record_argument() {
    let run = build_and_run(
        "builds_returned_ref_to_record_argument",
        "class Node { value: i32 }\n\
         fun keep(node: Node): Ref<Node> { return node }\n\
         fun main(): i32 {\n\
             val node = Node(7)\n\
             val reference = keep(node)\n\
             if val live = reference { return live.value }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_record_owned_field_payloads() {
    let run = build_and_run(
        "builds_record_owned_field_payloads",
        "class Holder { text: String\n\
         values: [i32; 2] }\n\
         fun make(): Ref<Holder> {\n\
             val text = concat(\"a\", \"b\")\n\
             val values = [11, 22]\n\
             val holder = Holder(text, values)\n\
             return holder\n\
         }\n\
         fun main() {\n\
             val reference = make()\n\
             if val holder = reference {\n\
                 println(holder.text)\n\
                 println(holder.values[1])\n\
             }\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "ab\n22\n");
}

#[test]
fn builds_record_owned_field_assignment_payloads() {
    let run = build_and_run(
        "builds_record_owned_field_assignment_payloads",
        "class Holder { text: String\n\
         values: [i32; 2] }\n\
         fun main() {\n\
             val holder = Holder(\"init\", [0, 0])\n\
             {\n\
                 holder.text = concat(\"x\", \"y\")\n\
                 holder.values = [11, 22]\n\
             }\n\
             println(holder.text)\n\
             println(holder.values[1])\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "xy\n22\n");
}

#[test]
fn builds_record_ref_array_owned_payload() {
    let run = build_and_run(
        "builds_record_ref_array_owned_payload",
        "class Node { value: i32 }\n\
         class Holder { refs: [Ref<Node>; 2] }\n\
         fun make(): Ref<Holder> {\n\
             val node = Node(7)\n\
             val reference: Ref<Node> = node\n\
             val refs = [reference, None]\n\
             val holder = Holder(refs)\n\
             return holder\n\
         }\n\
         fun main(): i32 {\n\
             val reference = make()\n\
             if val holder = reference {\n\
                 val candidate = holder.refs[0]\n\
                 if val node = candidate { return node.value }\n\
             }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn build_rejects_invalid_managed_ref_targets() {
    for (name, source) in [
        ("rejects_ref_string", "fun main() { val x: Ref<String> = None }\n"),
        (
            "rejects_ref_array",
            "fun main() { val x: Ref<[i32; 2]> = None }\n",
        ),
        (
            "rejects_ref_nullable_class",
            "class Box { value: i32 }\nfun main() { val x: Ref<Box?> = None }\n",
        ),
    ] {
        let dir = scratch_dir(name);
        let (build, binary) = bork_build(&dir, source);
        let stderr = String::from_utf8_lossy(&build.stderr);
        assert_eq!(build.status.code(), Some(1), "stderr: {stderr}");
        assert!(
            stderr.contains("managed Ref target must be a non-nullable class"),
            "stderr: {stderr}"
        );
        assert!(!binary.exists());
    }
}

#[test]
fn build_rejects_invalid_safe_class_field() {
    for (name, source) in [
        (
            "rejects_safe_owned_class",
            "class Box { value: i32 }\nfun main() {\n val box = Box(1)\n box?.value\n}\n",
        ),
        (
            "rejects_safe_nullable_class",
            "class Box { value: i32 }\nfun main() {\n val box: Box? = None\n box?.value\n}\n",
        ),
    ] {
        let dir = scratch_dir(name);
        let (build, binary) = bork_build(&dir, source);
        let stderr = String::from_utf8_lossy(&build.stderr);
        assert_eq!(build.status.code(), Some(1), "stderr: {stderr}");
        assert!(stderr.contains("?. on a class field requires Ref<T>"), "stderr: {stderr}");
        assert!(!binary.exists());
    }
}

#[test]
fn builds_safe_ref_navigation_cleanup() {
    let run = build_and_run(
        "builds_safe_ref_navigation_cleanup",
        "class Box { value: i32 }\n\
         fun main(): i32 {\n\
             val item = Box(7)\n\
             val reference: Ref<Box> = item\n\
             val value = reference?.value\n\
             return value ?: 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn builds_for_loop_sum() {
    let run = build_and_run(
        "builds_for_loop_sum",
        "fun main(): i32 {\n\
             var total = 0\n\
             for (i in 0..5) {\n\
                 total = total + i\n\
             }\n\
             return total\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(10));
}

#[test]
fn builds_nested_loops_with_regions_and_early_return() {
    let run = build_and_run(
        "builds_nested_loops_with_regions_and_early_return",
        "fun main(): i32 {\n\
             var total = 0\n\
             for (i in 0..4) {\n\
                 for (j in i..4) {\n\
                     if (j > i) { total = total + 1 } else { { total = total + 0 } }\n\
                 }\n\
             }\n\
             for (k in 0..100) {\n\
                 if (k == 3) { return total * 10 + k }\n\
             }\n\
             return 0\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(63));
}

fn stdout_of(run: &std::process::Output) -> &str {
    std::str::from_utf8(&run.stdout).unwrap()
}

#[test]
fn builds_println_literal() {
    let run = build_and_run(
        "builds_println_literal",
        "fun main() {\n    println(\"hi\")\n}\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "hi\n");
}

#[test]
fn builds_shared_read_from_nested_block() {
    let run = build_and_run(
        "builds_shared_read_from_nested_block",
        "fun main() {\n\
             val s = \"x\"\n\
             {\n\
                 println(s)\n\
             }\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "x\n");
}

#[test]
fn builds_move_then_print_strings_and_ints() {
    let run = build_and_run(
        "builds_move_then_print_strings_and_ints",
        "fun main() {\n\
             var s = \"ab\"\n\
             val t = move s\n\
             println(t)\n\
             print(4)\n\
             println(2)\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "ab\n42\n");
}

#[test]
fn builds_trailing_print_without_newline_is_flushed() {
    let run = build_and_run(
        "builds_trailing_print_without_newline_is_flushed",
        "fun main() {\n\
             println(1)\n\
             print(\"tail\")\n\
             print(7)\n\
         }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "1\ntail7");
}

#[test]
fn build_rejects_mvp_sample_at_codegen_gate() {
    let dir = scratch_dir("build_rejects_mvp_sample_at_codegen_gate");
    let (build, binary) = bork_build(&dir, bork::MVP_SAMPLE);
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert_eq!(build.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("codegen"), "stderr: {stderr}");
    assert!(stderr.contains("not supported"), "stderr: {stderr}");
    assert!(!binary.exists());
}

/// Requires `cargo test --features codegen` and LLVM 23 (see CI `bundle-llvm` smoke).
#[test]
fn builds_reference_param_in_while() {
    let run = build_and_run(
        "builds_reference_param_in_while",
        "fun bump(buf: &[i32; 2]) {\n\
            buf[0] = 9\n\
        }\n\
        fun main(): i32 {\n\
            var a: [i32; 2] = [1, 2]\n\
            var i = 0\n\
            while (i < 1) {\n\
                bump(&a)\n\
                i = i + 1\n\
            }\n\
            return a[0]\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(9));
}

#[test]
fn builds_index_assign_i32() {
    let run = build_and_run(
        "builds_index_assign_i32",
        "fun main(): i32 {\n\
            var a: [i32; 3] = [1, 2, 3]\n\
            var i = 1\n\
            a[i] = 9\n\
            return a[1]\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(9));
}

#[test]
fn builds_index_assign_moved_string() {
    let run = build_and_run(
        "builds_index_assign_moved_string",
        "fun main() {\n\
            var a: [String; 2] = [\"a\", \"b\"]\n\
            var s = \"z\"\n\
            a[0] = move s\n\
            println(a[0])\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout_of(&run), "z\n");
}

#[test]
fn builds_array_index_and_slice() {
    let run = build_and_run(
        "builds_array_index_and_slice",
        "fun main(): i32 {\n\
            val a = [10, 20, 30]\n\
            val b = a[0..2]\n\
            return b[1]\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(20));
}

#[test]
fn build_rejects_returning_string_from_inner_region() {
    let dir = scratch_dir("build_rejects_returning_string_from_inner_region");
    let source = "fun mk(): String {\n    var s = \"esc\"\n    {\n        val x = move s\n        return x\n    }\n}\n\
                  fun main() {\n    println(mk())\n}\n";
    let (build, binary) = bork_build(&dir, source);
    let stderr = String::from_utf8_lossy(&build.stderr);

    assert_eq!(build.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("inner region"),
        "stderr: {stderr}"
    );
    assert!(!binary.exists());
}

#[test]
fn builds_logical_short_circuit() {
    let run = build_and_run(
        "builds_logical_short_circuit",
        "fun side(): i32 { return 1 }\n\
         fun main(): i32 {\n\
            var n: i32 = 0\n\
            if (false && side() == 1) { n = 1 }\n\
            if (true || side() == 1) { return n }\n\
            return 99\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(0));
}

#[test]
fn builds_while_with_break() {
    let run = build_and_run(
        "builds_while_with_break",
        "fun main(): i32 {\n\
            var i: i32 = 0\n\
            while (i < 10) {\n\
                if (i == 3) { break }\n\
                i = i + 1\n\
            }\n\
            return i\n\
        }\n",
    );
    assert_eq!(run.status.code(), Some(3));
}

#[test]
fn builds_chained_presence_short_circuit_and_single_evaluation() {
    for fail in 0..=3 {
        let source = format!(
            r#"
class Node {{ value: i32 }}
fun source(step: i32, fail: i32): Ref<Node> {{
    println(step)
    if (step == fail) {{ return None }}
    return Node(step)
}}
fun main() {{
    if val (a = source(1, {fail}), b = source(2, {fail}), c = source(3, {fail})) {{
        println(a.value + b.value + c.value)
    }} else {{ println(9) }}
}}
"#
        );
        let run = build_and_run(&format!("chained_short_circuit_{fail}"), &source);
        assert!(run.status.success(), "{:?}", run);
        let expected = match fail {
            0 => "1\n2\n3\n6\n",
            1 => "1\n9\n",
            2 => "1\n2\n9\n",
            _ => "1\n2\n3\n9\n",
        };
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected);
    }
}

#[test]
fn builds_chained_presence_loops_and_owned_results() {
    let run = build_and_run(
        "chained_loops_owned",
        r#"
class Node {
    name: String
    next: Ref<Node>
}
fun main(): i32 {
    val r: Ref<Node> = Node("live")
    if val start = r { start.next = r }
    var n = 0
    while (n < 20) {
        n = n + 1
        if val (a = r, b = a.next) {
            if (n < 19) { continue }
            break
        }
    }
    val text: String = if val (a = r, b = a.next) { b.name } else { "missing" }
    println(text)
    val missing: Ref<Node> = None
    val refs: Ref<Node> = if val (a = r, b = a.next) { b.next } else { missing }
    if val (a = refs, b = a.next) { return n + 23 }
    return 0
}
"#,
    );
    assert_eq!(run.status.code(), Some(42), "{:?}", run);
    assert_eq!(String::from_utf8_lossy(&run.stdout), "live\n");
}

#[test]
fn builds_chained_presence_nested_rhs_and_early_returns() {
    for early in [false, true] {
        let source = format!(
            r#"
class Node {{ value: i32 }}
fun main(): i32 {{
    val r: Ref<Node> = Node(42)
    if val (a = r, b = if ({early}) {{ return 7
 r }} else {{ r }}) {{ return b.value }} else {{ return 0 }}
    return 0
}}
"#
        );
        let run = build_and_run(&format!("chained_rhs_return_{early}"), &source);
        assert_eq!(run.status.code(), Some(if early { 7 } else { 42 }));
    }
}

#[test]
fn builds_chained_presence_releases_partial_pins_before_else() {
    let run = build_and_run(
        "chained_partial_cleanup",
        r#"
class Node { parent: Ref<Node> }
class Holder { saved: Ref<Node> }
fun clear(holder: &Holder): Ref<Node> {
    holder.saved = None
    val missing: Ref<Node> = None
    return missing
}
fun main(): i32 {
    val holder = Holder()
    var child_ref: Ref<Node> = None
    {
        val target = Node()
        holder.saved = target
        child_ref = Node(target)
    }
    // The target's initial root lease ended with the inner scope; holder owns it.
    if val (a = holder.saved, b = clear(&holder)) { return 1 } else {
        if val (child = child_ref, parent = child.parent) { return 2 }
    }
    return 42
}
"#,
    );
    assert_eq!(run.status.code(), Some(42), "{:?}", run);
}

#[test]
fn builds_chained_presence_pin_survives_source_replacement() {
    let run = build_and_run(
        "chained_pin_replacement",
        r#"
class Node { value: i32 }
class Holder { saved: Ref<Node> }
fun make(n: i32): Ref<Node> { return Node(n) }
fun replace(holder: &Holder): Ref<Node> {
    holder.saved = make(2)
    return holder.saved
}
fun main(): i32 {
    val holder = Holder()
    {
        val old = Node(40)
        holder.saved = old
    }
    if val (a = holder.saved, b = replace(&holder)) { return a.value + b.value }
    return 0
}
"#,
    );
    assert_eq!(run.status.code(), Some(42), "{:?}", run);
}

#[test]
fn builds_chained_presence_unwinds_loop_exits_from_each_rhs() {
    for stage in 0..3 {
        for action in ["break", "continue"] {
            let mut values = ["r".to_string(), "r".to_string(), "r".to_string()];
            values[stage] = format!("if (n == 1) {{ {action}\n r }} else {{ r }}");
            let source = format!(
                r#"
class Node {{ value: i32 }}
fun main(): i32 {{
    val r: Ref<Node> = Node(40)
    var n = 0
    while (n < 2) {{
        n = n + 1
        if val (a = {}, b = {}, c = {}) {{ println(c.value) }}
    }}
    return n
}}
"#,
                values[0], values[1], values[2]
            );
            let run = build_and_run(&format!("chained_rhs_{stage}_{action}"), &source);
            assert_eq!(
                run.status.code(),
                Some(if action == "break" { 1 } else { 2 })
            );
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                if action == "break" { "" } else { "40\n" }
            );
        }
    }
}

#[test]
fn builds_chained_presence_array_result_survives_guard_cleanup() {
    let run = build_and_run("chained_array_result", r#"
class Node {
    values: [i32; 2]
    next: Ref<Node>
}
fun main(): i32 {
    val r: Ref<Node> = Node([20, 22])
    if val a = r { a.next = r }
    val values: [i32; 2] = if val (a = r, b = a.next) { b.values } else { [0, 0] }
    return values[0] + values[1]
}
"#);
    assert_eq!(run.status.code(), Some(42), "{:?}", run);
}

#[test]
fn builds_presence_values_preserve_surrounding_operands() {
    for (form, guard) in [
        ("single", "if val a = r { 20 } else { 30 }"),
        ("chain", "if val (a = r, b = r) { 20 } else { 30 }"),
        ("when", "when r {\n Some(a) => { 20 }\n None => { 30 }\n}"),
    ] {
        for (present, initializer, want) in [(true, "Node()", 30), (false, "None", 40)] {
            for (context, expression, expected) in [
                ("add", format!("10 + {guard}"), want),
                ("call", format!("add(10, {guard})"), want),
                ("array", format!("[10, {guard}][0]"), 10),
            ] {
                let source = format!("class Node {{}}\nfun add(a: i32, b: i32): i32 {{ return a + b }}\nfun main(): i32 {{\n val r: Ref<Node> = {initializer}\n return {expression}\n}}");
                let run = build_and_run(
                    &format!("presence_operands_{form}_{present}_{context}"),
                    &source,
                );
                assert_eq!(run.status.code(), Some(expected), "{source}\n{run:?}");
            }
        }
    }
}
