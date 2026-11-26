//! `VecB` (the Bork `Vec`, #285) must behave like the compiler's `Vec` on the same programs. Each program in
//! `PARITY` is rewritten (`Vec::<T>(..)` -> `vecBNew` / `vecBWithCapacity`, `Vec<` -> `VecB<`), built and run, and must exit
//! with the original's `// exit:` code. The list grows as `VecB` gains indexing, moves, `Ref`s and views (plan Tasks 5-8);
//! when `Vec` becomes the Bork class (Task 10) this file is deleted.
#![cfg(feature = "codegen")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The same program with the Bork class: `Vec` -> `VecB` where it is a whole word (`VecIter`, `Vec2` stay).
fn rewrite(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("Vec") {
        let before_ok = rest[..at].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after = &rest[at + 3..];
        let after_ok = after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        out.push_str(&rest[..at]);
        out.push_str(if before_ok && after_ok { "VecB" } else { "Vec" });
        rest = after;
    }
    out.push_str(rest);
    out
}

fn directive(source: &str, key: &str) -> Option<String> {
    source.lines().take(30).find_map(|l| l.trim().strip_prefix("//").and_then(|r| r.trim().strip_prefix(key)).map(|v| v.trim().to_string()))
}

fn programs(dir: &str) -> Vec<(String, String)> {
    let mut out: Vec<_> = std::fs::read_dir(root().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "bork"))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    out.sort();
    out
}

fn tmp(name: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("bork-vec-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join(format!("{name}.bork")), dir.join(name));
    std::fs::write(&src, source).unwrap();
    (src, bin)
}

/// `(exit code, stdout, build stderr)`; the code is -1 when the build fails.
fn build_and_run(source: &str, name: &str, debug: bool) -> (i32, String, String) {
    let (src, bin) = tmp(name, source);
    let mut build = Command::new(env!("CARGO_BIN_EXE_bork"));
    build.arg("build");
    if debug {
        build.arg("-g");
    }
    let build = build.arg("-o").arg(&bin).arg(Path::new(&src)).output().expect("spawn bork build");
    let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
    if !build.status.success() {
        return (-1, String::new(), stderr);
    }
    let run = Command::new(&bin).output().expect("run built program");
    (run.status.code().unwrap_or(1), String::from_utf8_lossy(&run.stdout).into_owned(), stderr)
}

#[test]
fn rewrite_turns_constructors_and_types_into_vecb() {
    let got = rewrite("var a = Vec::<Vec<i32>>()\nval c: Vec<i32> = iter(VecIter(Vec2))\n");
    assert_eq!(got, "var a = VecB::<VecB<i32>>()\nval c: VecB<i32> = iter(VecIter(Vec2))\n");
}

/// Every `programs/build/vec` program, rewritten to `VecB`, must exit with the original's code and print the same
/// `// stdout:`, in a plain and in a `-g` (debug runtime) build.
#[test]
fn vecb_matches_vec_on_the_build_corpus() {
    let mut failures = Vec::new();
    for (name, original) in programs("programs/build/vec") {
        let want_exit: i32 = directive(&original, "exit:").expect("`// exit:`").parse().expect("numeric exit");
        let want_out = directive(&original, "stdout:").map(|s| s.replace("\\n", "\n"));
        for debug in [false, true] {
            let (code, out, stderr) = build_and_run(&rewrite(&original), &name, debug);
            let out_ok = want_out.as_ref().is_none_or(|w| *w == out);
            if code != want_exit || !out_ok {
                failures.push(format!("{name} (VecB{}): exit {code} want {want_exit}, stdout {out:?} want {want_out:?}:\n{stderr}", if debug { ", -g" } else { "" }));
            }
        }
    }
    assert!(failures.is_empty(), "{} failing:\n{}", failures.len(), failures.join("\n"));
}

/// Every `programs/check_fail/vec` program, rewritten to `VecB`, must be rejected with the original's `// diag:` text.
#[test]
fn vecb_matches_vec_on_the_check_fail_corpus() {
    let mut failures = Vec::new();
    for (name, original) in programs("programs/check_fail/vec") {
        let (src, _) = tmp(&format!("cf_{name}"), &rewrite(&original));
        let out = Command::new(env!("CARGO_BIN_EXE_bork")).arg(&src).output().expect("spawn bork");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).replace("VecB", "Vec");
        let wants: Vec<String> = original.lines().take(30).filter_map(|l| l.trim().strip_prefix("//").and_then(|r| r.trim().strip_prefix("diag:")).map(|d| d.trim().to_string())).collect();
        let missing: Vec<_> = wants.iter().filter(|w| !text.contains(w.as_str())).collect();
        if out.status.success() || !missing.is_empty() {
            failures.push(format!("{name}: accepted={} missing {missing:?}; got: {}", out.status.success(), text.lines().next().unwrap_or("")));
        }
    }
    assert!(failures.is_empty(), "{} failing:\n{}", failures.len(), failures.join("\n"));
}
