#![cfg(feature = "codegen")]

use std::fs;
use std::process::Command;

#[test]
fn debug_build_emits_source_info_and_registry_without_changing_program_result() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "class Box { value: i32 }\nfun main(): i32 {\n    val item = Box(7)\n    val reference: Ref<Box> = item\n    return reference?.value ?: 0\n}\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(7));
    let sections = Command::new("readelf")
        .args(["-S", "--wide"])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(sections.status.success());
    let sections = String::from_utf8_lossy(&sections.stdout);
    assert!(sections.contains(".debug_info"), "{sections}");
    assert!(sections.contains(".debug_line"), "{sections}");
    let symbols = Command::new("nm").arg(&binary).output().unwrap();
    assert!(symbols.status.success());
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    assert!(symbols.contains("bork_debug_registry"), "{symbols}");
}

#[test]
fn debug_api_requires_source_context() {
    use bork::codegen::{build_with_options, BuildOptions};
    let checked = bork::frontend::check("fun main() {}\n");
    let dir = tempfile::tempdir().unwrap();
    let result = build_with_options(
        &checked,
        &dir.path().join("main"),
        BuildOptions { debug: true },
        None,
    );
    assert!(result.is_err());
    assert!(!dir.path().join("main").exists());
}

#[test]
fn debug_build_rejects_a_runtime_without_the_debug_abi() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "fun main() {}\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .env("BORK_RUNTIME_DEBUG_LIB", env!("BORK_RUNTIME_LIB"))
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(2));
    assert!(!binary.exists());
    let errors = String::from_utf8_lossy(&build.stderr);
    assert!(errors.contains("bork_debug_abi_v1"), "{errors}");
}

#[test]
fn ordinary_build_does_not_include_debug_registry() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(&source, "fun main(): i32 { return 7 }\n").unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(7));
    let symbols = Command::new("nm").arg(&binary).output().unwrap();
    assert!(symbols.status.success());
    assert!(!String::from_utf8_lossy(&symbols.stdout).contains("bork_debug_registry"));
}

/// Enum values get DWARF names (`bork::Object<E>` for the handle, `bork::Payload<E>` for the slot)
/// and their `Ref` payload slots a registry label, `E.Variant.field`.
#[test]
fn debug_build_names_enums() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(
        &source,
        "class Box { value: i32 }\nenum Slot { Empty, Held(node: Ref<Box>, tag: i8) }\n\
         fun main(): i32 {\n    val item = Box(7)\n    val reference: Ref<Box> = item\n    \
         val slot = Slot.Held(reference, 1)\n    return 0\n}\n",
    )
    .unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(0));
    let info = Command::new("readelf").args(["--debug-dump=info"]).arg(&binary).output().unwrap();
    let info = String::from_utf8_lossy(&info.stdout);
    assert!(info.contains("bork::Object<Slot>"), "{info}");
    assert!(info.contains("bork::Payload<Slot>"), "{info}");
    let strings = Command::new("strings").arg(&binary).output().unwrap();
    assert!(String::from_utf8_lossy(&strings.stdout).contains("Slot.Held.node"));
}

#[test]
fn debug_build_leaves_scopes_on_break_and_continue_in_nested_loops() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(
        &source,
        "fun main(): i32 {\n    var total = 0\n    for (i in 0..4) {\n        var j = 0\n        while (j < 4) {\n            j = j + 1\n            if (j == 2) { continue }\n            if (j == 3) { break }\n            total = total + i\n        }\n    }\n    return total - 3\n}\n",
    )
    .unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).args(["build", "-g", "-o"]).arg(&binary).arg(&source).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(3));
}

#[test]
fn debug_build_names_generic_instances() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(
        &source,
        "class Box<T> { v: T }\nenum Maybe<T> { Just(v: T), Nothing }\n\
         fun main(): i32 {\n    val a = Box(1)\n    val b = Box(\"s\")\n    \
         val m: Maybe<i32> = Maybe.Just(2)\n    return a.v - 1\n}\n",
    )
    .unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).args(["build", "-g", "-o"]).arg(&binary).arg(&source).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(0));
    let info = Command::new("readelf").args(["--debug-dump=info"]).arg(&binary).output().unwrap();
    let info = String::from_utf8_lossy(&info.stdout);
    // One described type per instance, named with its arguments (the generic `Box` itself has no layout).
    for name in ["bork::Object<Box<i32>>", "bork::Object<Box<String>>", "bork::Payload<Box<i32>>", "bork::Object<Maybe<i32>>"] {
        assert!(info.contains(name), "missing `{name}` in DWARF");
    }
}

#[test]
fn debug_build_describes_nullable_as_an_option_enum_handle() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("main.bork");
    let binary = dir.path().join("main");
    fs::write(
        &source,
        "fun main(): i32 {\n    val n: i32? = Some(42)\n    val s: String? = Some(\"hi\")\n    \
         val p: Option<i32> = Option.None\n    return 0\n}\n",
    )
    .unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork"))
        .args(["build", "-g", "-o"])
        .arg(&binary)
        .arg(&source)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert_eq!(Command::new(&binary).status().unwrap().code(), Some(0));
    let info = Command::new("readelf").args(["--debug-dump=info"]).arg(&binary).output().unwrap();
    let info = String::from_utf8_lossy(&info.stdout);
    for name in ["bork::Object<Option<i32>>", "bork::Object<Option<String>>"] {
        assert!(info.contains(name), "{name}: {info}");
    }
    // The old ABI's flag and inline payload must not be described for the `T?` aliases.
    assert!(!info.contains("): present\n"), "{info}");
}

/// Total bytes in use over all live arenas at the line after `find` returned, for a loop of `target + 1` iterations (#338).
fn arena_bytes_after_loop(dir: &std::path::Path, target: u32) -> usize {
    let source = dir.join(format!("loop{target}.bork"));
    let binary = dir.join(format!("loop{target}"));
    fs::write(
        &source,
        format!(
            "class Node {{ n: i32 }}\n\
fun pick(v: &Vec<Ref<Node>>, i: i32): Option<Ref<Node>> {{\n    if (i >= v.len()) {{ return None }}\n    return Some(v.get(i))\n}}\n\
fun isNone(o: Option<Ref<Node>>): bool {{\n    when (o) {{\n        Some(r) => {{ return false }}\n        None => {{ return true }}\n    }}\n    return true\n}}\n\
fun make(n: i32): (i32) -> i32 {{\n    return move (n) {{ x -> x + n }}\n}}\n\
fun apply(f: (i32) -> i32, x: i32): i32 {{\n    return f(x)\n}}\n\
fun find(v: &Vec<Ref<Node>>, target: i32): Option<Ref<Node>> {{\n    var i = 0\n    var sum = 0\n    while (true) {{\n        val p = pick(&v, i)\n        val f = make(i)\n        sum = sum + apply(f, 0)\n        if (isNone(p)) {{ return None }}\n        if (i == target) {{ return p }}\n        i = i + 1\n    }}\n    return None\n}}\n\
fun main(): i32 {{\n    var v = Vec::<Ref<Node>>()\n    var k = 0\n    while (k < 400) {{\n        v.push(Node(k))\n        k = k + 1\n    }}\n    val found = find(&v, {target})\n    println(1)\n    return 0\n}}\n"
        ),
    )
    .unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).args(["build", "-g", "-o"]).arg(&binary).arg(&source).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let line = fs::read_to_string(&source).unwrap().lines().position(|l| l.contains("println(1)")).unwrap() + 1;
    let adapter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/bork-debug/bork_gdb.py");
    let gdb = Command::new("gdb")
        .env("DEBUGINFOD_URLS", "")
        .args(["-q", "-nx", "-batch", "-ex", "set pagination off", "-ex"])
        .arg(format!("source {}", adapter.display()))
        .arg("-ex")
        .arg(format!("break {}:{line}", source.display()))
        .args(["-ex", "run", "-ex", "bork-arenas"])
        .arg(&binary)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&gdb.stdout);
    assert!(out.contains("arena "), "no arenas listed:\n{out}\n{}", String::from_utf8_lossy(&gdb.stderr));
    out.lines()
        .filter(|l| l.starts_with("arena "))
        .filter_map(|l| l.split("bytes=").nth(1)?.split('/').next()?.parse::<usize>().ok())
        .sum()
}

/// A call result bound in a loop (a handle, a closure) stays in the iteration region: the arenas hold the same bytes after
/// 6 iterations as after 301. Skipped without `gdb`.
#[test]
fn loop_call_results_do_not_pile_up_in_the_result_arena() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (short, long) = (arena_bytes_after_loop(dir.path(), 5), arena_bytes_after_loop(dir.path(), 300));
    assert_eq!(short, long, "result arena grows with the iteration count");
}

/// Weak count of the arena `Node`s live in (the one with the largest strong count) at each of `lines`, read with gdb.
fn node_arena_weak_counts(dir: &std::path::Path, name: &str, source: &str, lines: &[usize]) -> Vec<usize> {
    let (path, binary) = (dir.join(format!("{name}.bork")), dir.join(name));
    fs::write(&path, source).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).args(["build", "-g", "-o"]).arg(&binary).arg(&path).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let adapter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/bork-debug/bork_gdb.py");
    let mut gdb = Command::new("gdb");
    gdb.env("DEBUGINFOD_URLS", "").args(["-q", "-nx", "-batch", "-ex", "set pagination off", "-ex"]).arg(format!("source {}", adapter.display()));
    for line in lines {
        gdb.arg("-ex").arg(format!("break {}:{line}", path.display()));
    }
    gdb.args(["-ex", "run"]);
    for i in 0..lines.len() {
        if i > 0 {
            gdb.args(["-ex", "continue"]);
        }
        gdb.args(["-ex", "echo SNAPSHOT\\n", "-ex", "bork-arenas"]);
    }
    let out = gdb.arg(&binary).output().unwrap();
    let out = String::from_utf8_lossy(&out.stdout).into_owned();
    let field = |l: &str, key: &str| l.split(key).nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<usize>().ok());
    let counts: Vec<usize> = out
        .split("SNAPSHOT")
        .skip(1)
        .filter_map(|snap| snap.lines().filter(|l| l.starts_with("arena ")).max_by_key(|l| field(l, "strong=")).and_then(|l| field(l, "weak=")))
        .collect();
    assert_eq!(counts.len(), lines.len(), "missing snapshots:\n{out}");
    counts
}

/// A returned `Option<Ref<Node>>` or closure owning a `Ref` adds exactly one weak handle to the `Node` arena while it
/// lives, however many iterations the loop ran, and releases it with its arena (#338).
#[test]
fn returned_loop_results_own_exactly_one_handle() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let option = "class Node { n: i32 }\nfun direct(r: Ref<Node>): Option<Ref<Node>> {\n    return Some(r)\n}\nfun looped(r: Ref<Node>): Option<Ref<Node>> {\n    var i = 0\n    while (true) {\n        val p = direct(r)\n        if (i == 2) { return p }\n        i = i + 1\n    }\n    return None\n}\nfun main(): i32 {\n    val node = Node(7)\n    val r: Ref<Node> = node\n    println(0)\n    {\n        val a = looped(r)\n        println(1)\n    }\n    println(2)\n    return 0\n}\n";
    let closure = "class Node { n: i32 }\nfun make(r: Ref<Node>): () -> i32 {\n    return move (r) { -> if val n = r { n.n } else { 0 } }\n}\nfun looped(r: Ref<Node>): () -> i32 {\n    var i = 0\n    while (true) {\n        val f = make(r)\n        if (i == 2) { return f }\n        i = i + 1\n    }\n    return make(r)\n}\nfun main(): i32 {\n    val node = Node(7)\n    val r: Ref<Node> = node\n    println(0)\n    {\n        val g = looped(r)\n        println(g())\n    }\n    println(2)\n    return 0\n}\n";
    // Before the block, inside it, after it.
    assert_eq!(node_arena_weak_counts(dir.path(), "option", option, &[17, 20, 22]), [0, 1, 0]);
    assert_eq!(node_arena_weak_counts(dir.path(), "closure", closure, &[17, 20, 22]), [0, 1, 0]);
}

/// Whether `gdb` can run. CI installs it, so there a missing `gdb` fails the test instead of silently skipping it.
fn gdb_available() -> bool {
    let present = Command::new("gdb").arg("--version").output().is_ok();
    assert!(present || std::env::var_os("CI").is_none(), "gdb is required in CI");
    if !present {
        eprintln!("gdb is unavailable; skipping");
    }
    present
}

/// Same as the option case for an enum of the program's own: `copy_ref_enum_into` clones the handle once and the copy
/// drops it with its arena (#338).
#[test]
fn returned_loop_user_enum_owns_exactly_one_handle() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = "class Node { n: i32 }\nenum E { Has(p: Ref<Node>, k: i32), Empty }\nfun build(r: Ref<Node>, k: i32): E {\n    return E.Has(r, k)\n}\nfun looped(r: Ref<Node>): E {\n    var i = 0\n    while (true) {\n        val e = build(r, i)\n        if (i == 2) { return e }\n        i = i + 1\n    }\n    return E.Empty\n}\nfun main(): i32 {\n    val node = Node(7)\n    val r: Ref<Node> = node\n    println(0)\n    {\n        val a = looped(r)\n        println(1)\n    }\n    println(2)\n    return 0\n}\n";
    assert_eq!(node_arena_weak_counts(dir.path(), "user_enum", source, &[18, 21, 23]), [0, 1, 0]);
}

/// An enum with several `Ref`-carrying variants, each returned from its own loop and held at once: the copy follows the
/// tag, so each owns one handle (#338).
#[test]
fn returned_loop_enum_variants_each_own_one_handle() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = "class Node { n: i32 }\nenum E { A(p: Ref<Node>), B(q: Ref<Node>, k: i32), C }\nfun mk(r: Ref<Node>, which: i32): E {\n    if (which == 0) { return E.A(r) }\n    if (which == 1) { return E.B(r, 5) }\n    return E.C\n}\nfun looped(r: Ref<Node>, which: i32): E {\n    var i = 0\n    while (true) {\n        val e = mk(r, which)\n        if (i == 2) { return e }\n        i = i + 1\n    }\n    return E.C\n}\nfun main(): i32 {\n    val node = Node(7)\n    val r: Ref<Node> = node\n    println(0)\n    {\n        val a = looped(r, 0)\n        val b = looped(r, 1)\n        val c = looped(r, 2)\n        println(1)\n    }\n    println(2)\n    return 0\n}\n";
    assert_eq!(node_arena_weak_counts(dir.path(), "variants", source, &[20, 25, 27]), [0, 2, 0]);
}

/// An enum holding another `Ref`-carrying enum by value: the inner copy registers its own drop, so the handle is released
/// once (#338).
#[test]
fn returned_loop_nested_enum_owns_exactly_one_handle() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = "class Node { n: i32 }\nenum In { Has(p: Ref<Node>), No }\nenum Out { Wrap(i: In, k: i32), Empty }\nfun mk(r: Ref<Node>): Out {\n    return Out.Wrap(In.Has(r), 3)\n}\nfun looped(r: Ref<Node>): Out {\n    var i = 0\n    while (true) {\n        val e = mk(r)\n        if (i == 2) { return e }\n        i = i + 1\n    }\n    return Out.Empty\n}\nfun main(): i32 {\n    val node = Node(7)\n    val r: Ref<Node> = node\n    println(0)\n    {\n        val a = looped(r)\n        println(1)\n    }\n    println(2)\n    return 0\n}\n";
    assert_eq!(node_arena_weak_counts(dir.path(), "nested", source, &[19, 22, 24]), [0, 1, 0]);
}

/// Total bytes in use over all live arenas when the program reaches its `println(1)` line, read with gdb.
fn total_arena_bytes(dir: &std::path::Path, name: &str, source: &str) -> usize {
    let (path, binary) = (dir.join(format!("{name}.bork")), dir.join(name));
    fs::write(&path, source).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_bork")).args(["build", "-g", "-o"]).arg(&binary).arg(&path).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let line = source.lines().position(|l| l.contains("println(1)")).unwrap() + 1;
    let adapter = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/bork-debug/bork_gdb.py");
    let out = Command::new("gdb")
        .env("DEBUGINFOD_URLS", "")
        .args(["-q", "-nx", "-batch", "-ex", "set pagination off", "-ex"])
        .arg(format!("source {}", adapter.display()))
        .arg("-ex")
        .arg(format!("break {}:{line}", path.display()))
        .args(["-ex", "run", "-ex", "bork-arenas"])
        .arg(&binary)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout);
    assert!(out.contains("arena "), "no arenas listed:\n{out}");
    out.lines().filter_map(|l| l.split("bytes=").nth(1)?.split('/').next()?.parse::<usize>().ok()).sum()
}

/// An enum holding a class object, bound in a loop and only matched, never leaves the iteration: `return` could not copy
/// it out whole, so only the `local_loop_calls` analysis keeps it out of the result sink (#327, #338).
#[test]
fn loop_matched_class_enum_does_not_pile_up() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = |n: u32| format!("class Obj {{ v: i32 }}\nenum W {{ Has(o: Obj), No }}\nfun mk(i: i32): W {{ return W.Has(Obj(i)) }}\nfun scan(n: i32): W {{\n    var i = 0\n    var sum = 0\n    while (i < n) {{\n        val w = mk(i)\n        when (w) {{\n            W.Has(o) => {{ sum = sum + o.v }}\n            W.No => {{ sum = sum + 0 }}\n        }}\n        i = i + 1\n    }}\n    return W.No\n}}\nfun main(): i32 {{\n    val s = scan({n})\n    println(1)\n    return 0\n}}\n");
    let (short, long) = (total_arena_bytes(dir.path(), "short", &source(5)), total_arena_bytes(dir.path(), "long", &source(300)));
    assert_eq!(short, long, "a class-holding enum result piles up per iteration");
}

/// `val q = w` before the `when`: the alias only reaches the scrutinee, so the call result still stays in the iteration (#344).
#[test]
fn loop_aliased_class_enum_does_not_pile_up() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = |n: u32| format!("class Obj {{ v: i32 }}\nenum W {{ Has(o: Obj), No }}\nfun mk(i: i32): W {{ return W.Has(Obj(i)) }}\nfun scan(n: i32): W {{\n    var i = 0\n    var sum = 0\n    while (i < n) {{\n        val w = mk(i)\n        val q = w\n        when (q) {{\n            W.Has(o) => {{ sum = sum + o.v }}\n            W.No => {{ sum = sum + 0 }}\n        }}\n        i = i + 1\n    }}\n    return W.No\n}}\nfun main(): i32 {{\n    val s = scan({n})\n    println(1)\n    return 0\n}}\n");
    let (short, long) = (total_arena_bytes(dir.path(), "short", &source(5)), total_arena_bytes(dir.path(), "long", &source(300)));
    assert_eq!(short, long, "an aliased class-holding enum result piles up per iteration");
}

/// An alias that is returned escapes the iteration, so the call result must keep the result sink (not `Sink::Local`) (#344).
#[test]
fn loop_alias_that_escapes_keeps_the_result_sink() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = |n: u32| format!("class Obj {{ v: i32 }}\nenum W {{ Has(o: Obj), No }}\nfun mk(i: i32): W {{ return W.Has(Obj(i)) }}\nfun scan(n: i32): W {{\n    var i = 0\n    while (i < n) {{\n        val w = mk(i)\n        val q = w\n        if (i == n - 1) {{\n            return q\n        }}\n        i = i + 1\n    }}\n    return W.No\n}}\nfun main(): i32 {{\n    val s = scan({n})\n    println(1)\n    return 0\n}}\n");
    let (short, long) = (total_arena_bytes(dir.path(), "short", &source(5)), total_arena_bytes(dir.path(), "long", &source(300)));
    assert_ne!(short, long, "an escaping alias must not be reset with the iteration region");
}

/// The same name declared again outside the loop (and returned) must not stop the loop's own `w` staying in the iteration (#344).
#[test]
fn loop_call_result_with_reused_name_does_not_pile_up() {
    if !gdb_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = |n: u32| format!("class Obj {{ v: i32 }}\nenum W {{ Has(o: Obj), No }}\nfun mk(i: i32): W {{ return W.Has(Obj(i)) }}\nfun scan(n: i32): W {{\n    var i = 0\n    var sum = 0\n    while (i < n) {{\n        val w = mk(i)\n        when (w) {{\n            W.Has(o) => {{ sum = sum + o.v }}\n            W.No => {{ sum = sum + 0 }}\n        }}\n        i = i + 1\n    }}\n    val w = mk(0)\n    return w\n}}\nfun main(): i32 {{\n    val s = scan({n})\n    println(1)\n    return 0\n}}\n");
    let (short, long) = (total_arena_bytes(dir.path(), "short", &source(5)), total_arena_bytes(dir.path(), "long", &source(300)));
    assert_eq!(short, long, "a reused name keeps the loop result on the result sink");
}
