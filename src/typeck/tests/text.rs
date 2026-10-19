use super::{diags_of, hir_of};

#[test]
fn string_byte_index_is_u8_for_owned_and_borrowed() {
    hir_of("fun f(s: &String): u8 { return s[0] }\nfun main(): u8 {\n val t = \"ab\"\n val b: u8 = t[1]\n return f(&t) + b }");
}

#[test]
fn string_index_requires_i32_and_rejects_writes() {
    let d = diags_of("fun main() {\n val t = \"ab\"\n val b = t[1 > 0]\n}");
    assert!(
        d.iter().any(|d| d.message.contains("must be `i32`")),
        "{d:?}"
    );
    let d = diags_of("fun main() {\n var t = \"ab\"\n t[0] = 1\n}");
    assert!(
        d.iter()
            .any(|d| d.message.contains("index-assign") || d.message.contains("expected an array")),
        "{d:?}"
    );
}

#[test]
fn string_content_equality_accepts_owned_and_borrowed_pairs() {
    hir_of(
        "fun f(a: &String, b: &String): bool { return a == b }\n\
         fun main(): bool {\n val x = \"a\"\n val y = concat(\"a\", \"\")\n\
         return (x == y) && (&x == y) && (x != &y) && f(&x, &y) }",
    );
}

#[test]
fn string_equality_keeps_nullable_semantics() {
    hir_of("fun main(): bool {\n val s: String? = None\n return s == None }");
    let d =
        diags_of("fun main(): bool {\n val s: String? = None\n val t = \"a\"\n return s == t }");
    assert!(
        d.iter().any(|d| d.message.contains("equality operands")),
        "{d:?}"
    );
}

#[test]
fn string_ordering_is_still_rejected() {
    diags_of("fun main(): bool { return \"a\" < \"b\" }");
}

#[test]
fn string_from_utf8_accepts_any_byte_array_length() {
    hir_of(
        "fun large(buf: &[u8; 8192]) { val result = stringFromUtf8(&buf, 0, 0) }\n\
         fun main(): i32 {\n var z: [u8; 0] = []\n var b: [u8; 3] = [1, 2, 3]\n val s = stringFromUtf8(&z, 0, 0)\n val t = stringFromUtf8(&b, 0, 3)\n return 0 }",
    );
}

#[test]
fn string_from_utf8_checks_buffer_and_arguments() {
    let d =
        diags_of("fun main() {\n var b: [i32; 2] = [1, 2]\n val s = stringFromUtf8(&b, 0, 1)\n}");
    assert!(
        d.iter().any(|d| d.message.contains("borrowed byte array")),
        "{d:?}"
    );
    let d = diags_of("fun main() {\n val t = \"ab\"\n val s = stringFromUtf8(&t, 0, 1)\n}");
    assert!(
        d.iter().any(|d| d.message.contains("borrowed byte array")),
        "{d:?}"
    );
    let d = diags_of("fun main() {\n var b: [u8; 2] = [1, 2]\n val s = stringFromUtf8(b, 0, 1)\n}");
    assert!(
        d.iter().any(|d| d.message.contains("borrowed byte array")),
        "{d:?}"
    );
    let d = diags_of("fun main() {\n var b: [u8; 2] = [1, 2]\n val s = stringFromUtf8(&b, 0)\n}");
    assert!(
        d.iter().any(|d| d.message.contains("expects 3 arguments")),
        "{d:?}"
    );
    let d = diags_of(
        "fun main() {\n var b: [u8; 2] = [1, 2]\n val s = stringFromUtf8(&b, 0, 1 > 0)\n}",
    );
    assert!(
        d.iter().any(|d| d.message.contains("must be `i32`")),
        "{d:?}"
    );

    for element in ["i8", "i32"] {
        let d = diags_of(&format!(
            "fun main() {{\n var b: [{element}; 2] = [1, 2]\n val s = stringFromUtf8(&b, 0, 1)\n}}"
        ));
        assert!(
            d.iter().any(|d| d.message.contains("borrowed byte array")),
            "{element}: {d:?}"
        );
    }
}

#[test]
fn string_from_utf8_rejects_trailing_closure() {
    let d = diags_of(
        "fun main() {\n\
         var bytes: [u8; 1] = [65]\n\
         val text = stringFromUtf8(&bytes, 0, 1) { x -> x }\n\
     }",
    );
    assert!(
        d.iter()
            .any(|d| d.message.contains("does not accept trailing closures")),
        "{d:?}"
    );
}

#[test]
fn ref_count_builtins_check_arity_and_trailing_closures() {
    let d = diags_of("fun main() {\n val n = strongCount()\n}");
    assert!(d.iter().any(|d| d.message.contains("`strongCount` expects 1 argument, got 0")), "{d:?}");
    let d = diags_of("class N {\n x: i32\n}\nfun main() {\n val r: Ref<N> = N(1)\n val n = weakCount(r) { x -> x }\n}");
    assert!(d.iter().any(|d| d.message.contains("`weakCount` does not accept trailing closures")), "{d:?}");
}

/// INV-1: builtin callee idents carry a real function type, not `Unknown`.
#[test]
fn special_builtin_callees_have_function_types() {
    use crate::hir::{HirExprKind, HirStmt, TyKind};
    let hir = hir_of(
        "class N {\n x: i32\n}\nfun main() {\n\
         val r: Ref<N> = N(1)\n val c = strongCount(r)\n\
         var b: [u8; 1] = [65]\n val s = stringFromUtf8(&b, 0, 1)\n}",
    );
    let mut seen = 0;
    for stmt in &hir.functions[0].body.stmts {
        let HirStmt::VarDecl { value, .. } = stmt else { continue };
        let HirExprKind::Call { callee, .. } = &value.kind else { continue };
        let TyKind::Func { ret, .. } = &callee.ty.kind else {
            panic!("callee type is {:?}", callee.ty)
        };
        assert_eq!(**ret, value.ty);
        seen += 1;
    }
    assert_eq!(seen, 2);
}
