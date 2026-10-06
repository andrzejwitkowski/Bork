use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub structs: Vec<StructDecl>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<StructField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub kind: BindingKind,
    pub name: crate::span::SpannedName,
    pub ty: Type,
}

/// Top-level item folded into [`Program`].
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Struct(StructDecl),
    Fun(Function),
}

impl Program {
    pub fn from_items(items: Vec<Item>) -> Self {
        let mut structs = Vec::new();
        let mut functions = Vec::new();
        for item in items {
            match item {
                Item::Struct(s) => structs.push(s),
                Item::Fun(f) => functions.push(f),
            }
        }
        Self { structs, functions }
    }
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
    Ref {
        inner: Box<Type>,
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
        match self {
            Type::Primitive {
                nullable: false, ..
            } => true,
            // ponytail: sema maps HIR Struct→Named; only String is non-Copy among Named
            Type::Named {
                name,
                nullable: false,
            } if name != "String" => true,
            _ => false,
        }
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
            Type::Ref { inner, .. } => Type::Ref { inner, nullable },
        }
    }

    pub fn is_reference(&self) -> bool {
        matches!(self, Type::Ref { .. })
    }

    pub fn reference_inner(&self) -> Option<&Type> {
        match self {
            Type::Ref { inner, .. } => Some(inner),
            _ => None,
        }
    }

    fn is_nullable(&self) -> bool {
        match self {
            Type::Primitive { nullable, .. }
            | Type::Named { nullable, .. }
            | Type::Array { nullable, .. }
            | Type::Func { nullable, .. }
            | Type::Ref { nullable, .. } => *nullable,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Primitive { name, .. } | Type::Named { name, .. } => f.write_str(name)?,
            Type::Array { elem, len, .. } => write!(f, "[{elem}; {len}]")?,
            Type::Func { params, ret, .. } => {
                f.write_str("(")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ")->{ret}")?;
            }
            Type::Ref { inner, .. } => write!(f, "&{inner}")?,
        }
        if self.is_nullable() {
            f.write_str("?")?;
        }
        Ok(())
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
        target: AssignTarget,
        value: Expr,
    },
    For {
        name: crate::span::SpannedName,
        iter: Expr,
        body: Block,
    },
    While {
        cond: Expr,
        body: Block,
    },
    Break {
        span: crate::span::Span,
    },
    Continue {
        span: crate::span::Span,
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
pub enum AssignTarget {
    Name {
        name: String,
        name_span: crate::span::Span,
    },
    Index {
        name: String,
        name_span: crate::span::Span,
        index: Expr,
        borrowed: bool,
    },
    Field {
        name: String,
        name_span: crate::span::Span,
        field: String,
        field_span: crate::span::Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
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
    StructNew {
        name: String,
        args: Vec<Expr>,
        span: crate::span::Span,
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
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    NotNullAssert,
    Not,
    Borrow,
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
