//! The MIR builder and the LLVM region schedule must agree on which regions push an arena and which
//! loops reset one (seed of the M3 shadow comparison, #216). Needs `--features codegen`.
#![cfg(feature = "codegen")]

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mir::build::build_program;
use bork::mir::shadow;
use bork::mono::monomorphize;

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
fn mir_shadow_agrees_with_plan_and_schedule() {
    let mut files = Vec::new();
    collect(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs/build"), &mut files);
    collect(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs/check"), &mut files);
    files.sort();
    let mut compared = 0;
    for file in files {
        let mut result = check(&fs::read_to_string(&file).unwrap());
        if !result.is_ok() {
            continue;
        }
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let Ok(mono) = monomorphize(&hir, &mut result.tcx, &report) else { continue };
        let Ok(bodies) = build_program(&mono, &result.tcx) else { continue };
        if let Err(e) = shadow::compare_all(&mono, &result.tcx, &bodies) {
            panic!("{}: {e}", file.display());
        }
        compared += 1;
    }
    assert!(compared > 200, "only {compared} programs compared");
}
