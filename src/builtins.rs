use crate::hir::{Prim, Ty, TyKind};
use crate::typeck::FunSig;

pub(crate) fn is_print(name: &str) -> bool {
    matches!(name, "print" | "println")
}

pub(crate) fn is_concat(name: &str) -> bool {
    name == "concat"
}

pub(crate) fn is_intrinsic(name: &str) -> bool {
    is_print(name) || is_concat(name)
}

pub(crate) fn supports_print_arg(ty: &Ty) -> bool {
    !ty.nullable
        && (matches!(&ty.kind, TyKind::Prim(Prim::I32 | Prim::I64))
            || matches!(&ty.kind, TyKind::Named(name) if name == "String"))
}

pub(crate) fn signatures() -> [(String, FunSig); 3] {
    [
        (
            "print".to_string(),
            FunSig {
                params: vec![Ty::i32()],
                return_ty: Ty::unit(),
            },
        ),
        (
            "println".to_string(),
            FunSig {
                params: vec![Ty::i32()],
                return_ty: Ty::unit(),
            },
        ),
        (
            "concat".to_string(),
            FunSig {
                params: vec![Ty::string(false), Ty::string(false)],
                return_ty: Ty::string(false),
            },
        ),
    ]
}
