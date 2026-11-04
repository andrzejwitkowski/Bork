//! Frozen baseline of `mir::layout` for every record and enum in the build corpus (RFC 0006 M1).
//! The LLVM cross-check lives in `codegen/llvm/context.rs`. Refresh with
//! `BORK_BLESS=1 cargo test --test mir_layout_golden` after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mir::layout::{DataLayout, Layouts};
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
    collect(&root().join("programs/build"), &mut files);
    let mut rels: Vec<String> = files.iter().map(|p| p.strip_prefix(root()).unwrap().to_string_lossy().replace('\\', "/")).collect();
    rels.sort();
    let mut out = String::new();
    for rel in rels {
        let mut result = check(&fs::read_to_string(root().join(&rel)).unwrap());
        if !result.is_ok() {
            continue;
        }
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let Ok(mono) = monomorphize(&hir, &mut result.tcx, &report) else { continue };
        let ls = Layouts::new(&result.tcx, &mono.hir, DataLayout::default());
        let mut body = String::new();
        for c in &mono.hir.classes {
            match ls.record(&c.name) {
                Some(l) => body.push_str(&format!("class {} size={} align={} fields={:?}\n", c.name, l.size, l.align, l.fields)),
                None => body.push_str(&format!("class {} unsupported\n", c.name)),
            }
        }
        for e in mono.hir.enums.iter().filter(|e| e.generics.is_empty()) {
            match ls.enum_(&e.name) {
                Some(l) => {
                    let vs: Vec<_> = l.variants.iter().map(|v| v.fields.clone()).collect();
                    body.push_str(&format!("enum {} size={} align={} variants={:?}\n", e.name, l.size, l.align, vs));
                }
                None => body.push_str(&format!("enum {} unsupported\n", e.name)),
            }
        }
        if !body.is_empty() {
            out.push_str(&format!("== {rel}\n{body}"));
        }
    }
    out
}

#[test]
fn golden_mir_layout_matches_snapshot() {
    let snapshot = root().join("tests/golden/mir_layout.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot).unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "MIR layout changed; if intended, rerun with BORK_BLESS=1");
}
