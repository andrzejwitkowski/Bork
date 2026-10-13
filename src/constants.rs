//! File-level `const NAME: Type = literal` declarations.

use std::collections::HashMap;

use crate::ast::{Expr, Program, Type};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirExprKind, Prim, Ty, TyKind};
use crate::span::Span;

/// A validated constant: its declared type and the literal every use lowers to.
#[derive(Debug, Clone)]
pub struct Constant {
    pub ty: Ty,
    pub value: HirExprKind,
    pub span: Span,
}

impl Constant {
    pub fn member(&self, name: &str) -> crate::typeck::MemberRef {
        let value = match &self.value {
            HirExprKind::Int { value } => value.to_string(),
            HirExprKind::Float { value } => value.to_string(),
            HirExprKind::Bool { value } => value.to_string(),
            HirExprKind::Str { value } => format!("{value:?}"),
            _ => String::new(),
        };
        crate::typeck::MemberRef::Constant {
            name: name.to_string(),
            ty: self.ty.clone(),
            value,
        }
    }
}

pub type ConstantTable = HashMap<String, Constant>;

pub fn collect(program: &Program) -> (ConstantTable, Vec<Diagnostic>) {
    let mut table = ConstantTable::new();
    let mut diagnostics = Vec::new();
    for decl in &program.constants {
        let name = &decl.name.name;
        let span = decl.name.span;
        let collision = if crate::builtins::is_intrinsic(name) {
            Some("a builtin function")
        } else if program.functions.iter().any(|f| &f.name == name) {
            Some("a function")
        } else if program.classes.iter().any(|c| &c.name == name) {
            Some("a class")
        } else {
            None
        };
        if let Some(what) = collision {
            diagnostics.push(error(
                format!("constant `{name}` conflicts with {what}"),
                span,
            ));
            continue;
        }
        if table.contains_key(name) {
            diagnostics.push(error(format!("duplicate constant `{name}`"), span));
            continue;
        }
        match literal(&decl.ty, &decl.value, decl.value_span) {
            Ok((ty, value)) => {
                table.insert(name.clone(), Constant { ty, value, span });
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    (table, diagnostics)
}

fn literal(ty: &Type, value: &Expr, span: Span) -> Result<(Ty, HirExprKind), Diagnostic> {
    let ty = match ty {
        Type::Primitive {
            name,
            nullable: false,
        } => match Prim::from_name(name) {
            Some(prim) if prim != Prim::Unit => Ty::prim(prim),
            _ => {
                return Err(error(
                    format!("constant type `{name}` is not supported"),
                    span,
                ))
            }
        },
        Type::Named {
            name,
            nullable: false,
        } if name == "String" => Ty::string(false),
        other => {
            return Err(error(
                format!("constant type `{other}` is not supported; use a number, bool or String"),
                span,
            ))
        }
    };
    let mismatch = || error(format!("constant initializer is not a {ty} literal"), span);
    let prim = match ty.kind {
        TyKind::Prim(prim) => Some(prim),
        _ => None,
    };
    let value = match (value, prim) {
        (Expr::Int(v), Some(prim)) if prim.is_integer() => {
            if !fits(*v, prim) {
                return Err(error(
                    format!("integer literal {v} is out of range for {ty}"),
                    span,
                ));
            }
            HirExprKind::Int { value: *v }
        }
        (Expr::Float(v), Some(Prim::F32)) if (*v as f32).is_infinite() => {
            return Err(error(
                format!("float literal {v} is out of range for {ty}"),
                span,
            ));
        }
        (Expr::Float(v), Some(Prim::F32 | Prim::F64)) => HirExprKind::Float { value: *v },
        (Expr::Bool(v), Some(Prim::Bool)) => HirExprKind::Bool { value: *v },
        (Expr::Str(v), None) => HirExprKind::Str { value: v.clone() },
        _ => return Err(mismatch()),
    };
    Ok((ty, value))
}

fn fits(value: i64, prim: Prim) -> bool {
    let (min, max): (i128, i128) = match prim {
        Prim::I8 => (i8::MIN.into(), i8::MAX.into()),
        Prim::I16 => (i16::MIN.into(), i16::MAX.into()),
        Prim::I32 => (i32::MIN.into(), i32::MAX.into()),
        Prim::I64 => (i64::MIN.into(), i64::MAX.into()),
        Prim::U8 => (0, u8::MAX.into()),
        Prim::U16 => (0, u16::MAX.into()),
        Prim::U32 => (0, u32::MAX.into()),
        _ => (0, u64::MAX.into()),
    };
    (min..=max).contains(&i128::from(value))
}

fn error(message: String, span: Span) -> Diagnostic {
    Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message,
        span: Some(span),
    }
}
