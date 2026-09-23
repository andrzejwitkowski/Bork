mod ty;

pub use ty::Ty;

use crate::ast;

pub type RegionId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseKind {
    Local,
    Copy,
    Shared,
    Move,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub name: String,
    pub params: Vec<HirParam>,
    pub return_ty: Ty,
    pub body: HirBlock,
    pub region: RegionId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirParam {
    pub kind: ast::BindingKind,
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Return {
        value: Option<HirExpr>,
    },
    Block(HirBlock),
    VarDecl {
        kind: ast::BindingKind,
        name: String,
        ty: Ty,
        value: HirExpr,
    },
    Assign {
        name: String,
        value: HirExpr,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    Int {
        value: i64,
    },
    Str {
        value: String,
    },
    Ident {
        name: String,
    },
}
