//! `Buf<T>` and its unsafe-only primitives (#285). User source cannot write `unsafe`, so these
//! snippets go through the prelude's parser, like a prelude chunk would.

use crate::diag::Diagnostic;

/// Typeck diagnostics for `source` parsed as prelude code (so `unsafe` is allowed), with the prelude added.
fn prelude_diags(source: &str) -> Vec<Diagnostic> {
    let mut program = crate::parse_unnumbered(source).expect("parses");
    let index = crate::ast_number::number_nodes(&mut program);
    crate::prelude::add_prelude(&mut program, source, index.len() as u32);
    crate::typeck::check_with_members(&program).diagnostics
}

const STACKISH: &str = "class S {\n    buf: Buf<i32>\n    n: i32\n\
    unsafe fun fresh(): Buf<i32> {\n        return bufAlloc::<i32>(arenaOf(this), 4)\n    }\n}\n";

#[test]
fn primitives_check_inside_unsafe() {
    let source = format!(
        "{STACKISH}fun f(b: &Buf<i32>) {{\n    unsafe {{\n        val x: i32 = slotLoad(&b, 0)\n\
         slotStore(&b, 1, x)\n        val g: Buf<i32> = bufGrow(&b, 8)\n        val c: i32 = g.cap()\n\
         if (c < 0) {{\n            abort()\n        }}\n    }}\n}}\nfun main() {{}}\n"
    );
    let d = prelude_diags(&source);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn slot_load_result_is_the_element_type() {
    let d = prelude_diags("unsafe fun f(b: &Buf<i32>) {\n    val x: bool = slotLoad(&b, 0)\n}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("has type i32, expected bool")), "{d:?}");
}

#[test]
fn slot_store_value_must_match_the_element() {
    let d = prelude_diags("unsafe fun f(b: &Buf<i32>) {\n    slotStore(&b, 0, true)\n}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("`slotStore` argument 3 has type bool, expected i32")), "{d:?}");
}

#[test]
fn each_primitive_outside_unsafe_is_rejected() {
    for (name, call) in [
        ("slotLoad", "val x: i32 = slotLoad(&b, 0)"),
        ("slotStore", "slotStore(&b, 0, 1)"),
        ("bufGrow", "val g: Buf<i32> = bufGrow(&b, 8)"),
        ("abort", "abort()"),
        ("bufAlloc", "val g: Buf<i32> = bufAlloc::<i32>(a, 4)"),
    ] {
        let source = format!("fun f(b: &Buf<i32>, a: Arena) {{\n    {call}\n}}\nfun main() {{}}\n");
        let d = prelude_diags(&source);
        let want = format!("`{name}` requires an `unsafe` context");
        assert!(d.iter().any(|d| d.message.contains(&want)), "{name}: {d:?}");
    }
    let d = prelude_diags(&format!("{}fun main() {{}}\n", STACKISH.replace("unsafe fun", "fun")));
    assert!(d.iter().any(|d| d.message.contains("`arenaOf` requires an `unsafe` context")), "{d:?}");
}

#[test]
fn arena_of_takes_only_this() {
    let d = prelude_diags("unsafe fun f(b: &Buf<i32>) {\n    val a = arenaOf(b)\n}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("`arenaOf` takes `this`")), "{d:?}");
}

#[test]
fn user_program_calling_a_primitive_is_rejected() {
    let d = super::diags_of("fun main() {\n    abort()\n}\n");
    assert!(d.iter().any(|d| d.message.contains("`abort` requires an `unsafe` context")), "{d:?}");
}

#[test]
fn a_user_buf_class_blocks_the_chunk() {
    super::hir_of("class Buf {\n    x: i32\n}\nfun main() {\n    val b = Buf(1)\n}\n");
}

#[test]
fn a_local_named_like_a_primitive_shadows_it() {
    super::hir_of("fun main() {\n    val bufGrow: (i32) -> i32 = { n -> n }\n    val x = bufGrow(2)\n}\n");
}

#[test]
fn a_method_named_like_a_primitive_is_called_implicitly() {
    super::hir_of("class C {\n    x: i32\n    fun abort() {}\n    fun run() {\n        abort()\n    }\n}\nfun main() {}\n");
}
