#![cfg(feature = "codegen")]

use std::fs;
use std::process::Command;

#[test]
fn debug_build_emits_source_info_and_registry_without_changing_program_result() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "class Box { value: i32 }\nfun main(): i32 {\n    val item = Box(7)\n    val reference: Ref<Box> = item\n    return reference?.value ?: 0\n}\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(7));
    let sections = Command::new("readelf")
        .args(["-S", "--wide"])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(sections.status.success());
    let sections = String::from_utf8_lossy(&sections.stdout);
    assert!(sections.contains(".debug_info"), "{sections}");
    assert!(sections.contains(".debug_line"), "{sections}");
    let symbols = Command::new("nm").arg(&binary).output().unwrap();
    assert!(symbols.status.success());
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    assert!(symbols.contains("bork_debug_registry"), "{symbols}");
}

#[test]
fn debug_api_requires_source_context() {
    use bork::codegen::{build_with_options, BuildOptions};
    let checked = bork::frontend::check("fun main() {}\n");
    let dir = tempfile::tempdir().unwrap();
    let result = build_with_options(
        &checked,
        &dir.path().join("main"),
        BuildOptions { debug: true },
        None,
    );
    assert!(result.is_err());
    assert!(!dir.path().join("main").exists());
}

#[test]
fn debug_build_rejects_a_runtime_without_the_debug_abi() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "fun main() {}\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .env("BORK_RUNTIME_DEBUG_LIB", env!("BORK_RUNTIME_LIB"))
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(2));
    assert!(!binary.exists());
    let errors = String::from_utf8_lossy(&build.stderr);
    assert!(errors.contains("bork_debug_abi_v1"), "{errors}");
}

#[test]
fn ordinary_build_does_not_include_debug_registry() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "fun main(): i32 { return 7 }\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(7));
    let symbols = Command::new("nm").arg(&binary).output().unwrap();
    assert!(symbols.status.success());
    assert!(!String::from_utf8_lossy(&symbols.stdout).contains("bork_debug_registry"));
}
