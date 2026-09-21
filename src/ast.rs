#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Primitive { name: String, nullable: bool },
    Named { name: String, nullable: bool },
    Func {
        params: Vec<Type>,
        ret: Box<Type>,
        nullable: bool,
    },
}

impl Type {
    pub fn from_ident(name: &str, nullable: bool) -> Self {
        let canonical = match name {
            "Int" => "i32",
            "Long" => "i64",
            "Byte" => "u8",
            "Float" => "f32",
            "Double" => "f64",
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64"
            | "bool" | "unit" => name,
            _ => {
                return Type::Named {
                    name: name.to_string(),
                    nullable,
                };
            }
        };
        Type::Primitive {
            name: canonical.to_string(),
            nullable,
        }
    }

    pub fn unit(nullable: bool) -> Self {
        Type::Primitive {
            name: "unit".into(),
            nullable,
        }
    }

    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            Type::Primitive {
                nullable: false,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Val,
    Var,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Block(Block),
    VarDecl {
        kind: BindingKind,
        name: String,
        name_span: crate::span::Span,
        ty: Option<Type>,
        value: Expr,
    },
    Assign {
        name: String,
        name_span: crate::span::Span,
        value: Expr,
    },
    For {
        name: String,
        iter: Expr,
        body: Block,
    },
    MoveBlock {
        captures: Vec<crate::span::SpannedName>,
        body: Block,
    },
    Return(Option<Expr>),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Str(String),
    Ident {
        name: String,
        span: crate::span::Span,
    },
    None,
    Some(Box<Expr>),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Field {
        receiver: Box<Expr>,
        name: String,
        safe: bool,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        trailing: Option<Closure>,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<crate::span::SpannedName>,
    pub body: Block,
    pub is_move: bool,
    pub captures: Vec<crate::span::SpannedName>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    RangeTo,
    Elvis,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    NotNullAssert,
}

pub fn unescape_string_literal(raw: &str) -> String {
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}
