# Bork MVP 0.1 LALRPOP Parser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up crate `bork` that parses the MVP 0.1 sample program into a Debug-printable AST via LALRPOP’s built-in lexer.

**Architecture:** `build.rs` runs `lalrpop::process_src()` on `src/parser.lalrpop`. `src/ast.rs` owns AST types. `src/lib.rs` exposes `parse(source: &str) -> Result<Program, ParseError>`. `src/main.rs` demos the sample. Region collapse and semantic checks are out of scope.

**Tech Stack:** Rust edition 2021, LALRPOP 0.23.x (`lalrpop` build-dep, `lalrpop-util` runtime with default/built-in lexer).

**Spec:** `docs/superpowers/specs/2026-09-21-lalrpop-mvp-parser-design.md`

## Global Constraints

- Lexer: LALRPOP built-in only (no logos / external lexer).
- `if` is expression-only; `else` required.
- `val`/`var` via `BindingKind`; no assignability checks yet.
- Types support `T?` nullability; no `null` keyword/literal.
- `..` is `BinOp::RangeTo` (infix), not `Expr::Range`.
- Bare `{...}` stays nested in AST; do not flatten regions in the parser.
- Skip whitespace and `//` line comments.
- Unary `-` omitted in MVP.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Package + deps |
| `build.rs` | Invoke LALRPOP on `src/` |
| `src/ast.rs` | AST enums/structs |
| `src/parser.lalrpop` | Tokens + grammar → AST |
| `src/lib.rs` | `lalrpop_mod!`, `parse()`, unit test |
| `src/main.rs` | CLI demo of sample parse |

---

### Task 1: Scaffold crate + minimal grammar that builds

**Files:**
- Create: `Cargo.toml`
- Create: `build.rs`
- Create: `src/ast.rs` (stub `Program`)
- Create: `src/parser.lalrpop` (minimal)
- Create: `src/lib.rs`
- Create: `src/main.rs` (placeholder)

**Interfaces:**
- Consumes: (none)
- Produces: crate builds; generated `parser` module; stub `Program`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "bork"
version = "0.1.0"
edition = "2021"

[dependencies]
lalrpop-util = "0.23"

[build-dependencies]
lalrpop = "0.23"
```

- [ ] **Step 2: Create `build.rs`**

```rust
fn main() {
    lalrpop::process_src().unwrap();
}
```

- [ ] **Step 3: Create stub `src/ast.rs`**

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub functions: Vec<()>,
}
```

- [ ] **Step 4: Create minimal `src/parser.lalrpop`**

```lalrpop
use crate::ast::Program;

grammar;

pub Program: Program = {
    => Program { functions: vec![] },
};
```

- [ ] **Step 5: Create `src/lib.rs`**

```rust
pub mod ast;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;
```

- [ ] **Step 6: Create placeholder `src/main.rs`**

```rust
fn main() {
    println!("bork MVP 0.1");
}
```

- [ ] **Step 7: Build**

Run: `cargo build`
Expected: success (LALRPOP generates parser under `target/.../out/`)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock build.rs src/ast.rs src/parser.lalrpop src/lib.rs src/main.rs
git commit -m "Scaffold bork crate with minimal LALRPOP grammar."
```

---

### Task 2: Full AST definitions

**Files:**
- Modify: `src/ast.rs` (replace stub)

**Interfaces:**
- Consumes: Task 1 scaffold
- Produces: types used by grammar and `parse()` — exact names below

- [ ] **Step 1: Replace `src/ast.rs` with full AST**

```rust
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
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: success (minimal grammar still returns empty `Program` — temporarily update stub grammar if type mismatch: change `Program { functions: vec![] }` still valid)

- [ ] **Step 3: Commit**

```bash
git add src/ast.rs
git commit -m "Define MVP 0.1 AST types for Bork parser."
```

---

### Task 3: Failing smoke test + `parse` API

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/parser.lalrpop` only if needed to keep build green

**Interfaces:**
- Consumes: `ast::Program`, generated `parser::ProgramParser`
- Produces:
  - `pub fn parse(source: &str) -> Result<Program, ParseError<usize, Token<'_>, &str>>`
  - shared sample source constant for test/main

- [ ] **Step 1: Add `parse` + sample + failing test to `src/lib.rs`**

```rust
pub mod ast;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;

use lalrpop_util::ParseError;
use parser::Token;

pub type Error<'input> = ParseError<usize, Token<'input>, &'static str>;

pub fn parse(source: &str) -> Result<Program, Error<'_>> {
    parser::ProgramParser::new().parse(source)
}

pub const MVP_SAMPLE: &str = r#"
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main() {
        var accumulator = 0
        val threshold = 5

        for (i in 0..10) {
            accumulator = action(accumulator, i) { acc, current ->
                if (current > threshold) {
                    acc + current
                } else {
                    acc
                }
            }
        }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mvp_sample() {
        let prog = parse(MVP_SAMPLE).expect("sample should parse");
        assert_eq!(prog.functions.len(), 2);
        assert_eq!(prog.functions[0].name, "action");
        assert_eq!(prog.functions[1].name, "main");
    }
}
```

- [ ] **Step 2: Run test — expect FAIL**

Run: `cargo test parses_mvp_sample -- --nocapture`
Expected: FAIL (empty grammar returns 0 functions, or parse error once grammar rejects input). Either failure mode is fine before Task 4; after Step 1 with empty grammar, prefer asserting and seeing `assert_eq!(prog.functions.len(), 2)` fail with `left == 0`.

- [ ] **Step 3: Commit test harness**

```bash
git add src/lib.rs
git commit -m "Add parse API and failing MVP sample test."
```

---

### Task 4: Full LALRPOP grammar

**Files:**
- Replace: `src/parser.lalrpop`

**Interfaces:**
- Consumes: all `ast::*` types from Task 2
- Produces: `ProgramParser::parse(&str) -> Result<Program, _>` accepting MVP sample

**Notes for implementer:**
- Prefer shift when `)` is followed by `{` so trailing closures attach to calls. Trailing form **requires** `->` (MVP). A bare block immediately after a call without `->` is a parse error — acceptable.
- `for (Ident in Expr)` — not `for (1..10)`.
- `..` → `BinOp::RangeTo`.
- No `null` token.

- [ ] **Step 1: Write complete `src/parser.lalrpop`**

```lalrpop
use crate::ast::{
    BindingKind, BinOp, Block, Closure, Expr, Function, Param, Program, Stmt, Type,
};

grammar;

match {
    r"\s+" => { },
    r"//[^\n\r]*" => { },
    r"fun" => "fun",
    r"val" => "val",
    r"var" => "var",
    r"for" => "for",
    r"in" => "in",
    r"return" => "return",
    r"if" => "if",
    r"else" => "else",
    r"[0-9]+" => "INT",
    r"[a-zA-Z_][a-zA-Z0-9_]*" => "IDENT",
    r"\.\." => "..",
    r"->" => "->",
    r"==" => "==",
    r"!=" => "!=",
    r"<=" => "<=",
    r">=" => ">=",
    r"<" => "<",
    r">" => ">",
    r"\+" => "+",
    r"-" => "-",
    r"\*" => "*",
    r"/" => "/",
    r"=" => "=",
    r"\?" => "?",
    r":" => ":",
    r"," => ",",
    r"\(" => "(",
    r"\)" => ")",
    r"\{" => "{",
    r"\}" => "}",
}

pub Program: Program = {
    <funcs:Function*> => Program { functions: funcs },
};

Function: Function = {
    "fun" <name:Ident> "(" <params:ParamList?> ")" ":" <ret:Type> <body:Block> => {
        Function {
            name,
            params: params.unwrap_or_default(),
            return_type: ret,
            body,
        }
    },
};

ParamList: Vec<Param> = {
    <p:Param> => vec![p],
    <ps:ParamList> "," <p:Param> => {
        let mut ps = ps;
        ps.push(p);
        ps
    },
};

Param: Param = {
    <name:Ident> ":" <ty:Type> => Param { name, ty },
};

Type: Type = {
    <t:TypePrimary> <q:"?"?> => match t {
        Type::Named { name, .. } => Type::Named {
            name,
            nullable: q.is_some(),
        },
        Type::Func { params, ret, .. } => Type::Func {
            params,
            ret,
            nullable: q.is_some(),
        },
    },
};

TypePrimary: Type = {
    <name:Ident> => Type::Named {
        name,
        nullable: false,
    },
    "(" <params:TypeList?> ")" "->" <ret:Type> => Type::Func {
        params: params.unwrap_or_default(),
        ret: Box::new(ret),
        nullable: false,
    },
};

TypeList: Vec<Type> = {
    <t:Type> => vec![t],
    <ts:TypeList> "," <t:Type> => {
        let mut ts = ts;
        ts.push(t);
        ts
    },
};

Block: Block = {
    "{" <stmts:Stmt*> "}" => Block { stmts },
};

Stmt: Stmt = {
    <b:Block> => Stmt::Block(b),
    "val" <name:Ident> "=" <value:Expr> => Stmt::VarDecl {
        kind: BindingKind::Val,
        name,
        value,
    },
    "var" <name:Ident> "=" <value:Expr> => Stmt::VarDecl {
        kind: BindingKind::Var,
        name,
        value,
    },
    <name:Ident> "=" <value:Expr> => Stmt::Assign { name, value },
    "for" "(" <name:Ident> "in" <iter:Expr> ")" <body:Block> => Stmt::For {
        name,
        iter,
        body,
    },
    "return" <value:Expr?> => Stmt::Return(value),
    <e:Expr> => Stmt::Expr(e),
};

Expr: Expr = {
    <e:ExprRange> => e,
};

ExprRange: Expr = {
    <lhs:ExprRange> ".." <rhs:ExprCompare> => Expr::Binary {
        op: BinOp::RangeTo,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <e:ExprCompare> => e,
};

ExprCompare: Expr = {
    <lhs:ExprCompare> ">" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Gt,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprCompare> "<" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Lt,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprCompare> ">=" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Ge,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprCompare> "<=" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Le,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprCompare> "==" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Eq,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprCompare> "!=" <rhs:ExprAdd> => Expr::Binary {
        op: BinOp::Ne,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <e:ExprAdd> => e,
};

ExprAdd: Expr = {
    <lhs:ExprAdd> "+" <rhs:ExprMul> => Expr::Binary {
        op: BinOp::Add,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprAdd> "-" <rhs:ExprMul> => Expr::Binary {
        op: BinOp::Sub,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <e:ExprMul> => e,
};

ExprMul: Expr = {
    <lhs:ExprMul> "*" <rhs:ExprCall> => Expr::Binary {
        op: BinOp::Mul,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <lhs:ExprMul> "/" <rhs:ExprCall> => Expr::Binary {
        op: BinOp::Div,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    },
    <e:ExprCall> => e,
};

ExprCall: Expr = {
    <callee:ExprCall> "(" <args:ExprList?> ")" <trailing:TrailingClosure?> => Expr::Call {
        callee: Box::new(callee),
        args: args.unwrap_or_default(),
        trailing,
    },
    <e:Atom> => e,
};

TrailingClosure: Closure = {
    "{" <params:ClosureParamList?> "->" <stmts:Stmt*> "}" => Closure {
        params: params.unwrap_or_default(),
        body: Block { stmts },
    },
};

ClosureParamList: Vec<String> = {
    <n:Ident> => vec![n],
    <ns:ClosureParamList> "," <n:Ident> => {
        let mut ns = ns;
        ns.push(n);
        ns
    },
};

ExprList: Vec<Expr> = {
    <e:Expr> => vec![e],
    <es:ExprList> "," <e:Expr> => {
        let mut es = es;
        es.push(e);
        es
    },
};

Atom: Expr = {
    <n:INT> => Expr::Int(n),
    <id:Ident> => Expr::Ident(id),
    "(" <e:Expr> ")" => e,
    "if" "(" <cond:Expr> ")" <then_block:Block> "else" <else_block:Block> => Expr::If {
        cond: Box::new(cond),
        then_block,
        else_block,
    },
};

Ident: String = {
    <i:IDENT> => i.to_string(),
};

INT: i64 = {
    <n:INT> => n.parse().unwrap(),
};
```

**Fix if LALRPOP rejects the `match` / terminal naming:** LALRPOP 0.23 built-in lexer often uses quoted terminals like `"fun"` without a separate `match` block, and regex terminals declared as:

```lalrpop
Ident: String = {
    r"[a-zA-Z_][a-zA-Z0-9_]*" => <>.to_string(),
};
Num: i64 = {
    r"[0-9]+" => <>.parse().unwrap(),
};
```

If Step 1 fails to compile, rewrite tokens to that style (same grammar rules; only terminal definitions change). Keep keyword strings as `"fun"`, `"val"`, etc.

- [ ] **Step 2: Build grammar**

Run: `cargo build`
Expected: success. If shift/reduce or token errors, fix grammar until clean (do not disable conflicts silently without understanding — resolve assignment vs call vs trailing).

- [ ] **Step 3: Run smoke test**

Run: `cargo test parses_mvp_sample -- --nocapture`
Expected: PASS

- [ ] **Step 4: Optional extra asserts (same test or new)**

Add if useful:
- `action` param `block` has `Type::Func { .. }`
- `main` body contains a `Stmt::For`
- for-body contains `Stmt::Assign` whose value is `Expr::Call { trailing: Some(..), .. }`

- [ ] **Step 5: Commit**

```bash
git add src/parser.lalrpop src/lib.rs
git commit -m "Implement LALRPOP grammar for MVP 0.1 Bork syntax."
```

---

### Task 5: Demo `main` + final verification

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `bork::parse`, `bork::MVP_SAMPLE`
- Produces: binary that prints parse success + AST

- [ ] **Step 1: Implement `src/main.rs`**

```rust
use bork::{parse, MVP_SAMPLE};

fn main() {
    match parse(MVP_SAMPLE) {
        Ok(program) => {
            println!("Parsed OK");
            println!("{program:#?}");
        }
        Err(err) => {
            eprintln!("Parse error: {err}");
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 2: Run demo**

Run: `cargo run`
Expected: stdout starts with `Parsed OK` and shows a `Program { functions: [ ... ] }` Debug dump including `action` and `main`.

- [ ] **Step 3: Full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "Add parser demo main for MVP sample."
```

---

## Spec coverage checklist (self-review)

| Spec item | Task |
|-----------|------|
| Cargo.toml + lalrpop deps | Task 1 |
| build.rs (`process_src`) | Task 1 |
| AST with Val/Var, nullable types, Call trailing, RangeTo | Task 2 |
| Grammar: fun, stmts, exprs, for-in, trailing, if-expr | Task 4 |
| `parse` + unit test sample | Task 3–4 |
| Demo main | Task 5 |
| No null literal | Task 4 lexer (no `null` token) |
| Nested bare blocks preserved | Task 4 `Stmt::Block` (no flatten) |
| `..` as BinOp::RangeTo | Task 2 + 4 |

## Placeholder / consistency scan

- Used `lalrpop::process_src()` (0.23 API), not deprecated `process_root()`.
- AST names match between Task 2 and Task 4.
- `MVP_SAMPLE` shared by test and main.
- Lexer `match` block may need rewrite to regex-terminals style — Step 1 of Task 4 documents the fallback explicitly (not a TBD).
