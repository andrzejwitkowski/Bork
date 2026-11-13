//! Fixed-length array `[T; N]` codegen. A value is a `{ ptr, len }` descriptor over a buffer in
//! the arena the array was created in.
//!
//! - `layout`: the descriptor, element size/stride and slot addressing
//! - `alloc`: arena allocation, array literals, copying into another arena
//! - `bounds`: the runtime index check

mod alloc;
mod bounds;
mod layout;
