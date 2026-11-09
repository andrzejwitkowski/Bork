//! M3 shadow mode without LLVM (#216): the memory plan and push flags derived from MIR equal the old passes on
//! the corpus, the derived plan agrees with the frozen `tests/golden/memory_plan.txt`, and the set of programs the
//! comparison cannot cover is pinned so it cannot grow silently.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bork::frontend::check;
use bork::mir::build::{build_program, BuildError};
use bork::mir::shadow::{compare, derive_plan};
use bork::mir::{AllocKind, StmtKind};
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

#[test]
fn mir_plan_and_push_flags_match_and_agree_with_the_golden() {
    let mut files = Vec::new();
    for suite in ["check", "build"] {
        collect(&root().join("programs").join(suite), &mut files);
    }
    files.sort();
    let golden = fs::read_to_string(root().join("tests/golden/memory_plan.txt")).unwrap();
    let (mut compared, mut skipped) = (0, BTreeSet::new());
    let mut golden_lines = 0;
    for file in files {
        let rel = file.strip_prefix(root()).unwrap().to_string_lossy().to_string();
        let mut result = check(&fs::read_to_string(&file).unwrap());
        assert!(result.is_ok(), "{rel}: {:?}", result.diagnostics);
        let (hir, report) = (result.hir.take().unwrap(), result.report.take().unwrap());
        let mono = monomorphize(&hir, &mut result.tcx, &report).unwrap_or_else(|_| panic!("{rel}: mono failed"));
        let bodies = match build_program(&mono, &result.tcx) {
            Ok(b) => b,
            Err(BuildError::Unsupported(_)) => {
                skipped.insert(rel);
                continue;
            }
            Err(e) => panic!("{rel}: {e:?}"),
        };
        compare(&mono, &bodies).unwrap_or_else(|e| panic!("{rel}: {e}"));
        compared += 1;

        // Only `build` samples have a `main`; mono keeps just what `main` reaches, the golden predates mono.
        if !rel.starts_with("programs/build/") {
            continue;
        }
        // Golden: every frozen line of a non-generic sample reappears in the plan derived from MIR.
        let derived = derive_plan(&bodies);
        let mut mir_lines = BTreeSet::new();
        for s in bodies.iter().flat_map(|b| &b.blocks).flat_map(|b| &b.stmts) {
            let (StmtKind::Alloc { kind: AllocKind::Object { .. } | AllocKind::EnumSlot { .. }, .. }, Some(node)) = (&s.kind, s.node) else { continue };
            let (class, program) = derived[&node];
            let domain = if program { "Program" } else { "LexicalScope" };
            mir_lines.insert(format!("{rel}: {}..{} {class:?} {domain}", s.span.start, s.span.end));
        }
        for line in golden.lines().filter(|l| l.starts_with(&format!("{rel}: "))) {
            golden_lines += 1;
            // Mono renames generic instances (`Option` -> `Option<i32>`), so the class name is not compared.
            let (head, tail) = line.split_once(": ").unwrap();
            let mut parts = tail.splitn(4, ' ');
            let (span, _name, class, domain) = (parts.next().unwrap(), parts.next(), parts.next().unwrap(), parts.next().unwrap());
            let domain = if domain.starts_with("LexicalScope") { "LexicalScope" } else { domain };
            let norm = format!("{head}: {span} {class} {domain}");
            assert!(mir_lines.contains(&norm), "{rel}: golden line `{line}` not derived from MIR; MIR has {mir_lines:?}");
        }
    }
    assert!(compared > 250, "only {compared} programs compared");
    assert!(golden_lines > 50, "only {golden_lines} golden lines checked");
    // Exactly the gate-reject programs under `check`/`build` (RFC 0006 3.10); a new entry here needs a reason.
    assert_eq!(skipped, BTreeSet::from(["programs/check/methods/trailing_closure.bork".to_string()]));
}
