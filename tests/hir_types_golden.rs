//! Frozen baseline: HIR function signatures for every frontend-passing corpus sample.
//! Refresh with `BORK_BLESS=1 cargo test --test hir_types_golden` after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::BindingKind;

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
    // build/ samples also pass the frontend; check_fail/build_fail yield no HIR.
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
        let result = check(&source);
        assert!(result.is_ok(), "{rel}: frontend rejected a passing sample: {:?}", result.diagnostics);
        for f in &result.hir.expect("hir present when ok").functions {
            let params: Vec<String> = f
                .params
                .iter()
                .map(|p| {
                    let kind = if p.kind == BindingKind::Var { "var " } else { "" };
                    format!("{kind}{}: {}", p.name, p.ty)
                })
                .collect();
            out.push_str(&format!("{rel}::{}({}) -> {}\n", f.name, params.join(", "), f.return_ty));
        }
    }
    out
}

#[test]
fn golden_hir_types_match_snapshot() {
    let snapshot = root().join("tests/golden/hir_types.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "HIR types changed; if intended, rerun with BORK_BLESS=1");
}
