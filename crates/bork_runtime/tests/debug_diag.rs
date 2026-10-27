#![cfg(all(feature = "debug-runtime", unix))]

use std::{
    ffi::{c_char, c_void},
    os::unix::process::ExitStatusExt,
    process::Command,
    ptr,
};

#[repr(C)]
struct Site {
    path: *const c_char,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

extern "C" {
    fn bork_arena_push() -> *mut c_void;
    fn bork_arena_alloc(a: *mut c_void, size: usize, align: usize) -> *mut c_void;
    fn bork_arena_pop(a: *mut c_void);
    fn bork_arena_reset(a: *mut c_void) -> *mut c_void;
    fn bork_arena_free(a: *mut c_void, p: *mut c_void, size: usize);
    fn bork_arena_grow(
        a: *mut c_void,
        p: *mut c_void,
        old: usize,
        new: usize,
        align: usize,
    ) -> *mut c_void;
    fn bork_arena_register_drop(
        a: *mut c_void,
        p: *mut c_void,
        callback: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn bork_debug_check(p: *const c_void, size: usize, op: u8, site: *const Site);
    fn bork_debug_check_handle(a: *const c_void, p: *const c_void, site: *const Site);
    fn bork_debug_label_allocation(
        p: *const c_void,
        name: *const c_char,
        ty: *const c_char,
        site: *const Site,
        declaration: u8,
    );
    fn bork_debug_site(site: *const Site);
    fn bork_debug_frame_enter(name: *const c_char, site: *const Site);
    fn bork_debug_copy_origin(source: *const c_void, destination: *const c_void);
    fn bork_ref_is_none_ptr(p: *const c_void) -> bool;
    fn bork_ref_release_observation_ptr(p: *const c_void);
    fn bork_vec_drop_refs(p: *mut c_void);
    fn bork_string_hash(p: *const u8, size: usize) -> u64;
}

unsafe extern "C" fn cleanup(p: *mut c_void) {
    let callback = Site {
        path: c"callback.bork".as_ptr(),
        start: 90,
        end: 99,
        line: 9,
        column: 1,
    };
    bork_debug_site(&callback);
    bork_debug_check(p, 64, 0, &callback);
}

unsafe fn child(case: &str) {
    let site = Site {
        path: c"fixture.bork".as_ptr(),
        start: 12,
        end: 27,
        line: 2,
        column: 5,
    };
    bork_debug_frame_enter(c"main".as_ptr(), &site);
    for _ in 0..3 {
        bork_debug_frame_enter(c"recursive".as_ptr(), &site);
    }
    let arena = bork_arena_push();
    let p = bork_arena_alloc(arena, 64, 8);
    bork_debug_label_allocation(p, c"item".as_ptr(), c"Node<i32>".as_ptr(), &site, 1);
    let release = Site {
        path: c"fixture.bork".as_ptr(),
        start: 50,
        end: 58,
        line: 4,
        column: 1,
    };
    bork_debug_site(&release);
    match case {
        "pop" => bork_arena_pop(arena),
        "reset" => {
            bork_arena_reset(arena);
        }
        "destructor" => {
            bork_arena_register_drop(arena, p, Some(cleanup));
            bork_arena_pop(arena);
        }
        "grow" => {
            bork_arena_alloc(arena, 64, 8);
            bork_arena_grow(arena, p, 64, 256, 8);
        }
        "handle" => {
            bork_arena_pop(arena);
            let next = bork_arena_push();
            assert_eq!(p, bork_arena_alloc(next, 64, 8));
            bork_debug_check_handle(arena, p, &site);
            return;
        }
        "promotion" => {
            let next = bork_arena_push();
            let q = bork_arena_alloc(next, 64, 8);
            bork_debug_copy_origin(p, q);
            bork_arena_pop(arena);
            bork_arena_pop(next);
            bork_debug_check(q, 1, 0, &site);
            return;
        }
        _ => bork_arena_free(arena, p, 64),
    }
    match case {
        "ref" => {
            bork_ref_is_none_ptr(p);
        }
        "observation" => bork_ref_release_observation_ptr(p),
        "vec" => bork_vec_drop_refs(p),
        "string" => {
            bork_string_hash(p.cast(), 64);
        }
        "print" => bork_runtime::bork_print_str(p.cast(), 64),
        _ => bork_debug_check(p, 64, u8::from(case == "write"), &site),
    }
}

#[test]
fn invalid_runtime_access_aborts_before_payload_read() {
    std::hint::black_box(bork_runtime::bork_print_i64 as *const ());
    if let Ok(case) = std::env::var("BORK_DIAG_CASE") {
        unsafe {
            child(&case);
        }
        return;
    }
    for case in [
        "read",
        "write",
        "pop",
        "reset",
        "grow",
        "handle",
        "ref",
        "observation",
        "vec",
        "string",
        "print",
        "promotion",
        "destructor",
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "invalid_runtime_access_aborts_before_payload_read",
            ])
            .env("BORK_DIAG_CASE", case)
            .output()
            .unwrap();
        assert_eq!(output.status.signal(), Some(6), "{case}: {:?}", output);
        let report = String::from_utf8_lossy(&output.stderr);
        for expected in [
            "Bork DebugDiag",
            "item",
            "Node<i32>",
            "fixture.bork:2:5 [bytes 12..27]",
            "fixture.bork:4:1 [bytes 50..58]",
            "arena chain:",
            "Bork call stack:",
        ] {
            assert!(
                report.contains(expected),
                "{case}: missing {expected}: {report}"
            );
        }
        assert_eq!(
            report.matches("recursive at").count(),
            3,
            "{case}: {report}"
        );
        if case == "handle" {
            assert!(report.contains("previous occupancy (history)"), "{report}");
        }
        if case == "destructor" {
            assert!(report.contains("released: fixture.bork:4:1"), "{report}");
        }
    }
}

#[test]
fn zero_length_access_does_not_read_payload() {
    unsafe {
        bork_debug_check(ptr::null(), 0, 0, ptr::null());
        bork_debug_check(ptr::without_provenance(usize::MAX), 0, 1, ptr::null());
    }
}
