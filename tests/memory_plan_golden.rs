//! Frozen baseline: the memory plan of every frontend-passing corpus sample.
//! Refresh with `BORK_BLESS=1 cargo test --test memory_plan_golden` after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;

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
        let result = check(&source);
        assert!(result.is_ok(), "{rel}: frontend rejected a passing sample: {:?}", result.diagnostics);
        for a in &result.memory_plan.expect("plan present when ok").allocations {
            out.push_str(&format!(
                "{rel}: {}..{} {} {:?} {:?}\n",
                a.allocation_site.start, a.allocation_site.end, a.class_name, a.class, a.lifetime_domain
            ));
        }
    }
    out
}

#[test]
fn golden_memory_plan_matches_snapshot() {
    let snapshot = root().join("tests/golden/memory_plan.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "memory plan changed; if intended, rerun with BORK_BLESS=1");
}
