# Phase 2 — Native LLVM Codegen (`bork build`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit a Linux x86_64 native binary from subset-B Bork programs via Inkwell (LLVM 18), linked with a bump-arena runtime, including `print`/`println` and correct Shared/`move` string lowering.

**Architecture:** `frontend::check` already yields `CheckResult { report, hir, diagnostics }`. Phase 2 adds (1) builtins so `print`/`println` typecheck, (2) a codegen **capability gate** on HIR, (3) a **region schedule** derived from `ArenaReport` + HIR nesting (HIR has no `RegionId` today), (4) Inkwell emission + link of `bork_runtime`. No MIR.

**Tech Stack:** Rust 2021, existing HIR/typeck/sema, Inkwell (LLVM 18 feature), `clang` link, small `bork_runtime` staticlib (Rust C ABI from `src/arena.rs`).

**Spec:** [docs/superpowers/specs/2026-09-23-typed-hir-llvm-design.md](../specs/2026-09-23-typed-hir-llvm-design.md) (Phase 2 section).  
**Memory model:** [docs/memory-model.md](../../memory-model.md).

## Global Constraints

- Target: **Linux x86_64** only.
- LLVM pin: **18** via Inkwell feature; confirm `llvm-config` at implement time (bump to 19 only if 18 unavailable).
- Feature `codegen` gates Inkwell + `bork build` (default off or on — lock: **default features keep LSP; `codegen` optional** so contributors without LLVM can still `cargo test`).
- Subset B only for emission; anything else → `Phase::Codegen` diagnostic with span; **no binary**.
- NYI at build (check may still pass): trailing closures, `Some`/`None`/`?:`/`!!`, field access beyond what subset needs.
- Strings: fat pointer `{ptr, len}`; literal → `.rodata`; Shared → copy descriptor; `move` → `arena_alloc` + `memcpy` into destination arena.
- Arenas: **4096**-byte bump, pool recycle; push on region entry, reset on exit; `for` body resets each iteration.
- Exit codes: `0` OK; `1` user diagnostics; `2` toolchain (missing LLVM/clang/link).
- Do not re-implement ownership in codegen; trust HIR `UseKind` + dead bindings already rejected by check.
- Restore or reconstruct **Shared** for non-Copy parent `val` reads (Phase 1 removed `UseKind::Shared` — Task 2 below).

---

## File map

| Path | Role |
|------|------|
| `crates/bork_runtime/` or `runtime/` | C ABI: arena push/reset/alloc, print_i64, print_str, println_* |
| `src/arena.rs` | Keep as reference / share constants; runtime may `include` or duplicate thin C ABI wrapper |
| `src/codegen/mod.rs` | `emit` / `build` entry; feature-gated |
| `src/codegen/gate.rs` | Walk HIR → codegen diagnostics (NYI) |
| `src/codegen/regions.rs` | Build region schedule from `ArenaReport` + HIR |
| `src/codegen/llvm/` | Inkwell context, types, function emission, strings |
| `src/codegen/link.rs` | Write `.o`, invoke `clang` + runtime |
| `src/typeck/` (+ maybe `src/builtins.rs`) | Recognize `print`/`println` as builtins |
| `src/main.rs` | `bork build [-o out] <file>` |
| `src/hir/mod.rs` | Re-add `UseKind::Shared` if Task 2 chooses that path |
| `tests/codegen/` or `tests/build/` | Golden `.bork` → build → run |
| `Cargo.toml` | workspace/`bork_runtime`, optional `inkwell` |
| `README.md` | LLVM 18 + clang prerequisites for `build` |

---

### Task 1: Builtins `print` / `println` in the frontend

**Files:**
- Create: `src/builtins.rs` (signatures)
- Modify: `src/typeck/expr/call.rs` (or env fun_sigs) to resolve builtins
- Modify: `src/hir/` if needed (`HirExprKind::BuiltinCall { name, args }` **or** lower as ordinary calls to known names — lock: **`HirExprKind::Call` to ident `print`/`println`** with synthetic fun sigs injected into typeck)
- Test: `src/typeck/tests/mod.rs`

**Interfaces:**
- Produces: typeck accepts:
  - `print(x: i32|i64|String)` and `println(...)` same; overload by arg type (exact one arg for v1)
  - Return type `unit`
- Lock: only **one** argument; i32/i64/String (non-null) for v1

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn print_i32_typechecks() {
    let src = r#"
fun main(): i32 {
    println(42)
    return 0
}
"#;
    assert!(crate::frontend::check(src).is_ok());
}

#[test]
fn print_unknown_name_still_errors() {
    let src = r#"fun main(): i32 { printlnn(1); return 0 }"#;
    assert!(crate::frontend::check(src).diagnostics.iter().any(|d| d.phase == crate::diag::Phase::Type));
}
```

- [ ] **Step 2: Inject builtin signatures into typeck `fun_sigs` before walking bodies** (names reserved; user `fun print` → type error “cannot redefine builtin” — optional YAGNI: skip redefine check until collision appears)

- [ ] **Step 3: Tests PASS + commit**

```bash
git commit -m "feat(typeck): add print/println builtins for subset-B I/O"
```

---

### Task 2: Restore Shared use-kind for codegen

**Why:** Phase 1 emits `UseKind::Local` for all non-Copy idents. Spec string lowering needs Shared vs Move vs Copy.

**Files:**
- Modify: `src/hir/mod.rs` — re-add `UseKind::Shared`
- Modify: `src/typeck/expr/mod.rs` `check_ident` — if not Move and not Copy: prefer Shared when binding is `val` (parent-safe); `var` bare read stays Local/error already at ownership — for typeck use **Shared for `val` non-Copy**, **Local for same-region**, approximate: **`val` → Shared, else Local** (ownership already forbade illegal cases). Better: pass `&ArenaReport` into typeck and match observations — lock for this task: **`val` non-Copy → Shared, `var` non-Copy → Local** (move path sets Move).
- Test: assert HIR use_kind on parent `val` string read inside nested block is `Shared`; `move s` is `Move`.

- [ ] **Step 1: Tests for UseKind on string val/move**

- [ ] **Step 2: Implement + commit**

```bash
git commit -m "feat(hir): restore Shared use-kind for non-Copy val reads"
```

---

### Task 3: Codegen capability gate (no LLVM yet)

**Files:**
- Create: `src/codegen/mod.rs`, `src/codegen/gate.rs`
- Modify: `src/lib.rs` — `#[cfg(feature = "codegen")] pub mod codegen;`
- Modify: `Cargo.toml` — feature `codegen = []` empty stub first
- Test: unit tests on HIR fixtures / string sources via `frontend::check` then `gate`

**Interfaces:**
- Produces: `pub fn gate(hir: &HirProgram) -> Vec<Diagnostic>`
- Reject with `Phase::Codegen` + span when seeing:
  - trailing-closure call shape (if represented), `Some`/`None`/`!!`/`Elvis` binary, unsupported fields
  - any `HirExprKind` outside allowlist
- Allowlist: Int, Str, Ident, Binary (arith/cmp/`RangeTo` only), Call (no trailing), If, Field only if we decide NYI for `.length` in build — **lock: `.length` is NYI at build** (nullable sample may check but not build)

- [ ] **Step 1: Test** trailing-closure program checks OK, `gate` returns codegen diagnostic

```rust
#[test]
fn gate_rejects_trailing_closure() {
    let r = frontend::check(crate::MVP_SAMPLE);
    assert!(r.is_ok());
    let diags = codegen::gate(r.hir.as_ref().unwrap());
    assert!(diags.iter().any(|d| d.phase == Phase::Codegen));
}
```

- [ ] **Step 2: Implement walk + commit**

```bash
git commit -m "feat(codegen): subset-B capability gate without LLVM"
```

---

### Task 4: `bork_runtime` C ABI static library

**Files:**
- Create: `crates/bork_runtime/Cargo.toml`, `crates/bork_runtime/src/lib.rs`
- Modify: root `Cargo.toml` as workspace if needed
- Mirror alloc/reset/pool from `src/arena.rs` (`ARENA_CAPACITY = 4096`)

**Interfaces (C ABI, `#[no_mangle] extern "C"`):**

```c
void* bork_arena_push(void);           // acquire slab, return opaque handle
void  bork_arena_reset(void* arena); // reset bump (loop iter / exit before release)
void  bork_arena_pop(void* arena);   // reset + release to pool
void* bork_arena_alloc(void* arena, size_t size, size_t align);
void  bork_print_i64(int64_t);
void  bork_print_str(const uint8_t* ptr, size_t len);
void  bork_println_i64(int64_t);
void  bork_println_str(const uint8_t* ptr, size_t len);
```

- [ ] **Step 1: Unit tests in runtime crate** (alloc, reset, overflow panic or null — lock: **panic on overflow** matching `Arena`)

- [ ] **Step 2: `cargo build -p bork_runtime` produces `libbork_runtime.a`**

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(runtime): bump arena C ABI and print helpers"
```

---

### Task 5: Region schedule from `ArenaReport` + HIR

**Problem:** HIR has no region ids; codegen must push/pop arenas.

**Approach (lock):** Build an explicit `RegionSchedule` while emitting: stack of opaque runtime handles. Structural mapping:
- Enter `HirFunction` body → push
- Enter `HirStmt::MoveBlock` / nested `HirStmt::Block` that corresponds to a sema region → push (nested bare blocks may be peeled in sema — **match dump tree**, not every HIR Block)
- Enter `HirStmt::For` body → push once; **each iteration** call `reset` at loop latch; pop after loop
- `HirExprKind::If` branches → each branch body that opened a region in report → push/pop around branch emission

Practical MVP for Task 5 deliverable:

- [ ] Define `struct RegionEmitter` with `push`/`reset`/`pop` IR builders (calls to runtime), used by LLVM tasks
- [ ] Document mapping table in `src/codegen/regions.rs` comments from `ArenaNode` labels (`fun`, `for`, `move`, …) to HIR stmt kinds
- [ ] Unit-test: pure Rust schedule on a small program’s `CheckResult.report` shape (no LLVM) — count push/pop equals nest depth for `fun main() { { val x = 1 } }` after peel rules

If schedule cannot be derived reliably from report alone, **fallback lock:** annotate HIR in a new `codegen::lower` pass that walks AST+report in lockstep with typeck’s stmt structure and attaches `region_enter`/`region_exit` markers as a thin `CodegenHir` — only if Task 5 schedule proves insufficient. Prefer schedule first.

- [ ] Commit: `feat(codegen): region push/reset schedule for arena runtime`

---

### Task 6: Inkwell skeleton + hello i32

**Files:**
- `Cargo.toml`: `inkwell = { version = "…", features = ["llvm18-0"], optional = true }`, `codegen = ["dep:inkwell"]`
- `src/codegen/llvm/mod.rs`, `context.rs`, `emit_fn.rs`
- CI: document apt packages `llvm-18-dev`, `clang` (or soft-fail job without codegen)

**Interfaces:**
- `pub fn emit_module(hir: &HirProgram, report: &ArenaReport) -> Result<inkwell::module::Module, Diagnostic>`
- First slice: only `fun main(): i32 { return N }` → LLVM function `main` returning i32, **no** arena calls yet

- [ ] **Step 1: Integration test** (ignored without feature):

```rust
#[cfg(feature = "codegen")]
#[test]
fn builds_return_constant() {
    // check → gate → emit → write .o → clang -o → Command::output assert exit 7
}
```

Fixture:

```bork
fun main(): i32 {
    return 7
}
```

- [ ] **Step 2: Implement emit + object write + `clang -o` link with libc only**

- [ ] **Step 3: `bork build` CLI**

```text
bork build [-o <path>] <file.bork>
```

Pipeline: `check` → if diags exit 1 → `gate` → if diags exit 1 → emit → link → exit 0; toolchain errors exit 2.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(codegen): Inkwell hello main and bork build CLI"
```

---

### Task 7: Locals, assign, if, arithmetic, calls

**Files:** `src/codegen/llvm/*`

- Alloca for `var`/`val` slots (primitives)
- Load/store; `if` with basic blocks
- Binary ops for i32/i64 (extend/trunc as needed from `Ty`)
- Direct calls between Bork `fun`s
- Arena push/pop around function body (and blocks per schedule)

- [ ] **Tests:**

```bork
fun add(a: i32, b: i32): i32 { return a + b }
fun main(): i32 {
    val x = add(40, 2)
    if (x > 40) { return x } else { return 0 }
}
```

Expect exit code 42.

- [ ] Commit: `feat(codegen): locals, control flow, and calls`

---

### Task 8: `for` + per-iteration arena reset

- Emit loop: header / body / latch
- Body region: `push` before loop or once; **`reset` at end of each iteration**; `pop` after loop
- Loop index as i32 local

- [ ] Test: sum `0..5` returns 10; optional: allocate string each iter and only last visible (if strings already in) — can wait for Task 9

- [ ] Commit: `feat(codegen): for-loops with arena reset each iteration`

---

### Task 9: Strings + print + move/Shared lowering

**LLVM type:** `{ i8*, i64 }` (or `ptr` + `i64`)

| UseKind / situation | Lowering |
|---------------------|----------|
| `Str` literal | global constant bytes + descriptor |
| Ident `Copy` | N/A for String |
| Ident `Shared` | copy struct registers/stack |
| Ident `Move` / Expr move | `alloc(len)` in **current** arena, `memcpy`, new descriptor |
| `print`/`println` | call runtime with ptr/len or i64 |

Regional `move { }` / `MoveBlock`: push region; emits body; pop.

- [ ] Golden tests (run binary):

1. `println("hi")` → stdout `hi\n`, exit 0  
2. Shared: parent `val s = "x"`; nested block `println(s)`  
3. Move: `var s = "ab"; val t = move s; println(t)`  
4. Gate: MVP_SAMPLE build fails codegen NYI  

- [ ] Commit: `feat(codegen): string descriptors, move/Shared, and print`

---

### Task 10: README + CI codegen job + acceptance

- [ ] README: install LLVM 18 + clang; `cargo run --features codegen -- build file.bork`
- [ ] CI: matrix or separate job with `llvm-18-dev` running `cargo test --features codegen`
- [ ] Spec success criteria 2–5 verified manually once
- [ ] Commit: `docs: phase 2 build prerequisites and codegen CI`

---

## Spec coverage (self-review)

| Spec item | Task |
|-----------|------|
| Subset B emission | 3, 6–9 |
| NYI gate with span | 3 |
| Runtime 4096 + pool | 4 |
| String fat pointer Shared/move/rodata | 2, 9 |
| `for` arena reset | 5, 8 |
| Inkwell LLVM 18 + clang link | 6, 10 |
| `bork build` | 6 |
| print/println | 1, 9 |
| Ownership errors before LLVM | existing check + 6 CLI order |
| Region identity without HIR RegionId | 5 |

## Out of scope

- Float literals / float codegen (typeck gap remains unless a side task)
- Nullable / closures codegen
- JIT `bork run`, Windows/macOS, MIR, `-O` productization

## Execution note

Create branch `feat/phase2-llvm-codegen` from updated `main`. Implement Tasks 1→10 in order; Task 6 is the first “binary runs” milestone.
