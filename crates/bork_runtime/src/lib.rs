use std::ffi::c_void;
use std::io::{self, Write};
use std::slice;
use std::sync::{Mutex, OnceLock};

const ARENA_CAPACITY: usize = 4096;

struct Arena {
    buf: [u8; ARENA_CAPACITY],
    offset: usize,
}

impl Arena {
    fn new() -> Self {
        Self {
            buf: [0; ARENA_CAPACITY],
            offset: 0,
        }
    }

    fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        assert!(align.is_power_of_two(), "align must be a power of two");
        let base = self.buf.as_mut_ptr() as usize;
        let addr = base
            .checked_add(self.offset)
            .expect("allocation address overflow");
        let aligned_addr = addr
            .checked_add(align - 1)
            .expect("aligned address overflow")
            & !(align - 1);
        let aligned = aligned_addr
            .checked_sub(base)
            .expect("aligned offset underflow");
        let end = aligned.checked_add(size).expect("allocation size overflow");
        if end > ARENA_CAPACITY {
            panic!("arena overflow: need {end} bytes, capacity {ARENA_CAPACITY}");
        }
        self.offset = end;
        aligned_addr as *mut u8
    }

    fn reset(&mut self) {
        self.offset = 0;
    }
}

#[derive(Default)]
struct ArenaPool {
    free: Vec<Box<Arena>>,
}

impl ArenaPool {
    fn acquire(&mut self) -> Box<Arena> {
        self.free.pop().unwrap_or_else(|| Box::new(Arena::new()))
    }

    fn release(&mut self, mut arena: Box<Arena>) {
        arena.reset();
        self.free.push(arena);
    }
}

fn pool() -> &'static Mutex<ArenaPool> {
    static POOL: OnceLock<Mutex<ArenaPool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(ArenaPool::default()))
}

unsafe fn arena_mut<'a>(arena: *mut c_void) -> &'a mut Arena {
    assert!(!arena.is_null(), "arena handle must not be null");
    // SAFETY: Arena handles are created by `bork_arena_push` and remain owned
    // by the caller until exactly one matching `bork_arena_pop`.
    unsafe { &mut *arena.cast::<Arena>() }
}

#[no_mangle]
pub extern "C" fn bork_arena_push() -> *mut c_void {
    let arena = pool().lock().unwrap().acquire();
    Box::into_raw(arena).cast()
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_reset(arena: *mut c_void) {
    // SAFETY: The caller supplies a live, uniquely borrowed arena handle.
    unsafe { arena_mut(arena) }.reset();
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_pop(arena: *mut c_void) {
    assert!(!arena.is_null(), "arena handle must not be null");
    // SAFETY: The handle came from `bork_arena_push`, and ownership is
    // transferred back exactly once by this call.
    let arena = unsafe { Box::from_raw(arena.cast::<Arena>()) };
    pool().lock().unwrap().release(arena);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_alloc(
    arena: *mut c_void,
    size: usize,
    align: usize,
) -> *mut c_void {
    // SAFETY: The caller supplies a live, uniquely borrowed arena handle.
    unsafe { arena_mut(arena) }.alloc(size, align).cast()
}

#[no_mangle]
pub extern "C" fn bork_print_i64(value: i64) {
    let mut stdout = io::stdout().lock();
    let _ = write!(stdout, "{value}");
    let _ = stdout.flush();
}

#[no_mangle]
pub unsafe extern "C" fn bork_print_str(ptr: *const u8, len: usize) {
    // SAFETY: The caller supplies `len` readable bytes at `ptr`.
    unsafe { write_bytes(ptr, len, false) };
}

#[no_mangle]
pub extern "C" fn bork_println_i64(value: i64) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}

#[no_mangle]
pub unsafe extern "C" fn bork_println_str(ptr: *const u8, len: usize) {
    // SAFETY: The caller supplies `len` readable bytes at `ptr`.
    unsafe { write_bytes(ptr, len, true) };
}

unsafe fn write_bytes(ptr: *const u8, len: usize, newline: bool) {
    let mut stdout = io::stdout().lock();
    if len == 0 {
        if newline {
            let _ = writeln!(stdout);
        }
        let _ = stdout.flush();
        return;
    }
    assert!(!ptr.is_null(), "string pointer must not be null");
    // SAFETY: The caller guarantees that `ptr` references `len` readable bytes.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let _ = stdout.write_all(bytes);
    if newline {
        let _ = stdout.write_all(b"\n");
    }
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_advances_and_respects_alignment() {
        let arena = bork_arena_push();
        // SAFETY: `arena` is live until the matching pop below.
        let (first, second) =
            unsafe { (bork_arena_alloc(arena, 3, 1), bork_arena_alloc(arena, 8, 8)) };
        assert_ne!(first, second);
        assert_eq!(second as usize % 8, 0);
        // SAFETY: This is the only pop of the live handle.
        unsafe { bork_arena_pop(arena) };
    }

    #[test]
    fn reset_reclaims_allocations() {
        let arena = bork_arena_push();
        // SAFETY: `arena` is live and accessed sequentially.
        let (before, after) = unsafe {
            let before = bork_arena_alloc(arena, 64, 8);
            bork_arena_reset(arena);
            let after = bork_arena_alloc(arena, 64, 8);
            (before, after)
        };
        assert_eq!(before, after);
        // SAFETY: This is the only pop of the live handle.
        unsafe { bork_arena_pop(arena) };
    }

    #[test]
    #[should_panic(expected = "arena overflow")]
    fn overflow_panics() {
        let mut arena = Arena::new();
        arena.alloc(ARENA_CAPACITY, 1);
        arena.alloc(1, 1);
    }
}
