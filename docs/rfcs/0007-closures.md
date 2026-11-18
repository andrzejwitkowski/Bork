# RFC 0007: Closures as Fn traits, and their native lowering (TS-P8-01e, issue #277)

- **Status:** Partly implemented (section 8 says which steps).
- **Date:** 2026-11-18 (updated after implementation, 2026-11-19)
- **Issue:** #277 (part of #68)
- **Builds on:** [RFC 0002](0002-ownership-typeck-contract.md) section 3 (Copy and drop), [RFC 0003](0003-borrows-regions-generics.md) (regions, borrows), [RFC 0004](0004-borrow-carrying-types.md) (view types), [RFC 0006](0006-mir.md) section 3.10 (the closures note this RFC answers).
- **Related:** #276 (Vec/Range as prelude intrinsic classes, the pattern used for `Fn*`), #220 (MIR-D).

## 1. Motivation

Before this RFC a closure was only a trailing closure after a call: `p.apply(2) { v -> v + 1 }`, or `move (a, b) { x -> ... }`. Typeck checked it inline against the last parameter's `TyData::Func` and then dropped it: HIR kept only `has_trailing_closure`. MIR build answered `Unsupported(Capability::Closure)` and a call through a `Func` value answered `Capability::IndirectCall`. So no program with a closure compiled to a binary, and a generic function could not say "give me something callable".

This RFC fixes the model so the pieces could be built in order: closure body kept in HIR, a closure type, `Fn`/`FnMut`/`FnOnce` bounds, native codegen.

## 2. Goals, non-goals

**Goals.** Closures are first-class values. A closure expression can stand alone (`val f = { a, b -> a + b }`), be stored in a `val`/`var`, be passed, and be returned. A `Func`-typed parameter can be called. A generic parameter can be bounded `F: Fn(A) -> R` (and `FnMut`, `FnOnce`). No written lifetimes (RFC 0004 section 2 still holds).

**Non-goals (v1).**
- No recursion through closures (a closure cannot name itself).
- No change to the existing spelling of capture lists: `move (a, b) { ... }`, and an omitted list means "free variables".
- No new arrow-lambda syntax.

**Syntax.** The braces form of the trailing closure also works as an expression: `{ a, b -> body }` and `move (n) { a -> body }`. The parser tells a closure from a plain block by the `->` after the parameter list; the standalone form always has the `->` (`{ -> 1 }` for no parameters), because a bare `move { ... }` is already a `move` block statement. The trailing form `f(x) { a -> ... }` stays as sugar for passing the closure as the last argument.

First-class is split by ownership, as everywhere else in Bork (section 5): a `move` closure owns its captures and can go anywhere a value can. A plain closure borrows its captures, so it is a view (RFC 0004) and stays where views stay.

## 3. Types

**3.1 The closure type.** Each closure expression gets its own type `TyData::Closure { id, params, ret, captures }`. It is created by typeck, never written by users, and prints as `closure#N(params)->ret`. `captures` holds the types of the outer bindings the body uses, so substitution in a generic body reaches them. The closure's kind (`Fn`, `FnMut`, `FnOnce`) and whether it is `move` live in a table on `TyCtx` indexed by `id`. Users name a closure's type through `Func` (3.2) or a generic bound (3.3).

**3.2 Coercion.** `Closure` unifies with `TyData::Func { params, ret }` when the signatures match. `Func` is the written type of any callable value: a parameter, `val`, or return type. At the ABI both are one pointer to the closure's environment (6.3); there is no separate fat value.

**3.3 `Fn`, `FnMut`, `FnOnce`.** Prelude traits declared in `src/prelude/fn.bork` as `trait Fn<A> {}` (the grammar has no `intrinsic trait`; `intrinsic` only precedes `class` and `fun`), loaded by name like `vec.bork`. `TyCtx` finds them by name. The bound syntax is `F: Fn(i32) -> i32`; the parser desugars it to `Fn<(i32) -> i32>`: one argument that is a `Func` type (Bork has no tuple type). There is no `Output` projection; the return type is part of that argument. There are no user impls: the solver (`typeck/solve.rs`) has built-in rules.
- A closure or `Func` type implements an `Fn*` trait for its own signature, if its kind allows it. **Kind inference** (`typeck/captures.rs`, run on the closure's HIR body): the body only reads its captures → `Fn`; it assigns to a capture, or calls a mutating `Vec` method on it → `FnMut`; it moves a capture out → `FnOnce`. `Fn ⊂ FnMut ⊂ FnOnce`: a closure satisfies a bound of its own kind or a weaker one, and the diagnostic names why a stronger closure is rejected.
- A plain `Func` value counts as `Fn`.
- A parameter `f: F` with `F: Fn(A) -> R` is callable, also with a trailing closure; the signature is read from the bound.
- Matching is by trait name. A program's own trait called `Fn` would get the built-in rules (the prelude chunk is skipped then); give it a prelude-origin check if that ever happens.

**3.4 Copy and drop (RFC 0002 section 3).** A closure is never Copy. A `move` closure owns its captures: it needs drop exactly when one of them does. A plain closure owns nothing. There is no drop-glue pointer in the environment (an earlier draft had one): an owned capture is copied into the environment's arena, which frees it with the arena, and what needs more than that (6.5) is not lowered yet.

## 4. HIR

The closure body stays in HIR as `HirExprKind::Closure { params, body, is_move, captures, kind }`; `captures` and `kind` are computed right after the body is checked (`typeck/captures.rs`, a scan of the HIR). `Call.has_trailing_closure` became `Call.trailing: Option<Box<HirExpr>>`. `number_nodes`, `rewrite` and `resolve_types` handle the node. Sema's `"Closure"` region label is unchanged; `RegionSite::Closure` and `region_walk.rs` keep matching by the label prefix, so the arena report goldens did not move.

Sema still computes free variables and the loop-move ban on the AST (`sema/free_vars.rs`, `loop_move_ban`): that code needs sema's own binding state (moved, Copy), which HIR does not carry. The HIR scan is a second, independent computation used for the type's captures and the kind. Moving sema onto HIR is not part of this RFC.

shortcut: the HIR scan is flow-insensitive. A name declared anywhere inside the closure shadows an outer one for the whole body.

After monomorphization the node is gone: `mono/lift.rs` turns each closure into a function (6.2) and the expression into `HirExprKind::MakeClosure { function, captures, kind, is_move }`. Typeck never produces `MakeClosure`.

## 5. Captures, regions and escape

- **`move (caps)`** moves each non-Copy capture into the closure and copies each Copy one. The source binding is dead afterwards (as for any `move`).
- **Plain closure** captures by view. The closure type is a view (`stores_borrow` is true when it has any capture), the same as a `&T`: it lives in `val` locals and parameters, and returning one is rejected with "returning a reference is not supported" (`return_rules::returns_reference`). A plain closure without captures stores nothing and may be returned.
- **Move closure.** `stores_borrow` is true only if one of its captures stores a borrow, so it is an ordinary owned value. Its environment goes to the arena of its destination: the result sink when the function returns a closure, else the current region.
- **Region.** The closure gets the region sema already opens (`open_move` / `open_ordinary`).
- **Type arguments.** A closure passed for a type parameter (`F: Fn`) is exempt from "a type argument cannot store a borrow" (`first_borrow_arg`): it lives for the call. Returning such a generic value is not checked yet.
- **Not checked yet.** Storing a plain closure through a `Func`-typed field or `var` is not rejected; only returning it is. Storing any closure in a field or `Vec` is untested.

## 6. MIR and codegen

**6.1 Value.** `Rvalue::Closure { function, captures: Vec<Operand>, by_ref, arena }`. `arena` holds the environment. `TermKind::CallClosure { callee, args, hidden, dest, target }` calls through a closure value and replaces the `IndirectCall` rejection for closures; `IndirectCall` stays for unknown non-closure callees.

**6.2 Body.** Mono lifts every closure into an ordinary function `outer$closureN(captures..., params...)` (`mono/lift.rs`), after specialisation, so a closure inside a generic function is lifted once per instance. Each lifted closure gets its own report root (the sema `"Closure"` node is detached from its parent and relabelled), so the function/root 1:1 invariant of mono and MIR build holds and neither MIR build nor codegen needed a second kind of body. A trailing closure becomes the last argument of its call. The lifted function records `env_params` (how many leading parameters are captures) and `env_mode`:
- `ByValue`: a copy made when the closure is built, reloaded on each call (`Fn`).
- `ByAddress`: the environment holds the outer variable's address (a plain `FnMut` closure), so assignments reach the variable.
- `InPlace`: the environment owns the value and the body updates it there (a `move` `FnMut` closure).

The environment parameter replaces the captures in the LLVM signature; the prologue loads them (or takes their address) from it.

**6.3 Layout.** A closure value is one pointer to the environment `{ code pointer, captures... }`. With no captures it is a constant global, so it may outlive any region. A call loads the code pointer from the environment and passes the environment as the first non-hidden argument. Functions that return a `Func` use the result-sink ABI (`uses_arena_storage` is true for function types).

**6.4 Calls.** A call through a `Func` or closure value becomes `CallClosure`. A generic `F: Fn` is monomorphized per closure type (`apply<closure#0(i32)->i32>`), so it is a call through the environment, not a direct call.

**6.5 What lowers.** `Capability::Closure` still exists, narrowed: MIR build rejects a closure that is `FnOnce` or whose `move` captures are not trivially copyable or a `String` or an array of copyable elements. Rejected today:
- objects and `Ref`: a class value captured by a returned `move` closure fails the ownership check ("freed before the value that stores it"), before MIR; a `Ref` needs `RefOp::Store` and a registered drop;
- `Vec`, nullables: need registration in the environment's arena;
- `FnOnce`: running it moves a capture out of the environment, so the environment must not drop it again.

All of these come down to registering drops for the captures in the environment's arena (`RegisterDrop`, as `store_owned` does for object fields).

## 7. Invariants

- INV-C1: a plain closure with a capture is a view (`stores_borrow`), and the view checks reject returning it. A `move` closure is a view only if a capture is.
- INV-C2: a lifted function's capture parameters equal the closure's `captures` (same order), and the `MakeClosure` operand count equals `env_params`; codegen's `supported` check rejects a mismatch.
- INV-C3 (changed): the closure type's captures equal what the HIR scan found; the old AST `free_vars` is independent and unchanged.
- INV-C4: a lifted closure function has the same report root as its closure node (checked by mono: closures and report nodes must match in number).

## 8. Implementation status

| Step | State |
|---|---|
| Parser: closure expression | done |
| HIR keeps the body | done |
| `Closure` type, coercion to `Func` | done |
| `Fn`/`FnMut`/`FnOnce`, bound syntax, kind inference | done |
| Captures on the type; view rule; return rejected | done |
| Mono lifting | done |
| MIR `Rvalue::Closure`, `CallClosure`; LLVM lowering | done |
| `FnMut` (by address and in place), returned `move` closures, `String` and copy-array captures | done |
| Objects, `Ref`, `Vec`, nullable captures; `FnOnce` | open (6.5) |
| Delete `Capability::Closure` | open: needs the row above |
| Closure in a field or `Vec` | open, untested |
| Sema `free_vars` on HIR | dropped (section 4) |
| `language.md`, `memory-model.md`, `type-system/inventory.md` | open |

## 9. Test matrix

Programs under `programs/build/closures/`: trailing closure with a capture, closure in a `val`, `F: Fn` bound, `FnMut` counter and bound, `move` closure returned (adder), `move` counter (own state), `String` capture returned, a returned `String` capture that outlives its frame. `programs/build_fail/move_closure_with_captures.bork` is the closure that still hits `Capability::Closure`. Unit tests: `src/typeck/tests/closures.rs` (signature from annotation, argument, bound, kind mismatch, captures, view returned), `src/mono/tests.rs` (lifting, nesting), `tests/parser.rs`.

Still to add: `FnOnce`, closure in a field and in a `Vec`, move of an outer name inside a loop (`check_fail`), closure over a `&rw` capture while the owner is used.

## 10. Risks

- The `"Closure"` label is the only tie between HIR sites and sema arena nodes. Lifting depends on the report's closure nodes appearing in the same post-order as the HIR closures; a mismatch is an internal error.
- Capture by address (`FnMut`) points into the caller's frame. It is safe because a plain closure with captures cannot be returned or stored beyond its region, but the escape check covers only returning today (section 5).
- `uses_arena_storage` is true for every function type, so any function returning a closure takes a result sink, whether or not the closure captures anything.
- A `move` closure in a function that returns a closure gets its environment in the caller's arena even if it is not the returned value. This only costs memory.
- Distinguishing a closure from a block by `->` must not change how existing blocks parse (`braces_without_arrow_stay_blocks`, the golden corpus).

## 11. Mapping to issues

#277 covers the table in section 8. Follow-ups: owned captures with drop registration and `FnOnce` (then removing `Capability::Closure`), closures in fields and `Vec`s, recursion through closures.
