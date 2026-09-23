use std::path::Path;
use std::process::Command;

/// Links `object` into an executable with `clang`, against libc only.
pub fn link_executable(object: &Path, output: &Path) -> Result<(), String> {
    let result = Command::new("clang")
        .arg("-o")
        .arg(output)
        .arg(object)
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
