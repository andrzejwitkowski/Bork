//! Every `HirExpr` in every passing corpus sample gets a distinct, numbered `NodeId`.
//! Reads the ids out of the HIR's `Debug` text so it does not share the numbering walker.

use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;

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
fn every_hir_expr_has_a_distinct_node_id() {
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs").join(suite), &mut files);
    }
    let mut total = 0;
    for file in files {
        let result = check(&fs::read_to_string(&file).unwrap());
        assert!(result.is_ok(), "{}: {:?}", file.display(), result.diagnostics);
        let text = format!("{:?}", result.hir.expect("hir"));
        let ids: Vec<u64> = text
            .split("id: NodeId(")
            .skip(1)
            .map(|rest| rest.split(')').next().unwrap().parse().unwrap())
            .collect();
        let exprs = text.matches("HirExpr {").count();
        assert_eq!(ids.len(), exprs, "{}: id count != HirExpr count", file.display());
        assert!(ids.iter().all(|id| *id != u32::MAX as u64), "{}: unnumbered expr", file.display());
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "{}: duplicate ids", file.display());
        assert_eq!(sorted, (0..ids.len() as u64).collect::<Vec<_>>(), "{}: ids not 0..n", file.display());
        total += ids.len();
    }
    assert!(total > 1000, "only {total} exprs checked");
    eprintln!("checked {total} expressions");
}
