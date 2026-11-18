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

**Goals.** Closures are first-class values. A closure expression can stand alone (`val f = { a, b -> a + b }`), be stored in a `val`/`var`, a field, a `Vec` or an enum payload, be passed, and be returned. A `Func`-typed parameter can be called. A generic parameter can be bounded `F: Fn(A) -> R` (and `FnMut`, `FnOnce`). No written lifetimes (RFC 0004 section 2 still holds).

**Non-goals (v1).**
- No recursion through closures (a closure cannot name itself).
- No change to the existing spelling of capture lists: `move (a, b) { ... }`, and an omitted list means "free variables" (`resolve_move_captures`).
- No new arrow-lambda syntax.

**Syntax.** The braces form of the trailing closure also works as an expression: `{ a, b -> body }` and `move (n) { a -> body }`. The parser tells a closure from a plain block by the `->` after the parameter list; the standalone form always has the `->` (`{ -> 1 }` for no parameters), because a bare `move { ... }` is already a `move` block statement. The trailing form `f(x) { a -> ... }` stays as sugar for passing the closure as the last argument.

First-class is split by ownership, as everywhere else in Bork (section 5): a `move` closure owns its captures and can go anywhere a value can. A plain closure borrows its captures, so it is a view (RFC 0004) and stays where views stay.

## 3. Types

**3.1 The closure type.** Each closure expression gets its own type `Closure(id)`. It is created by typeck, never written by users, and prints as `closure#N` in diagnostics. It carries the parameter types, the return type and the capture list (name, type, by-move or by-view). Users name a closure's type through `Func` (3.2) or a generic bound (3.3).

**3.2 Coercion.** `Closure(id)` coerces to `TyData::Func { params, ret }` when the signatures match; that is how `check_trailing_closure` accepts a closure for a `(i32) -> i32` parameter today. `Func` is the written type of any callable value: a parameter, `val`, field, `Vec<(i32) -> i32>` element or return type. It is a fat value `{ fn ptr, env ptr }` at the ABI (section 6.3), so closures of different bodies share one type once coerced.

**3.3 `Fn`, `FnMut`, `FnOnce`.** Prelude traits declared in `src/prelude/fn.bork` as ordinary `trait Fn<Args> { type Output }` (the grammar has no `intrinsic trait`; `intrinsic` only precedes `class` and `fun`). Like `Vec` and `Range` (#276) each gets a `DefTable` field, recorded in `defs.rs` by name from the prelude chunk. The bound syntax is `F: Fn(i32) -> i32`; the parser desugars it to `Fn<(i32) -> i32>`: one argument that is a `Func` type (Bork has no tuple type) and the `Output` taken from its return type. There are no user impls: the solver (`typeck/solve.rs`) has built-in rules.
- `Closure(id)` implements the strongest trait its body allows. **Kind inference:** the body only reads its captures → `Fn`; it assigns to or `&rw`-borrows a capture → `FnMut`; it moves a capture out → `FnOnce`. `Fn ⊂ FnMut ⊂ FnOnce`, so a stronger kind satisfies a weaker bound.
- `Func { params, ret }` implements `Fn` (a non-capturing callable).
- A bound is checked at the call: passing a closure whose kind is `FnMut` to an `Fn` bound is a type error that names the capture that forces the kind.

**3.4 Copy and drop (RFC 0002 section 3).** A closure is never Copy. A `move` closure owns its by-move captures, so it needs drop glue exactly when one of them does, and `Func` values made from it carry the glue through the environment. A plain closure owns nothing and has no drop.

## 4. HIR

The closure body stays in HIR as a node holding the parameters, the body block, `is_move` and the resolved capture list. `HirExprKind::Call` loses `has_trailing_closure` in favour of an optional trailing argument that is that node. `number_nodes`, `rewrite` and `resolve_types` handle it like any block. Sema's `"Closure"` region label is kept unchanged; `RegionSite::Closure` and `region_walk.rs` keep matching by the label prefix, so the arena report and region goldens do not move.

`free_vars` and the `loop_move_ban` check move from the AST to this HIR node, with the same rules: a name counts as captured when it is in the environment, not moved, and not Copy; a bare field captures `this`; moving an outer name inside a loop body is an error, also when the move is in a closure.

## 5. Captures, regions and escape

- **`move (caps)`** moves each non-Copy capture into the closure and copies each Copy one. The source binding is dead afterwards (as for any `move`).
- **Plain closure** captures by view: a read capture is `&x`, an assigned one is `&rw x`. The closure is therefore a view type (RFC 0004 section 3): its borrow set is its captures' owners, and the existing aliasing checks (`check_aliased_borrows`) apply while it is live.
- **Region.** The closure gets the region sema already opens (`open_move` / `open_ordinary`). Captured owners live in enclosing regions.
- **Where each kind may live.**
  - A *plain* closure lives exactly where `&T` lives today: `val` locals, parameters, `this`. It is not a `var`, not returned, not stored in a field or `Vec`, not `move`d out of its region. `mir::escape` rejects it with the diagnostics RFC 0004 section 3.3 uses for views. It dies with the owners it borrows; no outlives check.
  - A *move* closure is an owned value. It may be a `var`, a field, a `Vec` element, an enum payload, a type argument, and the function result. Its environment is allocated in the arena of its destination; returning one uses the same result-sink ABI as returned `String` and `Vec` values (`call_abi.rs`), and `Promote` / `CopyInto` rehome it like any owned value.
  - A `move` closure whose captured value is itself a view (a `&`/`&rw` binding) is a view too and follows the plain rule. A closure never owns a borrow it could outlive.
- **Aliasing.** Two closures that both hold `&rw x` are an error under the existing call-site and view rules; a `move` closure that took `x` leaves `x` dead.
- A *callee* that stores a `Func` parameter must know it is owned. A `Func` parameter whose argument is a plain closure is a view, so storing it is rejected at the callee for the same reason as any view parameter.

## 6. MIR and codegen

**6.1 Aggregate.** `Rvalue::Closure { body: FnId, captures: Vec<Operand> }`. Captured places are operands: by-move captures are moves/copies, by-view captures are `&`/`&rw` references. (RFC 0006 section 3.10 asked for exactly this; the earlier `Rvalue::Aggregate` was removed in #237 because nothing used it.)

**6.2 Body.** The closure body is lowered to its own MIR function whose first parameter is the environment (a pointer to the capture record). It follows the hidden-arena ABI of `codegen/llvm/call_abi.rs`: the closure receives the caller's arena like any callee. The body function is named after its enclosing function and the closure index (`outer$closureN`). It is not a `HirFunction`: the function/root 1:1 invariant of mono and MIR build (`mono.rs`, `mir/build/mod.rs`) stays, and closure bodies are extra MIR bodies built from the enclosing function's builder.

**6.3 Calls.** A call through a `Func` value, or a `F: Fn*` generic parameter, becomes a new terminator `CallClosure { callee: Operand, args, hidden, dest, target }`. It replaces the `IndirectCall` capability. Generic bounds are monomorphized (`mono.rs`), so `F: Fn` becomes a direct call when the argument is a known closure and the indirect form only when it is a `Func` parameter.

**6.4 Layout and drop.** The environment is `{ drop glue ptr, captures... }`. A plain closure's environment lives in the arena of the enclosing region and is freed with it. A move closure's environment is allocated in the destination arena (section 5), and a `RegisterDrop` entry runs the glue stored in the environment, so a `Func` value can be dropped without knowing which closure it came from. A fat `Func` value is `{ code: ptr, env: ptr }`.

**6.5 Capability.** `Capability::Closure` is deleted when lowering lands. `IndirectCall` stays for calls MIR still cannot name (and for non-closure unknown callees).

## 7. Invariants

- INV-C1: a closure that captures a borrow is a view (`TyCtx::contains_borrow` is true for it) and the existing view checks (`ill_formed`, `mir::escape`) reject it in a `var`, field, `Vec`, enum payload, type argument and return type. A closure that captures no borrow is an ordinary owned type and has no such restriction.
- INV-C2: every capture in a closure's list is either moved, copied or viewed; none is both.
- INV-C3: the HIR closure node's capture list equals what the old AST `free_vars` produced for the same source (pinned by moving the existing sema tests, not rewriting them).
- INV-C4: MIR verification checks that a `Closure` rvalue's capture operand count equals its body's environment arity.

## 8. Incremental migration

Each step leaves the tree green.
1. HIR keeps the body (no behaviour change; goldens `hir_types`, `hover` updated). Parser: closure expression `{ a -> ... }` / `move (n) { ... }` (new grammar rule, `tests/parser.rs`).
2. Closure type, `fn.bork` prelude chunk, solver rules; `Func` calls still rejected at MIR.
3. `free_vars` / `loop_move_ban` on HIR; the AST versions deleted.
4. MIR `Rvalue::Closure` + `CallClosure` + verifier; goldens.
5. LLVM lowering (aggregate, body function, indirect call, glue, debug type).
6. Delete `Capability::Closure`; flip `programs/build_fail/methods/trailing_closure.bork` to a build program; docs (`language.md`, `memory-model.md`, `type-system/inventory.md`).
7. Programs and tests for every row of section 9.

## 9. Test matrix

Trailing closure; `move` closure; omitted capture list; `Func` parameter called; `F: Fn`, `F: FnMut`, `F: FnOnce` bounds; kind mismatch (`FnMut` closure to `Fn` bound, `check_fail`); move closure returned, stored in a field and in a `Vec`, called later (build programs); plain closure returned / stored in a field (`check_fail`); closure in a `val` and in a `var`; move of an outer name inside a loop (`check_fail`); closure over a `&rw` capture while the owner is used (`check_fail`).

## 10. Risks

- The `"Closure"` label is the only tie between HIR sites and sema arena nodes. Changing it changes the arena-report goldens; step 1 keeps it.
- A fat `Func` value is a new ABI shape in `call_abi.rs`; debug info (`debug/types.rs`) needs a type for it.
- Kind inference needs the "moves a capture out" fact; sema already tracks moves per binding, but typeck runs first. If the inference cannot be done in typeck, kind is computed by a small HIR pass after type resolution and the bound is checked there. This is the main open point. Until the HIR pass lands, every closure is accepted for all three bounds; the implementation plan marks this as a known gap.
- Owned closures put drop glue behind a pointer in the environment. Every `Func` value pays one indirection to drop; if it matters, a closure with no owning captures can set the glue pointer to null.
- Environments allocated in a destination arena depend on the result-sink ABI handling a value whose size is not known to the caller. `Func` is a fixed-size fat value and the environment is out of line, so this should hold, but it is the first thing step 5 must prove.
- Distinguishing a closure from a block by `->` must not change how existing blocks parse (`parses_trailing_move_closure` and the golden corpus pin this).

## 11. Mapping to issues

#277 covers steps 1-7. Follow-up, not in scope: recursion through closures.
