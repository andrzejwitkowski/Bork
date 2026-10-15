# Paged arenas with grow/free primitive

## Why
A region was one fixed 4 KiB buffer; going past it aborted the program (`arena overflow`). `concat`, `stringFromUtf8` and array copies already allocate runtime-sized data, and the planned Vec/Set/HashMap need a region that grows and a way to move a growing buffer.

## Decisions
- A region is a list of 4096-byte pages. Allocations over 2048 bytes get their own system block (rounded up to 4096). Nothing in the compiler or `bork_arena_alloc` ABI changes.
- Published objects never move (invariant of `2026-10-06-managed-ref-lifetime-a-design.md`). Only a collection's internal buffer is moved, through `bork_arena_grow`; the collection header `{ptr,len,cap}` stays put.
- A borrow of an element blocks mutation of the collection (checked by the future Vec work, not here). `Vec<Ref<T>>` works because only the handles are copied and the objects stay.
- A collection registers one drop for its header, never one per element: `bork_arena_register_ref_drop` stores the slot address, which a move would invalidate.
- Abandoned blocks are reused: free blocks go on per-size-class lists (16 ... 2048 bytes). A freed block that ends at its page's bump pointer rewinds the pointer instead. Tails of older pages are tried before a new page is added. Blocks under 16 bytes are not recycled; no coalescing.
- Capacity policy (initial size, doubling, `with_capacity`) belongs to the Vec spec.

## Runtime API (`crates/bork_runtime/src/arena.rs`)
- `bork_arena_alloc(arena, size, align)`: unchanged.
- `bork_arena_grow(arena, ptr, old, new, align)`: extends in place when the block is last on its page, otherwise allocates, copies, frees the old block. The returned pointer replaces `ptr`.
- `bork_arena_free(arena, ptr, size)`: `size` must be what the block was allocated or last grown to.
- Reset/pop keeps the first page (so a recycled arena keeps its address) and drops the other pages and large blocks.

## Debug ABI v2
`DebugArena` is 14 words: `payload` became `pages` (pointer to `DebugPage {base, used, capacity}` array) plus `page_count`; `used`/`capacity` are sums. `DebugRegistry` header is unchanged apart from `version = 2`. `tools/bork-debug` locates an object in any page. The `bork_debug_abi_v1` symbol keeps its name.

## Known limits
- Nothing in codegen calls grow/free yet; unit tests cover them.
- Page lookup and first-fit are linear in the page count.
- Extra pages are returned to the system on reset, not pooled.
