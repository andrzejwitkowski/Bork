//! AST type → HIR `Ty` lowering.

use std::collections::HashMap;

use crate::ast::Type;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirStructDef, Ty, TyKind};

pub(crate) fn lower_type(
    ty: &Type,
    structs: &HashMap<String, HirStructDef>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Ty {
    match ty {
        Type::Array { nullable, .. } if *nullable => {
            push_err(diagnostics, "nullable array types `[T]?` are not supported", None);
            Ty::unknown()
        }
        Type::Array { elem, len, .. } => Ty::new(
            TyKind::Array {
                elem: Box::new(lower_type(elem, structs, diagnostics)),
                len: *len,
            },
            false,
        ),
        Type::Named { name, nullable } if name == "String" => Ty::string(*nullable),
        Type::Named { name, nullable } if structs.contains_key(name) => {
            Ty::new(TyKind::Struct(name.clone()), *nullable)
        }
        Type::Named { name, .. } => {
            push_err(diagnostics, format!("unknown named type `{name}`"), None);
            Ty::unknown()
        }
        Type::Func {
            params,
            ret,
            nullable,
        } => Ty::new(
            TyKind::Func {
                params: params
                    .iter()
                    .map(|param| lower_type(param, structs, diagnostics))
                    .collect(),
                ret: Box::new(lower_type(ret, structs, diagnostics)),
            },
            *nullable,
        ),
        Type::Ref { inner, nullable } => lower_ref(inner, *nullable, structs, diagnostics),
        _ => Ty::from_ast(ty),
    }
}

fn lower_ref(
    inner: &Type,
    nullable: bool,
    structs: &HashMap<String, HirStructDef>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Ty {
    if nullable {
        push_err(
            diagnostics,
            "nullable reference types `&T?` are not supported",
            None,
        );
    }
    let inner_ty = lower_type(inner, structs, diagnostics);
    if !inner_ty.is_unknown() && inner_ty.is_copy() {
        push_err(
            diagnostics,
            format!("cannot borrow Copy type `{inner_ty}`"),
            None,
        );
    }
    Ty::new(TyKind::Ref(Box::new(inner_ty)), false)
}

fn push_err(diagnostics: &mut Vec<Diagnostic>, message: impl Into<String>, span: Option<crate::span::Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    });
}
