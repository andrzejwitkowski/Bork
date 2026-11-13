//! A program that `check` and the codegen gate accept must not fail `bork build` with "MIR lowering does not
//! cover" (#257): the gate and the LSP cannot see that check, so any hit is a shape the gate must reject (e.g. `==` on objects, an index read through a projection).
#![cfg(feature = "codegen")]

use std::fs;
use std::path::{Path, PathBuf};

use bork::codegen::{emit_ir, BuildError, BuildOptions};
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
fn programs_the_gate_accepts_never_hit_the_lowering_coverage_check() {
    let mut files = Vec::new();
    collect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("programs"), &mut files);
    files.sort();
    let (mut gaps, mut built) = (Vec::new(), 0);
    for file in files {
        let checked = check(&fs::read_to_string(&file).unwrap());
        if !checked.is_ok() {
            continue;
        }
        built += 1;
        if let Err(BuildError::Diagnostics(diagnostics)) = emit_ir(&checked, BuildOptions::default()) {
            if diagnostics.iter().any(|d| d.message.contains("does not cover")) {
                gaps.push(file.display().to_string());
            }
        }
    }
    assert!(built > 100, "only {built} programs reached codegen");
    assert!(gaps.is_empty(), "gate accepts but lowering does not cover: {gaps:#?}");
}
