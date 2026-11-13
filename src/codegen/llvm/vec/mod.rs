//! `Vec<T>` codegen.
//!
//! A vector value is a pointer to a header `{ buf, len, cap, arena }` allocated once in the owner
//! arena; the header never moves, only `buf` does (`bork_arena_grow`). The header keeps the owner
//! arena so `push` can grow without knowing where the vector lives.
//!
//! - `layout`: header field access and slot addressing
//! - `alloc`: `Vec<T>()` / `Vec<T>(n)`
//! - `grow`: the resize that `push` runs before storing

mod alloc;
mod grow;
mod layout;

/// Capacity of the first buffer; later growth doubles.
const FIRST_CAPACITY: u64 = 4;
const HEADER_SIZE: u64 = 32;
const HEADER_ALIGN: usize = 8;
pub(super) const BUF: u32 = 0;
pub(super) const LEN: u32 = 1;
pub(super) const CAP: u32 = 2;
pub(super) const ARENA: u32 = 3;
