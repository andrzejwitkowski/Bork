# Bork Memory Model

Bork manages memory with **hierarchical arenas** tied to curly-brace regions (`{}`). There is no garbage collector: leaving a block resets that region’s arena in one shot.

## Philosophy

- Each meaningful `{ … }` region owns a fixed arena (MVP model: **4 KB / 4096 bytes**).
- Allocations inside a region are a pointer bump: check `offset + size ≤ 4096`, then advance `offset`.
- Exiting the region sets `offset = 0` — instant bulk deallocation — and returns the slab to an **arena pool** for reuse by later regions.
- Loop bodies reset their arena each iteration so memory stays O(1) across iterations (same slab, no pool round-trip required).
- Cross-region data flow uses **Copy** (primitives), **Shared** (parent `val` reads), or **Move** (`var` / explicit consume / escape). No dangling pointers into a dead arena.

The 4 KB size matches common OS/database **page** granularity so hot working sets stay cache-friendly when LLVM codegen lands.

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

## What comes next

LLVM codegen uses per-region bump arenas (`bork_runtime`, optional `codegen` feature). Ownership and arena layout are validated in `sema`; see `src/codegen/regions.rs` for the region schedule.

## TODO: allocation without redundant cross-region copies

Planned compiler behavior (not all implemented yet). Goal: keep the **no-GC, no general `&` / borrow-checker** model while avoiding hot-path `memcpy` when a value’s **owner** already lives in a longer region.

### 1. Assign-up (mutate outer `var` from inner region)

- **Rule:** `outer.x = rhs` (or `outer = rhs`) where `outer` / `x` is bound in an **ancestor** arena is **mutation of the outer binding**, not “use of outer `var` inside the child” (no false “move into `Block`” on the LHS).
- **Sema:** treat assign to a binding in a strictly older region as allowed for `var`; RHS still checked for move/Copy as today.
- **Status:** partial — field-read / capture fixes exist; assign-up policy still TODO.

### 2. Sink allocation (allocate the **assignment result** in the sink arena)

- **Rule:** for `sink = rhs`, owned payload of the **value stored in `sink`** is bumped in **`arena(sink)`**, even if evaluation runs lexically inside a child `{ … }`.
- **Not:** “every allocation anywhere in `rhs` uses the sink arena” (that would bloat the parent with expression junk — see call temps below).
- **Codegen:** thread an `AllocArena` (or equivalent) through expression lowering when emitting the RHS of an assign whose destination binding is in an older region.
- **Examples:**
  - `outer.x = "literal"` inside a child block → string bytes (or descriptor target) in `outer`’s arena.
  - `outer.x = build()` → **return value** of `build` in `outer`’s arena; internals of `build` use `build`’s own region.

### 3. Call / expression temporaries (e.g. `concat`)

- **Rule:** scratch buffers inside a call or compound expression use a **short-lived** allocation context (call frame, inner expr scope, or current region), then are discarded on completion.
- **Only** the final value written to the assign sink (or returned) uses **sink allocation** (§2).
- **Example:** `outer.x = concat(a, b)` in a child block — `a`/`b` read via Copy/Shared; concat workspaces reset with the call/temp scope; result string in `arena(outer.x)`.

### 4. Escape hoisting (init locals in the arena they **definitely** escape to)

- **Problem:** `var inner_s = …` in a child, later `outer = inner_s` — naive codegen allocates in the child then **copies** to `outer` on move/assign.
- **Rule:** after a **function or block** analysis pass, if a binding’s **only** non-local use is a **definite move** into a known outer binding (same function, no conflicting uses), set **`alloc_site(binding) = escape_arena(binding)`** (e.g. parent of the declaring block) so initialization already runs in that arena.
- **Requires:** use-def / escape info (not a full Rust-style borrow checker); second pass or deferred choice at end of block.
- **Limits:** conditional escape (`if` only one branch assigns out), use-after-assign in child, multiple sinks, loops (fresh `inner_s` per iteration), and closures — hoist only when **definite**; otherwise keep child arena + explicit **`promote`** (§5).

### 5. `promote` (explicit escape for **existing** values)

- **Rule:** `promote(expr)` — deep-relocate owned arena payload (e.g. `String`, future structs) into a chosen **ancestor** arena (typically immediate parent or `arena(sink)`), then **invalidate** the source binding (move semantics). For values already built in a shorter region.
- **When:** assign-up + sink allocation + hoisting do not apply (value already materialized in the wrong arena).
- **Syntax / keyword:** TBD; may overlap with extending `move` for “upward” escape.

### 6. What we are **not** planning here

- General **`&` / `&mut`** and field-stored references across regions (Rust-like borrow checking).
- Implicit deep copy on every cross-region read (use **Shared** for parent `val`, **move** / **promote** for ownership).
- Storing borrows from a **child** arena into a **parent** field (parent must own bytes in its arena or observe parent `val` via **Shared**).

### 7. Implementation checklist

| Item | Area | Notes |
|------|------|--------|
| Assign-up sema for `var` in ancestor | `sema` | LHS assign ≠ transfer sink on outer `var` |
| `AllocArena` on assign RHS | `typeck` / `hir` / `codegen` | Sink §2 |
| Call-scoped temp arenas | `codegen` | §3 |
| `escape_arena` / hoist pass | `sema` or `typeck` | §4 |
| `promote` surface syntax + deep relocate | `sema`, `codegen` | §5; mirror string `move` reloc |
| Document temp lifetime in parent for sink-only rule | this doc | §2 vs §3 distinction |
