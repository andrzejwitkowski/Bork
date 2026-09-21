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

### The `move` keyword

Transfer ownership into a child block or trailing closure (typical for `var`, or to consume a `val`):

```bork
move (user, score) {
    // user and score live here; parent names are Moved
}

action(1, 2) move (acc) { x, y ->
    acc + x + y
}

// Omit the capture list to move every parent local the body references:
action(1, 2) move { x, y ->
    acc + x + y
}
```

The compiler enforces: non-Copy **`var`** values may not be observed from a child arena without `move`. Parent **`val`** may be Shared. After a move, using the parent name is an error.

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

LLVM will emit real per-region bump arenas using this model. MVP 0.3 validates layout and ownership in the compiler only — programs are not executed yet.
