use std::fmt;

use crate::ast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Unit,
}

impl Prim {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "i8" => Prim::I8,
            "i16" => Prim::I16,
            "i32" => Prim::I32,
            "i64" => Prim::I64,
            "u8" => Prim::U8,
            "u16" => Prim::U16,
            "u32" => Prim::U32,
            "u64" => Prim::U64,
            "f32" => Prim::F32,
            "f64" => Prim::F64,
            "bool" => Prim::Bool,
            "unit" => Prim::Unit,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Prim::I8 => "i8",
            Prim::I16 => "i16",
            Prim::I32 => "i32",
            Prim::I64 => "i64",
            Prim::U8 => "u8",
            Prim::U16 => "u16",
            Prim::U32 => "u32",
            Prim::U64 => "u64",
            Prim::F32 => "f32",
            Prim::F64 => "f64",
            Prim::Bool => "bool",
            Prim::Unit => "unit",
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Prim::I8
                | Prim::I16
                | Prim::I32
                | Prim::I64
                | Prim::U8
                | Prim::U16
                | Prim::U32
                | Prim::U64
        )
    }

    pub fn is_numeric(self) -> bool {
        self.is_integer() || matches!(self, Prim::F32 | Prim::F64)
    }
}

impl fmt::Display for Prim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyKind {
    Prim(Prim),
    Named(String),
    Func { params: Vec<Ty>, ret: Box<Ty> },
    Range(Box<Ty>),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ty {
    pub kind: TyKind,
    pub nullable: bool,
}

impl Ty {
    pub fn new(kind: TyKind, nullable: bool) -> Self {
        Self { kind, nullable }
    }

    pub fn prim(prim: Prim) -> Self {
        Self::new(TyKind::Prim(prim), false)
    }

    pub fn i32() -> Self {
        Self::prim(Prim::I32)
    }

    pub fn bool() -> Self {
        Self::prim(Prim::Bool)
    }

    pub fn unit() -> Self {
        Self::prim(Prim::Unit)
    }

    pub fn unknown() -> Self {
        Self::new(TyKind::Unknown, false)
    }

    pub fn range(elem: Ty) -> Self {
        Self::new(TyKind::Range(Box::new(elem)), false)
    }

    pub fn string(nullable: bool) -> Self {
        Self::new(TyKind::Named("String".into()), nullable)
    }

    pub fn from_ast(t: &ast::Type) -> Self {
        match t {
            ast::Type::Primitive { name, nullable } => Self::new(
                Prim::from_name(name).map_or(TyKind::Unknown, TyKind::Prim),
                *nullable,
            ),
            ast::Type::Named { name, nullable } => {
                Self::new(TyKind::Named(name.clone()), *nullable)
            }
            ast::Type::Func {
                params,
                ret,
                nullable,
            } => Self::new(
                TyKind::Func {
                    params: params.iter().map(Self::from_ast).collect(),
                    ret: Box::new(Self::from_ast(ret)),
                },
                *nullable,
            ),
        }
    }

    pub fn is_copy(&self) -> bool {
        !self.nullable && matches!(self.kind, TyKind::Prim(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(&self.kind, TyKind::Named(name) if name == "String")
    }

    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self.kind, TyKind::Unknown)
    }

    pub fn is_numeric(&self) -> bool {
        !self.nullable && matches!(self.kind, TyKind::Prim(p) if p.is_numeric())
    }

    pub fn is_integer(&self) -> bool {
        !self.nullable && matches!(self.kind, TyKind::Prim(p) if p.is_integer())
    }

    /// `Range` and `Unknown` have no surface nullable form, so `with_nullable`
    /// on them yields a type that cannot be written in source.
    pub fn supports_nullable(&self) -> bool {
        !matches!(self.kind, TyKind::Range(_) | TyKind::Unknown)
    }

    pub fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            kind: self.kind.clone(),
            nullable,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TyKind::Prim(prim) => f.write_str(prim.as_str())?,
            TyKind::Named(name) => f.write_str(name)?,
            TyKind::Func { params, ret } => {
                f.write_str("(")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ")->{ret}")?;
            }
            TyKind::Range(elem) => write!(f, "Range<{elem}>")?,
            TyKind::Unknown => f.write_str("<unknown>")?,
        }
        if self.nullable {
            f.write_str("?")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Prim, Ty};
    use crate::ast::Type;

    #[test]
    fn string_named_is_not_copy() {
        let t = Ty::from_ast(&Type::Named {
            name: "String".into(),
            nullable: false,
        });
        assert!(t.is_string());
        assert!(!t.is_copy());
    }

    #[test]
    fn i32_is_copy() {
        let t = Ty::from_ast(&Type::Primitive {
            name: "i32".into(),
            nullable: false,
        });
        assert!(t.is_copy());
        assert_eq!(t, Ty::i32());
    }

    #[test]
    fn nullable_primitive_is_not_copy() {
        assert!(!Ty::i32().with_nullable(true).is_copy());
    }

    #[test]
    fn numeric_and_integer_exclude_nullable_and_float() {
        assert!(Ty::i32().is_integer());
        assert!(Ty::prim(Prim::F64).is_numeric());
        assert!(!Ty::prim(Prim::F64).is_integer());
        assert!(!Ty::i32().with_nullable(true).is_numeric());
        assert!(!Ty::bool().is_numeric());
    }

    #[test]
    fn with_nullable_round_trips() {
        let t = Ty::string(false);
        assert_eq!(t.with_nullable(true).with_nullable(false), t);
    }

    #[test]
    fn range_and_unknown_have_no_nullable_form() {
        assert!(!Ty::range(Ty::i32()).supports_nullable());
        assert!(!Ty::unknown().supports_nullable());
        assert!(Ty::i32().supports_nullable());
    }

    #[test]
    fn display_renders_surface_syntax() {
        assert_eq!(Ty::i32().to_string(), "i32");
        assert_eq!(Ty::string(true).to_string(), "String?");
        assert_eq!(
            Ty::new(
                super::TyKind::Func {
                    params: vec![Ty::i32()],
                    ret: Box::new(Ty::i32()),
                },
                false
            )
            .to_string(),
            "(i32)->i32"
        );
        assert_eq!(Ty::range(Ty::i32()).to_string(), "Range<i32>");
        assert_eq!(Ty::unknown().to_string(), "<unknown>");
    }
}
