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
}
