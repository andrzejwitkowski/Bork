//! Object-lifetime errors come from MIR (`mir::escape`, run by `check`): every `check_fail` program that expects one
//! gets it from MIR alone, and no program that must check or build gets a false one.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::object_lifetime_errors;

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

const LIFETIME: [&str; 2] = ["is freed before the value that stores it", "is declared inside a loop"];

fn expected(source: &str) -> Option<&'static str> {
    let diag = source.lines().take(30).find_map(|l| l.trim().strip_prefix("//")?.trim().strip_prefix("diag:").map(|d| d.trim().to_owned()))?;
    LIFETIME.into_iter().find(|m| diag.contains(m))
}

#[test]
fn check_fail_lifetime_errors_come_from_mir() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs");
    let mut files = Vec::new();
    collect(&root.join("check_fail"), &mut files);
    files.sort();
    let (mut pinned, mut missing) = (0, Vec::new());
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        let Some(wanted) = expected(&source) else { continue };
        pinned += 1;
        if !object_lifetime_errors(&source).iter().any(|d| d.message.contains(wanted)) {
            missing.push(file.strip_prefix(&root).unwrap().display().to_string());
        }
    }
    assert!(pinned >= 10, "only {pinned} lifetime programs found");
    assert!(missing.is_empty(), "MIR does not report: {missing:?}");
}

#[test]
fn no_lifetime_error_on_programs_that_check_or_build() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs");
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root.join(suite), &mut files);
    }
    files.sort();
    for file in files {
        let found = object_lifetime_errors(&fs::read_to_string(&file).unwrap());
        assert!(found.is_empty(), "{}: {found:?}", file.display());
    }
}
