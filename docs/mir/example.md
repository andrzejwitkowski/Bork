# From Bork source to MIR: a worked example

This page takes one small Bork program and shows what `bork build --emit-mir` prints for it,
line by line: what each MIR construct is, and which earlier compiler stage produced the
information behind it. The program is `docs/mir/example.bork`; the output below is the real
output of this branch (`cargo run -- build --emit-mir docs/mir/example.bork`).

Background: RFC 0006 (`docs/rfcs/0006-mir.md`). MIR is built from *monomorphized HIR* by
`src/mir/build/`, printed by `src/mir/print.rs`, checked by `src/mir/verify.rs`. Nothing
consumes it yet; LLVM codegen still works from HIR.

## 1. The program

```
class Counter {
    n: i32
}

fun bump(c: &rw Counter, by: i32): i32 {
    c.n = c.n + by
    return c.n
}

fun main(): i32 {
    var c = Counter(0)
    var total = 0
    for (i in 0..3) {
        val label = "step"
        if (i > 0) {
            total = total + bump(&rw c, i)
        }
    }
    println(total)
    return total
}
```

It has an object (`Counter`) that is passed to a function, a loop, a branch, a string literal
declared inside the loop, and a builtin call (`println`). That is enough to show regions,
arenas, control flow and calls.

## 2. What the compiler knows before MIR exists

MIR does not discover facts; it copies them from earlier stages. Three inputs matter here.

| Input | Produced by | What MIR takes from it |
|---|---|---|
| Typed, monomorphic HIR (`Mono.hir`) | parser, typeck, `mono.rs` | The statements and expressions, with a type on every expression. |
| `ArenaReport` (`Mono.report`) | `sema` (ownership analysis) | The *region tree*: one node per function, block, loop body, `if` branch, `when` arm, each with a `codegen_push` flag. |
| `MemoryPlan` (`Mono.memory_plan`) | `src/memory.rs` | For each object/enum construction: arena class (Lexical or Dynamic) and lifetime domain. |

You can see the region tree for this program with `bork --dump-arenas docs/mir/example.bork`:

```
fun main
    ├── c [Local]
    ├── total [Local]
    └── ForLoop (i)
        ├── i [Local]
        ├── label [Local]
        └── IfThen
```

That tree is exactly the `region` lines you will see at the top of `main`'s MIR.

## 3. The MIR of `main`

```
fn main (parent=false, sink=false) {
  let _0: i32 [Return]
  let _1: unit [Hidden(DebugScope)]
  let _2: unit [Hidden(DynamicArena)]
  let _3: Counter [Temp]
  let _4: Counter [Var] c
  let _5: i32 [Var] total
  let _6: i32 [Temp]
  let _7: i32 [Var] i
  let _8: unit [Hidden(DebugScope)]
  let _9: bool [Temp]
  let _10: unit [Hidden(RegionArena)]
  let _11: unit [Hidden(DebugScope)]
  let _12: String [Val] label
  let _13: bool [Temp]
  let _14: unit [Hidden(DebugScope)]
  let _15: &rw Counter [Temp]
  let _16: i32 [Temp]
  let _17: i32 [Temp]
  region r0 Block
  region r1 For in r0
  region r2 IfThen in r1
  bb0:
    storage_live _1
    region_enter r0
    _2 = arena_create_dynamic parent=root scoped=false
    register_strong_root current <- _2
    _3 = alloc Object { class: "Counter" } in fresh2  // label "Counter": Counter
    _3.0 = 0  // label "Counter.n": i32 decl owner=fresh2
    _4 = copy _3  // label "c": Counter decl
    _5 = 0  // label "total": i32 decl
    _6 = 3
    _7 = 0  // label "i": i32 decl
    region_enter r1
    storage_live _8
    goto bb1
  bb1:
    _9 = lt(copy _7, copy _6)
    switch copy _9 [ 1 -> bb2, else -> bb4]
  bb2:
    storage_live _11
    _12 = "step"  // label "label": String decl
    _13 = gt(copy _7, 0)
    switch copy _13 [ 1 -> bb5, else -> bb6]
  bb3:
    _7 = add(copy _7, 1)
    _10 = region_reset r1
    goto bb1
  bb4:
    storage_dead _8
    region_exit r1
    intrinsic PrintlnInt(copy _5)
    _0 = copy _5
    storage_dead _1
    region_exit r0
    return
  bb5:
    storage_live _14
    _15 = borrow_rw _4
    _16 = call bump(copy _15, copy _7) +current -> bb7
  bb6:
    storage_dead _11
    goto bb3
  bb7:
    _17 = add(copy _5, copy _16)
    _5 = copy _17
    storage_dead _14
    goto bb6
}
```

### 3.1 The header: `fn main (parent=false, sink=false)`

These are the function's *hidden parameters* (`CallAbi`). A function gets a `caller_parent`
arena when it is not `main`, and a `result_sink` arena when it returns something allocated in
an arena (an object, a `String`, a `Vec`, a `Ref`). `main` has neither. `bump` (below) has
`parent=true` because it is an ordinary function. The rule is `mir::build::call_abi`, copied
from `codegen/llvm/call_abi.rs`.

### 3.2 The locals table

Every value lives in a numbered local `_N` with a type and a *kind*.

| Local | Kind | Where it came from |
|---|---|---|
| `_0` | `Return` | The function's return value. Always local 0. |
| `_3 c`, `_4 total`, `_6 i` | `Var` | `var` declarations and the loop variable. |
| `_9 label` | `Val` | A `val` declaration. |
| `_2`, `_5`, `_7`, `_10`..`_13` | `Temp` | Intermediate results the builder invented: an object under construction, the loop bound `3`, comparison results, a borrow, a call result. Source code has no name for them. |
| `_1` | `Hidden(DynamicArena)` | The arena made for `Counter` (see 3.4). |
| `_8` | `Hidden(RegionArena)` | The arena token of region `r1`, reassigned on each loop iteration (see 3.6). |

### 3.3 The region lines

```
region r0 Block
region r1 For in r0
region r2 IfThen in r1
```

A *region* is a stretch of code that may own an arena. These three lines are the `ArenaReport`
tree flattened: `r0` is the function body, `r1` the `for` body (child of `r0`), `r2` the
`then` branch (child of `r1`). Names `Block`, `For`, `IfThen` are `RegionSite` values from
`src/region_walk.rs`.

The builder does not guess which blocks open regions; it walks the HIR in the same order as
`region_walk` and takes the next child of the report each time (`FnBuilder::take_child`).
That is why `r2` exists but never appears as a `region_enter`: the report says its
`codegen_push` is false, because the `then` branch allocates nothing.

### 3.4 Creating the object: `bb0`

```
region_enter r0
_1 = arena_create_dynamic parent=root scoped=false
register_strong_root current <- _1
_2 = alloc Object { class: "Counter" } in fresh1
_2.0 = 0
_3 = copy _2  // label "c": Counter decl
```

Source: `var c = Counter(0)`. Step by step:

- `region_enter r0` opens the function's arena (the report says the root always pushes).
- `arena_create_dynamic ... scoped=false` and `register_strong_root`: the `MemoryPlan`
  classified this `Counter` as **Dynamic with Program lifetime**. The reason is in
  `src/memory.rs`: calls have no escape summaries, so a record passed to a call is assumed to
  live for the whole program. A dynamic arena is a separate arena that is kept alive by a
  strong reference from its owner (`current`), not tied to the region it was made in.
- `_2 = alloc Object ... in fresh1`: the allocation names its arena explicitly. `fresh1` is
  "the arena created by local `_1`". This is the key difference from LLVM codegen, which tracks
  the target arena in hidden emitter state (`alloc_sink`); in MIR it is an operand of the op.
- `_2.0 = 0`: store the constructor argument into field 0.
- `_3 = copy _2  // label ...`: bind the name `c`. The `// label` part is a `DebugLabel`
  (name, type, "this is a declaration"). It is what later lets the runtime's debug registry
  name this allocation.

### 3.5 The loop: `bb1`, `bb2`, `bb3`, `bb4`

Source: `for (i in 0..3) { ... }`.

```
_5 = 3          // upper bound, evaluated once
_6 = 0          // i = 0
region_enter r1 // the loop body region is opened before the header
goto bb1
bb1: _7 = lt(copy _6, copy _5)
     switch copy _7 [ 1 -> bb2, else -> bb4]
```

A structured `for` becomes four blocks: a *header* (`bb1`, tests `i < 3`), the *body*
(`bb2`...), a *latch* (`bb3`, increments `i`, goes back), and an *exit* (`bb4`). `switch`
is the terminator that ends a block: value `1` (true) goes to the body, anything else to the
exit. `break` would be a `goto bb4` and `continue` a `goto bb3`, each preceded by the
`region_exit`s for any regions they leave.

### 3.6 The back edge: `bb3`

```
_6 = add(copy _6, 1)
_8 = region_reset r1
goto bb1
```

`_6 = add(...)` is `i = i + 1`. `region_reset r1` is the important one. At run time,
`bork_arena_reset` destroys the loop-body arena (freeing `label` and anything else allocated
in the iteration) and returns a **new** arena token. So the region's arena is a *local*
(`_8`, kind `RegionArena`) that the reset reassigns. This was a finding from reading the
runtime and is recorded in RFC 0006 section 3.2.2.

### 3.7 The branch and the call: `bb2`, `bb5`, `bb7`

Source: `if (i > 0) { total = total + bump(&rw c, i) }`.

```
bb2: _9 = "step"                   // val label = "step"
     _10 = gt(copy _6, 0)
     switch copy _10 [ 1 -> bb5, else -> bb6]
bb5: _11 = borrow_rw _3            // &rw c
     _12 = call bump(copy _11, copy _6) +current -> bb7
bb7: _13 = add(copy _4, copy _12)
     _4 = copy _13                 // total = ...
     goto bb6
bb6: goto bb3
```

- `borrow_rw _3` is the `&rw c` borrow. It produces a reference-typed temp (`&rw Counter`).
  Making borrows explicit operations is what lets later passes (#219) detect conflicting
  borrows of one place.
- `call bump(...) +current -> bb7` is a *terminator*: a call ends its block and names the
  block to continue in. `+current` is the hidden argument `bump` asks for (its
  `caller_parent`): the caller's current arena.
- `if` without `else` has no separate else block; the false edge goes straight to the join
  `bb6`.

Where is `r2`? It is declared but has no `region_enter`: the `then` branch allocates nothing,
so the report's `codegen_push` is false and the builder honours that. The `then` code simply
runs inside `r1`.

### 3.8 The exit: `bb4`

```
region_exit r1
intrinsic PrintlnInt(copy _4)
_0 = copy _4
region_exit r0
return
```

`region_exit r1` pops the loop arena. `println(total)` becomes an `intrinsic`: builtins
(`print`, `println`, `concat`, `strongCount`...) are matched by name and are not ordinary
calls. `return total` becomes `_0 = copy _4` (store into the return local), then the exit of
every region still open (`r0`), then `return`. A `return` in the middle of a loop would emit
`region_exit r1` first, then `region_exit r0`; that is what replaces `unwind_to_depth` in the
LLVM emitter.

## 4. The MIR of `bump`

```
fn bump (parent=true, sink=false) {
  let _0: i32 [Return]
  let _1: &rw Counter [Arg] c
  let _2: i32 [Arg] by
  let _3: unit [Hidden(DebugScope)]
  let _4: i32 [Temp]
  region r0 Block
  bb0:
    storage_live _3
    region_enter r0
    _4 = add(copy _1.0, copy _2)
    _1.0 = copy _4  // label "Counter.n": i32 decl owner=object_arena(_1)
    _0 = copy _1.0
    storage_dead _3
    region_exit r0
    return
}
```

- Parameters are `Arg` locals, numbered after the return local.
- `_1.0` is a *place*: local `_1`, projection `.0` (field 0 of `Counter`, i.e. `n`).
  Source `c.n` is just this place; there is no separate "load field" operation.
- `c.n = c.n + by` becomes compute into a temp (`_3`), then store to the place `_1.0`.
  The right side is evaluated before the store, as in the source.
- No `alloc`, no arena operand: `bump` allocates nothing, so there is nothing to name.

## 5. Where each fact came from (summary)

| You see in MIR | Decided by |
|---|---|
| Blocks, `switch`, `goto`, `call -> bbN` | `mir::build` (this step): structured HIR becomes a graph |
| `region rN ...` lines, which `region_enter`s exist | `sema` `ArenaReport` + `codegen_push` (stamped by `region_walk`) |
| `arena_create_dynamic`, `fresh1`, `in fresh1` | `MemoryPlan` (`src/memory.rs`) |
| `+current`, `parent=`, `sink=` | `CallAbi` rule from `codegen/llvm/call_abi.rs` |
| `// label "c"` | `DebugLabel`, modelled on the `debug_*` hooks in `codegen/llvm/debug/` |
| `_N` naming and kinds | builder: params, declared names, and invented temps |
| `intrinsic PrintlnInt` | `src/builtins.rs` name matching |

## 6. What this example does not show

`when` (a `match` terminator with patterns), `Ref` handles (`ref_create`, `ref_store`,
`observe` for `if val`), moves and promotes (`move`, `copy_into`), arrays with bounds checks
(`abort Bounds`), `Vec` operations and nullables (`T?` is an ordinary `Option` enum). For
those, look at the goldens: `tests/golden/mir/enums.txt`, `managed_refs.txt`, `vec.txt`.

Known gaps in this step (tracked in #237): no `storage_live`/`storage_dead`, debug labels only
on declarations, the signed `MIN / -1` division guard is missing, and the verifier does not yet
check initialisation or arena lifetimes.
