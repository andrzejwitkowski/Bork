//! M4b (#218): the memory plan derived from MIR (built with a neutral plan) equals `memory::plan` on the corpus.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::memory::MemoryPlan;
use bork::mir::build::{build_program, BuildError};
use bork::mir::plan::derive_memory_plan;
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

fn corpus() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root().join("programs").join(suite), &mut files);
    }
    files.sort();
    files
}

#[test]
fn mir_plan_equals_memory_plan() {
    let files = corpus();
    let (mut compared, mut diffs) = (0, Vec::new());
    for file in files {
        let rel = file.strip_prefix(root()).unwrap().to_string_lossy().to_string();
        let mut result = check(&fs::read_to_string(&file).unwrap());
        assert!(result.is_ok(), "{rel}: {:?}", result.diagnostics);
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let mono = monomorphize(&hir, &mut result.tcx, &report).unwrap_or_else(|_| panic!("{rel}: mono failed"));
        // The reference is the old pass, run again; `mono.memory_plan` itself already comes from MIR.
        let (old, _) = bork::memory::plan(&mono.hir, &result.tcx, &mono.report);
        let bodies = match build_program(&mono, &result.tcx) {
            Ok(b) => b,
            Err(BuildError::Unsupported(_)) => continue,
            Err(e) => panic!("{rel}: {e:?}"),
        };
        let new = derive_memory_plan(&bodies, &result.tcx);
        compared += 1;
        let key = |p: &MemoryPlan| -> Vec<String> {
            let mut v: Vec<_> = p
                .allocations
                .iter()
                .map(|a| format!("{}..{} {} {:?} {:?}", a.allocation_site.start, a.allocation_site.end, a.class_name, a.class, a.lifetime_domain))
                .collect();
            v.sort();
            v
        };
        let (o, n) = (key(&old), key(&new));
        if o != n {
            let missing: Vec<_> = o.iter().filter(|x| !n.contains(x)).collect();
            let extra: Vec<_> = n.iter().filter(|x| !o.contains(x)).collect();
            diffs.push(format!("{rel}\n   old only {missing:?}\n   new only {extra:?}"));
        }
    }
    assert!(compared > 250, "only {compared} programs compared");
    assert!(diffs.is_empty(), "{} of {compared} programs differ:\n{}", diffs.len(), diffs.join("\n"));
}

fn flagged(node: &bork::sema::ArenaNode, out: &mut BTreeSet<usize>) {
    if node.codegen_push {
        out.insert(node.id);
    }
    node.children.iter().for_each(|c| flagged(c, out));
}

#[test]
fn mir_push_flags_equal_codegen_push() {
    let files = corpus();
    let (mut compared, mut diffs) = (0, Vec::new());
    for file in files {
        let rel = file.strip_prefix(root()).unwrap().to_string_lossy().to_string();
        let mut result = check(&fs::read_to_string(&file).unwrap());
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let mono = monomorphize(&hir, &mut result.tcx, &report).unwrap();
        let mut old_report = mono.report.clone();
        bork::legacy_stamp_codegen_push(&mono.hir, &result.tcx, &mut old_report).unwrap();
        let bodies = match bork::mir::build::build_program_neutral(&mono, &result.tcx) {
            Ok(b) => b,
            Err(BuildError::Unsupported(_)) => continue,
            Err(e) => panic!("{rel}: {e:?}"),
        };
        compared += 1;
        for (body, root) in bodies.iter().zip(&old_report.roots) {
            let mut want = BTreeSet::new();
            flagged(root, &mut want);
            let got: BTreeSet<usize> = bork::mir::plan::derive_push(body).into_iter().collect();
            if want != got {
                diffs.push(format!("{rel} `{}`: old only {:?}, new only {:?}", body.name, want.difference(&got).collect::<Vec<_>>(), got.difference(&want).collect::<Vec<_>>()));
            }
        }
    }
    assert!(compared > 250, "only {compared} programs compared");
    assert!(diffs.is_empty(), "{} functions differ:\n{}", diffs.len(), diffs.join("\n"));
}

/// #245: every `programs/build` sample lowers in MIR, so `plan_instances` never needs the old `memory.rs` path
/// for a program that can be built (`move-aggregate` and closures only occur in `programs/check`, gate-rejected).
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
