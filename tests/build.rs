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
fn gate_rejection_exits_one_without_binary() {
    let dir = scratch_dir("gate_rejection_exits_one_without_binary");
    let (build, binary) = bork_build(&dir, "fun main(): i32 { return \"x\".length }\n");
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("codegen"));
    assert!(!binary.exists());
}
