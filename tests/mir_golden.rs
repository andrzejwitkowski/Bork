//! Frozen `--emit-mir` output of every `programs/build` sample, one file per topic directory
//! (`tests/golden/mir/<topic>.txt`). Refresh with `BORK_BLESS=1 cargo test --test mir_golden`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mir::emit::emit_mir;

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

/// Topic name -> rendered text; files directly under `programs/build` go to `root`.
fn render() -> BTreeMap<String, String> {
    let build = root().join("programs/build");
    let mut files = Vec::new();
    collect(&build, &mut files);
    files.sort();
    let mut topics: BTreeMap<String, String> = BTreeMap::new();
    for file in files {
        let rel = file.strip_prefix(&build).unwrap();
        let topic = rel.components().next().filter(|_| rel.components().count() > 1).map_or("root".to_string(), |c| c.as_os_str().to_string_lossy().to_string());
        let text = match emit_mir(&check(&fs::read_to_string(&file).unwrap())) {
            Ok(text) => text,
            Err(diagnostics) => diagnostics.iter().map(|d| format!("error: {}\n", d.message)).collect(),
        };
        let entry = topics.entry(topic).or_default();
        entry.push_str(&format!("== {}\n{text}", rel.to_string_lossy().replace('\\', "/")));
    }
    topics
}

#[test]
fn golden_mir_matches_snapshots() {
    let dir = root().join("tests/golden/mir");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::create_dir_all(&dir).unwrap();
        for (topic, text) in &actual {
            fs::write(dir.join(format!("{topic}.txt")), text).unwrap();
        }
        return;
    }
    for (topic, text) in &actual {
        let path = dir.join(format!("{topic}.txt"));
        let expected = fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", path.display()));
        assert_eq!(text, &expected, "MIR for topic `{topic}` changed; if intended, rerun with BORK_BLESS=1");
    }
    let on_disk = fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(on_disk, actual.len(), "stale golden files in {}", dir.display());
}
