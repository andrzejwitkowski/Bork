# Move `Vec<T>` into Bork (#285, replaces steps 2-4 of the issue) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete every compiler-side `Vec` (`VecOp`, `VecNew`, `DropGlue::Vec`, `LangItem::Vec`, `bork_vec_*`) and make `Vec<T>` an `opaque` prelude class written in Bork on `Buf<T>`, for all element types `Vec` accepts today.

**Architecture:** `Buf<T>` (#354) learns element glue (copy on grow/promote, drop on arena death, `Ref` slot store) from `T` inside MIR build/codegen, so Bork code in `Vec` cannot tell `Vec<i32>` from `Vec<Ref<C>>`. The Bork `Vec` first lands as `VecB` beside the intrinsic `Vec` (no choice by `T`, just a second name), is proven against the whole `vec` corpus, then takes the name `Vec` and the intrinsic goes away.

**Tech Stack:** Rust, LLVM codegen (`src/codegen/llvm`), `.bork` corpus in `programs/`, `crates/bork_runtime`.

**Spec:** the #285 grilling session of 2026-11-26 (decisions in Global Constraints); issue #285; `docs/superpowers/plans/2026-11-25-buf-unsafe-prelude-prototype.md` (step 1, merged as #354; #350/#357 made `Stack` opaque).

## Global Constraints

- Branch from `main` after #357 (opaque classes) is in. One PR per Part below, each green on its own.
- No choice of implementation by `T`. Before Part 3 two names exist (`Vec`, `VecB`); after it one.
- Element set and rejections unchanged: whatever `is_vec_elem_supported` accepts today (scalars, `String`, arrays, `Ref<C>`, non-`FnOnce` closures/functions, `T?`, enums, `Vec`); classes stay rejected (`check_fail/vec/class_element.bork`); every `programs/check_fail/vec/*.bork` expectation unchanged.
- Growth policy as today: capacity 0 -> 4 -> double (`FIRST_CAPACITY = 4`, `src/codegen/llvm/vec/mod.rs`). `capacity()` observable values unchanged.
- `Vec(n)` stays special syntax in typeck; `n < 0` aborts (`negative_capacity_aborts.bork`).
- `Vec` is an `opaque` class `{ buf: Buf<T>, n: i32 }`; the 32-byte header is gone. `mir_layout`, `arena_report`, `hir_types`, `mono_instances`, `memory_plan` goldens may change, and only for programs using `Vec`.
- `impl Trait for Vec` / `Iterator` on `Vec` are out of scope (separate issue, linked from #285); `check_fail/vec/impl_trait.bork` keeps failing.
- Test runs: follow `AGENTS.md` (`LD_LIBRARY_PATH`, `nice -n 19 cargo test -j1 ... -- --test-threads=1`, background, narrow targets).

## Review Focus

- `Vec` moved/promoted to an outer arena must not leave `buf` pointing into the dying inner arena. Pinned in Task 6.
- `Vec<Ref<C>>`: every live `Ref` released exactly once on arena death, none after `pop`/`clear`/`set` overwrite. Pinned in Tasks 2 and 7.
- Grow with elements needing glue (`Ref`, closures, `String`) keeps old slots valid until the arena dies and does not double-release. Pinned in Task 2.
- `get`/`set`/`v[i]` with `i < 0` and `i >= len` abort, not read stale slots between `len` and `cap`. Pinned in Task 4.
- `pop` on empty and `Vec<Ref<C>>` `pop` returning the `Ref` (not `Option`) as today. Pinned in Task 4.
- `&v[i]` view locks the `Vec` (all `element_*` check_fail programs) now that `Vec` is a class with fields. Pinned in Task 8.
- A program that does not mention `Vec` pulls neither chunk in (goldens identical). Pinned in Task 3.

---

## Part 1 (PR 1): element glue and `Ref` store in `Buf`

### Task 1: Spike - two facts the plan relies on

**Files:** none changed; findings go in the commit message of Task 2 and in a note appended to this plan.

- [ ] **Step 1:** Run a debug build (`tests/debug_build.rs` style) of a program holding `Stack<i32>` and read it with `tools/bork-debug`; record whether the reader prints a class `{buf: Buf<T>, n}` and what it does for `Buf`. Expected fact: either it already handles fields-with-handle, or Task 9 owns a reader change.
- [ ] **Step 2:** Confirm in `src/codegen/llvm` (`elem_storage`) that `String` is a handle with no `DropGlue` registration; record the answer. If it has glue, Task 2 treats it like `Ref`.
- [ ] **Step 3: Commit** the note: `docs: record Buf/String spike findings (#285)`.

### Task 2: `Buf<T>` element glue

**Files:**
- Modify: `src/mir/build/buf.rs` (register drop glue at `bufAlloc`/`bufGrow`), `src/mir/body.rs` (`DropGlue`: replace `Vec(TyId)` with `Buf(TyId)`), `src/codegen/llvm/glue.rs`, `src/codegen/llvm/buf.rs` (`buf_grow` copy per element kind), `crates/bork_runtime/src/refs.rs` (rename `bork_vec_drop_refs` to `bork_buf_drop`, take pointer + cap + element drop fn), `src/mir/plan.rs`, `src/mir/escape.rs` (treat a `Buf` slot store as a store into the buffer's arena).
- Test: `src/typeck/tests/buf.rs`, `programs/build/buf/stack_ref_elems_release.bork`, `programs/build/buf/stack_string_elems_grow.bork`, `crates/bork_runtime` unit test for `bork_buf_drop`.

**Interfaces:**
- Produces: `bufAlloc`/`bufGrow` for `T` that has glue (`Ref<C>`, enum with glue, closure with environment, `T?` of those) register one `DropGlue::Buf(elem)` drop in the buffer's arena; a new `slotStore` into a slot already holding a live `Ref` releases the old value (`bork_ref_store_take` semantics); `slotLoad` of a `Ref` gives an owned `Ref` as `Vec::get` does today.
- Produces (runtime): `bork_buf_drop(ptr: *mut c_void, cap: usize, elem_drop: DropFn)` releases `0..cap` slots that are non-null.

- [ ] **Step 1: Write failing tests.** `stack_ref_elems_release.bork`: push three `Ref<Node>`, let the arena die, assert via `ref_counts_across_arenas`-style counter that each target is released once. `stack_string_elems_grow.bork`: push 9 strings (past 4 and 8), read back all. Runtime test `bork_buf_drop_skips_null_slots`.
- [ ] **Step 2: Run** `cargo test --features codegen --test programs -- buf` (throttled); expect FAIL.
- [ ] **Step 3: Implement** the glue as above; `bufGrow` copies bytes only (old slots are not dropped, they are dead copies; ownership moves to the new buffer, whose glue is the only registered live one: register glue for the new buffer, mark the old one's registration cleared). Decide and record in a code comment how the old registration is cleared.
- [ ] **Step 4: Run** the same plus `cargo test --lib buf`; expect PASS. Run `memory_plan_golden`, `mir_golden`: existing `Vec` goldens must not change in this PR.
- [ ] **Step 5: Commit** `feat: Buf element glue and Ref slot store (#285)`.

---

## Part 2 (PR 2): `VecB` beside `Vec`

### Task 3: prelude chunk `VecB` and its wiring

**Files:**
- Create: `src/prelude/vec_b.bork`.
- Modify: `src/prelude.rs` (new `Chunk`, names `["VecB"]`, `needs: &[OPTION]`; bump `CHUNKS` length), `src/defs.rs` (nothing lang-item related: `VecB` is an ordinary `opaque class`).
- Test: `src/prelude.rs` tests (neighbours of `buf_index_points_at_the_buf_chunk`).

**Interfaces:**
- Produces: `opaque class VecB<T> { buf: Buf<T>; n: i32 }` with public methods `push(v: T)`, `pop(): T?`, `get(i: i32): T`, `set(i: i32, v: T)`, `len(): i32`, `capacity(): i32`, `clear()`; factory `fun vecBWithCapacity<T>(n: i32): VecB<T>` (aborts when `n < 0`). Growth constants `const VEC_FIRST_CAPACITY: i32 = 4`.

- [ ] **Step 1: Write failing tests:** `a_program_not_naming_vecb_does_not_pull_the_chunk`, `vecb_chunk_is_pulled_by_its_name`.
- [ ] **Step 2: Run** `cargo test --lib prelude`; expect FAIL.
- [ ] **Step 3: Implement** the chunk and methods in Bork (`unsafe` blocks over `slotLoad`/`slotStore`/`bufGrow`, as `stack.bork`); `pop` on empty returns `None`; `clear` sets `n = 0` and releases slots (element glue from Task 2 covers drop; `clear` must release live `Ref`s now, by `slotStore`-overwrite semantics or a `slotRelease` primitive added in Task 2 if overwrite is not enough).
- [ ] **Step 4: Run** the same; expect PASS.
- [ ] **Step 5: Commit** `feat: VecB prelude class on Buf (#285)`.

### Task 4: corpus parity for methods

**Files:**
- Create: `programs/build_b/vec/` is NOT used; instead add `tests/vec_parity.rs` that, for every `programs/build/vec/*.bork` that uses only methods (no `v[i]`, no `&v[i]`, no `.iter()`), rewrites the source `Vec` -> `VecB` and `Vec<` -> `VecB<` in memory and requires the same `// exit:`/output as the original.
- Test: `tests/vec_parity.rs`.

**Interfaces:**
- Produces: `fn parity_sources() -> Vec<(PathBuf, String)>` listing the eligible programs; grows in Tasks 5-8 as constructs are ported.

- [ ] **Step 1: Write the harness** and a failing run (all eligible fail until `VecB(n)` and generic calls work).
- [ ] **Step 2: Run** `cargo test --features codegen --test vec_parity`; expect FAIL.
- [ ] **Step 3: Make it pass:** typeck special case in `src/typeck/expr/vec.rs` `check_vec_new` also accepts `VecB::<T>(n)` and lowers to a call of `vecBWithCapacity` (`n` omitted -> 0); fix whatever the corpus shows (negative index aborts, `pop` empty, `capacity`).
- [ ] **Step 4: Run** the same; expect PASS for all method-only programs including `negative_capacity_aborts`, `with_capacity`, `grow_past_page`, `pop_set_clear`, `strings`.
- [ ] **Step 5: Commit** `test: VecB parity with Vec on method-only programs (#285)`.

### Task 5: indexing `v[i]` / `v[i] = x`

**Files:**
- Modify: `src/mir/build/place.rs:93` (index on a `VecB` calls `VecB.get`/`set` instead of `VecOp`), `src/typeck/expr/` index checking, `src/mir/build/place.rs` Vec index-assign path (~line 207).
- Test: `tests/vec_parity.rs` (add `index_from_element`, `index_through_field`, `vec_field_repeated`, `assign_concat_through_borrowed_vec`), `src/typeck/tests/`.

**Interfaces:**
- Consumes: `VecB.get(i: i32): T`, `VecB.set(i: i32, v: T)` from Task 3.

- [ ] **Step 1: Write failing tests:** extend parity list; add typeck test that `v[i] = x` on a `val` Vec is rejected like `push_on_val.bork`.
- [ ] **Step 2: Run;** expect FAIL. **Step 3: Implement** by desugaring in MIR build; keep the existing borrow/closure arena handling (`place.rs:151`) by routing through the same helper. **Step 4: Run;** PASS. **Step 5: Commit** `feat: index VecB (#285)`.

### Task 6: move/promote across arenas

**Files:**
- Modify: `src/mir/build/place.rs:232` (`copy into arena` for `VecB`), `src/codegen/llvm/glue.rs`/`src/codegen/llvm/buf.rs` (a `buf_promote(arena, buf, elem)` that allocates in the destination and copies `cap` slots with element glue: bytes for scalars/`String`, retain for `Ref`, environment re-allocation for closures as today in `closure_elems_promote*.bork`).
- Test: parity list gains `move_to_outer`, `move_vec_field`, `ref_promote`, `ref_promote_strong`, `ref_counts_across_arenas`, `return_vec_of_linked_refs`, all `closure_elems_promote*`.

**Interfaces:**
- Produces: `buf_promote` in `src/codegen/llvm/buf.rs`; `CopyInto` of a class containing a `Buf` field calls it.

- [ ] **Step 1:** add the programs to parity plus `programs/build/buf/stack_promote_outlives_inner.bork` (inner arena dies, outer reads all elements). **Step 2:** run; FAIL (dangling or wrong release counts). **Step 3:** implement. **Step 4:** run; PASS. **Step 5:** commit `feat: promote Buf contents with their arena (#285)`.

### Task 7: `Vec<Ref<C>>` and closure elements parity

- [ ] **Step 1:** add all remaining `ref_*`, `closure_elems_*` programs to the parity list; run; expect FAIL where release counts or escape plan differ.
- [ ] **Step 2:** fix in `src/mir/plan.rs` and `src/mir/escape.rs` the rules that today special-case `VecOp` for `Ref` element writes: derive them from ordinary method calls and `Buf` slot stores (a push of a `Ref` into `VecB` extends the `Ref`'s required region to the Vec's arena).
- [ ] **Step 3:** run `cargo test --features codegen --test vec_parity --test mir_escape --test mir_plan`; PASS. **Step 4: Commit** `feat: escape rules for Ref elements without VecOp (#285)`.

### Task 8: iterators and `&v[i]` views

**Files:**
- Modify: `src/prelude/iter.bork` (`VecIter<T>` over `VecB<T>` as well; add a `needs`), `src/mir/borrows.rs` and typeck borrow code for `&v[i]` on `VecB`, `src/mir/build/buf.rs` (new primitive `slotAddr(buf: &Buf<T>, i: i32): ...` or the minimum needed to give a borrow of a slot; its exact surface is decided in the first step).
- Test: all `programs/build/vec/element_*`, `programs/build/iter/*` rewritten to `VecB` through the parity harness; all `programs/check_fail/vec/element_*`, `borrow_element_*` through a check_fail parity run.

- [ ] **Step 1:** write the parity cases and a one-paragraph design note in this plan for how an element view locks the `VecB` (reuse of #282 lock on the receiver). **Step 2:** run; FAIL. **Step 3:** implement. **Step 4:** run; PASS for build and check_fail pairs. **Step 5: Commit** `feat: element views and iterators over VecB (#285)`.

### Task 9: debug reader and docs for the new layout

- [ ] **Step 1:** per Task 1 findings, update `tools/bork-debug` (and `crates/bork_runtime/src/arena/debug` if it decodes `Vec` headers) so a `VecB`/`Vec` class instance prints elements; test in `tests/debug_build.rs`. **Step 2:** run; PASS. **Step 3: Commit** `feat: bork-debug reads class-based Vec (#285)`. Open PR 2.

---

## Part 3 (PR 3): switch the name, delete the intrinsic

### Task 10: rename and delete

**Files:**
- Delete: `src/prelude/vec.bork`, `src/codegen/llvm/vec/`, `src/typeck/expr/vec.rs` (keep `Vec(n)` special case, moved to the class path), `VecOp` in `src/hir/mod.rs`, `StmtKind::VecOp`, `AllocKind::Vec`, `DropGlue::Vec` leftovers, `LangItem::Vec` in `src/defs.rs`, `vec_methods`, `bork_vec_*` in `crates/bork_runtime`, `vec_elem`/`is_vec`/`vec_of` in `src/ty/query.rs` (replace uses by a class-based element query for `Vec`).
- Modify: `src/prelude/vec_b.bork` -> `src/prelude/vec.bork` with `VecB` renamed `Vec`, `vecBWithCapacity` -> `vecWithCapacity`; `src/prelude.rs`; `tests/vec_parity.rs` becomes unnecessary - delete it; every `programs/**/vec/*` already passes unchanged.
- Test: whole `programs/build/vec`, `programs/check_fail/vec`, `programs/build/iter`, plus `check_fail/vec/user_defined_vec.bork` (a user class named `Vec` still suppresses the chunk).

- [ ] **Step 1:** rename and delete; fix compile errors the compiler lists. **Step 2:** run `cargo build --features codegen`; PASS. **Step 3:** run narrow suites: `--test programs`, `--test mir_golden --test mir_layout_golden --test mono_instances_golden --test memory_plan_golden --test arena_report_golden --test hir_types_golden`; inspect each golden diff, accept only `Vec`-related ones, update the IR test that expected `@bork_vec_drop_refs` to `@bork_buf_drop`. **Step 4:** full suite once, throttled, in background per `AGENTS.md`. **Step 5: Commit** `refactor: Vec is a prelude class; remove compiler Vec (#285)`.

### Task 11: issue bookkeeping (only after the user agrees)

- [ ] Propose a rewritten body of #285 (steps 2-4 replaced by Parts 1-3 here) and a new issue "impl Trait for Vec and Iterator on Vec" (the old step 4, blocked by Part 3); post only after the user approves.

---

## Status after the first execution session (2026-11-26)

Branch `feat/285-vec-in-bork` (on `origin/main` + the #350 commit). Tasks 1-6 done; Task 7 partial; Tasks 8-11 not started.

- Build parity (exit code only, `tests/vec_parity.rs`): all `programs/build/vec` programs that use methods, `v[i]`, `promote`/`move`, `Ref` and closure elements pass on `VecB`, except 3 closure programs (`closure_elems_inner_ref_capture`, `_promote_object_fields`, `_promote_ref_capture`), kept in an `#[ignore]` test. Cause is open: building the closure in the vector's arena trips `MoveEscapesArena`; the "result sink" explanation was not proven (compare `inner`'s decl sink between `Vec` and `VecB`; sema still sees `outer = promote inner`, which the typeck rewrite to `assignFrom` hides from `mir::build::sinks`).
- **check_fail parity was never part of the plan's tests and fails: 24 of 31 `programs/check_fail/vec` programs differ on `VecB`.** The rules live in `typeck/expr/vec.rs` and `Vec<T>` type lowering and do not apply to an ordinary class: `val` receiver mutation (`push_on_val`, `push_through_val_field`), element-type rules (`class_element`, `closure_fnonce_element`, `nullable`), closure-push restrictions (`closure_*_push`), `Vec(n)` arity (`two_arguments`), `user_defined_vec`, `impl_trait`, and all `&v[i]` view / lock programs (Task 8). Our own `vecb/methods.bork` and `return_from_function.bork` push through `val` and encode the wrong behaviour.
- Not yet checked: `// stdout:` lines and `-g` builds in the parity harness; `pop` of a `Ref` (old `Vec` returns the `Ref` itself and moves it out of the slot, `VecB.pop` returns `Some(clone)` and leaves it); Task 9 (`bork-debug` layout).
- Decisions needed from the owner before Task 8/10: (1) how `&v[i]` locks a class-based `Vec`; (2) where the `val`-receiver and element-type rules live once `Vec` is a prelude class.
