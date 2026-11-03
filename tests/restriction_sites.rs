//! RFC 0003 section 6 / #196: each borrow restriction message is produced at exactly one site.
//! Add a row when a restriction (or a new borrow form) gains a rule.
use std::fs;
use std::path::Path;

fn sources(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            if !name.ends_with("/tests") {
                sources(root, &path, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
            out.push((name, fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn every_borrow_restriction_has_one_enforcement_site() {
    // (message, the one file that may contain it exactly once; None = must not appear anywhere)
    let rows: &[(&str, Option<&str>)] = &[
        ("borrowed references cannot be stored in class fields", Some("src/typeck/mod.rs")),
        ("borrowed references cannot be stored in enum payloads", Some("src/typeck/mod.rs")),
        ("returning a reference is not supported", Some("src/escape.rs")),
        ("to a reference type; use `val`", Some("src/typeck/stmt/mod.rs")),
        // RFC 0003 section 5 (#198).
        ("cannot index-assign through the read-only borrow", Some("src/typeck/stmt/assign.rs")),
        ("cannot mutate through the read-only borrow", Some("src/typeck/expr/vec.rs")),
        ("as `&rw`; declare it `var`", Some("src/typeck/expr/borrow.rs")),
        ("cannot take `&rw` of the read-only borrow", Some("src/typeck/expr/borrow.rs")),
        ("array element type `{}` is not supported", Some("src/ty/query.rs")),
        ("`&rw` needs it to itself", Some("src/typeck/expr/call.rs")),
        // Removed sema duplicates must stay gone.
        ("to a borrow; use `val`", None),
        ("cannot bind a borrow view", None),
        ("cannot move borrow `{name}`", None),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(root, &root.join("src"), &mut files);
    for (message, site) in rows {
        let found: Vec<(&str, usize)> = files
            .iter()
            .filter_map(|(name, text)| Some((name.as_str(), text.matches(message).count())).filter(|&(_, n)| n > 0))
            .collect();
        let want: Vec<(&str, usize)> = site.iter().map(|&file| (file, 1)).collect();
        assert_eq!(found, want, "`{message}`");
    }
}
