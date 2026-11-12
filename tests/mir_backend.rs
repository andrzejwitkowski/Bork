//! The default MIR backend against `--hir-backend` on `programs/build` (RFC 0006 M5a, #222): the same region
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
    run_with(source, out, mir, false)
}

fn run_with(source: &Path, out: &Path, mir: bool, debug: bool) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bork"));
    cmd.arg("build");
    if debug {
        cmd.arg("-g");
    }
    if !mir {
        cmd.arg("--hir-backend");
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
        let Ok(hir) = emit_ir(&result, BuildOptions { debug: false, hir_backend: true }) else {
            continue; // not buildable (gate); `tests/programs.rs` owns that
        };
        let mir = emit_ir(&result, BuildOptions { debug: false, hir_backend: false })
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

/// `bork_debug_*` runtime calls per function, sorted: the two backends create basic blocks in a
/// different order, so only the multiset of calls in each function is comparable (M5e, #226 oracle).
fn debug_calls(ir: &str) -> Vec<Vec<String>> {
    ir.split("\ndefine ")
        .skip(1)
        .map(|function| {
            let mut calls: Vec<String> = function
                .lines()
                .take_while(|l| *l != "}")
                .filter(|l| l.contains("call ") && l.contains("@bork_debug_"))
                .map(|l| l.split('@').nth(1).unwrap_or("").split('(').next().unwrap_or("").to_string())
                // Assertions, not registry contents: HIR re-checks the handle once per `when` arm, MIR reads the tag once.
                // `bork_debug_site` is stamped before every runtime call, and HIR makes calls MIR skips.
                .filter(|c| !matches!(c.as_str(), "bork_debug_check" | "bork_debug_check_handle" | "bork_debug_site"))
                .collect();
            calls.sort();
            calls
        })
        .collect()
}

/// Calls whose count is part of the registry contents. Allocation labels, copy origins and scope exits
/// are only checked as a subset (MIR must not add a kind of event): HIR copies strings the MIR backend proves unnecessary and duplicates
/// cleanup paths, which is not a registry difference.
fn exact(call: &String) -> bool {
    matches!(call.as_str(), "bork_debug_label_arena" | "bork_debug_register_slot" | "bork_debug_label_slot" | "bork_debug_frame_enter" | "bork_debug_frame_exit" | "bork_debug_scope_enter")
}

/// `-g` on MIR must register exactly what `-g` on HIR registers (RFC 0006 section 1.2: registry contents
/// stay identical), and `lowered` counts programs where MIR emitted at least one function.
#[test]
fn debug_mir_backend_registers_what_the_hir_emitter_registers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs/build");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let (mut compared, mut lowered) = (0, 0);
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let result = check(&text);
        let source = || Some(SourceContext { path: &file, text: &text });
        let Ok(hir) = emit_ir_with_source(&result, BuildOptions { debug: true, hir_backend: true }, source()) else {
            continue;
        };
        let mir = emit_ir_with_source(&result, BuildOptions { debug: true, hir_backend: false }, source())
            .unwrap_or_else(|e| panic!("-g MIR backend failed where HIR succeeded for {}: {e:?}", file.display()));
        compared += 1;
        if hir != mir {
            lowered += 1;
            // The registry checks run at run time under `-g`: both backends must behave the same.
            let bin = std::env::temp_dir().join(format!("bork-mir-g-{}", std::process::id()));
            let expected = run_with(&file, &bin, false, true);
            assert_eq!(run_with(&file, &bin, true, true), expected, "-g output differs for {}", file.display());
            let _ = fs::remove_file(&bin);
        }
        let (h, m) = (debug_calls(&hir), debug_calls(&mir));
        for (i, (h, m)) in h.iter().zip(&m).enumerate() {
            let strip = |v: &Vec<String>| -> (Vec<String>, std::collections::BTreeSet<String>) {
                (v.iter().filter(|c| exact(c)).cloned().collect(), v.iter().filter(|c| !exact(c)).cloned().collect())
            };
            // MIR may elide copies (and so their labels) but never invent a kind of registry event.
            if strip(h).0 != strip(m).0 || !strip(m).1.is_subset(&strip(h).1) {
                let only = |a: &Vec<String>, b: &Vec<String>| {
                    let mut rest = b.clone();
                    a.iter().filter(|c| rest.iter().position(|r| r == *c).map(|i| rest.remove(i)).is_none()).cloned().collect::<Vec<_>>()
                };
                panic!("bork_debug_* calls differ in function #{i} of {}: only HIR {:?}, only MIR {:?}", file.display(), only(h, m), only(m, h));
            }
        }
        assert_eq!(h.len(), m.len());
    }
    eprintln!("-g mir-backend: {lowered} of {compared} programs have lowered functions");
    assert!(compared > 0);
}
