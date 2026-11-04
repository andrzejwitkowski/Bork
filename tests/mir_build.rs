//! Lowers every `programs/{build,check}` sample to MIR and verifies it (RFC 0006 M2b).
//! `MIR_VERBOSE=1` lists what is still not lowered.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mir::build::{build_program, BuildError};
use bork::mir::verify::verify;
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
fn corpus_builds_and_verifies() {
    let mut files = Vec::new();
    for suite in ["build", "check"] {
        collect(&root().join("programs").join(suite), &mut files);
    }
    files.sort();
    let (mut built, mut failures) = (0, Vec::new());
    let mut pending: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in files {
        let rel = file.strip_prefix(root()).unwrap().to_string_lossy().to_string();
        let mut result = check(&fs::read_to_string(&file).unwrap());
        if !result.is_ok() {
            continue;
        }
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let Ok(mono) = monomorphize(&hir, &mut result.tcx, &report) else { continue };
        match build_program(&mono, &result.tcx) {
            Ok(bodies) => {
                for body in &bodies {
                    for e in verify(body, &result.tcx) {
                        failures.push(format!("{rel}: fn {}: {e}", body.name));
                    }
                }
                built += 1;
            }
            Err(BuildError::NotYet { what, .. }) => pending.entry(what).or_default().push(rel),
            Err(BuildError::Unsupported(u)) => unsupported.entry(u.capability.to_string()).or_default().push(rel),
            Err(BuildError::Internal(m)) => failures.push(format!("{rel}: internal: {m}")),
        }
    }
    if std::env::var_os("MIR_VERBOSE").is_some() {
        for (what, files) in &pending {
            eprintln!("not yet: {what} ({} programs, e.g. {})", files.len(), files[0]);
        }
        for (cap, files) in &unsupported {
            eprintln!("unsupported `{cap}`: {files:?}");
        }
        eprintln!("built {built}, pending {}", pending.values().map(Vec::len).sum::<usize>());
    }
    assert!(failures.is_empty(), "{} problems:\n{}", failures.len(), failures.join("\n"));
}
