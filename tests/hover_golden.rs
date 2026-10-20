//! Frozen baseline: LSP hover at every byte offset of every corpus sample, as a count and a
//! hash per file. Pins which member/binding each position resolves to.
//! Refresh with `BORK_BLESS=1 cargo test --test hover_golden` after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use bork::lsp::{analyze_source, byte_offset_to_position, hover};

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

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *hash = (*hash ^ *b as u64).wrapping_mul(0x100000001b3);
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
        let source = fs::read_to_string(root().join(&rel)).unwrap();
        let analysis = analyze_source(&source);
        let (mut hits, mut hash) = (0, 0xcbf29ce484222325u64);
        for offset in (0..=source.len()).filter(|o| source.is_char_boundary(*o)) {
            let position = byte_offset_to_position(&source, offset);
            if let Some(text) = hover(&analysis, &source, position) {
                hits += 1;
                fnv(&mut hash, &offset.to_le_bytes());
                fnv(&mut hash, text.as_bytes());
            }
        }
        out.push_str(&format!("{rel}: hits={hits} hash={hash:016x}\n"));
    }
    out
}

#[test]
fn golden_hover_matches_snapshot() {
    let snapshot = root().join("tests/golden/hover.txt");
    let actual = render();
    if std::env::var_os("BORK_BLESS").is_some() {
        fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("{} missing; run with BORK_BLESS=1", snapshot.display()));
    assert_eq!(actual, expected, "hover changed; if intended, rerun with BORK_BLESS=1");
}
