# Bork MVP 0.1 — LALRPOP Parser & AST Design

**Date:** 2026-09-21  
**Status:** Approved for implementation planning  
**Scope:** Initial LALRPOP grammar, AST, build setup, and parse smoke test for the core syntax subset.

## Goal

Stand up a greenfield Rust crate that parses the MVP 0.1 Bork surface into a clean AST using LALRPOP with its **built-in lexer**. Success is: the sample program below parses without error and can be Debug-printed.

## Non-goals (explicitly deferred)

- Region/arena lowering and brace-wrapper collapse implementation
- Semantic checks (`val` reassignment, typechecking, `T?` meaning)
- Spans / pretty diagnostics / error recovery
- Strings, methods, modules, multi-file
- LLVM / codegen
- `null` literal or null-safety operators (`?.`, `?:`)

## Decisions locked in

| Topic | Choice |
|--------|--------|
| Lexer | LALRPOP built-in lexer |
| `if` | Expression-only (Kotlin-style); `else` required in MVP so if always yields a value |
| Bare `{ ... }` | Allowed as `Stmt::Block` (nested braces preserved in AST) |
| Region collapse | **Parse faithfully; collapse later** — nested `{{{{ }}}}` stay nested AST nodes; a future pass collapses brace wrappers that only wrap another bare block. Control-flow blocks (`fun`/`if`/`for`/closure) each count as regions. Example intent: `main {{{{ }}}}` → one region after collapse; `main { if {} }` → two regions |
| `val` / `var` | Explicit `BindingKind::Val \| Var` on decls; assignability checked later |
| Nullability | Kotlin-style `T?` on types only; **no `null` value/keyword** (Rust-like) |
| Trailing closures | Attached to `Call` as optional trailing lambda after `)` |
| Range `..` | Infix binary op (`BinOp::RangeTo`), not a dedicated `Expr::Range`; later may map to builtin infix `..(a, b)` |

## Crate layout

```
bork/
  Cargo.toml          # lalrpop-util (+ lexer), lalrpop build-dep
  build.rs            # lalrpop::process_root()
  src/
    lib.rs            # ast + include generated parser + parse()
    main.rs           # demo: parse sample, print OK + AST
    ast.rs            # AST definitions
    parser.lalrpop    # grammar + tokens
```

**Pipeline:** source `&str` → LALRPOP → `ast::Program` → `Result` with `lalrpop_util::ParseError`.

## AST

```text
Program        = functions: Vec<Function>

Function       = name: String
                 params: Vec<Param>
                 return_type: Type
                 body: Block

Param          = name: String, ty: Type

Type           = Named { name: String, nullable: bool }
               | Func  { params: Vec<Type>, ret: Box<Type>, nullable: bool }

Block          = stmts: Vec<Stmt>

BindingKind    = Val | Var

Stmt           = Block(Block)
               | VarDecl { kind: BindingKind, name: String, value: Expr }
               | Assign { name: String, value: Expr }
               | For { name: String, iter: Expr, body: Block }
               | Return(Option<Expr>)
               | Expr(Expr)

Expr           = Int(i64)
               | Ident(String)
               | Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> }
               | Call { callee: Box<Expr>, args: Vec<Expr>, trailing: Option<Closure> }
               | If { cond: Box<Expr>, then_block: Block, else_block: Block }

Closure        = params: Vec<String>, body: Block

BinOp          = Add | Sub | Mul | Div
               | Gt | Lt | Ge | Le | Eq | Ne
               | RangeTo   // infix `..`  (a .. b)
```

Notes:

- No spans in MVP.
- No `Expr::Block` unless grammar needs it; statement bare blocks cover region braces.
- `Assign` is parseable for any ident; rejecting `val` targets is a later semantic pass.
- `for (i in 0..10)` uses any `Expr` as the iterable; `0..10` is `Binary { op: RangeTo, ... }`. There is no `for (1..10)` form without `in` + binder.
- User-defined infix functions are out of MVP; `..` is a fixed operator token that fits the infix mental model.

## Grammar sketch

**Program:** `Function*`

**Function:** `fun Ident "(" ParamList? ")" ":" Type Block`

**Param:** `Ident ":" Type`

**Type:**

- `TypePrimary "?"?`
- `TypePrimary → Ident | "(" TypeList? ")" "->" Type`

**Block:** `"{" Stmt* "}"`

**Stmt:**

- `Block`
- `val Ident "=" Expr` / `var Ident "=" Expr`
- `Ident "=" Expr`
- `return Expr?`
- `for "(" Ident "in" Expr ")" Block`
- `Expr` (expression statement)

**Trailing closure:** after call `")"` , optional `"{" IdentList? "->" Stmt* "}"` → `Call.trailing`.

**Expression precedence (tight → loose):**

1. Atom — int, ident, `( Expr )`, `if "(" Expr ")" Block "else" Block`
2. Call — `Atom "(" Args? ")" TrailingClosure?`
3. `*` `/`
4. `+` `-`
5. Comparisons `>` `<` `>=` `<=` `==` `!=`
6. Infix range-to `Expr ".." Expr` → `BinOp::RangeTo`

Unary `-` omitted in MVP (sample does not use it).

**Lexer:** keywords (`fun`, `val`, `var`, `for`, `in`, `return`, `if`, `else`), idents, ints, operators, punctuation including `->` and `..`, skip whitespace and `//` line comments. Do **not** add a `null` keyword.

## Region semantics (documented for later; not implemented in 0.1)

- Curly braces introduce candidate regions in the language model.
- Pure nested bare blocks that only wrap another bare block (e.g. `{{{{ }}}}`) collapse to a **single** region.
- Distinct control-flow / function / closure / non-trivial bare blocks remain separate regions (e.g. `main { if { } }` → two arenas after analysis).
- Implementation lives in a post-parse pass; the parser must not flatten these away.

## Testing & demo

**Sample (must parse):**

```text
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
```

**Demo `main`:** parse the sample; on success print confirmation + `{:#?}` AST; on failure print parse error.

**Unit test:** `parses_mvp_sample` asserts `parse(sample).is_ok()`.

**Errors:** return/display `lalrpop_util::ParseError` as-is.

## Success criteria

1. `cargo build` succeeds with LALRPOP codegen via `build.rs`.
2. Sample program parses successfully.
3. AST distinguishes `val` vs `var`, nullable vs non-nullable types, trailing closures, `for` ranges, and if-expressions.
4. No `null` literal in the language surface for this MVP.
