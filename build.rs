use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    lalrpop::process_src().unwrap();
    if env::var_os("CARGO_FEATURE_CODEGEN").is_some() {
        build_runtime();
    }
}

/// Compiles `bork_runtime` with plain `rustc` so `bork build` can link it into user programs.
/// The runtime has no dependencies; a nested `cargo` would contend for the outer build's locks.
fn build_runtime() {
    let source = PathBuf::from("crates/bork_runtime/src/lib.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("libbork_runtime.a");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .args([
            "--crate-name",
            "bork_runtime",
            "--crate-type",
            "staticlib",
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
        ])
        .arg("-o")
        .arg(&output)
        .arg(&source)
        .status()
        .expect("failed to run rustc for bork_runtime");
    assert!(status.success(), "compiling bork_runtime failed: {status}");
    println!("cargo:rustc-env=BORK_RUNTIME_LIB={}", output.display());
}
