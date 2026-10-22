//! Structured unification failures. They carry types, not spans: the checker that calls
//! `unify` knows where the constraint came from and attaches the span itself.

use super::VarKind;
use crate::ty::{InferVar, TyCtx, TyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeError {
    /// Two types with different shapes (or different array lengths / parameters).
    Mismatch { expected: TyId, found: TyId },
    /// Binding `var` to `ty` would make `ty` contain itself (`?T = [?T; 3]`).
    Occurs { var: InferVar, ty: TyId },
    /// Function types with different parameter counts.
    Arity { expected: usize, found: usize },
    /// An integer or float literal variable met a type it can never be.
    Kind { kind: VarKind, ty: TyId },
    /// Resolving would produce `T??`, which has no representation.
    NestedNullable,
}

impl TypeError {
    /// One-line text for a diagnostic.
    pub fn describe(&self, tcx: &TyCtx) -> String {
        match *self {
            TypeError::Mismatch { expected, found } => {
                format!("expected {}, found {}", tcx.display(expected), tcx.display(found))
            }
            TypeError::Occurs { ty, .. } => {
                format!("cannot infer an infinite type: it would contain itself in {}", tcx.display(ty))
            }
            TypeError::Arity { expected, found } => {
                format!("expected {expected} parameters, found {found}")
            }
            TypeError::Kind { kind, ty } => {
                let what = match kind {
                    VarKind::General => "a type",
                    VarKind::Int => "an integer type",
                    VarKind::Float => "a floating-point type",
                };
                format!("expected {what}, found {}", tcx.display(ty))
            }
            TypeError::NestedNullable => "a nullable type cannot be made nullable again".to_string(),
        }
    }
}
