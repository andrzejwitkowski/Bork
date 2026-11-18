# Vec<T> design

Built-in growable array, handled like `Array` in the type system (no general generics).
Builds on [paged arenas](2026-10-15-paged-arenas-design.md), which defers the Vec capacity
policy, the borrow rule and the header drop to this document.

## Goals / non-goals

- Goals: `Vec<T>` with `Vec::<T>()` / `Vec::<T>(capacity)`, `push`, `pop`, `get`, `set`, `len`, `capacity`, `clear`, `v[i]`;
  amortised O(1) push by doubling; correct `move` / `promote` across arenas.
- Non-goals (v1): user generics, `Vec<Vec<T>>`, `Vec<T?>`, plain class elements (`Vec<Node>`),
  shrinking, iteration helpers, `return` of a Vec (rejected like arrays until a return arena exists).

## Layout

- Header `{ ptr, len, cap }` is allocated once with `bork_arena_alloc` in the owner arena and never
  moves. Only the element buffer moves, through `bork_arena_grow`.
- Capacity: first push allocates `cap = 4` (at least 16 bytes, smaller blocks are not recycled by the
  arena), then `cap * 2`. `Vec::<T>(n)` allocates `n` up front, growth then doubles from `n`; a negative `n` aborts. `capacity()` exposes the value. A buffer past the 2048-byte
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
  recomputes `RefKind` against the new owner. The source binding is moved (unreadable) but its allocation stays until its arena dies, exactly as
  `copy_array_into_arena` does: the same routine also serves plain copies, where zeroing the source
  `len` would empty a live vector. Strings are copied byte-wise.
- Mixed arenas inside one Vec are valid: each handle carries its own `control` and `kind`. A `push`
  classifies the new slot against the header's current arena.
- Objects put into a Vec are planned as dynamic, as for `[Ref<T>; N]`.

## Borrows

A borrowed element (`get`, `v[i]`) would block `push`, `pop`, `clear`, `move` and `promote` of that Vec for
the borrow's lifetime. The rule has nothing to enforce today: `v[i]` and `get` yield an owned value (a `String`/array descriptor copy, a `Ref` clone or a scalar), `&v[i]` is rejected
("`&` borrows a name") and `v[i]` does not coerce to `&T`, so no borrow of an element exists and a `push` that moves the buffer cannot leave one dangling.
`programs/check_fail/vec/borrow_element_*.bork` pin this. Borrowing an element (`&v[i]`) is a separate feature with its own lock on the Vec (RFC 0004 section 3.6). Index reads follow the array rules (runtime `i32`, out of range aborts).

## Constructors

`Vec::<T>()` and `Vec::<T>(capacity)` are expressions (the turbofish form, since `Vec<T>(n)` would read as two comparisons).

## Not done yet

- Borrowing an element (`&v[i]`) does not exist, so there is no element-borrow lock; index reads and methods only
  borrow the receiver for the call.
- `bork-debug` does not know the `Vec` header layout.
