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
    let (build, binary) = bork_build(&dir, "fun main(): i32 { return \"x\".length }\n");
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
