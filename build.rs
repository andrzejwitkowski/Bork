use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/parser.lalrpop");
    lalrpop::process_src().unwrap();
    if env::var_os("CARGO_FEATURE_CODEGEN").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        build_runtime(false);
        build_runtime(true);
    }
}

/// Compiles `bork_runtime` with plain `rustc` so `bork build` can link it into user programs.
/// The runtime has no dependencies; a nested `cargo` would contend for the outer build's locks.
fn build_runtime(debug: bool) {
    let source = PathBuf::from("crates/bork_runtime/src/lib.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=crates/bork_runtime/src/arena.rs");
    println!("cargo:rerun-if-changed=crates/bork_runtime/src/refs.rs");
    println!("cargo:rerun-if-changed=crates/bork_runtime/src/text.rs");
    println!("cargo:rerun-if-changed=crates/bork_runtime/src/arena/debug");
    let filename = if debug {
        "libbork_runtime_debug.a"
    } else {
        "libbork_runtime.a"
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(filename);
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let mut command = Command::new(rustc);
    command.args([
        "--crate-name",
        "bork_runtime",
        "--crate-type",
        "staticlib",
        "--edition",
        "2021",
    ]);
    if debug {
        command.args([
            "--cfg",
            "feature=\"debug-runtime\"",
            "-C",
            "opt-level=0",
            "-C",
            "debuginfo=2",
        ]);
    } else {
        command.args(["-C", "opt-level=2"]);
    }
    let status = command
        .arg("-o")
        .arg(&output)
        .arg(&source)
        .status()
        .expect("failed to run rustc for bork_runtime");
    assert!(status.success(), "compiling bork_runtime failed: {status}");
    let variable = if debug {
        "BORK_RUNTIME_DEBUG_LIB"
    } else {
        "BORK_RUNTIME_LIB"
    };
    println!("cargo:rustc-env={variable}={}", output.display());
}
