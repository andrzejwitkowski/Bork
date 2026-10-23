//! Differential check for #46: a generic body that passes ownership checking before
//! monomorphization must pass for every instantiation. There is no mono pass yet, so each
//! instance is made by substituting the type parameter textually.

use bork::frontend::check;

const PRELUDE: &str = "class P {\n    n: i32\n}\n";
const INSTANCES: [&str; 4] = ["i32", "String", "P", "Ref<P>"];

/// Replaces the identifier `T` with `ty` outside string literals, and drops the `<T>` of each
/// `fun` declaration (top-level or method).
fn instantiate(src: &str, ty: &str) -> String {
    let src: Vec<String> = src
        .lines()
        .map(|line| match line.trim_start().starts_with("fun ") {
            true => line.replacen("<T>", "", 1),
            false => line.to_string(),
        })
        .collect();
    let chars: Vec<char> = src.join("\n").chars().collect();
    let is_word = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    };
    let (mut out, mut in_string) = (String::new(), false);
    for (i, &c) in chars.iter().enumerate() {
        in_string ^= c == '"';
        if c == 'T' && !in_string && !is_word(i.checked_sub(1)) && !is_word(Some(i + 1)) {
            out.push_str(ty);
        } else {
            out.push(c);
        }
    }
    out
}

fn source(body: &str) -> String {
    format!("{PRELUDE}{body}\nfun main() {{}}\n")
}

struct Case {
    name: &'static str,
    body: &'static str,
    /// Whether the pre-mono check is expected to accept the generic body.
    accepted: bool,
    /// Instances that fail for a type-level reason, not an ownership one. Such an instance is now
    /// rejected when the generic function is *called* with it (`src/mono/tests.rs`,
    /// `programs/check_fail/generics/{ref_of_non_class,vec_of_class}.bork`), but this test
    /// substitutes `T` textually into a body nobody calls, so those instances stay skipped here.
    skip: &'static [&'static str],
}

const fn case(name: &'static str, body: &'static str, accepted: bool) -> Case {
    Case {
        name,
        body,
        accepted,
        skip: &[],
    }
}

const CASES: &[Case] = &[
    case("identity", "fun f<T>(x: T): T {\n    return x\n}", true),
    case("call_through", "fun id<T>(x: T): T {\n    return x\n}\nfun f<T>(x: T): T {\n    return id(x)\n}", true),
    case("branches", "fun f<T>(x: T, c: bool): T {\n    if (c) {\n        return x\n    }\n    return x\n}", true),
    case("local_copy_of_val", "fun f<T>(x: T) {\n    val y = x\n}", true),
    case("move_into_local", "fun f<T>(var x: T) {\n    var y = move x\n}", true),
    // `Ref<T>` is only valid for a class `T`.
    Case {
        name: "nested_ref",
        body: "fun f<T>(var x: Ref<T>) {\n    val y = move x\n}",
        accepted: true,
        skip: &["i32", "String", "Ref<P>"],
    },
    case("method", "class W {\n    n: i32\n    fun m<T>(x: T): T {\n        return x\n    }\n}", true),
    case("literal_t_is_not_a_type", "fun f<T>(x: T) {\n    val s = \"T\"\n}", true),
    // `Vec<T>` is accepted for a parameter `T`, but not `Vec<P>` for a class `P`.
    Case {
        name: "move_into_vec",
        body: "fun f<T>(var x: T) {\n    var v = Vec<T>()\n    v.push(move x)\n}",
        accepted: true,
        skip: &["P"],
    },
    // `return move x` is rejected for `String`, so it must be rejected for an opaque `T`.
    case("return_moved", "fun f<T>(var x: T): T {\n    return move x\n}", false),
    case(
        "return_moved_in_branch",
        "fun f<T>(var x: T, c: bool): T {\n    if (c) {\n        var y = move x\n        return move y\n    }\n    return x\n}",
        false,
    ),
    case(
        "assign_out_of_region",
        "fun f<T>(var x: T, var o: T, c: bool) {\n    if (c) {\n        var y = move x\n        o = move y\n    }\n}",
        false,
    ),
    case(
        "if_value_from_move",
        "fun f<T>(var x: T, c: bool): T {\n    val y = if (c) { move x } else { x }\n    return y\n}",
        false,
    ),
    case("double_move", "fun f<T>(var x: T) {\n    val a = move x\n    val b = move x\n}", false),
];

#[test]
fn accepted_generic_bodies_are_accepted_for_every_instance() {
    for case in CASES {
        let pre = check(&source(case.body));
        assert_eq!(
            pre.is_ok(),
            case.accepted,
            "{}: pre-mono verdict changed: {:?}",
            case.name,
            pre.diagnostics
        );
        if !pre.is_ok() {
            continue;
        }
        for ty in INSTANCES.iter().filter(|ty| !case.skip.contains(ty)) {
            let post = check(&source(&instantiate(case.body, ty)));
            assert!(
                post.is_ok(),
                "{}: accepted pre-mono but rejected for {ty}: {:?}",
                case.name,
                post.diagnostics
            );
        }
    }
}

#[test]
fn rejected_generic_bodies_are_rejected_for_some_instance() {
    // The converse is not required, but a rejection should never be gratuitous.
    for case in CASES.iter().filter(|case| !case.accepted) {
        let some = INSTANCES
            .iter()
            .any(|ty| !check(&source(&instantiate(case.body, ty))).is_ok());
        assert!(some, "{}: rejected pre-mono but every instance is accepted", case.name);
    }
}

#[test]
fn instantiate_leaves_string_literals_and_longer_names_alone() {
    let out = instantiate("fun f<T>(x: T) {\n    val s = \"T\"\n    val t = Tree(x)\n}", "i32");
    assert_eq!(out, "fun f(x: i32) {\n    val s = \"T\"\n    val t = Tree(x)\n}");
}
