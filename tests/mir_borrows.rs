//! Borrow conflicts on MIR (#219): no false positives on the accepted corpus, and the
//! `val b = &a; f(&rw a, &b)` hole is closed.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check_with;
use bork::mir::build::build_program;
use bork::mir::borrows;
use bork::mono::monomorphize_unchecked;

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


fn diagnostics(source: &str) -> Option<Vec<String>> {
    let mut r = check_with(source, false);
    let (hir, report) = (r.hir.take()?, r.report.take()?);
    let mono = monomorphize_unchecked(&hir, &mut r.tcx, &report).ok()?;
    let bodies = build_program(&mono, &r.tcx).ok()?;
    Some(bodies.iter().flat_map(borrows::check).map(|(_, m)| m).collect())
}

const F: &str = "fun f(x: &rw [i32; 2], y: &[i32; 2]) {\n    x[0] = y[1]\n}\n";

#[test]
fn no_conflicts_on_accepted_programs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root.join("programs").join(suite), &mut files);
    }
    files.sort();
    for file in files {
        let found = diagnostics(&fs::read_to_string(&file).unwrap()).unwrap_or_default();
        assert!(found.is_empty(), "{}: {found:?}", file.display());
    }
}

#[test]
fn borrow_held_in_a_variable_conflicts_with_a_later_rw_borrow() {
    let src = format!("{F}fun main() {{\n    var a: [i32; 2] = [1, 2]\n    val b = &a\n    f(&rw a, &b)\n}}\n");
    let found = diagnostics(&src).expect("builds");
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`&rw a`") && found[0].contains("held by `b`"), "{found:?}");
}

#[test]
fn dead_borrow_does_not_conflict() {
    let src = format!("{F}fun main() {{\n    var a: [i32; 2] = [1, 2]\n    val b = &a\n    val x = b[0]\n    var c: [i32; 2] = [x, 2]\n    f(&rw a, &c)\n}}\n");
    assert_eq!(diagnostics(&src).expect("builds"), Vec::<String>::new());
}
