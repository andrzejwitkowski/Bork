//! Frozen baseline: the monomorphized instances of every generic corpus sample.
//! Refresh with `BORK_BLESS=1 cargo test --test mono_instances_golden` after an intended change.

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

fn render() -> String {
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root().join("programs").join(suite), &mut files);
    }
    let mut rels: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(root()).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
        })
        .collect();
    rels.sort();

    let mut out = String::new();
    for rel in rels {
        let source = fs::read_to_string(root().join(&rel)).unwrap();
        let mut result = check(&source);
        assert!(result.is_ok(), "{rel}: frontend rejected a passing sample: {:?}", result.diagnostics);
        let hir = result.hir.take().expect("hir present when ok");
        let generic = hir.functions.iter().any(|f| !f.generics.is_empty()) || hir.classes.iter().any(|c| !c.generics.is_empty());
        if !generic {
            continue;
        }
        let report = result.report.take().expect("report present when ok");
        out.push_str(&format!("== {rel}\n"));
        match monomorphize(&hir, &mut result.tcx, &report) {
            Ok(mono) => {
                for instance in &mono.instances {
                    out.push_str(&format!("fun {}\n", instance.name));
                }
                for class in &mono.hir.classes {
                    out.push_str(&format!("class {}\n", class.name));
                }
            }
            Err(diagnostics) => {
                for d in diagnostics {
                    out.push_str(&format!("error: {}\n", d.message));
                }
            }
        }
    }
    out
}

#[test]
fn golden_mono_instances_match_snapshot() {
    let snapshot = root().join("tests/golden/mono_instances.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "mono instances changed; if intended, rerun with BORK_BLESS=1");
}
