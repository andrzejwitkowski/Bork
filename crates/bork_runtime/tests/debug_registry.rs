#![cfg(feature = "debug-runtime")]

use std::ffi::c_void;

extern "C" {
    fn bork_debug_abi_v1();
    fn bork_arena_push() -> *mut c_void;
    fn bork_arena_alloc(owner: *mut c_void, size: usize, align: usize) -> *mut c_void;
    fn bork_arena_pop(owner: *mut c_void);
    fn bork_arena_state(owner: *mut c_void) -> u8;
    static bork_debug_registry: [usize; 6];
}

#[test]
fn exported_header_publishes_real_flat_arrays() {
    // Retain the runtime rlib when exercising its C symbols directly.
    std::hint::black_box(bork_runtime::bork_print_i64 as *const ());
    let runtime_first = std::env::var_os("BORK_TEST_RUNTIME_TLS_FIRST").is_some();
    std::thread::spawn(move || unsafe {
        if runtime_first {
            // A read can initialize RUNTIME before the first debug mutation.
            assert_eq!(bork_arena_state(std::ptr::without_provenance_mut(1)), 0);
        }
        bork_debug_abi_v1();
        let owner = bork_arena_push();
        bork_arena_alloc(owner, 17, 1);
        let header = std::ptr::read_volatile(&raw const bork_debug_registry);
        assert_eq!(header[0], 1);
        assert_eq!(header[1] % 2, 0);
        let arenas = std::slice::from_raw_parts(header[2] as *const [usize; 13], header[3]);
        let record = arenas.iter().find(|a| a[0] == owner as usize).unwrap();
        assert_eq!(record[6], 0);
        assert_eq!(record[7], 1);
        assert_ne!(record[8], 0);
        assert_eq!(record[9], 17);
        assert_eq!(record[10], 4096);
        bork_arena_pop(owner);
        let after = std::ptr::read_volatile(&raw const bork_debug_registry);
        assert!(after[1] > header[1]);
        assert_eq!(after[1] % 2, 0);
        assert_eq!(after[3], 1);
        // Leave a live payload behind so RuntimeState teardown also matters.
        bork_arena_push();
    })
    .join()
    .unwrap();
    let after_exit = unsafe { std::ptr::read_volatile(&raw const bork_debug_registry) };
    assert_eq!(
        after_exit[1] % 2,
        1,
        "thread exit must invalidate publication"
    );
    assert_eq!(&after_exit[2..], &[0, 0, 0, 0]);
    if !runtime_first {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "exported_header_publishes_real_flat_arrays"])
            .env("BORK_TEST_RUNTIME_TLS_FIRST", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
}
