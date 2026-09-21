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
    Named { name: String, nullable: bool },
    Func {
        params: Vec<Type>,
        ret: Box<Type>,
        nullable: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
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
        value: Expr,
    },
    Assign {
        name: String,
        value: Expr,
    },
    For {
        name: String,
        iter: Expr,
        body: Block,
    },
    Return(Option<Expr>),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Ident(String),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        trailing: Option<Closure>,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Block,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<String>,
    pub body: Block,
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
}
