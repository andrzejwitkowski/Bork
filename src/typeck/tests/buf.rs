//! `Buf<T>` and its unsafe-only primitives (#285). User source cannot write `unsafe`, so these
//! snippets go through the prelude's parser, like a prelude chunk would.

use crate::diag::Diagnostic;

/// Typeck diagnostics for `source` parsed as prelude code (so `unsafe` is allowed), with the prelude added.
fn prelude_diags(source: &str) -> Vec<Diagnostic> {
    let mut program = crate::parse_unnumbered(source).expect("parses");
    program.mark_prelude();
    let index = crate::ast_number::number_nodes(&mut program);
    crate::prelude::add_prelude(&mut program, source, index.len() as u32);
    crate::typeck::check_with_members(&program).diagnostics
}

const STACKISH: &str = "class S {\n    buf: Buf<i32>\n    n: i32\n\
    unsafe fun fresh(): Buf<i32> {\n        return bufAlloc::<i32>(arenaOf(this), 4)\n    }\n\
    unsafe fun grown(b: &Buf<i32>): Buf<i32> {\n        return bufGrow(&b, 8)\n    }\n}\n";

#[test]
fn primitives_check_inside_unsafe() {
    let source = format!(
        "{STACKISH}fun f(b: &Buf<i32>) {{\n    unsafe {{\n        val x: i32 = slotLoad(&b, 0)\n\
         slotStore(&b, 1, x)\n        val c: i32 = b.cap()\n\
         if (c < 0) {{\n            abort()\n        }}\n    }}\n}}\nfun main() {{}}\n"
    );
    let d = prelude_diags(&source);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn buf_grow_outside_a_method_is_rejected() {
    let d = prelude_diags("unsafe fun f(b: &Buf<i32>) {\n    val g: Buf<i32> = bufGrow(&b, 8)\n}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("`bufGrow` can only be called inside a method")), "{d:?}");
}

#[test]
fn arena_of_this_outside_a_method_is_rejected() {
    let d = prelude_diags("unsafe fun f() {\n    val a = arenaOf(this)\n}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("`arenaOf` can only be called inside a method")), "{d:?}");
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

/// Typeck diagnostics for a user program (no `unsafe`) that uses the prelude's `Stack`.
fn stack_user_diags(body: &str) -> Vec<Diagnostic> {
    super::diags_of(&format!("fun main() {{\n    var s = newStack::<i32>()\n{body}}}\n"))
}

fn assert_private(body: &str, want: &str) {
    let d = stack_user_diags(body);
    assert!(d.iter().any(|d| d.message.contains(want)), "{body}: {d:?}");
}

#[test]
fn user_cannot_read_a_stack_field() {
    assert_private("    val b = s.buf\n", "field `buf` of `Stack` is private to the prelude");
    assert_private("    val n = s.n\n", "field `n` of `Stack` is private to the prelude");
}

#[test]
fn user_cannot_write_a_stack_field() {
    assert_private("    s.n = 0\n", "field `n` of `Stack` is private to the prelude");
}

#[test]
fn user_cannot_construct_an_opaque_class() {
    let d = super::diags_of("fun main() {\n    val t: StackSeed<i32> = StackSeed(0)\n}\n");
    assert!(d.iter().any(|d| d.message.contains("construction of `StackSeed` is private to the prelude")), "{d:?}");
}

#[test]
fn user_cannot_call_an_unsafe_prelude_method() {
    let mut program = crate::parse_unnumbered("class P {\n    n: i32\n    unsafe fun f() {}\n}\nfun mk(): P {\n    return P(0)\n}\n").expect("parses");
    program.mark_prelude();
    program.functions.extend(crate::parse_unnumbered("fun main() {\n    val p = mk()\n    p.f()\n}\n").expect("parses").functions);
    let index = crate::ast_number::number_nodes(&mut program);
    crate::prelude::add_prelude(&mut program, "", index.len() as u32);
    let d = crate::typeck::check_with_members(&program).diagnostics;
    assert!(d.iter().any(|d| d.message.contains("`P.f` is an `unsafe` method private to the prelude")), "{d:?}");
}

#[test]
fn user_cannot_name_buf() {
    let d = super::diags_of("fun f(b: &Buf<i32>) {}\nfun main() {}\n");
    assert!(d.iter().any(|d| d.message.contains("`Buf` is private to the prelude")), "{d:?}");
    let d = super::diags_of("fun main() {\n    val b: Buf<i32>? = null\n}\n");
    assert!(d.iter().any(|d| d.message.contains("`Buf` is private to the prelude")), "{d:?}");
}

#[test]
fn user_stack_api_stays_available() {
    super::hir_of("fun main() {\n    var s = newStack::<i32>()\n    s.push(1)\n    val x: i32 = s.get(0)\n    val n: i32 = s.len()\n}\n");
}

#[test]
fn a_user_class_named_buf_still_works() {
    super::hir_of("class Buf {\n    x: i32\n}\nfun f(b: Buf): i32 {\n    return b.x\n}\nfun main() {}\n");
}

#[test]
fn vecb_index_reads_the_element_type() {
    super::hir_of("fun main() {\n    var v = vecNew::<i32>()\n    v.push(1)\n    val x: i32 = v[0]\n}\n");
}

#[test]
fn vecb_index_assign_checks_the_element_type() {
    let diags = super::diags_of("fun main() {\n    var v = vecNew::<i32>()\n    v.push(1)\n    v[0] = \"s\"\n}\n");
    assert!(diags.iter().any(|d| d.message.contains("mismatch") || d.message.contains("expected")), "{diags:?}");
}

#[test]
fn vecb_index_assign_on_a_val_is_rejected() {
    let diags = super::diags_of("fun main() {\n    val v = vecNew::<i32>()\n    v[0] = 1\n}\n");
    assert!(diags.iter().any(|d| d.message.contains("immutable `val` binding `v`")), "{diags:?}");
}

#[test]
fn a_user_class_named_vecb_is_not_indexable() {
    let diags = super::diags_of("class Vec<T> {\n    n: i32\n}\nfun main() {\n    val v: Vec<i32> = Vec(1)\n    val x = v[0]\n}\n");
    assert!(diags.iter().any(|d| d.message.contains("indexing requires an array type")), "{diags:?}");
}
