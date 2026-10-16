//! Fixed-length array `[T; N]` codegen. A value is a `{ ptr, len }` descriptor over a buffer in
//! the arena the array was created in.
//!
//! - `layout`: the descriptor, element size/stride and slot addressing
//! - `alloc`: arena allocation, array literals, copying into another arena
//! - `access`: `a[i]`, `a[i] = v`, `.length`, slices
//! - `bounds`: the runtime index check
//! - `debug`: debug-runtime labels for `[Ref<T>; N]`

mod access;
mod alloc;
mod bounds;
mod debug;
mod layout;
