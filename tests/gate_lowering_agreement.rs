//! A program that `check` and the codegen gate accept must build (#257): the gate and the LSP cannot see what
//! LLVM lowering or declaration would still reject, so any program that builds with an error the gate did not
//! report is a shape the builder must lower or name as a `Capability`.
#![cfg(feature = "codegen")]

use std::fs;
use std::path::{Path, PathBuf};

use bork::codegen::{emit_ir, gate, BuildError, BuildOptions};
use bork::frontend::check;

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
fn every_build_error_is_reported_by_the_gate_first() {
    let mut files = Vec::new();
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("programs"), &mut files);
    files.sort();
    let (mut gaps, mut built) = (Vec::new(), 0);
    for file in files {
        let checked = check(&fs::read_to_string(&file).unwrap());
        // A file without `fun main` is a library: `build` says so, the gate (like the LSP) has nothing to object to.
        if !checked.is_ok() || !checked.hir.as_ref().unwrap().functions.iter().any(|f| f.name == "main") {
            continue;
        }
        built += 1;
        let gated = gate(checked.hir.as_ref().unwrap(), checked.report.as_ref().unwrap(), &checked.tcx);
        if let Err(BuildError::Diagnostics(diagnostics)) = emit_ir(&checked, BuildOptions::default()) {
            if gated.is_empty() {
                gaps.push(format!("{}: {}", file.display(), diagnostics[0].message));
            }
        }
    }
    assert!(built > 100, "only {built} programs reached codegen");
    assert!(gaps.is_empty(), "build fails but the gate accepted: {gaps:#?}");
}
