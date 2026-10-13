use crate::hir::{Prim, Ty, TyKind};
use crate::typeck::FunSig;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Builtin {
    Print,
    Println,
    Concat,
    StringFromUtf8,
}

pub(crate) fn resolve(name: &str) -> Option<Builtin> {
    match name {
        "print" => Some(Builtin::Print),
        "println" => Some(Builtin::Println),
        "concat" => Some(Builtin::Concat),
        "stringFromUtf8" => Some(Builtin::StringFromUtf8),
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
        (name.to_string(), FunSig { params, return_ty })
    })
}

impl Builtin {
    fn sig(self) -> (&'static str, Vec<Ty>, Ty) {
        match self {
            Builtin::Print => ("print", vec![Ty::i32()], Ty::unit()),
            Builtin::Println => ("println", vec![Ty::i32()], Ty::unit()),
            // Typeck checks `stringFromUtf8` itself: its buffer parameter is any `&[u8; N]`.
            Builtin::StringFromUtf8 => ("stringFromUtf8", Vec::new(), Ty::string(true)),
            Builtin::Concat => (
                "concat",
                vec![Ty::string(false), Ty::string(false)],
                Ty::string(false),
            ),
        }
    }
}
