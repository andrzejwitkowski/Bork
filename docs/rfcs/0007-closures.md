# RFC 0007: Closures as Fn traits, and their native lowering (TS-P8-01e, issue #277)

- **Status:** Proposed
- **Date:** 2026-11-18
- **Issue:** #277 (part of #68)
- **Builds on:** [RFC 0002](0002-ownership-typeck-contract.md) section 3 (Copy and drop), [RFC 0003](0003-borrows-regions-generics.md) (regions, borrows), [RFC 0004](0004-borrow-carrying-types.md) (view types), [RFC 0006](0006-mir.md) section 3.10 (the closures note this RFC answers).
- **Related:** #276 (Vec/Range as prelude intrinsic classes, the pattern used for `Fn*`), #220 (MIR-D).

Design only. No code changes.

## 1. Motivation

Today a closure is only a trailing closure after a call: `p.apply(2) { v -> v + 1 }`, or `move (a, b) { x -> ... }`. Typeck checks it inline against the last parameter's `TyData::Func` (`typeck/expr/call.rs`, `check_trailing_closure`) and then drops it: HIR keeps only `has_trailing_closure`. Sema walks the AST for `free_vars` and `loop_move_ban`. MIR build answers `Unsupported(Capability::Closure)`, and a call through a `Func` value answers `Capability::IndirectCall`. So no program with a closure compiles to a binary, and a generic function cannot say "give me something callable".

This RFC fixes the model so the pieces can be built in order: closure body kept in HIR, a closure type, `Fn`/`FnMut`/`FnOnce` bounds, a MIR aggregate, native codegen.

## 2. Goals, non-goals

**Goals.** Trailing closures (plain and `move`) compile and run. A `Func`-typed parameter can be called. A generic parameter can be bounded `F: Fn(A) -> R` (and `FnMut`, `FnOnce`). No written lifetimes (RFC 0004 section 2 still holds).

**Non-goals (v1).**
- No closure expression outside the trailing position (no `val f = { x -> x }`). The grammar does not have one and this RFC adds none.
- A closure cannot be stored in a `val`/`var`, a field, a `Vec`, or returned. It lives exactly for the call it is passed to.
- No recursion through closures, no closure-typed enum payloads.
- No change to the existing spelling of capture lists: `move (a, b) { ... }`, and an omitted list means "free variables" (`resolve_move_captures`).

The "cannot escape" rule is what lets the capture model stay simple (section 5): a closure value is a view type (RFC 0004) whose lifetime is the call.

## 3. Types

**3.1 The closure type.** Each trailing closure expression gets its own type `Closure(id)`. It is created by typeck, never written by users, and prints as `closure#N` in diagnostics. It carries the parameter types, the return type and the capture list (name, type, by-move or by-view).

**3.2 Coercion.** `Closure(id)` coerces to `TyData::Func { params, ret }` when the signatures match; that is how `check_trailing_closure` accepts a closure for a `(i32) -> i32` parameter today. `Func` stays the type of a *parameter* that receives a callable. It is a fat value `{ fn ptr, env ptr }` at the ABI (section 6.3).

**3.3 `Fn`, `FnMut`, `FnOnce`.** Intrinsic prelude traits, declared in `src/prelude/fn.bork` the way `Vec` and `Range` are (#276): a new `DefTable` field per trait, recorded in `defs.rs` from the `intrinsic` flag. The bound syntax is `F: Fn(i32) -> i32`; it desugars to the trait with an argument tuple and an associated `Output`, exactly like `Iterator`'s `Item`. There are no user impls: the solver (`typeck/solve.rs`) has built-in rules.
- `Closure(id)` implements the strongest trait its body allows. **Kind inference:** the body only reads its captures → `Fn`; it assigns to or `&rw`-borrows a capture → `FnMut`; it moves a capture out → `FnOnce`. `Fn ⊂ FnMut ⊂ FnOnce`, so a stronger kind satisfies a weaker bound.
- `Func { params, ret }` implements `Fn` (a non-capturing callable).
- A bound is checked at the call: passing a closure whose kind is `FnMut` to an `Fn` bound is a type error that names the capture that forces the kind.

**3.4 Copy and drop (RFC 0002 section 3).** A closure is never Copy. It owns its by-move captures, so it needs drop glue exactly when one of them does. A by-view capture adds no drop.

## 4. HIR

The closure body stays in HIR as a node holding the parameters, the body block, `is_move` and the resolved capture list. `HirExprKind::Call` loses `has_trailing_closure` in favour of an optional trailing argument that is that node. `number_nodes`, `rewrite` and `resolve_types` handle it like any block. Sema's `"Closure"` region label is kept unchanged; `RegionSite::Closure` and `region_walk.rs` keep matching by the label prefix, so the arena report and region goldens do not move.

`free_vars` and the `loop_move_ban` check move from the AST to this HIR node, with the same rules: a name counts as captured when it is in the environment, not moved, and not Copy; a bare field captures `this`; moving an outer name inside a loop body is an error, also when the move is in a closure.

## 5. Captures, regions and escape

- **`move (caps)`** moves each non-Copy capture into the closure and copies each Copy one. The source binding is dead afterwards (as for any `move`).
- **Plain closure** captures by view: a read capture is `&x`, an assigned one is `&rw x`. The closure is therefore a view type (RFC 0004 section 3): its borrow set is its captures' owners, and the existing aliasing checks (`check_aliased_borrows`) apply while it is live.
- **Region.** The closure gets the region sema already opens (`open_move` / `open_ordinary`). Captured owners live in enclosing regions, so a closure cannot outlive them, and the non-escape rule of section 2 makes that a structural fact: no new outlives check.
- **Escape.** A closure cannot be returned, stored, assigned to an outer binding or `move`d out of its region. `mir::escape` rejects it with the diagnostics RFC 0004 section 3.3 uses for views.
- A *callee* that stores its `Func` parameter anywhere is rejected for the same reason: the parameter is a view.

## 6. MIR and codegen

**6.1 Aggregate.** `Rvalue::Closure { body: FnId, captures: Vec<Operand> }`. Captured places are operands: by-move captures are moves/copies, by-view captures are `&`/`&rw` references. (RFC 0006 section 3.10 asked for exactly this; the earlier `Rvalue::Aggregate` was removed in #237 because nothing used it.)

**6.2 Body.** The closure body is lowered to its own MIR function whose first parameter is the environment (a pointer to the capture record). It follows the hidden-arena ABI of `codegen/llvm/call_abi.rs`: the closure receives the caller's arena like any callee. The body function is named after its enclosing function and the closure index.

**6.3 Calls.** A call through a `Func` value, or a `F: Fn*` generic parameter, becomes a new terminator `CallClosure { callee: Operand, args, hidden, dest, target }`. It replaces the `IndirectCall` capability. Generic bounds are monomorphized (`mono.rs`), so `F: Fn` becomes a direct call when the argument is a known closure and the indirect form only when it is a `Func` parameter.

**6.4 Layout and drop.** The environment is `{ captures... }` in the arena of the call (stack-like, freed with the region). A fat `Func` value is `{ code: ptr, env: ptr }`. Drop glue for by-move captures runs when the closure region ends (`RegisterDrop` as for other region-owned values).

**6.5 Capability.** `Capability::Closure` is deleted when lowering lands. `IndirectCall` stays for calls MIR still cannot name (and for non-closure unknown callees).

## 7. Invariants

- INV-C1: a closure type never appears in a field, `Vec`, enum payload, `val`/`var` type, return type or type argument (checked in typeck, by `contains_borrow`-style `ill_formed`).
- INV-C2: every capture in a closure's list is either moved, copied or viewed; none is both.
- INV-C3: the HIR closure node's capture list equals what the old AST `free_vars` produced for the same source (pinned by moving the existing sema tests, not rewriting them).
- INV-C4: MIR verification checks that a `Closure` rvalue's capture operand count equals its body's environment arity.

## 8. Incremental migration

Each step leaves the tree green.
1. HIR keeps the body (no behaviour change; goldens `hir_types`, `hover` updated).
2. Closure type, `fn.bork` prelude chunk, solver rules; `Func` calls still rejected at MIR.
3. `free_vars` / `loop_move_ban` on HIR; the AST versions deleted.
4. MIR `Rvalue::Closure` + `CallClosure` + verifier; goldens.
5. LLVM lowering (aggregate, body function, indirect call, glue, debug type).
6. Delete `Capability::Closure`; flip `programs/build_fail/methods/trailing_closure.bork` to a build program; docs (`language.md`, `memory-model.md`, `type-system/inventory.md`).
7. Programs and tests for every row of section 9.

## 9. Test matrix

Trailing closure; `move` closure; omitted capture list; `Func` parameter called; `F: Fn`, `F: FnMut`, `F: FnOnce` bounds; kind mismatch (`FnMut` closure to `Fn` bound, `check_fail`); closure returned / stored (`check_fail`); move of an outer name inside a loop (`check_fail`); closure over a `&rw` capture while the owner is used (`check_fail`).

## 10. Risks

- The `"Closure"` label is the only tie between HIR sites and sema arena nodes. Changing it changes the arena-report goldens; step 1 keeps it.
- A fat `Func` value is a new ABI shape in `call_abi.rs`; debug info (`debug/types.rs`) needs a type for it.
- Kind inference needs the "moves a capture out" fact; sema already tracks moves per binding, but typeck runs first. If the inference cannot be done in typeck, kind is computed by a small HIR pass after type resolution and the bound is checked there. This is the main open point.
- v1 forbids closures in variables. If that proves too limiting, relaxing it needs the escape rules of RFC 0005-style returned borrows; that is a separate RFC.

## 11. Mapping to issues

#277 covers steps 1-7. Follow-ups, not in scope: a closure expression outside the trailing position, closure values in variables, returned closures.
