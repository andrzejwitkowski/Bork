use crate::frontend::check;

fn shown(source: &str) -> Vec<(String, Option<(usize, usize)>)> {
    check(source)
        .diagnostics
        .iter()
        .map(|d| (d.message.clone(), d.span.map(|s| (s.start, s.end))))
        .collect()
}

/// Pins text, span and order of every declaration-clash diagnostic (moved into `DefTable`, #31).
#[test]
fn declaration_clash_diagnostics_are_pinned() {
    let source = "const print: i32 = 1\nconst dup: i32 = 1\nconst dup: i32 = 2\nconst fn1: i32 = 3\nconst Cls: i32 = 4\nconst bad: Foo = 5\n\
fun fn1() {}\nfun println(x: i32) {}\nclass Cls {\n a: i32\n b: i32\n fun a() {}\n fun m() {}\n fun m() {}\n}\nclass Cls {\n z: i32\n}\nclass String {\n q: i32\n}\nclass concat {\n r: i32\n}\nfun main() {}\n";
    let expected: [(&str, (usize, usize)); 11] = [
        ("constant `print` conflicts with a builtin function", (6, 11)),
        ("duplicate constant `dup`", (46, 49)),
        ("constant `fn1` conflicts with a function", (65, 68)),
        ("constant `Cls` conflicts with a class", (84, 87)),
        ("constant type `Foo` is not supported; use a number, bool or String", (114, 115)),
        ("cannot redefine builtin function `println`", (141, 142)),
        ("duplicate class `Cls`", (231, 232)),
        ("class `String` conflicts with a builtin type name", (256, 257)),
        ("class `concat` conflicts with another type or function", (281, 282)),
        ("class `Cls` already has a field named `a`", (185, 186)),
        ("duplicate method `m` in class `Cls`", (209, 210)),
    ];
    let expected: Vec<_> = expected.iter().map(|(m, s)| (m.to_string(), Some(*s))).collect();
    assert_eq!(shown(source), expected);
}
