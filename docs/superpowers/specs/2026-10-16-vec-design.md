# Vec<T> design

Built-in growable array, handled like `Array` in the type system (no general generics).
Builds on [paged arenas](2026-10-15-paged-arenas-design.md), which defers the Vec capacity
policy, the borrow rule and the header drop to this document.

## Goals / non-goals

- Goals: `Vec<T>` with `new`, `withCapacity`, `push`, `pop`, `get`, `set`, `len`, `clear`, `v[i]`;
  amortised O(1) push by doubling; correct `move` / `promote` across arenas.
- Non-goals (v1): user generics, `Vec<Vec<T>>`, `Vec<T?>`, plain class elements (`Vec<Node>`),
  shrinking, iteration helpers, `return` of a Vec (rejected like arrays until a return arena exists).

## Layout

- Header `{ ptr, len, cap }` is allocated once with `bork_arena_alloc` in the owner arena and never
  moves. Only the element buffer moves, through `bork_arena_grow`.
- Capacity: first push allocates `cap = 4` (at least 16 bytes, smaller blocks are not recycled by the
  arena), then `cap * 2`. `Vec.withCapacity(n)` allocates `n` up front. A buffer past the 2048-byte
  large threshold is grown with `realloc` by the arena; otherwise the last block on a page grows in
  place and any other block is copied, its old block going to the free list.

## Elements

Copy types, non-null `String`, `Ref<T>`. A class object is stored only as `Ref<T>`
(`Vec<Ref<Node>>`): plain class objects never move, so relocating a Vec of raw pointers on `promote`
would leave them pointing into a dying arena. `Ref<Vec<T>>` is an error (`Ref` targets classes only).

## Drop

One drop per `Vec<Ref<T>>`, registered on the header with `bork_arena_register_drop`
(`bork_vec_drop_refs`). It walks `len` slots and drops each `Ref`. Per-slot
`bork_arena_register_ref_drop` is not used: it registers the slot address, and `grow` moves the buffer.

## move / promote

- `move` inside one arena transfers the binding only.
- Across arenas (`promote`, move into a longer-lived sink): new header and a `cap = len` buffer in the
  destination, then each slot is cloned and stored through `bork_ref_store_take` (`rehome`), which
  recomputes `RefKind` against the new owner. The source is left intact, exactly as
  `copy_array_into_arena` does: the same routine also serves plain copies, where zeroing the source
  `len` would empty a live vector. Strings are copied byte-wise.
- Mixed arenas inside one Vec are valid: each handle carries its own `control` and `kind`. A `push`
  classifies the new slot against the header's current arena.
- Objects put into a Vec are planned as dynamic, as for `[Ref<T>; N]`.

## Borrows

Intended rule (not enforced yet, see "Not done yet"): a borrowed element (`get`, `v[i]`) blocks `push`, `pop`, `clear`, `move` and `promote` of that Vec for
the borrow's lifetime. Index reads follow the array rules (runtime `i32`, out of range aborts).

## Constructors

`Vec<T>()` and `Vec<T>(capacity)` are expressions (`Vec` is a keyword, so `Vec.new()` does not parse).

## Not done yet

- Element borrows do not block `push` / `pop` / `clear` / `move`; index reads and methods only
  borrow the receiver for the call.
- `bork-debug` does not know the `Vec` header layout.
