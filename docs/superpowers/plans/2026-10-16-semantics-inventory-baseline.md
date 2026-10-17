# Semantics Inventory & Regression Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Freeze today's accepted/rejected semantics as a doc inventory plus a golden HIR-type snapshot, with zero behaviour change (issue #28, TS-P0-01).

**Architecture:** One doc (`docs/type-system/inventory.md`) with feature x stage matrix, compat matrix, run instructions, orphan list. One new integration test (`tests/hir_types_golden.rs`) that runs `check()` over the passing corpus and diffs against `tests/golden/hir_types.txt`; a second test in the same file keeps the doc honest by asserting every AST/HIR variant name appears in the inventory.

**Tech Stack:** Rust, std only (no `insta`; bless via `BORK_BLESS=1`).

**Spec:** GitHub issue #28; cites `docs/superpowers/specs/2026-10-16-vec-design.md`.

## Global Constraints

- No compiler behaviour change; `tests/programs.rs` untouched.
- Doc file only `docs/type-system/inventory.md`; do not fix `docs/language.md`.
- Every matrix cell is `verified: <test path>` or `UNVERIFIED`.
- Snapshot must be deterministic (sorted file paths, repo-relative, `/` separators).

## Review Focus

- Corpus file that fails `check()` in `check/` or `build/`: golden test must name the file, not panic opaquely.
- Rename/addition of an AST/HIR variant: doc-coverage test fails naming the missing variant.
- Windows path separators / unsorted dir order making the snapshot flaky.
- `BORK_BLESS` unset and snapshot missing: fail with instructions, not silently pass.
- Functions with `Ty::Unknown` in output: printed as-is (that is baseline truth).

---

### Task 1: Golden HIR-type snapshot

**Files:**
- Create: `tests/hir_types_golden.rs`, `tests/golden/hir_types.txt`

**Interfaces:**
- Produces: `fn render(path_rel: &str, hir: &HirProgram) -> String` (test-local), one line per function `<rel path>::<name>(<kind? name: ty, ...>) -> <ret>` using `Ty`'s `Display`.

- [ ] **Step 1:** Write `golden_hir_types_match_snapshot`: collect `.bork` under `programs/check` and `programs/build` (build samples pass the frontend, so they widen coverage; `check_fail`/`build_fail` yield no HIR — note this deviation from the issue's "check*" wording in the doc), sort by relative path, call `bork::check(&src)` (confirm lib crate name in `Cargo.toml`), `assert!(result.is_ok())` with the path in the message, render `result.hir.unwrap().functions`, join, compare to `tests/golden/hir_types.txt`.
- [ ] **Step 2:** Run `cargo test --test hir_types_golden` — Expected: FAIL (snapshot missing, message says `BORK_BLESS=1`).
- [ ] **Step 3:** If `BORK_BLESS=1`, write the snapshot instead of comparing. Run `BORK_BLESS=1 cargo test --test hir_types_golden`, review the file by eye for sanity, commit it.
- [ ] **Step 4:** Run `cargo test --test hir_types_golden` twice — Expected: PASS both times (determinism).
- [ ] **Step 5:** Commit `test: golden HIR-type snapshot over passing corpus`.

### Task 2: Inventory doc and its coverage guard

**Files:**
- Create: `docs/type-system/inventory.md`
- Modify: `tests/hir_types_golden.rs` (add test)

**Interfaces:**
- Consumes: variant lists — `ast::Type` {Primitive, Named, Array, Func, Ref, ManagedRef, Vec}; `TyKind` {Prim, Named, Array, Func, Ref, ManagedRef, Vec, Range, Unknown}; `ast::Expr` {Int, Float, Bool, Str, ArrayLit, Index, Slice, VecNew, Ident, Move, Promote, None, Some, Binary, Unary, Field, Call, If, IfVal}; also `Stmt` {Block, VarDecl, Assign, For, While, Break, Continue, MoveBlock, Return, Expr} (issue omits Stmt; include it, cheap).

- [ ] **Step 1:** Write `inventory_mentions_every_variant`: read the doc, and for each hard-coded variant list above assert a `` `Name` `` token occurs. Run it — Expected: FAIL (file missing).
- [ ] **Step 2:** Write the doc sections: (a) feature matrix, rows = prim types, `T?`, `Ref<T>`, `&T`, `[T;N]`, `Vec<T>`, `Range`, `String`, classes, methods, consts, closures, `move`/`promote`; columns = parsed, typechecked, sema/escape, memory plan, codegen-gated (`src/codegen_gate.rs`), codegen-implemented. Fill each cell by grepping `src/` and `programs/`/`src/typeck/tests/` for a test; else `UNVERIFIED`. (b) Variant tables for Type/TyKind/Expr/Stmt. (c) Compat matrix: per row, which later phase may change it and what stays byte-identical (the golden snapshot + `programs/` expectations). (d) Cited decisions from the Vec spec: built-in, no general generics; non-goals `Vec<Vec<T>>`, `Vec<T?>`, `Vec<Node>`; element-borrow rule "not enforced yet". (e) Orphan/unwired tests from `TODO.md` (e.g. root-level `*.bork`, gate-vs-frontend sync test missing). (f) Known stale spots in `language.md` (Vec, trailing closures rejected in gate) — listed, not fixed.
- [ ] **Step 3:** Run `cargo test --test hir_types_golden` — Expected: PASS.
- [ ] **Step 4:** Commit `docs: type-system inventory and compatibility matrix`.

### Task 3: Run instructions and recorded results

**Files:**
- Modify: `docs/type-system/inventory.md`

- [ ] **Step 1:** Add "Running the suites": command list mirroring `.github/workflows/ci.yml` (`cargo test --workspace`, `cargo test programs_corpus_check`, `cargo test --workspace --no-default-features`, `LLVM_SYS_231_PREFIX=/usr/lib/llvm-23 cargo test --features codegen`).
- [ ] **Step 2:** Run default suites; record date, commit, and counts. Try `--features codegen`; if LLVM 23 is absent, write "NOT RUN: LLVM 23 unavailable" — do not guess results.
- [ ] **Step 3:** Commit `docs: record baseline test runs`.

## Self-review

Coverage: inventory (T2), matrix cells verified/UNVERIFIED (T2), golden (T1), run doc (T3), Vec spec cites (T2), orphan tests (T2). Proportion fine.
