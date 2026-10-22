//! File-level `const NAME: Type = literal` declarations.

use std::collections::HashMap;

use crate::ast::{Expr, Program, Type};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirExprKind, Prim};
use crate::span::Span;
use crate::ty::{TyCtx, TyData, TyId};

/// A validated constant: its declared type and the literal every use lowers to.
#[derive(Debug, Clone)]
pub struct Constant {
    pub ty: TyId,
    pub value: HirExprKind,
    pub span: Span,
    pub id: crate::span::AstId,
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
            ty: self.ty,
            value,
        }
    }
}

pub type ConstantTable = HashMap<String, Constant>;

pub(crate) fn collect_into(
    program: &Program,
    defs: &crate::defs::DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> ConstantTable {
    let mut table = ConstantTable::new();
    for (index, decl) in program.constants.iter().enumerate() {
        let name = &decl.name.name;
        let span = decl.name.span;
        let collisions = defs.diagnostics(defs.const_id(index), |_| true);
        if !collisions.is_empty() {
            diagnostics.extend(collisions);
            continue;
        }
        // A constant with an invalid literal is not in `table`, so only valid ones count as the original.
        if table.contains_key(name) {
            diagnostics.push(error(format!("duplicate constant `{name}`"), span));
            continue;
        }
        match literal(&decl.ty, &decl.value, decl.value_span, tcx) {
            Ok((ty, value)) => {
                table.insert(name.clone(), Constant { ty, value, span, id: decl.name.id });
            }
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    table
}

fn literal(ty: &Type, value: &Expr, span: Span, tcx: &mut TyCtx) -> Result<(TyId, HirExprKind), Diagnostic> {
    let ty = constant_type(ty, span, tcx)?;
    let value = literal_value(ty, value, span, tcx)?;
    Ok((ty, value))
}

fn constant_type(ty: &Type, span: Span, tcx: &mut TyCtx) -> Result<TyId, Diagnostic> {
    Ok(match ty {
        Type::Primitive {
            name,
            nullable: false,
        } => match Prim::from_name(name) {
            Some(prim) if prim != Prim::Unit => tcx.prim(prim),
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
            ..
        } if name == "String" => tcx.string(false),
        other => {
            return Err(error(
                format!("constant type `{other}` is not supported; use a number, bool or String"),
                span,
            ))
        }
    })
}

fn literal_value(ty: TyId, value: &Expr, span: Span, tcx: &TyCtx) -> Result<HirExprKind, Diagnostic> {
    let shown = tcx.display(ty);
    let mismatch = || error(format!("constant initializer is not a {shown} literal"), span);
    let prim = match tcx.data(ty) {
        TyData::Prim(prim) => Some(*prim),
        _ => None,
    };
    Ok(match (value, prim) {
        (Expr::Int(v), Some(prim)) if prim.is_integer() => {
            if !fits(*v, prim) {
                return Err(error(
                    format!("integer literal {v} is out of range for {shown}"),
                    span,
                ));
            }
            HirExprKind::Int { value: *v }
        }
        (Expr::Float(v), Some(Prim::F32)) if (*v as f32).is_infinite() => {
            return Err(error(
                format!("float literal {v} is out of range for {shown}"),
                span,
            ));
        }
        (Expr::Float(v), Some(Prim::F32 | Prim::F64)) => HirExprKind::Float { value: *v },
        (Expr::Bool(v), Some(Prim::Bool)) => HirExprKind::Bool { value: *v },
        (Expr::Str(v), None) => HirExprKind::Str { value: v.clone() },
        _ => return Err(mismatch()),
    })
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
