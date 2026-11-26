//! `VecB` (the Bork `Vec`, #285) must behave like the compiler's `Vec` on the same programs. Each program in
//! `PARITY` is rewritten (`Vec::<T>(..)` -> `vecBNew` / `vecBWithCapacity`, `Vec<` -> `VecB<`), built and run, and must exit
//! with the original's `// exit:` code. The list grows as `VecB` gains indexing, moves, `Ref`s and views (plan Tasks 5-8);
//! when `Vec` becomes the Bork class (Task 10) this file is deleted.
#![cfg(feature = "codegen")]

use std::path::{Path, PathBuf};
use std::process::Command;

const PARITY: &[&str] = &["strings", "negative_capacity_aborts", "push_grow_sum", "pop_set_clear", "grow_past_page", "index_through_field", "vec_field_repeated", "assign_concat_through_borrowed_vec", "index_from_element", "move_to_outer", "move_vec_field"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `Vec::<T>()` -> `vecBNew::<T>()`, `Vec::<T>(n)` -> `vecBWithCapacity::<T>(n)`, then `Vec<` -> `VecB<`.
fn rewrite(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("Vec::<") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "Vec::<".len()..];
        let mut depth = 1;
        let close = after
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
            .map(|(i, _)| i)
            .expect("balanced `Vec::<T>`");
        let (ty, tail) = (&after[..close], &after[close + 1..]);
        assert!(tail.starts_with('('), "`Vec::<{ty}>` is followed by `(`");
        if tail.starts_with("()") {
            out.push_str(&format!("vecBNew::<{ty}>()"));
            rest = &tail[2..];
        } else {
            out.push_str(&format!("vecBWithCapacity::<{ty}>("));
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out.replace("Vec<", "VecB<")
}

fn want_exit(source: &str) -> i32 {
    source
        .lines()
        .find_map(|l| l.trim().strip_prefix("// exit:"))
        .expect("`// exit:` directive")
        .trim()
        .parse()
        .expect("numeric exit")
}

fn build_and_run(source: &str, name: &str) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("bork-vec-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin): (PathBuf, PathBuf) = (dir.join(format!("{name}.bork")), dir.join(name));
    std::fs::write(&src, source).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).arg("build").arg("-o").arg(&bin).arg(Path::new(&src)).output().expect("spawn bork build");
    let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
    if !build.status.success() {
        return (-1, stderr);
    }
    let run = Command::new(&bin).output().expect("run built program");
    (run.status.code().unwrap_or(1), stderr)
}

#[test]
fn rewrite_turns_constructors_and_types_into_vecb() {
    let got = rewrite("var a = Vec::<Vec<i32>>()\nvar b = Vec::<String>(2)\nval c: Vec<i32> = x\n");
    assert_eq!(got, "var a = vecBNew::<VecB<i32>>()\nvar b = vecBWithCapacity::<String>(2)\nval c: VecB<i32> = x\n");
}

#[test]
fn vecb_matches_vec_on_the_parity_programs() {
    for name in PARITY {
        let original = std::fs::read_to_string(root().join("programs/build/vec").join(format!("{name}.bork"))).unwrap();
        let (code, stderr) = build_and_run(&rewrite(&original), name);
        assert_eq!(code, want_exit(&original), "{name} (VecB):\n{stderr}");
    }
}
