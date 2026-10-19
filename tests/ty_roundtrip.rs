//! Every signature and field type of the passing corpus survives `Ty -> TyId -> Ty` (#32).

use std::fs;
use std::path::{Path, PathBuf};

use bork::defs::DefTable;
use bork::frontend::check;
use bork::ty::TyCtx;

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
fn corpus_types_round_trip_through_the_interner() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs");
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root.join(suite), &mut files);
    }
    assert!(!files.is_empty());
    let mut interned = 0;
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        let result = check(&source);
        assert!(result.is_ok(), "{}: {:?}", file.display(), result.diagnostics);
        let hir = result.hir.expect("hir present when ok");
        let mut ctx = TyCtx::new(&DefTable::build(&bork::parse(&source).unwrap()));
        let signature_types = hir
            .functions
            .iter()
            .flat_map(|f| f.params.iter().map(|p| &p.ty).chain([&f.return_ty]));
        let field_types = hir.classes.iter().flat_map(|c| c.fields.iter().map(|f| &f.ty));
        for ty in signature_types.chain(field_types) {
            let id = ctx.intern(ty).unwrap_or_else(|e| panic!("{}: {e:?}", file.display()));
            assert_eq!(&ctx.to_ty(id), ty, "{}", file.display());
            assert_eq!(ctx.intern(ty).unwrap(), id, "{}: not idempotent", file.display());
            interned += 1;
        }
    }
    assert!(interned > 100, "only {interned} types interned");
}
