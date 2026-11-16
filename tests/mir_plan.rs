//! #245: every `programs/build` sample lowers in MIR, so the plan and the push flags always come from MIR
//! (closures only occur in `programs/check`, which the gate rejects).

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mono::monomorphize;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "bork") {
            out.push(path);
        }
    }
}

#[test]
fn every_build_program_lowers_in_mir() {
    let (mut files, mut failures) = (Vec::new(), Vec::new());
    collect(&root().join("programs").join("build"), &mut files);
    files.sort();
    for file in &files {
        let rel = file.strip_prefix(root()).unwrap().to_string_lossy().to_string();
        let mut result = check(&fs::read_to_string(file).unwrap());
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let mono = monomorphize(&hir, &mut result.tcx, &report).unwrap_or_else(|_| panic!("{rel}: mono failed"));
        if let Err(e) = bork::mir::build::build_program_neutral(&mono, &result.tcx) {
            failures.push(format!("{rel}: {e:?}"));
        }
    }
    assert!(failures.is_empty(), "{} build programs do not lower:\n{}", failures.len(), failures.join("\n"));
}
