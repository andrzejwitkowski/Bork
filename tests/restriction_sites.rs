//! RFC 0003 section 6 / #196: each borrow restriction message is produced at exactly one site.
//! Add a row when a restriction (or a new borrow form) gains a rule.
use std::fs;
use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            if !name.ends_with("/tests") {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
            out.push((name, fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn every_borrow_restriction_has_one_enforcement_site() {
    let rows: &[(&str, &[&str])] = &[
        ("borrowed references cannot be stored in class fields", &["src/typeck/mod.rs"]),
        ("borrowed references cannot be stored in enum payloads", &["src/typeck/mod.rs"]),
        ("returning a reference is not supported", &["src/escape.rs"]),
        ("to a reference type; use `val`", &["src/typeck/stmt/mod.rs"]),
        // Removed duplicates (sema): they must stay gone.
        ("to a borrow; use `val`", &[]),
        ("cannot bind a borrow view", &[]),
    ];
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    for (message, expected) in rows {
        let found: Vec<String> = files
            .iter()
            .filter(|(_, text)| text.contains(message))
            .map(|(name, text)| format!("{}x{}", name.rsplit("/Bork/").next().unwrap(), text.matches(message).count()))
            .collect();
        let want: Vec<String> = expected.iter().map(|f| format!("{f}x1")).collect();
        assert_eq!(found, want, "`{message}`");
    }
}
