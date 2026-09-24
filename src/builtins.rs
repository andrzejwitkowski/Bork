use crate::hir::{Prim, Ty, TyKind};
use crate::typeck::FunSig;

pub(crate) fn is_print(name: &str) -> bool {
    matches!(name, "print" | "println")
}

pub(crate) fn supports_print_arg(ty: &Ty) -> bool {
    !ty.nullable
        && (matches!(&ty.kind, TyKind::Prim(Prim::I32 | Prim::I64))
            || matches!(&ty.kind, TyKind::Named(name) if name == "String"))
}

pub(crate) fn signatures() -> [(String, FunSig); 2] {
    ["print", "println"].map(|name| {
        (
            name.to_string(),
            FunSig {
                params: vec![Ty::i32()],
                return_ty: Ty::unit(),
            },
        )
    })
}
