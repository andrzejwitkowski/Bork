use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// System libraries Rust's std needs when `bork_runtime` is linked as a staticlib.
const RUNTIME_NATIVE_LIBS: &[&str] = &["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl"];

/// Links `object` into an executable with `clang`, against `bork_runtime` and libc.
pub fn link_executable(object: &Path, output: &Path) -> Result<(), String> {
    link_executable_with_debug(object, output, false)
}

pub fn link_executable_with_debug(object: &Path, output: &Path, debug: bool) -> Result<(), String> {
    let runtime = runtime_library(debug);
    let override_variable = if debug {
        "BORK_RUNTIME_DEBUG_LIB"
    } else {
        "BORK_RUNTIME_LIB"
    };
    if !Path::new(&runtime).is_file() {
        return Err(format!(
            "bork runtime library not found at {} (set {override_variable})",
            Path::new(&runtime).display()
        ));
    }
    let mut command = Command::new("clang");
    if debug {
        command.arg("-Wl,--require-defined=bork_debug_abi_v1");
    }
    let result = command
        .arg("-o")
        .arg(output)
        .arg(object)
        .arg(&runtime)
        .args(RUNTIME_NATIVE_LIBS)
        .output()
        .map_err(|err| format!("failed to run clang: {err}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "clang failed ({}): {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim_end()
        ))
    }
}

/// `BORK_RUNTIME_LIB` at run time overrides the archive `build.rs` compiled.
fn runtime_library(debug: bool) -> OsString {
    if debug {
        std::env::var_os("BORK_RUNTIME_DEBUG_LIB")
            .unwrap_or_else(|| env!("BORK_RUNTIME_DEBUG_LIB").into())
    } else {
        std::env::var_os("BORK_RUNTIME_LIB").unwrap_or_else(|| env!("BORK_RUNTIME_LIB").into())
    }
}
