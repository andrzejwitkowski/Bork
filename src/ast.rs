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
    pub kind: BindingKind,
    pub name: crate::span::SpannedName,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Primitive { name: String, nullable: bool },
    Named { name: String, nullable: bool },
    Array {
        elem: Box<Type>,
        len: u32,
        nullable: bool,
    },
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

    pub fn with_nullable(self, nullable: bool) -> Self {
        match self {
            Type::Primitive { name, .. } => Type::Primitive { name, nullable },
            Type::Named { name, .. } => Type::Named { name, nullable },
            Type::Array { elem, len, .. } => Type::Array {
                elem,
                len,
                nullable,
            },
            Type::Func { params, ret, .. } => Type::Func {
                params,
                ret,
                nullable,
            },
        }
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
        name: crate::span::SpannedName,
        iter: Expr,
        body: Block,
    },
    MoveBlock {
        /// `None` = omitted list (infer free vars); `Some(vec![])` = explicit empty.
        captures: Option<Vec<crate::span::SpannedName>>,
        body: Block,
    },
    Return(Option<Expr>),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    ArrayLit {
        elements: Vec<Expr>,
        span: crate::span::Span,
    },
    Index {
        receiver: Box<Expr>,
        index: Box<Expr>,
        span: crate::span::Span,
    },
    Slice {
        receiver: Box<Expr>,
        lo: Box<Expr>,
        hi: Box<Expr>,
        span: crate::span::Span,
    },
    Ident {
        name: String,
        span: crate::span::Span,
    },
    Move {
        name: String,
        span: crate::span::Span,
    },
    Promote {
        name: String,
        span: crate::span::Span,
    },
    None {
        span: crate::span::Span,
    },
    Some {
        expr: Box<Expr>,
        span: crate::span::Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: crate::span::Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: crate::span::Span,
    },
    Field {
        receiver: Box<Expr>,
        name: String,
        safe: bool,
        span: crate::span::Span,
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
    /// `None` = omitted list (infer free vars); `Some(vec![])` = explicit empty.
    pub captures: Option<Vec<crate::span::SpannedName>>,
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
