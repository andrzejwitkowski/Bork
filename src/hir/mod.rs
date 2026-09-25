//! Typed HIR for the frontend pipeline.
//!
//! HIR carries no region identity. Arenas and nesting live in
//! `sema::ArenaReport`; codegen should read region identity from there.

mod ty;
pub use ty::{Prim, Ty, TyKind};

use crate::ast;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseKind {
    Local,
    Shared,
    Copy,
    Move,
    Promote,
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
        /// When set, owned payload for `value` is allocated in the named outer binding's arena.
        alloc_in_binding: Option<String>,
    },
    Assign {
        target: HirAssignTarget,
        value: HirExpr,
    },
    Expr(HirExpr),
    For {
        name: String,
        iter: HirExpr,
        body: HirBlock,
    },
    While {
        cond: HirExpr,
        body: HirBlock,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    MoveBlock {
        /// `None` = capture list omitted in the source; `Some(vec![])` = explicit empty.
        captures: Option<Vec<String>>,
        body: HirBlock,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirAssignTarget {
    Name {
        name: String,
    },
    Index {
        name: String,
        index: HirExpr,
    },
}

impl HirAssignTarget {
    pub fn name(&self) -> &str {
        match self {
            HirAssignTarget::Name { name } | HirAssignTarget::Index { name, .. } => name,
        }
    }

    pub fn index(&self) -> Option<&HirExpr> {
        match self {
            HirAssignTarget::Index { index, .. } => Some(index),
            HirAssignTarget::Name { .. } => None,
        }
    }
}

/// An expression annotated with the type it was inferred or checked at.
#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub ty: Ty,
    pub span: Option<Span>,
    pub kind: HirExprKind,
}

impl HirExpr {
    pub fn new(kind: HirExprKind, ty: Ty) -> Self {
        Self {
            ty,
            span: None,
            kind,
        }
    }

    pub fn spanned(kind: HirExprKind, ty: Ty, span: Span) -> Self {
        Self {
            ty,
            span: Some(span),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    Int {
        value: i64,
    },
    Float {
        value: f64,
    },
    Str {
        value: String,
    },
    ArrayLit {
        elements: Vec<HirExpr>,
    },
    Index {
        receiver: Box<HirExpr>,
        index: Box<HirExpr>,
        use_kind: UseKind,
    },
    Slice {
        receiver: Box<HirExpr>,
        lo: Box<HirExpr>,
        hi: Box<HirExpr>,
    },
    Ident {
        name: String,
        use_kind: UseKind,
    },
    None,
    Some(Box<HirExpr>),
    Binary {
        op: ast::BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Unary {
        op: ast::UnaryOp,
        expr: Box<HirExpr>,
    },
    Field {
        receiver: Box<HirExpr>,
        name: String,
        safe: bool,
    },
    Call {
        callee: Box<HirExpr>,
        args: Vec<HirExpr>,
        has_trailing_closure: bool,
    },
    If {
        cond: Box<HirExpr>,
        then_block: HirBlock,
        else_block: Option<HirBlock>,
    },
    Bool {
        value: bool,
    },
}

/// Peel nested bare `{ ... }` wrappers (same rule as `sema::peel_blocks` on AST).
pub fn peel_blocks(block: &HirBlock) -> (&HirBlock, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [HirStmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner;
    }
    (current, compacted)
}

pub fn peel_to_body(block: &HirBlock) -> &HirBlock {
    peel_blocks(block).0
}
