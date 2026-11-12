//! `--mir-backend` against the HIR emitter on `programs/build` (RFC 0006 M5a, #222): the same region
//! runtime calls and the same program behaviour wherever MIR lowers a function.
#![cfg(feature = "codegen")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bork::codegen::{emit_ir, emit_ir_with_source, BuildOptions, SourceContext};
use bork::frontend::check;

/// Keeps a regressed MIR backend (one that lowers almost nothing) from passing.
const MIN_PROGRAMS_WITH_LOWERED_FUNCTIONS: usize = 190;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "bork") {
            out.push(path);
        }
    }
}

fn count(ir: &str, needle: &str) -> usize {
    ir.matches(needle).count()
}

fn region_calls(ir: &str) -> [usize; 4] {
    [
        count(ir, "call ptr @bork_arena_push()"),
        count(ir, "call ptr @bork_arena_push_child("),
        count(ir, "@bork_arena_pop("),
        count(ir, "@bork_arena_reset("),
    ]
}

/// Counts of `Ref` protocol, registration and counter runtime calls (M5d, #225); `call` lines only.
fn protocol_calls(ir: &str) -> Vec<usize> {
    ["@bork_ref_", "@bork_arena_register_", "@bork_arena_count_", "@bork_arena_strong_count(", "@bork_arena_weak_count("]
        .iter()
        // Not `release_observation`: HIR also releases an absent (null-guard, no-op) observation; MIR skips it.
        .map(|p| ir.lines().filter(|l| l.contains("call ") && l.contains(p) && !l.contains("release_observation")).count())
        .collect()
}

fn run(source: &Path, out: &Path, mir: bool) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bork"));
    cmd.arg("build");
    if mir {
        cmd.arg("--mir-backend");
    }
    let build = cmd.arg("-o").arg(out).arg(source).output().expect("spawn bork build");
    assert!(build.status.success(), "{} failed to build (mir={mir}):\n{}", source.display(), String::from_utf8_lossy(&build.stderr));
    let run = Command::new(out).output().expect("run binary");
    (run.status.code().unwrap_or(-1), String::from_utf8_lossy(&run.stdout).into_owned())
}

#[test]
fn mir_backend_matches_the_hir_emitter_on_the_build_corpus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs/build");
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bork-mir-backend");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    let mut files = Vec::new();
    collect(&root, &mut files);

    let (mut lowered, mut compared) = (0, 0);
    for file in files {
        let result = check(&fs::read_to_string(&file).unwrap());
        let Ok(hir) = emit_ir(&result, BuildOptions::default()) else {
            continue; // not buildable (gate); `tests/programs.rs` owns that
        };
        let mir = emit_ir(&result, BuildOptions { debug: false, mir_backend: true })
            .unwrap_or_else(|e| panic!("MIR backend failed where HIR succeeded for {}: {e:?}", file.display()));
        compared += 1;
        assert_eq!(region_calls(&hir), region_calls(&mir), "push/push_child/pop/reset counts differ for {}", file.display());
        assert_eq!(protocol_calls(&hir), protocol_calls(&mir), "Ref protocol call counts differ for {}", file.display());
        if hir == mir {
            continue; // every function fell back
        }
        lowered += 1;
        let bin = scratch.join(format!("p{lowered}"));
        let expected = run(&file, &bin, false);
        assert_eq!(run(&file, &bin, true), expected, "output differs for {}", file.display());
    }
    eprintln!("mir-backend: {lowered} of {compared} programs have lowered functions");
    assert!(lowered >= MIN_PROGRAMS_WITH_LOWERED_FUNCTIONS, "only {lowered} programs use the MIR backend");
}

/// Ordered `bork_debug_*` runtime calls with the callee name, one entry per call (M5e, #226 oracle).
fn debug_calls(ir: &str) -> Vec<String> {
    ir.lines()
        .filter(|l| l.contains("call ") && l.contains("@bork_debug_"))
        .map(|l| l.split('@').nth(1).unwrap_or("").split('(').next().unwrap_or("").to_string())
        .collect()
}

/// `-g` on MIR must register exactly what `-g` on HIR registers (RFC 0006 section 1.2: registry contents
/// stay identical), and `lowered` counts programs where MIR emitted at least one function.
#[test]
fn debug_mir_backend_registers_what_the_hir_emitter_registers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs/build");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut compared = 0;
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let result = check(&text);
        let source = || Some(SourceContext { path: &file, text: &text });
        let Ok(hir) = emit_ir_with_source(&result, BuildOptions { debug: true, mir_backend: false }, source()) else {
            continue;
        };
        let mir = emit_ir_with_source(&result, BuildOptions { debug: true, mir_backend: true }, source())
            .unwrap_or_else(|e| panic!("-g MIR backend failed where HIR succeeded for {}: {e:?}", file.display()));
        compared += 1;
        assert_eq!(debug_calls(&hir), debug_calls(&mir), "bork_debug_* calls differ for {}", file.display());
    }
    assert!(compared > 0);
}
