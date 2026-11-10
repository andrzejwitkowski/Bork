//! Shadow of the object-lifetime rules of `escape.rs` on MIR (#217): everything the MIR analysis reports
//! must be a diagnostic the old pass reports too, and the cases it covers are pinned.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check_with;
use bork::mir::build::build_program;
use bork::mir::escape;
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

#[test]
fn mir_escape_is_a_subset_of_the_old_pass_and_covers_the_pinned_cases() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for suite in ["check_fail", "check", "build", "build_fail"] {
        collect(&root.join("programs").join(suite), &mut files);
    }
    files.sort();
    let (mut covered, mut same) = (BTreeSet::new(), BTreeSet::new());
    for file in files {
        let rel = file.strip_prefix(&root).unwrap().to_string_lossy().to_string();
        let source = fs::read_to_string(&file).unwrap();
        let old: BTreeSet<_> = check_with(&source, true).diagnostics.iter().map(|d| (d.span.map(|s| (s.start, s.end)), d.message.clone())).collect();
        let mut new = check_with(&source, false);
        let (Some(hir), Some(report)) = (new.hir.take(), new.report.take()) else { continue };
        let Ok(mono) = monomorphize_unchecked(&hir, &mut new.tcx, &report) else { continue };
        let Ok(bodies) = build_program(&mono, &new.tcx) else { continue };
        for (span, message) in bodies.iter().flat_map(escape::check) {
            // MIR sees the dangling store itself; the old pass may report the same fact later (at the `return`).
            assert!(old.iter().any(|(_, m)| m.contains("freed before") || m.contains("declared inside a loop") || m.contains("inner region")), "{rel}: MIR reports `{message}` at {span:?}, the old pass has no escape diagnostic");
            if old.contains(&(Some((span.start, span.end)), message.clone())) {
                same.insert(rel.clone());
            }
            covered.insert(rel.clone());
        }
    }
    let pin = |names: &[&str]| -> BTreeSet<String> { names.iter().map(|n| format!("programs/check_fail/{n}.bork")).collect() };
    // Same span and message as `escape.rs`.
    assert_eq!(same, pin(&["enums/assign_local_object_to_outer_var", "enums/return_loop_local_stored", "enums/return_value_if_loop_local", "option/return_some_loop_local"]));
    // MIR also sees the dangling store in `return_alias_hoisted`, which `escape.rs` reports later, at the `return`.
    // Not covered yet (stored into an `Option` slot): option/assign_some_local_to_outer, see #243.
    assert_eq!(covered.difference(&same).cloned().collect::<BTreeSet<_>>(), pin(&["enums/return_alias_hoisted"]));
}
