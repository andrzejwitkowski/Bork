# Typed HIR + Full Typecheck (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `frontend::check` that parses, runs ownership sema, typechecks the **full** current grammar into a **Typed HIR**, surfaces unified diagnostics via `bork check` / default CLI / LSP — with **no** LLVM yet.

**Architecture:** Keep `sema::analyze` as ownership source of truth. Add `Diagnostic` + `hir` + `typeck` that lowers `Program` + ownership facts into `HirProgram`. `frontend::check` orchestrates parse → analyze → typeck and returns `Result<HirProgram, Vec<Diagnostic>>` (Ok only when both ownership and typeck are clean; always collect both error sets when parse succeeds).

**Tech Stack:** Rust 2021, existing LALRPOP AST/`sema`, no new crates in phase 1.

**Spec:** [docs/superpowers/specs/2026-09-23-typed-hir-llvm-design.md](../specs/2026-09-23-typed-hir-llvm-design.md)

## Global Constraints

- Phase 1 only: **no** Inkwell, `bork build`, or runtime link.
- Typecheck covers **full grammar** (nullable, `?:`, `!!`, trailing closures, `Some`/`None`).
- Ownership rules stay in `sema`; typeck must not re-implement move policy.
- `String` is a builtin non-Copy type (AST often has `Type::Named { name: "String", .. }`).
- Non-null primitives are Copy; `Type::is_copy()` already encodes that for primitives.
- Prefer small focused modules under `src/hir/`, `src/typeck/`, `src/diag.rs`, `src/frontend.rs`.

---

## File map

| File | Role |
|------|------|
| `src/diag.rs` | `Diagnostic`, `Phase`, `Severity`; convert parse/`SemaError` |
| `src/hir/mod.rs` | `HirProgram`, stmts/exprs, `UseKind`, `RegionId`, `Ty` |
| `src/hir/ty.rs` | Canonical `Ty` helpers (`is_copy`, `string`, nullable wrap) |
| `src/typeck/mod.rs` | `typeck(program) -> (Option<HirProgram>, Vec<Diagnostic>)` |
| `src/typeck/env.rs` | Local type env, function sigs |
| `src/typeck/expr.rs` | Expression typing |
| `src/typeck/stmt.rs` | Statement / block typing |
| `src/frontend.rs` | `check(source) -> Result<HirProgram, Vec<Diagnostic>>` |
| `src/lib.rs` | Export new modules |
| `src/main.rs` | `check` subcommand; bare `bork <file>` = check |
| `src/lsp.rs` | Drive diagnostics through `frontend::check` |
| `src/typeck/tests/*.rs` | Typecheck unit tests (string programs) |

---

### Task 1: Unified `Diagnostic` type

**Files:**
- Create: `src/diag.rs`
- Modify: `src/lib.rs`
- Test: `src/diag.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub enum Phase { Parse, Ownership, Type, Codegen }`
  - `pub enum Severity { Error, Warning }`
  - `pub struct Diagnostic { pub phase: Phase, pub severity: Severity, pub message: String, pub span: Option<Span> }`
  - `pub fn from_sema(err: &SemaError) -> Diagnostic`
  - `pub fn from_parse(err: &crate::Error) -> Diagnostic`

- [ ] **Step 1: Write `src/diag.rs` with test**

```rust
use crate::sema::SemaError;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Parse,
    Ownership,
    Type,
    Codegen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub phase: Phase,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
}

pub fn from_sema(err: &SemaError) -> Diagnostic {
    Diagnostic {
        phase: Phase::Ownership,
        severity: Severity::Error,
        message: err.message.clone(),
        span: err.span,
    }
}

pub fn from_parse(err: &crate::Error) -> Diagnostic {
    Diagnostic {
        phase: Phase::Parse,
        severity: Severity::Error,
        message: err.to_string(),
        span: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sema_error() {
        let err = SemaError {
            message: "use after move: x".into(),
            name: Some("x".into()),
            span: Some(Span::new(1, 2)),
        };
        let d = from_sema(&err);
        assert_eq!(d.phase, Phase::Ownership);
        assert_eq!(d.message, "use after move: x");
        assert_eq!(d.span, Some(Span::new(1, 2)));
    }
}
```

Wire in `src/lib.rs`: `pub mod diag;`

- [ ] **Step 2: Run test**

Run: `cargo test -p bork diag::tests::maps_sema_error -- --nocapture`  
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/diag.rs src/lib.rs
git commit -m "feat(diag): add unified Diagnostic with ownership/parse mapping"
```

---

### Task 2: HIR type + skeleton program types

**Files:**
- Create: `src/hir/ty.rs`
- Create: `src/hir/mod.rs`
- Modify: `src/lib.rs`
- Test: `src/hir/ty.rs`

**Interfaces:**
- Produces:
  - `pub enum Ty { Primitive { name: String, nullable: bool }, Named { name: String, nullable: bool }, Func { params: Vec<Ty>, ret: Box<Ty>, nullable: bool }, Range { elem: Box<Ty> }, Unknown }`
  - `impl Ty { pub fn from_ast(t: &crate::ast::Type) -> Self; pub fn is_copy(&self) -> bool; pub fn is_string(&self) -> bool; pub fn string(nullable: bool) -> Self; pub fn i32() -> Self; }`
  - `pub type RegionId = u32;`
  - `pub enum UseKind { Local, Copy, Shared, Move }`
  - `pub struct HirProgram { pub functions: Vec<HirFunction> }`
  - `pub struct HirFunction { pub name: String, pub params: Vec<HirParam>, pub return_ty: Ty, pub body: HirBlock, pub region: RegionId }`
  - `pub struct HirParam { pub kind: crate::ast::BindingKind, pub name: String, pub ty: Ty }`
  - Minimal `HirBlock` / `HirStmt` / `HirExpr` (expand in later tasks); start with `Return`, `VarDecl`, `Assign`, `Int`, `Str`, `Ident`

- [ ] **Step 1: Unit tests for `Ty`**

```rust
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
```

- [ ] **Step 2: Implement modules; `pub mod hir;` in `lib.rs`**

`is_copy`: non-null primitive only.  
`is_string`: name `String` on `Named` (or Primitive if ever aliased).

- [ ] **Step 3: Run**

Run: `cargo test -p bork hir:: -- --nocapture`  
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/hir src/lib.rs
git commit -m "feat(hir): add Ty helpers and HirProgram skeleton"
```

---

### Task 3: `frontend::check` orchestration + CLI

**Files:**
- Create: `src/frontend.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Test: `src/frontend.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn check(source: &str) -> Result<HirProgram, Vec<Diagnostic>>`
- This task (pre-typeck): parse → analyze → if ownership errors `Err`; else lower function **shells** (name, params, return_ty, empty body, fresh `RegionId`) → `Ok`

- [ ] **Step 1: Tests**

```rust
#[test]
fn check_rejects_parse_error() {
    let err = check("fun oops(").unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Parse));
}

#[test]
fn check_ok_on_mvp_sample_ownership() {
    let hir = check(crate::MVP_SAMPLE).expect("mvp ownership clean");
    assert_eq!(hir.functions.len(), 2);
}
```

- [ ] **Step 2: Implement `frontend::check` + `pub mod frontend`**

- [ ] **Step 3: Update `src/main.rs`**

- `bork check <file>` and bare `bork [--dump-arenas] <file>` both run `frontend::check`
- Print: `error: {phase:?}: {message}` (lowercase phase name is fine)
- Exit `1` on diagnostics; `--dump-arenas` still uses `parse` + `analyze` + `dump_arenas` when parse succeeds
- Usage string: `Usage: bork [check] [--dump-arenas] <file.bork>`

- [ ] **Step 4: Run**

Run: `cargo test -p bork frontend:: -- --nocapture`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/frontend.rs src/main.rs src/lib.rs
git commit -m "feat(frontend): add check() pipeline and bork check CLI"
```

---

### Task 4: Typeck scaffold — assign-to-`val`, return types

**Files:**
- Create: `src/typeck/mod.rs`
- Create: `src/typeck/env.rs`
- Create: `src/typeck/stmt.rs`
- Create: `src/typeck/expr.rs`
- Create: `src/typeck/tests/mod.rs`
- Modify: `src/frontend.rs` to call typeck after ownership
- Test: `src/typeck/tests/mod.rs`

**Interfaces:**
- Produces: `pub fn check(program: &Program) -> (Option<HirProgram>, Vec<Diagnostic>)`
- Frontend: run typeck after analyze; `Ok` only if ownership **and** type diagnostics empty

**Lock:** `Expr::Int` always types as `i32` in HIR for phase 1.

- [ ] **Step 1: Failing tests**

```rust
use crate::diag::Phase;
use crate::frontend::check;

#[test]
fn assign_to_val_is_type_error() {
    let src = r#"
fun main(): i32 {
    val x: i32 = 1
    x = 2
    return x
}
"#;
    let err = check(src).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn return_type_mismatch() {
    let src = r#"
fun main(): i32 {
    return "nope"
}
"#;
    let err = check(src).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p bork typeck::tests::assign_to_val_is_type_error -- --nocapture`

- [ ] **Step 3: Minimal typeck implementation**

- Fun sig map from AST
- `VarDecl`: infer or check annotation vs init
- `Assign`: error if binding is `val`
- `Return`: unify with function return ty
- Emit corresponding HIR

- [ ] **Step 4: Tests PASS + commit**

```bash
git add src/typeck src/frontend.rs
git commit -m "feat(typeck): assign-to-val and return type checking"
```

---

### Task 5: Literals, binaries, `if`, local inference

**Files:**
- Modify: `src/typeck/expr.rs`, `src/typeck/stmt.rs`, `src/hir/mod.rs`
- Test: `src/typeck/tests/mod.rs`

- [ ] **Step 1: Tests**

```rust
#[test]
fn adds_i32() {
    assert!(check(r#"fun main(): i32 { return 1 + 2 }"#).is_ok());
}

#[test]
fn rejects_add_string_int() {
    let err = check(r#"fun main(): i32 { return 1 + "a" }"#).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn if_cond_must_be_bool() {
    let src = r#"fun main(): i32 { if (1) { return 1 } else { return 0 } }"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}
```

- [ ] **Step 2: Implement**

- `Str` → `Ty::string(false)`
- Arith ops: numeric primitives only; result = lhs ty
- Compare ops → `bool`
- `if` condition must be `bool`
- Unannotated `val x = expr` uses expr ty

- [ ] **Step 3: PASS + commit**

```bash
git commit -m "feat(typeck): literals, binary ops, and if conditions"
```

---

### Task 6: Calls — arity and argument types

**Files:**
- Modify: `src/typeck/expr.rs`, `src/typeck/env.rs`
- Test: `src/typeck/tests/mod.rs`

- [ ] **Step 1: Tests**

```rust
#[test]
fn call_arity_mismatch() {
    let src = r#"
fun f(x: i32): i32 { return x }
fun main(): i32 { return f() }
"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn call_arg_type_mismatch() {
    let src = r#"
fun f(x: i32): i32 { return x }
fun main(): i32 { return f("a") }
"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn call_ok() {
    let src = r#"
fun f(x: i32): i32 { return x + 1 }
fun main(): i32 { return f(41) }
"#;
    assert!(check(src).is_ok());
}
```

- [ ] **Step 2: Resolve `Expr::Ident` callees against fun sigs; ignore trailing for now (Task 8)**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(typeck): function call arity and argument types"
```

---

### Task 7: Nullable, `Some`/`None`, `?:`, `!!`

**Files:**
- Modify: `src/typeck/expr.rs`, `src/hir/ty.rs`
- Create: `src/typeck/tests/nullable.rs`
- Modify: `src/typeck/tests/mod.rs` to `mod nullable;`

- [ ] **Step 1: Tests**

```rust
#[test]
fn elvis_requires_nullable_lhs() {
    let src = r#"
fun main(): String {
    val s: String = "a"
    return s ?: "b"
}
"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn elvis_ok() {
    let src = r#"
fun main(): String {
    val s: String? = None
    return s ?: "Guest"
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn not_null_assert_ok() {
    let src = r#"
fun main(): String {
    val s: String? = Some("x")
    return s!!
}
"#;
    assert!(check(src).is_ok());
}
```

- [ ] **Step 2: Rules**

- `None` typed from expected context as `T?`; else error cannot infer
- `Some(e)` → `T?` for `e: T`
- `?:` → lhs `T?`, rhs `T`, result `T`
- `!!` → `T?` → `T`

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(typeck): nullable Some/None elvis and not-null assert"
```

---

### Task 8: Trailing closures

**Files:**
- Modify: `src/typeck/expr.rs`
- Create: `src/typeck/tests/closures.rs`

- [ ] **Step 1: Tests**

```rust
#[test]
fn trailing_closure_arg_count() {
    let src = r#"
fun action(a: i32, b: i32, block: (i32, i32) -> i32): i32 {
    return block(a, b)
}
fun main(): i32 {
    return action(1, 2) { x -> x }
}
"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn mvp_sample_typechecks() {
    let r = check(crate::MVP_SAMPLE);
    assert!(r.is_ok(), "{:?}", r.err());
}
```

- [ ] **Step 2: Implement**

- When trailing closure present, last formal must be `Ty::Func`
- Param count must match; bind params to formal param tys; type body; result matches `ret`

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(typeck): trailing closure parameter and result typing"
```

---

### Task 9: `for`, `move`, fields, `UseKind`, `RegionId`

**Files:**
- Modify: `src/typeck/*`, `src/hir/mod.rs`
- Test: `src/typeck/tests/mod.rs`

- [ ] **Step 1: Tests**

```rust
#[test]
fn for_range_ok() {
    let src = r#"
fun main(): i32 {
    var a: i32 = 0
    for (i in 0..3) {
        a = a + i
    }
    return a
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn move_expr_preserves_type() {
    let src = r#"
fun main(): String {
    var s: String = "hi"
    val t = move s
    return t
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn process_user_sample_types() {
    let r = check(crate::PROCESS_USER_SAMPLE);
    assert!(r.is_ok(), "{:?}", r.err());
}
```

- [ ] **Step 2: Implement**

- `BinOp::RangeTo` → `Ty::Range { elem }`
- `for`: iter is `Range`; loop var gets `elem` ty; new `RegionId` for body
- `Expr::Move` → binding ty + `UseKind::Move`
- `Expr::Ident` → `UseKind::Copy` if `ty.is_copy()`, else `UseKind::Local` (set `Shared` when matching `ArenaReport` observation exists)
- Builtin field: `String.length` / nullable safe field on `String?` → `i32` / `i32?` as needed for `PROCESS_USER_SAMPLE`
- Unknown field → type error
- Unknown named types (not `String`) in annotations → type error

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(typeck): for ranges, move expr, fields, use-kind on HIR"
```

---

### Task 10: Combined diagnostics + LSP

**Files:**
- Modify: `src/frontend.rs`
- Modify: `src/lsp.rs`
- Test: `src/frontend.rs`, lsp tests

- [ ] **Step 1: Test**

```rust
#[test]
fn reports_ownership_and_type_together() {
    let src = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
    let err = check(src).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Ownership));
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}
```

- [ ] **Step 2: Frontend always runs analyze + typeck after successful parse; concatenate diags; never panic on ownership failure**

- [ ] **Step 3: LSP maps `frontend` diagnostics (message may include phase prefix `ownership:` / `type:`)`

- [ ] **Step 4: Run**

Run: `cargo test -p bork --features lsp`  
Expected: PASS (update assertion strings if messages changed)

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(frontend): combine ownership and type diagnostics; wire LSP"
```

---

### Task 11: Phase 1 acceptance + README

**Files:**
- Modify: `README.md`
- Modify: `src/typeck/tests/acceptance.rs` (or `mod.rs`)

- [ ] **Step 1: Tests**

```rust
#[test]
fn samples_check_clean() {
    assert!(check(crate::MVP_SAMPLE).is_ok());
    assert!(check(crate::PROCESS_USER_SAMPLE).is_ok());
}

#[test]
fn unknown_named_type_errors() {
    let src = r#"fun main(): Foo { return 1 }"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
}
```

- [ ] **Step 2: README blurb**

```markdown
## Typecheck

`bork check file.bork` (or `bork file.bork`) runs parse, ownership, and full typecheck.
Native codegen / `bork build` is phase 2 — see `docs/superpowers/specs/2026-09-23-typed-hir-llvm-design.md`.
```

- [ ] **Step 3: Full suite**

Run: `cargo test -p bork`  
Expected: all PASS

- [ ] **Step 4: Commit**

```bash
git commit -m "test(typeck): phase 1 acceptance samples and README check docs"
```

---

## Spec coverage (self-review)

| Spec item (phase 1) | Task |
|---------------------|------|
| `frontend::check` | 3, 4, 10 |
| Full grammar typecheck | 5–9 |
| Ownership stays in sema | 3, 10 |
| Unified diagnostics | 1, 10 |
| `bork check` / bare file | 3 |
| LSP same pipeline | 10 |
| HIR + Ty + UseKind + RegionId | 2, 9 |
| String builtin | 2, 5, 7, 9 |
| Samples check clean | 8, 9, 11 |
| No LLVM | all |

**Phase 2 (separate plan later):** Inkwell, `bork build`, runtime, codegen NYI gate.

## Out of scope

- `bork build`, Inkwell, `bork_runtime`, MIR
