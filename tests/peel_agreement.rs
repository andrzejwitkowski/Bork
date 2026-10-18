//! AST and HIR peel the same number of bare-block wrappers off every corpus function body.

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
fn ast_and_hir_peel_counts_agree_on_the_corpus() {
    let programs = Path::new(env!("CARGO_MANIFEST_DIR")).join("programs");
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&programs.join(suite), &mut files);
    }
    let mut compared = 0;
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        let ast = bork::parse(&source).expect("corpus sample parses");
        let hir = check(&source).hir.expect("passing sample has HIR");
        let ast_bodies = ast
            .functions
            .iter()
            .map(|f| (f.name.clone(), &f.body))
            .chain(ast.classes.iter().flat_map(|c| {
                c.methods.iter().map(move |m| (format!("{}.{}", c.name, m.name), &m.body))
            }));
        for (name, ast_body) in ast_bodies {
            let Some(hir_fn) = hir.functions.iter().find(|f| f.name == name) else {
                continue; // intrinsics and similar have no HIR twin
            };
            let (_, ast_peeled) = bork::ast::peel_blocks(ast_body);
            let (_, hir_peeled) = bork::hir::peel_blocks(&hir_fn.body);
            assert_eq!(ast_peeled, hir_peeled, "{}: {name}", file.display());
            compared += 1;
        }
    }
    assert!(compared > 0, "no function bodies were compared");
}
