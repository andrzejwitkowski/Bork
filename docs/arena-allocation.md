# Arena allocation, growth and reuse

How a region's arena gets memory today (`crates/bork_runtime/src/arena.rs`). For the lifetime rules of arenas (who may point where) see [memory-model.md](memory-model.md); this page is only about where the bytes come from, how freed space is found again, and what moves.

## Shape of an arena

```text
Arena
├─ pages[0]  4096 B ┌──────────────┬───────────────┬─ ─ ─ ─ ─ ┐
│                   │ obj A  (40)  │ obj B  (100)  │ free tail │   offset ──►
├─ pages[1]  4096 B ├──────────────┴───────────────┴─ ─ ─ ─ ─ ┘
├─ spare[]          pages kept by reset, reused before new ones
├─ free_lists[8]    freed blocks by size class: 16 32 64 128 256 512 1024 2048
└─ large[]          blocks over 2048 B: one system allocation each,
                    rounded up to a multiple of 4096
```

- Every page has its own bump offset. Pages never move once created.
- A block is never moved by the arena on its own. Only `bork_arena_grow` can move one, and the caller then owns the new address.
- Published objects (records, `Ref` targets) never move. Only a collection's internal buffer is meant to be grown, with the header `{ptr, len, cap}` staying put.

## `alloc(size, align)`: where does a block go?

Tried in this order, the first that fits wins:

1. **Large:** `size > 2048` → its own system block (`large`). Contiguous, any size.
2. **Free list:** `size >= 16` → walk the list of class `ceil_class(size)` and take the first block whose address satisfies `align`. A block taken this way can be bigger than the request: a block is filed under its size rounded *down* and a request looks under its size rounded *up*, so a 127-byte hole serves a 33-byte request. The slack is not tracked (the arena does not remember the real block size), and if the block is freed again with the smaller size it is filed lower and the slack is lost until `reset`.
3. **Bump, last page:** advance the offset of the newest page, aligning the address.
4. **Bump, older pages:** the unused tails of earlier pages, first fit. These tails appear when a request did not fit and a new page was started.
5. **New page:** from `spare` if there is one, otherwise a fresh 4096-byte page.
6. **Fallback:** the alignment is bigger than a page can give → a system block like case 1.

Ordinary allocations are not rounded up to a size class, so records do not waste memory. Only the free list uses classes.

Example (sizes in bytes, one fresh arena):

```text
alloc 2000   page0 [2000........]            bump
alloc 1900   page0 [2000|1900....] 196 left   bump
alloc 2000   page1 [2000........]            did not fit page0 → new page
alloc 2000   page1 [2000|2000..]   96 left    bump on the last page
alloc 150    page0 [2000|1900|150] 46 left    last page has 96, older tail has 196
alloc 10000  large                            own 12288-byte block
```

## `free(ptr, size)`: giving a block back

`size` must be what the block was allocated, or last grown, to. The pointer is looked up by membership, not by size.

| The block is... | Result |
|---|---|
| in `large` | released to the system immediately |
| the last block on its page (`ptr + size == offset`) | the page offset moves back: the space is plain tail again |
| any other block, `size >= 16` | pushed on the free list of class `floor_class(size)` (its first 8 bytes store the next pointer) |
| any other block, `size < 16` | dropped until the arena resets (known shortcut) |

`floor_class` rounds down, so a 100-byte hole goes to the 64-byte class and a later 64-byte request can use it. A request is never given a block that is too small.

```text
before   [A 64][B 8][C 64]
free(A)  [hole 64][B 8][C 64]       hole → free_lists[64]
alloc 64 → returns A's old address  (free list, step 2)
```

## `grow(ptr, old, new, align)`: moving a block

Only used for growing (`new <= old` returns `ptr` unchanged).

1. **Large block:** `realloc`. The address may change.
2. **Last block on its page and the page has room** (`start + new <= 4096`): the offset just advances, **same address, nothing copied**.
3. **Otherwise:** `alloc(new)` + copy `old` bytes + `free(old block)`. **The address changes.** The old space becomes a hole that later allocations of that class reuse.

```text
vec buffer 64 B, something allocated after it:
  [buf 64][wall 8]                 grow(buf, 64, 256)
  [hole 64][wall 8][buf' 256]      copied, old block on the 64-byte list
  the next 64-byte request lands in the hole
```

A block grown in place can end up above 2048 bytes while living in a page. That is fine because `free` and `grow` check where the block lives, not how big it is.

A growing collection should double its capacity: the holes it leaves are then 1x, 2x, 4x... of the final size, and each size class can serve a different collection.

## `reset` / `pop`

- Zero the offset of the first page. It stays, so a recycled arena keeps its address.
- Other pages go to `spare` (offset zeroed). They are reused before the system is asked for more. `spare` is not capped, so a pooled arena keeps its peak page count.
- Free lists are cleared, large blocks are released.
- The arena then goes back to the thread-local `ArenaPool`.

A loop body that resets its arena every iteration therefore reuses the same pages and holes.

## Limits

- Nothing in codegen calls `grow`/`free` yet. `concat`, `stringFromUtf8` and array copies only `alloc`; the unit tests in `arena.rs` cover grow/free.
- Page lookup (`free`, `grow`) and the older-page tail search are linear in the page count.
- Holes are not merged with their neighbours.
- Reusing a hole can waste part of it (up to about 3x the request, see step 2 of `alloc`). Collections that double their capacity use power-of-two sizes and do not lose anything this way. Accepted until a collection shows it matters; the alternative is to file and look up blocks by the same rounding, which wastes nothing but reuses fewer holes.
- A hole is found only by requests whose size rounds up to its class (a 100-byte hole sits in the 64-byte class, so a 100-byte request, which needs the 128 class, cannot use it).
- Passing a pointer the arena does not own, or a wrong size, to `free`/`grow` is a bug in the caller; debug builds assert on the pointer, not on the size.

## Debugging

Debug builds publish every page and large block (`DebugPage {base, used, capacity}`) through the registry, and `tools/bork-debug` shows `bytes=used/capacity` for the whole arena. See [debugging.md](debugging.md).
