//! Fixed-size bump arena model for future LLVM region runtime.

pub const ARENA_CAPACITY: usize = 4096;

/// Per-block bump allocator backed by a static 4 KiB buffer.
pub struct Arena {
    buf: [u8; ARENA_CAPACITY],
    offset: usize,
}

impl Arena {
    pub fn new() -> Self {
        Self {
            buf: [0; ARENA_CAPACITY],
            offset: 0,
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn capacity(&self) -> usize {
        ARENA_CAPACITY
    }

    /// Bump-allocate `size` bytes with `align` (power of two). Panics if the arena is full.
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        assert!(align.is_power_of_two() || align == 1, "align must be 1 or a power of two");
        let align = align.max(1);
        let aligned = (self.offset + align - 1) & !(align - 1);
        let end = aligned
            .checked_add(size)
            .expect("allocation size overflow");
        if end > ARENA_CAPACITY {
            panic!(
                "arena overflow: need {end} bytes, capacity {ARENA_CAPACITY}"
            );
        }
        self.offset = end;
        unsafe { self.buf.as_mut_ptr().add(aligned) }
    }

    /// Instant bulk free: reuse the whole buffer.
    pub fn reset(&mut self) {
        self.offset = 0;
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

/// Recycles released 4 KiB arena slabs so sequential regions do not allocate fresh pages.
///
/// Nested live regions still hold distinct arenas; when a region exits, `release`
/// returns its slab to the free list for the next `acquire`.
pub struct ArenaPool {
    free: Vec<Box<Arena>>,
}

impl ArenaPool {
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

    pub fn free_len(&self) -> usize {
        self.free.len()
    }

    /// Take a reset arena from the pool, or allocate a new one.
    pub fn acquire(&mut self) -> Box<Arena> {
        match self.free.pop() {
            Some(mut arena) => {
                arena.reset();
                arena
            }
            None => Box::new(Arena::new()),
        }
    }

    /// Return an arena slab to the pool after the region exits.
    pub fn release(&mut self, mut arena: Box<Arena>) {
        arena.reset();
        self.free.push(arena);
    }
}

impl Default for ArenaPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_allocates_and_advances_offset() {
        let mut arena = Arena::new();
        let p1 = arena.alloc(8, 1);
        assert_eq!(arena.offset(), 8);
        let p2 = arena.alloc(8, 1);
        assert_eq!(arena.offset(), 16);
        assert_ne!(p1, p2);
    }

    #[test]
    fn reset_reclaims_all() {
        let mut arena = Arena::new();
        arena.alloc(100, 1);
        arena.reset();
        assert_eq!(arena.offset(), 0);
        arena.alloc(ARENA_CAPACITY, 1);
        assert_eq!(arena.offset(), ARENA_CAPACITY);
    }

    #[test]
    #[should_panic(expected = "arena overflow")]
    fn overflow_panics() {
        let mut arena = Arena::new();
        arena.alloc(ARENA_CAPACITY, 1);
        arena.alloc(1, 1);
    }

    #[test]
    fn respects_alignment() {
        let mut arena = Arena::new();
        arena.alloc(1, 1);
        let p = arena.alloc(8, 8);
        assert_eq!(p as usize % 8, 0);
        assert_eq!(arena.offset(), 16);
    }

    #[test]
    fn pool_reuses_released_slab() {
        let mut pool = ArenaPool::new();
        let mut a = pool.acquire();
        a.alloc(64, 1);
        let ptr = a.buf.as_ptr();
        pool.release(a);
        assert_eq!(pool.free_len(), 1);

        let b = pool.acquire();
        assert_eq!(pool.free_len(), 0);
        assert_eq!(b.offset(), 0);
        assert_eq!(b.buf.as_ptr(), ptr);
    }

    #[test]
    fn pool_keeps_nested_arenas_distinct() {
        let mut pool = ArenaPool::new();
        let outer = pool.acquire();
        let inner = pool.acquire();
        assert_ne!(outer.buf.as_ptr(), inner.buf.as_ptr());
        pool.release(inner);
        pool.release(outer);
        assert_eq!(pool.free_len(), 2);
    }
}
