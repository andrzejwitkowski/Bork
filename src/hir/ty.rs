use std::fmt;

use crate::ast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Primitive {
        name: String,
        nullable: bool,
    },
    Named {
        name: String,
        nullable: bool,
    },
    Func {
        params: Vec<Ty>,
        ret: Box<Ty>,
        nullable: bool,
    },
    Range {
        elem: Box<Ty>,
    },
    Unknown,
}

impl Ty {
    pub fn from_ast(t: &ast::Type) -> Self {
        match t {
            ast::Type::Primitive { name, nullable } => Ty::Primitive {
                name: name.clone(),
                nullable: *nullable,
            },
            ast::Type::Named { name, nullable } => Ty::Named {
                name: name.clone(),
                nullable: *nullable,
            },
            ast::Type::Func {
                params,
                ret,
                nullable,
            } => Ty::Func {
                params: params.iter().map(Self::from_ast).collect(),
                ret: Box::new(Self::from_ast(ret)),
                nullable: *nullable,
            },
        }
    }

    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            Ty::Primitive {
                nullable: false,
                ..
            }
        )
    }

    pub fn is_string(&self) -> bool {
        match self {
            Ty::Named { name, .. } | Ty::Primitive { name, .. } => name == "String",
            _ => false,
        }
    }

    pub fn string(nullable: bool) -> Self {
        Ty::Named {
            name: "String".into(),
            nullable,
        }
    }

    pub fn i32() -> Self {
        Ty::Primitive {
            name: "i32".into(),
            nullable: false,
        }
    }

    pub fn is_nullable(&self) -> bool {
        matches!(
            self,
            Ty::Primitive { nullable: true, .. }
                | Ty::Named { nullable: true, .. }
                | Ty::Func { nullable: true, .. }
        )
    }

    pub fn with_nullable(&self, nullable: bool) -> Option<Self> {
        let mut ty = self.clone();
        match &mut ty {
            Ty::Primitive {
                nullable: current, ..
            }
            | Ty::Named {
                nullable: current, ..
            }
            | Ty::Func {
                nullable: current, ..
            } => *current = nullable,
            Ty::Range { .. } | Ty::Unknown => return None,
        }
        Some(ty)
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nullable_suffix = |nullable: bool| if nullable { "?" } else { "" };
        match self {
            Ty::Primitive { name, nullable } | Ty::Named { name, nullable } => {
                write!(f, "{name}{}", nullable_suffix(*nullable))
            }
            Ty::Func {
                params,
                ret,
                nullable,
            } => {
                f.write_str("(")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ")->{ret}{}", nullable_suffix(*nullable))
            }
            Ty::Range { elem } => write!(f, "Range<{elem}>"),
            Ty::Unknown => f.write_str("<unknown>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Ty;
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
    }

    #[test]
    fn display_renders_surface_syntax() {
        assert_eq!(Ty::i32().to_string(), "i32");
        assert_eq!(Ty::string(true).to_string(), "String?");
        assert_eq!(
            Ty::Func {
                params: vec![Ty::i32()],
                ret: Box::new(Ty::i32()),
                nullable: false,
            }
            .to_string(),
            "(i32)->i32"
        );
        assert_eq!(
            Ty::Range {
                elem: Box::new(Ty::i32())
            }
            .to_string(),
            "Range<i32>"
        );
    }
}
