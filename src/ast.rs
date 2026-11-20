use std::fmt;

pub use crate::span::AstId;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub constants: Vec<ConstDecl>,
    pub classes: Vec<Class>,
    pub enums: Vec<Enum>,
    pub functions: Vec<Function>,
    pub traits: Vec<Trait>,
    pub impls: Vec<Impl>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub generics: Vec<TypeParam>,
    pub fields: Vec<ClassField>,
    pub methods: Vec<Function>,
    /// `intrinsic class Vec<T> {}`: a compiler-known type (prelude only).
    pub intrinsic: bool,
}

/// `enum Shape { Circle(r: f64), Empty }`. Payload fields are named like class fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Enum {
    pub name: crate::span::SpannedName,
    pub generics: Vec<TypeParam>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: crate::span::SpannedName,
    pub fields: Vec<ClassField>,
}

/// One entry of a class body, in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Field(ClassField),
    Method(Function),
}

impl Class {
    pub fn from_members(name: String, generics: Vec<TypeParam>, members: Vec<ClassMember>) -> Self {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        for member in members {
            match member {
                ClassMember::Field(field) => fields.push(field),
                ClassMember::Method(method) => methods.push(method),
            }
        }
        Self {
            name,
            generics,
            fields,
            methods,
            intrinsic: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassField {
    pub name: crate::span::SpannedName,
    pub ty: Type,
}

/// `const NAME: Type = literal` at file level. `value` is validated by `constants::collect`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: crate::span::SpannedName,
    pub ty: Type,
    pub value: Expr,
    pub value_span: crate::span::Span,
}

/// Top-level item folded into [`Program`].
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Const(ConstDecl),
    Class(Class),
    Enum(Enum),
    Fun(Function),
    Trait(Trait),
    Impl(Impl),
}

impl Program {
    pub fn from_items(items: Vec<Item>) -> Self {
        let mut constants = Vec::new();
        let mut classes = Vec::new();
        let mut enums = Vec::new();
        let mut functions = Vec::new();
        let mut traits = Vec::new();
        let mut impls = Vec::new();
        for item in items {
            match item {
                Item::Const(constant) => constants.push(constant),
                Item::Class(class) => classes.push(class),
                Item::Enum(e) => enums.push(e),
                Item::Fun(function) => functions.push(function),
                Item::Trait(t) => traits.push(t),
                Item::Impl(i) => impls.push(i),
            }
        }
        Self {
            constants,
            classes,
            enums,
            functions,
            traits,
            impls,
        }
    }
}

/// `trait Shape { fun area(): f64 }`. Methods are signatures; they take `this: &Self` implicitly.
#[derive(Debug, Clone, PartialEq)]
pub struct Trait {
    pub name: crate::span::SpannedName,
    pub generics: Vec<TypeParam>,
    pub methods: Vec<TraitMethod>,
    /// `type Item` declarations.
    pub assoc_types: Vec<crate::span::SpannedName>,
    /// Set by `add_prelude` on traits it injects; a user declaration never has it.
    pub prelude: bool,
}

/// One entry of a trait body, before the parser splits them into `methods` and `assoc_types`.
pub enum TraitItem {
    Method(TraitMethod),
    Assoc(crate::span::SpannedName),
}

/// One entry of an impl body, before the parser splits them into `methods` and `assoc_types`.
pub enum ImplItem {
    Method(Function),
    Assoc(crate::span::SpannedName, Type),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: crate::span::SpannedName,
    pub generics: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub return_type: Type,
}

/// `impl<T> Show for Box<T> { .. }`. `trait_args` are the `<..>` after the trait name.
#[derive(Debug, Clone, PartialEq)]
pub struct Impl {
    pub generics: Vec<TypeParam>,
    pub trait_name: crate::span::SpannedName,
    pub trait_args: Vec<Type>,
    pub self_ty: Type,
    pub self_span: crate::span::Span,
    pub methods: Vec<Function>,
    /// `type Item = i32` definitions.
    pub assoc_types: Vec<(crate::span::SpannedName, Type)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub name_span: crate::span::Span,
    pub id: crate::span::AstId,
    pub generics: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Block,
    /// `intrinsic fun f(...)`: no body, the compiler supplies it (prelude only).
    pub intrinsic: bool,
}

/// `T` in `fun f<T>` / `class Box<T>`, with the bounds of `T: A + B<i32>` and `where T: C`.
/// Only function, method and impl parameters may have bounds (`generics::validate`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: crate::span::SpannedName,
    pub bounds: Vec<TraitRef>,
}

/// A bound `Trait<args>`; the bounded type is the implicit `Self`.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitRef {
    pub name: crate::span::SpannedName,
    pub args: Vec<Type>,
    /// `Item = X` in `Container<Item = X>`.
    pub bindings: Vec<(crate::span::SpannedName, Type)>,
}

/// `T: A + B` in a `where` clause, merged into `T`'s bounds by the parser.
#[derive(Debug, Clone, PartialEq)]
pub struct WherePred {
    pub param: crate::span::SpannedName,
    pub bounds: Vec<TraitRef>,
}

/// Adds the bounds of `preds` to the type parameter they name. `Err` when one names no parameter
/// of `generics`.
pub fn merge_where(generics: &mut [TypeParam], preds: Vec<WherePred>) -> Result<(), ()> {
    for pred in preds {
        match generics.iter_mut().find(|g| g.name.name == pred.param.name) {
            Some(param) => param.bounds.extend(pred.bounds),
            None => return Err(()),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub kind: BindingKind,
    pub name: crate::span::SpannedName,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Primitive {
        name: String,
        nullable: bool,
    },
    Named {
        name: String,
        args: Vec<Type>,
        nullable: bool,
    },
    Array {
        elem: Box<Type>,
        len: u32,
        nullable: bool,
    },
    Func {
        kind: crate::ty::ClosureKind,
        params: Vec<Type>,
        ret: Box<Type>,
        nullable: bool,
    },
    Ref {
        inner: Box<Type>,
        /// `&rw T`: the editable borrow (RFC 0003 section 5).
        rw: bool,
        nullable: bool,
    },
    ManagedRef {
        inner: Box<Type>,
    },
    /// `T::Item` / `Self::Item`.
    Assoc {
        base: Box<Type>,
        name: String,
    },
}

impl Type {
    /// This type with every `Self` replaced by `self_ty` (the type an `impl` is for).
    pub fn with_self(&self, self_ty: &Type) -> Type {
        let sub = |t: &Type| Box::new(t.with_self(self_ty));
        match self {
            Type::Named { name, args, nullable } if name == "Self" && args.is_empty() => match self_ty {
                Type::Named { name, args, nullable: inner } => {
                    Type::Named { name: name.clone(), args: args.clone(), nullable: *inner || *nullable }
                }
                other => other.clone(),
            },
            Type::Named { name, args, nullable } => {
                Type::Named { name: name.clone(), args: args.iter().map(|a| a.with_self(self_ty)).collect(), nullable: *nullable }
            }
            Type::Primitive { .. } => self.clone(),
            Type::Array { elem, len, nullable } => Type::Array { elem: sub(elem), len: *len, nullable: *nullable },
            Type::Func { kind, params, ret, nullable } => {
                Type::Func { kind: *kind, params: params.iter().map(|p| p.with_self(self_ty)).collect(), ret: sub(ret), nullable: *nullable }
            }
            Type::Ref { inner, rw, nullable } => Type::Ref { inner: sub(inner), rw: *rw, nullable: *nullable },
            Type::ManagedRef { inner } => Type::ManagedRef { inner: sub(inner) },
            Type::Assoc { base, name } => Type::Assoc { base: sub(base), name: name.clone() },
        }
    }

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
                    args: Vec::new(),
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
            Type::Named { name, args, .. } => Type::Named { name, args, nullable },
            Type::Array { elem, len, .. } => Type::Array {
                elem,
                len,
                nullable,
            },
            Type::Func { kind, params, ret, .. } => Type::Func {
                kind,
                params,
                ret,
                nullable,
            },
            Type::Ref { inner, rw, .. } => Type::Ref { inner, rw, nullable },
            Type::ManagedRef { inner } => Type::ManagedRef { inner },
            assoc @ Type::Assoc { .. } => assoc,
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

    pub fn managed_ref_inner(&self) -> Option<&Type> {
        match self {
            Type::ManagedRef { inner } => Some(inner),
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
            Type::ManagedRef { .. } | Type::Assoc { .. } => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Primitive { name, .. } => f.write_str(name)?,
            Type::Named { name, args, .. } => {
                f.write_str(name)?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    for (index, arg) in args.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{arg}")?;
                    }
                    f.write_str(">")?;
                }
            }
            Type::Assoc { base, name } => write!(f, "{base}::{name}")?,
            Type::Array { elem, len, .. } => write!(f, "[{elem}; {len}]")?,
            Type::Func { kind, params, ret, .. } => {
                if *kind != crate::ty::ClosureKind::Fn {
                    write!(f, "{kind:?}")?;
                }
                f.write_str("(")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ")->{ret}")?;
            }
            Type::Ref { inner, rw, .. } => write!(f, "&{}{inner}", if *rw { "rw " } else { "" })?,
            Type::ManagedRef { inner } => write!(f, "Ref<{inner}>")?,
        }
        if self.is_nullable() {
            f.write_str("?")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Option<crate::span::Span>,
    /// One source span per statement, in the same order as `stmts`.
    pub statement_spans: Vec<crate::span::Span>,
}

impl Block {
    pub fn from_spanned(
        span: crate::span::Span,
        statements: Vec<(Stmt, crate::span::Span)>,
    ) -> Self {
        let (stmts, statement_spans) = statements.into_iter().unzip();
        Self {
            stmts,
            span: Some(span),
            statement_spans,
        }
    }
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
        id: crate::span::AstId,
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
        id: crate::span::AstId,
    },
    Index {
        name: String,
        name_span: crate::span::Span,
        id: crate::span::AstId,
        index: Expr,
        borrowed: bool,
    },
    Field {
        receiver: Expr,
        name: String,
        span: crate::span::Span,
        id: crate::span::AstId,
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
        id: crate::span::AstId,
    },
    Move {
        name: String,
        span: crate::span::Span,
        id: crate::span::AstId,
    },
    Promote {
        name: String,
        span: crate::span::Span,
        id: crate::span::AstId,
    },
    /// Numbered like an identifier: typeck falls back to it when a binding named `None` shadows the prelude.
    None {
        span: crate::span::Span,
        id: crate::span::AstId,
    },
    /// Numbered for the same reason: a user function named `Some` makes this a plain call.
    Some {
        expr: Box<Expr>,
        span: crate::span::Span,
        id: crate::span::AstId,
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
        id: crate::span::AstId,
    },
    Call {
        callee: Box<Expr>,
        /// Explicit `f::<T>(..)` arguments; empty when omitted.
        type_args: Vec<Type>,
        args: Vec<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_block: Block,
        else_block: Option<Block>,
    },
    IfVal {
        bindings: ConditionalBindings,
        then_block: Block,
        else_block: Option<Block>,
    },
    /// `when scrutinee { pattern => block ... }` beyond the `Some(v)` / `None` pair, which
    /// parses to [`Expr::IfVal`].
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        span: crate::span::Span,
    },
    /// A closure written as a value: `{ a, b -> body }` or `move (n) { a -> body }` (RFC 0007).
    /// The `->` is required, so it never reads as a block.
    Closure(Closure),
}

impl Expr {
    /// A name. `None` is not a keyword: it reads as the absent value of `T?` / `Ref<T>`, and
    /// typeck decides from the expected type whether it is that or the `Option` enum's variant.
    pub fn ident(id: crate::span::SpannedName) -> Expr {
        if id.name == "None" {
            return Expr::None { span: id.span, id: crate::span::AstId::DUMMY };
        }
        Expr::Ident { name: id.name, span: id.span, id: crate::span::AstId::DUMMY }
    }

    /// A call. `Some(e)` is not a keyword either: a plain one-argument call of the name `Some`.
    pub fn call(callee: Expr, type_args: Vec<Type>, mut args: Vec<Expr>, trailing: Option<Closure>, span: crate::span::Span) -> Expr {
        // A trailing closure is just the last argument, so `Some() { .. }` reads like `Some({ .. })`.
        args.extend(trailing.map(Expr::Closure));
        let is_some = matches!(&callee, Expr::Ident { name, .. } if name == "Some");
        if is_some && type_args.is_empty() && args.len() == 1 {
            return Expr::Some { expr: Box::new(args.pop().expect("one argument")), span, id: crate::span::AstId::DUMMY };
        }
        Expr::Call { callee: Box::new(callee), type_args, args }
    }

    /// `expr?` on a `Result` (#274): `when expr { Ok(v) => { v } Err(e) => { return Result.Err(e) } }`.
    /// The binders `?v` / `?e` cannot be written in source, so they capture nothing.
    pub fn try_op(expr: Expr, span: crate::span::Span) -> Expr {
        use crate::span::{AstId, SpannedName};
        let name = |n: &str| SpannedName { name: n.to_string(), span, id: AstId::DUMMY };
        let ident = |n: &str| Expr::Ident { name: n.to_string(), span, id: AstId::DUMMY };
        let block = |stmt: Stmt| Block::from_spanned(span, vec![(stmt, span)]);
        let ok = Pattern::unqualified_variant(name("Ok"), vec![Pattern::Binding(name("?v"))], span);
        let err = Pattern::unqualified_variant(name("Err"), vec![Pattern::Binding(name("?e"))], span);
        let err_variant = Expr::Field { receiver: Box::new(ident("Result")), name: "Err".to_string(), safe: false, span, id: AstId::DUMMY };
        let rethrow = Expr::call(err_variant, Vec::new(), vec![ident("?e")], None, span);
        let arms = vec![
            MatchArm { pat: ok, body: block(Stmt::Expr(ident("?v"))) },
            MatchArm { pat: err, body: block(Stmt::Return(Some(rethrow))) },
        ];
        // A local is moved into the match: a place scrutinee would bind `&T` payloads, not owned ones.
        Expr::Match { scrutinee: Box::new(expr.moved_if_local()), arms, span }
    }

    /// A bare local name becomes `move name`; any other expression is unchanged.
    fn moved_if_local(self) -> Expr {
        match self {
            Expr::Ident { name, span, id } => Expr::Move { name, span, id },
            other => other,
        }
    }

    /// `for (name in iter) body` (#275). A literal `a..b` stays a `For` (the counted-loop fast
    /// path); anything else desugars to `{ var ?it = iter; while (true) { if (val name = ?it.next()) body else break } }`.
    /// `?it` cannot be written in source, so it captures nothing.
    pub fn for_in(name: crate::span::SpannedName, iter: Expr, iter_span: crate::span::Span, body: Block) -> Stmt {
        use crate::span::AstId;
        if matches!(iter, Expr::Binary { op: BinOp::RangeTo, .. }) {
            return Stmt::For { name, iter, body };
        }
        let span = iter_span; // errors about `next` point at the iterable, not the loop variable
        let iter = iter.moved_if_local(); // the loop consumes a local iterator
        let it = || Expr::Ident { name: "?it".to_string(), span, id: AstId::DUMMY };
        let next = Expr::call(Expr::Field { receiver: Box::new(it()), name: "next".to_string(), safe: false, span, id: AstId::DUMMY }, Vec::new(), Vec::new(), None, span);
        let brk = Block::from_spanned(span, vec![(Stmt::Break { span }, span)]);
        let step = Expr::IfVal {
            bindings: ConditionalBindings { head: Box::new(ConditionalBinding { name, value: next, value_span: span }), tail: Vec::new() },
            then_block: body,
            else_block: Some(brk),
        };
        let looped = Stmt::While { cond: Expr::Bool(true), body: Block::from_spanned(span, vec![(Stmt::Expr(step), span)]) };
        let decl = Stmt::VarDecl { kind: BindingKind::Var, name: "?it".to_string(), name_span: span, id: AstId::DUMMY, ty: None, value: iter };
        Stmt::Block(Block::from_spanned(span, vec![(decl, span), (looped, span)]))
    }

    /// `when value { arms }`. The `Some(name)` + `None` pair parses as `if val`; typeck turns it back
    /// into a plain `when` unless the value is a `Ref`, which keeps its regions and codegen.
    pub fn when(value: Expr, value_span: crate::span::Span, mut arms: Vec<MatchArm>, span: crate::span::Span) -> Expr {
        if let [MatchArm { pat: Pattern::Some { inner, .. }, .. }, MatchArm { pat: Pattern::None { .. }, .. }] = &arms[..] {
            if let Pattern::Binding(name) = inner.as_ref() {
                let name = name.clone();
                let none = arms.pop().expect("two arms").body;
                let some = arms.pop().expect("two arms").body;
                return Expr::IfVal {
                    bindings: ConditionalBindings { head: Box::new(ConditionalBinding { name, value, value_span }), tail: Vec::new() },
                    then_block: some,
                    else_block: Some(none),
                };
            }
        }
        Expr::Match { scrutinee: Box::new(value), arms, span }
    }
}

impl Pattern {
    /// A bare name: `_`, the absent pattern `None`, or a binding.
    pub fn binding(name: crate::span::SpannedName) -> Pattern {
        if name.name == "_" {
            Pattern::Wildcard { span: name.span }
        } else if name.name == "None" {
            Pattern::None { span: name.span }
        } else {
            Pattern::Binding(name)
        }
    }

    /// `Name(p, ..)`; `Some(p)` with one sub-pattern is the present pattern.
    pub fn unqualified_variant(name: crate::span::SpannedName, mut args: Vec<Pattern>, span: crate::span::Span) -> Pattern {
        if name.name == "Some" && args.len() == 1 {
            return Pattern::Some { inner: Box::new(args.pop().expect("one pattern")), span };
        }
        Pattern::Variant { enum_name: None, name, args, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pat: Pattern,
    pub body: Block,
}

/// A pattern; `Variant` arguments are positional, in payload order.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`
    Wildcard { span: crate::span::Span },
    /// `name` binds the matched value.
    Binding(crate::span::SpannedName),
    Literal { value: PatLit, span: crate::span::Span },
    /// `V(p, ..)`, `E.V(p, ..)`, or a bare `V` / `E.V` for a payload-free variant.
    Variant {
        enum_name: Option<crate::span::SpannedName>,
        name: crate::span::SpannedName,
        args: Vec<Pattern>,
        span: crate::span::Span,
    },
    Some { inner: Box<Pattern>, span: crate::span::Span },
    None { span: crate::span::Span },
}

impl Pattern {
    pub fn span(&self) -> crate::span::Span {
        match self {
            Pattern::Binding(name) => name.span,
            Pattern::Wildcard { span }
            | Pattern::Literal { span, .. }
            | Pattern::Variant { span, .. }
            | Pattern::Some { span, .. }
            | Pattern::None { span } => *span,
        }
    }
}

/// shortcut: no negative or float literal patterns, add when a ticket needs them.
#[derive(Debug, Clone, PartialEq)]
pub enum PatLit {
    Int(i64),
    Bool(bool),
    Str(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    pub params: Vec<crate::span::SpannedName>,
    pub body: Block,
    pub is_move: bool,
    /// `None` = omitted list (infer free vars); `Some(vec![])` = explicit empty.
    pub captures: Option<Vec<crate::span::SpannedName>>,
    /// Names this closure through typeck, HIR and the arena report; set by [`crate::ast_number::number_nodes`].
    pub id: crate::span::AstId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalBinding {
    pub name: crate::span::SpannedName,
    pub value: Expr,
    pub value_span: crate::span::Span,
}

/// Non-empty `if val` / `when` header: `head` is the guard evaluated in the enclosing
/// arena; each `tail` source is evaluated inside the `Some` region.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalBindings {
    pub head: Box<ConditionalBinding>,
    pub tail: Vec<ConditionalBinding>,
}

impl ConditionalBindings {
    pub fn iter(&self) -> impl Iterator<Item = &ConditionalBinding> {
        std::iter::once(self.head.as_ref()).chain(&self.tail)
    }
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
    /// `&x`, or `&rw x` when `rw`.
    Borrow { rw: bool },
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

/// A block that may consist of exactly one nested bare `{ ... }`.
pub trait Peel {
    fn sole_inner_block(&self) -> Option<&Self>;
}

impl Peel for Block {
    fn sole_inner_block(&self) -> Option<&Self> {
        match self.stmts.as_slice() {
            [Stmt::Block(inner)] => Some(inner),
            _ => None,
        }
    }
}

/// Peel nested bare-block wrappers; the one rule shared by sema (AST) and HIR.
pub fn peel_blocks<B: Peel>(block: &B) -> (&B, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let Some(inner) = current.sole_inner_block() {
        compacted += 1;
        current = inner;
    }
    (current, compacted)
}
