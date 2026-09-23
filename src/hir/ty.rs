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
}
