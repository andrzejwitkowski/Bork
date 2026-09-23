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
