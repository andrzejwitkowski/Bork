# Bork Memory Model

Bork manages memory with **hierarchical arenas** tied to curly-brace regions (`{}`). There is no garbage collector: leaving a block resets that region’s arena in one shot.

## Philosophy

- Each meaningful `{ … }` region owns a fixed arena (MVP model: **4 KB / 4096 bytes**).
- Allocations inside a region are a pointer bump: check `offset + size ≤ 4096`, then advance `offset`.
- Exiting the region sets `offset = 0` — instant bulk deallocation — and returns the slab to an **arena pool** for reuse by later regions.
- Loop bodies reset their arena each iteration so memory stays O(1) across iterations (same slab, no pool round-trip required).
- Cross-region data flow uses **Copy** (primitives), **Shared** (parent `val` reads), or **Move** (`var` / explicit consume / escape). No dangling pointers into a dead arena.

The 4 KB size matches common OS/database **page** granularity so hot working sets stay cache-friendly in native codegen (`codegen` feature).

## Stack / registers vs arenas

```text
  CPU / stack frame                    Arena for block B
  ┌─────────────────┐                  ┌──────────────────────────┐
  │ return addr     │                  │ [====allocated====]....  │
  │ locals (scalars)│ ──pointers───►   │ bump ──┘                 │
  │                 │                  │ capacity = 4096 bytes    │
  └─────────────────┘                  └──────────────────────────┘

  leave B  ──►  arena.reset()  ──►  slab returned to ArenaPool
  enter C  ──►  pool.acquire() ──►  often reuses B's slab (if C is not live with B)
```

Scalars and control live on the stack/registers. Heap-like payloads for a region live in that region’s bump buffer.

### Arena pool

Released 4 KiB slabs go on a free list (`ArenaPool`). Nested live regions keep distinct slabs; sequential siblings `acquire` recycled ones. `release` resets before parking.

## Copy, Shared, and Move

| Kind | Cross-arena behavior |
|------|----------------------|
| **Copy** | Non-null primitives (`Int`/`i32`, `bool`, `unit`, …). Reading from a parent arena duplicates the value; parent stays live. |
| **Shared** | Parent **`val`** of a non-Copy type. Child observes the value in place (stack nesting: parent outlives child). Parent binding stays live. |
| **Move** | Parent **`var`** of a non-Copy type (required), or explicit `move` / future escape into a longer-lived arena. Ownership transfers; parent binding is **Moved** (unusable). |

```text
Parent A (val s)                   Child A'
┌──────────────────┐               ┌────────────┐
│ s  ──────────────┼── Shared ────►│  reads s   │  OK: A outlives A'
└──────────────────┘               └────────────┘
```

Moving a `val` *into* a shorter-lived child is rarely useful; Shared is the default. Explicit `move (s)` on a `val` is still allowed when you want to consume it.

Move does **not** punch a hole in the parent arena. Old bytes stay as unreachable dead storage until the parent arena resets. The compiler marks the **binding** dead so use-after-move is a hard error.

```text
Parent A                         Child A'
┌────────────────────┐           ┌────────────────────┐
│ … | S (dead slot) | … │         │ S' (live)          │
└────────────────────┘           └────────────────────┘
     binding S invalid                 only S' is usable
```

## How to transfer ownership (`move`)

Everyday style — **expression** and **call-site** `move` (no extra braces):

```bork
fun f(var a: String, var b: String) {
    // a and b owned here
}

fun main() {
    var s: String = "xxx"
    var owned = move s

    var x: String = "X"
    var y: String = "Y"
    f(move x, move y)
}
```

Rules:

- Non-Copy **`var`**: always write `move name` on assign RHS and call args when passing a **named binding**.
- String literals and other **fresh expressions** into `var` parameters do **not** need `move` (e.g. `f("hello")`).
- Bare `name: Type` params are **`val`**; use `var` when the callee should own.
- Passing a non-Copy **`val`** into a `var` parameter also requires `move`.
- `f(x)` with non-Copy **`var`** `x` is an error — use `f(move x)`.

(`Int` and other Copy types do **not** need `move` to cross arenas — they **Copy**. Parent `val` of a non-Copy type may cross as **Shared** without `move`.)

### Regional `move (…)` / `move { … }`

Regional `move (a, b) { … }` is for **multi-capture** blocks and **trailing closures** after a call. Names in the capture list are already Local inside — do **not** write `var t = move a` for those.

The examples below use two non-Copy locals:

```bork
var a: String = "A"
var b: String = "B"
```

Each regional shape works as a **statement block** or as a **trailing closure** after a call.

### 1. Explicit capture list — `move (…)`

Name exactly what leaves the parent. Listed names become **Moved** in the parent; they live in the child arena.

```bork
move (a, b) {
    // a and b are Local here
    val x = a
}
// val y = a  → error: use after move
// val z = b  → error: use after move
```

Move only one:

```bork
move (a) {
    val x = a
}
// a is Moved; b is still live in the parent
val keep = b
```

### 2. Explicit empty list — `move ()`

No inference. Nothing is moved. Useful when you want a move-region *shape* without consuming parents (or to opt out of inference).

```bork
move () {
    // a and b stay in the parent
    // val t = a  → error: var String is not Copy; need `move` or use a `val`
}
```

Same idea on a trailing closure:

```bork
action(1, 2) move () { x, y ->
    x + y
}
```

### 3. Omitted list — `move { … }` (inference)

Omit `(…)` entirely. The compiler moves every **live, non-Copy** parent local that the body **references** (free vars). Copy bindings are skipped. Names never mentioned stay put.

```bork
move {
    val t = a
    // only `a` is referenced → only `a` is Moved; `b` stays live
}
val still_ok = b
```

Both:

```bork
move {
    val x = a
    val y = b
}
```

Trailing closure with inference:

```bork
action(1, 2) move { x, y ->
    // references `a` → moves `a` (and any other non-Copy free locals used here)
    a
}
```

### Statement block vs trailing closure

| Form | Statement | After a call |
|------|-----------|--------------|
| Explicit | `move (a, b) { … }` | `f(…) move (a, b) { params -> … }` |
| Empty | `move () { … }` | `f(…) move () { params -> … }` |
| Inferred | `move { … }` | `f(…) move { params -> … }` |

Ordinary (non-`move`) nested `{ … }` and non-`move` closures do **not** transfer ownership. A child reading a parent **`var`** of a non-Copy type is an error unless you `move` it. A child reading a parent **`val`** non-Copy is **Shared**.

### Loops

A `for` body runs many times at runtime, but ownership of an **outer** binding can be transferred only once. Moving a name that lives **outside** the loop is a compile error:

```bork
var a: String = "A"
for (i in 1..10) {
    move {
        val t = a   // error: cannot move `a` inside a loop
    }
}
```

Moving something **created inside** the loop body is fine — each iteration gets a fresh local:

```bork
for (i in 1..10) {
    var a: String = "A"
    move (a) {
        val t = a
    }
}
```

### What is *not* a move

| Situation | What happens |
|-----------|----------------|
| Read parent `val` non-Copy in a child | **Shared** — parent stays live |
| Read parent Copy (`Int`, …) in a child | **Copy** — parent stays live |

Named non-Copy sources nested inside constructors still follow the same rule: `var x = Some(s)` / `f(Some(s))` require `move s` when `s` is a `var` (or the destination/`var` formal demands ownership). Literals and other fresh values do not.

### Quick reference with `a` and `b`

```bork
fun main() {
    var a: String = "A"
    var b: String = "B"

    move (a) {            // explicit: only a
        val x = a
    }
    // b still usable
    move {
        val y = b         // inferred: moves b
    }
}
```

## Nested braces

Pure wrappers like `{{{{ stmts }}}}` collapse to **one** arena. Dumps annotate compaction, e.g. `MainBlock (compacted 3 braces)`.

Control-flow regions (`fun`, `if`/`else`, `for`, closures, non-trivial blocks) stay separate arenas.

## `--dump-arenas`

Inspect the compile-time arena tree:

```bash
cargo run --quiet -- path/to/file.bork --dump-arenas
# or: bork --dump-arenas path/to/file.bork
```

Example shape:

```text
Arenas
└── fun main
    ├── accumulator [Local]
    ├── threshold [Local]
    └── ForLoop
        └── Closure (move)
            └── acc [Moved ← fun main]
```

Labels:

- **Local** — declared in this arena
- **Copy** — read from a parent arena via Copy (primitives)
- **Shared ← …** — parent `val` observed in place
- **Moved ← …** — ownership transferred from a parent (parent binding dead)

### Editor

In Cursor/VS Code with the Bork extension:

- Invalid moves show as diagnostics
- Hover a binding for arena + ownership
- Command **Bork: Dump Arenas** (`bork.dumpArenas`) prints the same ASCII tree

## Native codegen and region schedule

With the `codegen` feature, LLVM lowering uses per-region bump arenas (`bork_runtime`). Ownership and arena layout are validated in `sema`; `src/codegen/regions.rs` walks HIR in lockstep with the arena report.

**Sema vs codegen push:** the arena dump tree still has a child node for every region site (blocks, loops, `if` branches, …). After typeck, `region_walk::stamp_codegen_push` sets `ArenaNode.codegen_push` from typed HIR: codegen calls `bork_arena_push` only when that flag is true (function roots always push; trailing closures never push). A `{ … }` that only holds scalars and no sink allocation therefore has a report node but no extra runtime arena — matching the “no alloc in this region” optimization.

The sections below describe behavior that is **implemented today** for avoiding redundant cross-region copies while keeping the **no-GC, no general `&` / borrow-checker** model.

## Assign-up, sink allocation, and escape

Goal: owned payload for a **sink** (assignment target, `return`, or builtin `concat`) is bumped in that sink’s arena even when the expression is evaluated lexically inside a shorter-lived child region. Sema, codegen, hoist, `promote`, and `escape::place` cooperate on this; see also [language.md](language.md) for surface rules.

### Assign-up (mutate outer `var` from inner region)

- **Rule:** `outer = rhs` (or field assign to an outer `var`) where `outer` is bound in an **ancestor** arena is **mutation of the outer binding**, not “use of outer `var` inside the child” (no false “move into `Block`” on the LHS).
- **Sema:** assign to a binding in a strictly older region is allowed for `var`; the RHS is still checked for move/Copy. The RHS transfer sink is the **outer binding’s arena** (`sema/walk.rs`).

```bork
var outer = "a"
{
    var inner = "b"
    outer = move inner
}
```

### Sink allocation (`alloc_sink`)

- **Rule:** for `sink = rhs`, `return expr`, or `concat(…)` used as that value, owned **String** bytes for the result are allocated in **`arena(sink)`** while lowering runs, even inside a nested `{ … }`.
- **Not:** every sub-expression in `rhs` uses the sink (that would bloat the parent with temporaries). Only the **stored/returned** owned value uses the sink; callee bodies keep their own regions.
- **Codegen:** `alloc_sink` on `FnEmitter` — set to the destination slot’s `home_arena` on assign and the active sink while emitting the **result** of `concat` (`emit_fn`, `expr`). Operand and call-argument lowering clears `alloc_sink` so temporaries stay in the current region.

Examples:

- `outer = "literal"` in a child block → descriptor target in `outer`’s arena.
- `return s` for a parameter or a local whose bytes already live at function depth (e.g. a string literal initializer); `return "literal"` uses constant storage.
- `return move s`, `return concat(…)`, and inner-region `return` of owned strings are rejected until a caller-owned return arena exists (see **Still open**).

### `concat`

- Builtin `concat(a, b)` allocates **one** result buffer in the current sink arena (two `memcpy` from arguments). No separate call-scoped temp arena type yet; argument strings are read in place (Copy/Shared/move as usual).
- Typical use: `outer = concat(left, right)` in a child block → result in `outer`’s arena.

### Hoist (`hoist::annotate`)

- **Problem:** `var inner = …` in a child, then `outer = move inner` — without hoist, initialization runs in the child arena and move copies on assign.
- **Implemented pattern:** linear siblings in one block — `var inner = <hoistable init>` immediately followed by `outer = move inner`, where `outer` is already in scope (parameter or earlier `var`). HIR sets `alloc_in_binding` on `inner` so codegen initializes in `outer`’s arena (`hoist.rs`).
- **Hoistable inits:** string literals and calls (including `concat`) today.
- **Not hoisted:** conditional escape, loops, closures, multiple sinks, or `outer` not visible — use **`promote`** or restructure.

### `promote`

- **Syntax:** `promote name` (expression).
- **Rule:** deep-relocate owned payload into the arena of an outer **`var`** you are assigning to, then invalidate the source (move semantics). Valid only when sema provides an assign **transfer sink** into that outer binding.
- **Not implemented:** `promote` on `return` (return is not a sema sink); returning moved or concatenated strings is not supported yet.

### Arrays `[T; N]`

- Surface types carry the length **N**; runtime shape is still a descriptor `{ ptr, len }` with `len == N` for values of that type. Elements live in a contiguous buffer in the array value’s home arena (the arena active when the array was created, or the assign sink for `outer = …`).
- **Whole-array** `move` / `promote` / assignment requires the same `[T; N]` on both sides.
- **Slice** `a[lo..hi]` with compile-time literal bounds has type `[T; hi - lo]`. Codegen uses that **N** as the descriptor length and points into the same buffer (no copy). It does not emit a runtime bounds abort: typeck already rejected a slice that does not fit in the receiver. Escape rules treat the slice like the receiver array: it must not outlive the arena that owns the buffer.
- **Index read:** Copy elements copy by value. A `String` element is **Shared** (a view of the array buffer), whether the array binding is `val` or `var`. The index is a runtime `i32`, so an out-of-range index aborts.
- **Index assign:** `a[i] = v` on a `var` `[T; N]` writes element `T` (Copy or `String`). `val` arrays reject assignment. Non-Copy elements use the array binding’s arena as the assign sink (`move` / `promote` as for whole-binding assignment). Out-of-range index aborts like a read.

### Escape analysis (`escape::place`)

- After typecheck, `frontend::check` runs `escape::place` (same rules as codegen `alloc_sink` for strings). Diagnostics use phase **Ownership** (LSP sees them without `codegen`).
- Rejects, among others: assigning a string built in an inner region to an outer `var` without sink/hoist/promote, `if` branches that yield a moved string as the `if` value, returning a `String` from an inner region, and `return move` / `return concat(…)` until return-arena codegen exists.

## Not planned (and not on the roadmap here)

- General **`&` / `&mut`** and field-stored references across regions (Rust-like borrow checking).
- Implicit deep copy on every cross-region read (use **Shared** for parent `val`, **move** / **promote** for ownership).
- Storing borrows from a **child** arena into a **parent** field (parent must own bytes in its arena or observe parent `val` via **Shared**).

## Still open

| Topic | Notes |
|-------|--------|
| Caller-owned return arena | `return move`, `return concat`, and other returns that would bump in the callee arena before `bork_arena_pop`; needs a hidden `ret_arena` (or equivalent) from the caller. |
| Call-scoped temp arenas | General calls may still allocate scratch in the current region; only sink results and `concat` are specialized today. |
| `promote` on `return` | Would need a return transfer sink in sema + codegen. |
| Hoist beyond linear siblings | Use-def / definite escape for more patterns; else `promote`. |
| `memcpy` elision | When hoist or sink already placed bytes in the target arena. |
