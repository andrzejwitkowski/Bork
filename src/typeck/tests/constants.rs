use super::{diags_of, hir_of};
use crate::hir::{HirExprKind, HirStmt, Prim, Ty};

fn first_error(src: &str) -> String {
    diags_of(src)[0].message.clone()
}

#[test]
fn global_constants_literals_and_forward_use() {
    let src = r#"
fun main(): i32 {
    return port() + 1
}

fun port(): i32 { return PORT }

const PORT: i32 = 8080
const ENABLED: bool = true
const RATIO: f32 = 1.5
const RESPONSE: String = "OK\r\n"
const B: u8 = 255

fun use_all(): String {
    val b: u8 = B
    val on: bool = ENABLED
    val r: f32 = RATIO
    return RESPONSE
}
"#;
    let hir = hir_of(src);
    let port = hir.functions.iter().find(|f| f.name == "port").unwrap();
    let HirStmt::Return { value: Some(value) } = &port.body.stmts[0] else {
        panic!()
    };
    assert_eq!(value.kind, HirExprKind::Int { value: 8080 });
    assert_eq!(value.ty, Ty::i32());
    let all = hir.functions.iter().find(|f| f.name == "use_all").unwrap();
    let HirStmt::VarDecl { value, .. } = &all.body.stmts[0] else {
        panic!("{:?}", all.body.stmts[0])
    };
    assert_eq!(value.ty, Ty::prim(Prim::U8));
}

#[test]
fn global_constant_integer_range_checked() {
    assert!(first_error("const B: u8 = 256\nfun main() {}").contains("out of range"));
    assert!(first_error("const B: i8 = 128\nfun main() {}").contains("out of range"));
    hir_of(
        "const A: i8 = 127\nconst B: u8 = 255\nconst C: i64 = 9223372036854775807\nfun main() {}",
    );
}

#[test]
fn global_constant_float_overflow_rejected() {
    assert!(first_error(
        "const F: f32 = 99999999999999999999999999999999999999999.0\nfun main() {}"
    )
    .contains("out of range"));
    hir_of("const F: f64 = 99999999999999999999999999999999999999999.0\nfun main() {}");
}

#[test]
fn global_constant_initializer_must_be_typed_literal() {
    assert!(first_error("const A: i32 = 1 + 2\nfun main() {}").contains("not a i32 literal"));
    assert!(first_error("const A: i32 = \"x\"\nfun main() {}").contains("not a i32 literal"));
    assert!(first_error("const A: String? = \"x\"\nfun main() {}").contains("not supported"));
    assert!(
        first_error("const A: i32 = 1\nconst B: i32 = A\nfun main() {}")
            .contains("not a i32 literal")
    );
    assert!(first_error("const A: [i32; 2] = [1, 2]\nfun main() {}").contains("not supported"));
}

#[test]
fn global_constant_names_and_shadowing() {
    assert!(
        first_error("const A: i32 = 1\nconst A: i32 = 2\nfun main() {}")
            .contains("duplicate constant")
    );
    assert!(first_error("const print: i32 = 1\nfun main() {}").contains("builtin"));
    assert!(first_error("const f: i32 = 1\nfun f() {}").contains("conflicts with a function"));
    assert!(
        first_error("const C: i32 = 1\nclass C { x: i32 }\nfun main() {}")
            .contains("conflicts with a class")
    );

    let src = r#"
const N: i32 = 7
class K {
    N: i32
    fun get(): i32 { return N }
}
fun param(N: i32): i32 { return N }
fun local(): i32 {
    val N: i32 = 3
    return N
}
fun main(): i32 { return N }
"#;
    let hir = hir_of(src);
    let kind = |name: &str| {
        let f = hir.functions.iter().find(|f| f.name == name).unwrap();
        let HirStmt::Return { value: Some(v) } = f.body.stmts.last().unwrap() else {
            panic!()
        };
        v.kind.clone()
    };
    assert!(matches!(kind("main"), HirExprKind::Int { value: 7 }));
    assert!(!matches!(kind("param"), HirExprKind::Int { .. }));
    assert!(!matches!(kind("local"), HirExprKind::Int { .. }));
    assert!(matches!(kind("K.get"), HirExprKind::ObjectField { .. }));
}

#[test]
fn global_constant_cannot_be_assigned_moved_or_promoted() {
    assert!(
        first_error("const N: i32 = 1\nfun main() { N = 2 }").contains("cannot assign to constant")
    );
    assert!(
        first_error("const S: String = \"a\"\nfun main() { val t = move S }")
            .contains("constant `S`")
    );
    assert!(first_error(
        "const S: String = \"a\"\nfun main() { var t: String = \"b\"\n t = promote S }"
    )
    .contains("constant `S`"));
}

#[test]
fn global_constant_borrow_of_string_only() {
    hir_of("const S: String = \"a\"\nfun len(s: &String): i32 { return s.length }\nfun main(): i32 { return len(&S) }");
    assert!(first_error("const N: i32 = 1\nfun main() { val r = &N }").contains("Copy"));
}
