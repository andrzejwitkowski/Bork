# Reference type (`&Ty`) + post-merge borrow hardening — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After `main` includes LLVM 23, land a coherent borrow story: fix known gaps on use-site `&`, then add a real reference type `&Ty` in signatures so callers must pass a cross-region borrow explicitly.

**Architecture:** Keep use-site `&expr` as the only way to *create* a borrow. Add `Type::Ref` / `TyKind::Ref` for *positions* (parameters first; returns and fields still forbidden). Sema ties `Ref` parameters to `Ownership::Borrow` and forbids lifting; typeck enforces `f(&a)` when the formal is `&T`. Codegen ABI unchanged: reference parameters are descriptor pass-through (same as today’s `step(&a)` with a by-value array formal).

**Tech Stack:** Rust, `lalrpop`, `sema`, `typeck`, `hir`, `escape`, LLVM codegen (`codegen` feature).

## Global Constraints

- No field-stored references; no `return` of `&Ty` (unchanged product boundary).
- No `&mut` as a type in v1 of this plan (element mutation through `&[T; N]` is a dedicated rule; see **Locked decisions**).
- Minimize scope: parameters and locals typed `&T` only where needed; do not build Rust-full borrow checker.
- Follow existing patterns in `sema/walk.rs`, `typeck/expr/call.rs`, `docs/memory-model.md`.

---

## Locked decisions (product)

| Topic | Decision |
|-------|----------|
| Syntax | `&` prefixes a **type**: `fun poke(buf: &[i32; 100])`, not a new keyword (`ref buf`). |
| What can be referenced | Only **non-Copy** types (`String`, `[T; N]`, later named aggregates). `&i32` is a compile error. |
| Call site | Formal `&T` requires argument `&expr` where `expr` is a single `ident` (same as today’s borrow expr). Bare `a` or `move a` is rejected with a dedicated message. |
| Formal without `&` | `fun own(buf: [T; N])` still means ownership transfer (`move` at call site for outer `var`). |
| Mutability through `&[T; N]` | Index **read** and **assign** `buf[i] = …` allowed inside callee when formal is `&[T; N]` (mutates owner’s buffer; binding may stay `val`). Does **not** apply to plain `val buf: [T; N]` after move. |
| `&String` | **Locked:** read/shared descriptor only in v1; no mutation or reseat through `&String` (`&mut String` deferred). |
| Locals | `val v: &T = &a` allowed in v1; `var v: &T` rejected (mirror `var b = &a`). |
| LSP / dump | `Ownership::Borrow` + type display `&[i32; 3]` on formals and bindings. |
| Migration | New helpers should use `&T` formals; bare array formals remain for ownership APIs. |

## Approaches considered

1. **Convention only (status quo)** — `var a: [T; N]` + `f(&a)` at call site. **Rejected:** signature does not document intent; thermo review P0/P1.
2. **`&Ty` in types (chosen)** — explicit API; typeck enforces `&` at call site; sema aligns with `BindingOrigin::View`.
3. **Attributes** (`@borrow buf: [T; N]`) — same semantics, foreign syntax; **rejected** per author preference for `&Ty`.

---

## Phase 0 — Merge hygiene (no feature work)

**Precondition:** LLVM 23 PR merged to `main`.

| Step | Action |
|------|--------|
| 0.1 | Rebase `feat/region-borrow` onto `main` (or branch `feat/reference-type` from updated `main` with borrow commit cherry-picked). |
| 0.2 | Resolve conflicts; run `cargo test` (lib) and `cargo test --features codegen` where LLVM 23 prefix is available. |
| 0.3 | Commit outstanding deslop: `ast.rs` comment removal, `env.rs`, `memory-model.md` call-site bullet (if not already on branch). |
| 0.4 | Split optional follow-up PR: **borrow-only** diff without unrelated local files (`quicksort.bork`, patches). |

**Exit:** Green CI on branch; single logical commit series: merge base → borrow use-site → (next phases).

---

## Phase 1 — Harden use-site `&` (before `&Ty`)

Fix gaps found in deslop/thermo reviews so behavior matches docs.

### File map

| File | Change |
|------|--------|
| [`src/sema/walk.rs`](src/sema/walk.rs) | Reject `var dest = <view>` with same message family as `var b = &a`; do not rely on `move` hint. |
| [`src/sema/tests/regional.rs`](src/sema/tests/regional.rs) | `var_c_from_view_errors`, `val_d_from_view_reborrows` (if not covered). |
| [`docs/memory-model.md`](docs/memory-model.md) | Document: child **read** needs `&` or slice; child **write** to outer `var` may use `a[i] =` or `(&a)[i] =`; `while { helper(&a) }`. |
| [`tests/build.rs`](tests/build.rs) or [`tests/arrays.rs`](tests/arrays.rs) | Codegen smoke: `fun inc(var a: [i32; 3]) { a[0] = 1 }` + `while { inc(&a) }` (skip if no LLVM in env — gate like existing codegen tests). |

### Tasks

- [ ] **1.1** Add failing sema test: `val b = &s; var c = b` → error mentions borrow/view, not `move`.
- [ ] **1.2** In `walk_stmt` var decl, if `is_view_init(value)` → error before/alongside bind (same as `var b = &a`).
- [ ] **1.3** Run `cargo test regional` — pass.
- [ ] **1.4** Extend `memory-model.md` (read vs write in child regions; call-site `&` / `move`).
- [ ] **1.5** Add codegen integration test for `helper(&a)` in loop (feature `codegen`).
- [ ] **1.6** Commit: `fix(sema): reject assigning a borrow view to var`.

---

## Phase 2 — `&Ty` in AST, HIR, parser, printer

### File map

| File | Change |
|------|--------|
| [`src/parser.lalrpop`](src/parser.lalrpop) | `Type` rule: `"&" <ty:Type>` → `Type::Ref { inner, nullable: false }` (no `&T?` in v1). |
| [`src/ast.rs`](src/ast.rs) | `Type::Ref { inner: Box<Type>, nullable: bool }`; `with_nullable`, `is_copy` → false for Ref. |
| [`src/hir/ty.rs`](src/hir/ty.rs) | `TyKind::Ref(Box<Ty>)`; `Display`; `uses_arena_storage` → true when inner does; `ref_inner()` helper. |
| [`src/typeck/`](src/typeck/) | Lower `Type::Ref` to `TyKind::Ref`; resolve refs in signatures only (reject ref in field types when fields gain types later). |
| [`src/lsp.rs`](src/lsp.rs) / hover | Show `&[i32; N]` on signatures. |

### Tasks

- [ ] **2.1** Parser test: `fun f(p: &[i32; 2])` parses; `&i32` parse ok, typeck error later.
- [ ] **2.2** Implement `Type::Ref` + HIR lowering.
- [ ] **2.3** `cargo test` parser + typeck signature tests.
- [ ] **2.4** Commit: `feat(types): add reference type &T in signatures`.

---

## Phase 3 — Typecheck calls and bindings

### Rules

- Parameter `p: &T` → binding in function body is `val p` with type `&T` (or inner `T` + flag; prefer surface type `&T` in env).
- Call: `check(arg, Some(&TyKind::Ref(...)))` requires `Expr::Unary(Borrow, Ident)`; else error: `expected borrow &name for parameter p`.
- `val x: &T = &a` — ok; `var x: &T = …` — error.
- Assign `x = &a` when `x: &T` — ok if same region rules.

### File map

| File | Change |
|------|--------|
| [`src/typeck/expr/call.rs`](src/typeck/expr/call.rs) | Ref-aware argument checking (not only structural equality on `Ty`). |
| [`src/typeck/expr/mod.rs`](src/typeck/expr/mod.rs) | `check_unary_borrow` returns `TyKind::Ref(inner)` not inner alone. |
| [`src/typeck/stmt.rs`](src/typeck/stmt.rs) | `assign_binding`: allow index-assign when binding type is `Ref` to array. |
| [`src/sema/walk.rs`](src/sema/walk.rs) | On function entry, params with ref type → `BindingOrigin::View`; `note_borrow` on call args when formal is ref (even if arg syntactically bare — should not happen). |

### Tasks

- [ ] **3.1** Failing tests: `fun f(p: &[i32; 2])` + `f(a)` errors; `f(&a)` ok.
- [ ] **3.2** Implement call + decl rules.
- [ ] **3.3** Failing test: `p[0] = 1` inside body with `p: &[i32; N]` passes typeck.
- [ ] **3.4** Commit: `feat(typeck): enforce & arguments for &T parameters`.

---

## Phase 4 — Sema, escape, codegen

### Sema

- Callee region: param `&T` records **Borrow** from caller’s owner at **call site** (child observation on caller’s arena node for the loop/block).
- Forbid returning ident whose type is `Ref` (escape).
- Forbid storing `Ref` into outer `var` / fields (extend `escape::place`).

### Codegen

- Formal `&[T; N]` → same as today: parameter slot holds descriptor; no arena copy on entry.
- Ensure `emit_param` / `lookup` treats ref param as non-moving load.

### File map

| File | Change |
|------|--------|
| [`src/sema/walk.rs`](src/sema/walk.rs) | `open_function`: ref params → View; call walk: match formals to borrow. |
| [`src/sema/policy.rs`](src/sema/policy.rs) | Optional: `bare_ident_move_message` when formal type is `Ref` → suggest `&name` not `move`. |
| [`src/escape.rs`](src/escape.rs) | `TyKind::Ref` in return position / assign to outer. |
| [`src/codegen/llvm/`](src/codegen/llvm/) | Param types Ref → emit like inner arena type; no regression on `coerce_value_to_ty`. |

### Tasks

- [ ] **4.1** Sema tests: call `f(&a)` from `while` records borrow on `a` each iteration.
- [ ] **4.2** Escape tests: `return p` with `p: &String` fails.
- [ ] **4.3** Codegen test: mutating `p[i]` visible on caller’s `a` after `f(&a)`.
- [ ] **4.4** Commit: `feat(sema): reference parameters and escape rules`.

---

## Phase 5 — Docs and deprecation notes

- [x] **5.1** Rewrite **Borrow** in [`docs/memory-model.md`](docs/memory-model.md): `&Ty` vs use-site `&`; `T` vs `&T` formals; **`&String` read-only**.
- [x] **5.2** Update **Not planned** (no field/`return` refs; no `&mut`; `&String` read-only).
- [x] **5.3** [`docs/language.md`](docs/language.md): types table `&T`, `&name`, ownership rows, examples (`peek` / `bump`).
- [ ] **5.4** Commit: `docs: reference type &T and &String read-only` (after implementation phases or with spec-only commit as agreed).

---

## Phase 6 (optional, later) — `&mut Ty`

Out of scope for this plan unless explicitly scheduled:

- Exclusive borrow type, `&mut` in types, reseat `String`, conflict if two `&mut` to same owner.

---

## Testing matrix (acceptance)

| Scenario | Expected |
|----------|----------|
| `fun f(p: &[i32; N])` / `f(&a)` from child/`while` | OK, borrow recorded |
| `f(a)` / `f(move a)` with `&` formal | Error |
| `fun g(p: [i32; N])` / `g(move a)` | OK (ownership) |
| `var x: &T = &a` | Error |
| `val x: &T = &a` | OK |
| `p[i] = v` with `p: &[T; N]` | OK |
| `b[i] = v` with `val b = &a` | Still error (view name is `val`, type not `Ref`) — use `(&a)[i]` or typed `val p: &T` param |
| `return p` for `p: &T` | Error |
| `&i32` formal | Error |

---

## Suggested PR sequence (after merge)

1. `fix(sema): borrow view hardening + docs` (Phase 1)
2. `feat(types): &T in AST/HIR/parser` (Phase 2)
3. `feat(typeck): &T call checking` (Phase 3)
4. `feat(sema,escape,codegen): ref params` (Phase 4)
5. `docs: &T` (Phase 5)

Stack on one feature branch or merge 1 before 2–5; do not mix LLVM infra.

---

## Author decision (locked)

**`&String` in v1:** read-only only (documented in `memory-model.md` and `language.md`). Mutation/reseat waits for `&mut String` or owned `var` parameters.
