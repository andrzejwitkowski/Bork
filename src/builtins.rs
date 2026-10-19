use crate::hir::{Prim, Ty, TyKind};
use crate::typeck::FunSig;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Builtin {
    Print,
    Println,
    Concat,
    StringFromUtf8,
    StrongCount,
    WeakCount,
}

pub(crate) fn resolve(name: &str) -> Option<Builtin> {
    match name {
        "print" => Some(Builtin::Print),
        "println" => Some(Builtin::Println),
        "concat" => Some(Builtin::Concat),
        "stringFromUtf8" => Some(Builtin::StringFromUtf8),
        "strongCount" => Some(Builtin::StrongCount),
        "weakCount" => Some(Builtin::WeakCount),
        _ => None,
    }
}

pub(crate) fn is_print(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Print | Builtin::Println))
}

pub(crate) fn is_concat(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Concat))
}

/// Builtins whose result bytes are a new allocation in the consumer's sink (`concat`, `stringFromUtf8`).
pub(crate) fn is_string_alloc(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Concat | Builtin::StringFromUtf8))
}

pub(crate) fn is_borrowed_arg(name: &str, index: usize) -> bool {
    index == 0
        && matches!(
            resolve(name),
            Some(Builtin::StringFromUtf8 | Builtin::StrongCount | Builtin::WeakCount)
        )
}

/// `strongCount(ref)` / `weakCount(ref)`: diagnostic counters on the arena a `Ref<T>` points into.
pub(crate) fn is_ref_count(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::StrongCount | Builtin::WeakCount))
}

/// Signature and one-line meaning of a builtin, for LSP hover.
pub(crate) fn hover_doc(name: &str) -> Option<&'static str> {
    Some(match resolve(name)? {
        Builtin::Print => "`print(value: i32 | i64 | String)`\nWrites a value without a newline.",
        Builtin::Println => "`println(value: i32 | i64 | String)`\nWrites a value and a newline.",
        Builtin::Concat => "`concat(a: String, b: String): String`\nNew string allocated in the consumer's arena.",
        Builtin::StringFromUtf8 => {
            "`stringFromUtf8(&bytes: &[u8; N], start: i32, count: i32): String?`\nCopies a byte range; `None` if it is not valid UTF-8."
        }
        Builtin::StrongCount => {
            "`strongCount(r: Ref<T>): i32`\nStrong count of the arena `r` points into (shared by all its Refs). `None` reads as 0."
        }
        Builtin::WeakCount => {
            "`weakCount(r: Ref<T>): i32`\nWeak count of the arena `r` points into, including its child arenas. `None` reads as 0."
        }
    })
}

pub(crate) fn is_intrinsic(name: &str) -> bool {
    resolve(name).is_some()
}

pub(crate) fn supports_print_arg(ty: &Ty) -> bool {
    !ty.nullable
        && (matches!(&ty.kind, TyKind::Prim(Prim::I32 | Prim::I64))
            || matches!(&ty.kind, TyKind::Named(name) if name == "String"))
}

pub(crate) fn signatures() -> [(String, FunSig); 3] {
    [Builtin::Print, Builtin::Println, Builtin::Concat].map(|builtin| {
        let (name, params, return_ty) = builtin.sig();
        (name.to_string(), FunSig::new(params, return_ty))
    })
}

impl Builtin {
    fn sig(self) -> (&'static str, Vec<Ty>, Ty) {
        match self {
            Builtin::Print => ("print", vec![Ty::i32()], Ty::unit()),
            Builtin::Println => ("println", vec![Ty::i32()], Ty::unit()),
            // Typeck checks `stringFromUtf8` itself: its buffer parameter is any `&[u8; N]`.
            Builtin::StringFromUtf8 => ("stringFromUtf8", Vec::new(), Ty::string(true)),
            // Typeck checks the counters itself: the argument is any `Ref<T>`.
            Builtin::StrongCount => ("strongCount", Vec::new(), Ty::i32()),
            Builtin::WeakCount => ("weakCount", Vec::new(), Ty::i32()),
            Builtin::Concat => (
                "concat",
                vec![Ty::string(false), Ty::string(false)],
                Ty::string(false),
            ),
        }
    }
}
