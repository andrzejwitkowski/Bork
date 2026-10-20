//! Frozen baseline: the arena report and every diagnostic of every corpus sample.
//! Refresh with `BORK_BLESS=1 cargo test --test arena_report_golden` after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use bork::dump::dump_arenas;
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
    collect(&root().join("programs"), &mut files);
    let mut rels: Vec<String> = files
        .iter()
        .map(|p| p.strip_prefix(root()).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
        .collect();
    rels.sort();

    let mut out = String::new();
    for rel in rels {
        let result = check(&fs::read_to_string(root().join(&rel)).unwrap());
        out.push_str(&format!("== {rel}\n"));
        // Sorted: some checks iterate a HashMap, so diagnostic order is not stable across runs.
        let mut lines: Vec<String> = result
            .diagnostics
            .iter()
            .map(|d| {
                let span = d.span.map(|s| format!("{}..{}", s.start, s.end)).unwrap_or_default();
                format!("{:?} {:?} [{span}] {}\n", d.phase, d.severity, d.message)
            })
            .collect();
        lines.sort();
        out.extend(lines);
        if let Some(report) = &result.report {
            out.push_str(&dump_arenas(report));
        }
    }
    out
}

#[test]
fn golden_arena_report_matches_snapshot() {
    let snapshot = root().join("tests/golden/arena_report.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "arena report or diagnostics changed; if intended, rerun with BORK_BLESS=1");
}
