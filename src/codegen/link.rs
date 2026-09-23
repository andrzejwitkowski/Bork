use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// System libraries Rust's std needs when `bork_runtime` is linked as a staticlib.
const RUNTIME_NATIVE_LIBS: &[&str] = &["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl"];

/// Links `object` into an executable with `clang`, against `bork_runtime` and libc.
pub fn link_executable(object: &Path, output: &Path) -> Result<(), String> {
    let runtime = runtime_library();
    if !Path::new(&runtime).is_file() {
        return Err(format!(
            "bork runtime library not found at {} (set BORK_RUNTIME_LIB)",
            Path::new(&runtime).display()
        ));
    }
    let result = Command::new("clang")
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
fn runtime_library() -> OsString {
    std::env::var_os("BORK_RUNTIME_LIB").unwrap_or_else(|| env!("BORK_RUNTIME_LIB").into())
}
