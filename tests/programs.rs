//! Walk `programs/` and run each `.bork` file according to its parent directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suite {
    Build,
    Check,
    CheckFail,
    BuildFail,
}

#[derive(Debug, Default)]
struct Directives {
    exit: Option<i32>,
    stdout: Option<String>,
    diag: Vec<String>,
}

fn programs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs")
}

fn suite_from_dir(dir: &Path) -> Option<Suite> {
    let name = dir.file_name()?.to_string_lossy();
    match name.as_ref() {
        "build" => Some(Suite::Build),
        "check" => Some(Suite::Check),
        "check_fail" => Some(Suite::CheckFail),
        "build_fail" => Some(Suite::BuildFail),
        _ => None,
    }
}

fn parse_directives(source: &str) -> Directives {
    let mut d = Directives::default();
    for line in source.lines().take(30) {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("//") else {
            continue;
        };
        let rest = rest.trim();
        if let Some(v) = rest.strip_prefix("exit:") {
            d.exit = Some(
                v.trim()
                    .parse()
                    .expect("malformed `// exit:` directive in programs corpus"),
            );
        } else if let Some(v) = rest.strip_prefix("stdout:") {
            let s = v.trim().replace("\\n", "\n");
            d.stdout = Some(s);
        } else if let Some(v) = rest.strip_prefix("diag:") {
            d.diag.push(v.trim().to_string());
        }
    }
    d
}

fn collect_suite_tree(suite_dir: &Path, suite: Suite, out: &mut Vec<(Suite, PathBuf)>) {
    for entry in fs::read_dir(suite_dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_suite_tree(&path, suite, out);
        } else if path.extension().is_some_and(|e| e == "bork") {
            out.push((suite, path));
        }
    }
}

fn collect_bork_files(dir: &Path, out: &mut Vec<(Suite, PathBuf)>) {
    for entry in fs::read_dir(dir).expect("read programs dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(suite) = suite_from_dir(&path) {
                collect_suite_tree(&path, suite, out);
            }
        }
    }
}

fn bork_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bork"))
}

fn run_check(path: &Path) -> (i32, String) {
    let out = Command::new(bork_exe())
        .arg(path)
        .output()
        .expect("spawn bork");
    let code = out.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (code, stderr)
}

#[test]
fn programs_corpus_check_and_check_fail() {
    let mut files = Vec::new();
    collect_bork_files(&programs_root(), &mut files);
    files.sort_by(|a, b| a.1.cmp(&b.1));

    for (suite, path) in files {
        match suite {
            Suite::Check => {
                let (code, stderr) = run_check(&path);
                assert_eq!(
                    code,
                    0,
                    "{}: expected check success, stderr:\n{stderr}",
                    path.display()
                );
            }
            Suite::CheckFail => {
                let source = fs::read_to_string(&path).unwrap();
                let expect = parse_directives(&source);
                let (code, stderr) = run_check(&path);
                assert_ne!(
                    code,
                    0,
                    "{}: expected check failure",
                    path.display()
                );
                for needle in &expect.diag {
                    assert!(
                        stderr.contains(needle),
                        "{}: stderr missing `{needle}`:\n{stderr}",
                        path.display()
                    );
                }
            }
            Suite::Build | Suite::BuildFail => {}
        }
    }
}

#[cfg(feature = "codegen")]
mod codegen {
    use super::*;

    fn build_and_run(path: &Path, out_bin: &Path) -> (i32, String, String) {
        let build = Command::new(bork_exe())
            .args(["build", "-o"])
            .arg(out_bin)
            .arg(path)
            .output()
            .expect("spawn bork build");
        let build_code = build.status.code().unwrap_or(1);
        let build_stderr = String::from_utf8_lossy(&build.stderr).into_owned();
        if build_code != 0 {
            return (build_code, build_stderr, String::new());
        }
        let run = Command::new(out_bin).output().expect("run binary");
        let run_code = run.status.code().unwrap_or(1);
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        (run_code, build_stderr, stdout)
    }

    #[test]
    fn programs_corpus_build_and_build_fail() {
        let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bork-programs");
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&scratch).unwrap();

        let mut files = Vec::new();
        collect_bork_files(&programs_root(), &mut files);
        files.sort_by(|a, b| a.1.cmp(&b.1));

        for (suite, path) in files {
            match suite {
                Suite::Build => {
                    let source = fs::read_to_string(&path).unwrap();
                    let expect = parse_directives(&source);
                    let bin = scratch.join(path.file_stem().unwrap());
                    let (run_code, build_stderr, stdout) = build_and_run(&path, &bin);
                    assert!(
                        bin.exists(),
                        "{}: build failed:\n{build_stderr}",
                        path.display()
                    );
                    let want_exit = expect.exit.unwrap_or(0);
                    assert_eq!(
                        run_code,
                        want_exit,
                        "{}: exit {run_code}, want {want_exit}, stderr:\n{build_stderr}",
                        path.display()
                    );
                    if let Some(want_out) = &expect.stdout {
                        assert_eq!(
                            stdout,
                            *want_out,
                            "{}: stdout mismatch",
                            path.display()
                        );
                    }
                }
                Suite::BuildFail => {
                    let source = fs::read_to_string(&path).unwrap();
                    let expect = parse_directives(&source);
                    let bin = scratch.join(path.file_stem().unwrap());
                    let build = Command::new(bork_exe())
                        .args(["build", "-o"])
                        .arg(&bin)
                        .arg(&path)
                        .output()
                        .expect("spawn bork build");
                    assert_ne!(
                        build.status.code(),
                        Some(0),
                        "{}: expected build failure",
                        path.display()
                    );
                    let stderr = String::from_utf8_lossy(&build.stderr);
                    for needle in &expect.diag {
                        assert!(
                            stderr.contains(needle),
                            "{}: build stderr missing `{needle}`:\n{stderr}",
                            path.display()
                        );
                    }
                    assert!(!bin.exists(), "{}: binary should not exist", path.display());
                }
                Suite::Check | Suite::CheckFail => {}
            }
        }
    }
}
