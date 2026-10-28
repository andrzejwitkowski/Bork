use super::diag::{self, MemoryState};
use crate::arena::*;

fn state(ptr: *mut std::ffi::c_void, size: usize) -> Option<MemoryState> {
    diag::failure(ptr as usize, size, 0).map(|d| d.current.state)
}

#[test]
fn free_and_partial_reuse_validate_the_whole_range() {
    unsafe {
        let arena = bork_arena_push();
        let p = bork_arena_alloc(arena, 128, 8);
        assert_eq!(state(p, 128), None);
        bork_arena_free(arena, p, 128);
        assert_eq!(state(p, 1), Some(MemoryState::Freed));
        let q = bork_arena_alloc(arena, 16, 8);
        assert_eq!(p, q);
        assert_eq!(state(q, 16), None);
        assert!(state(q, 17).is_some());
        assert_eq!(state(p.byte_add(64), 1), Some(MemoryState::Freed));
        assert!(diag::failure(p.addr(), usize::MAX, 0).is_some());
        assert_eq!(state(std::ptr::null_mut(), 0), None);
        bork_arena_pop(arena);
    }
}

#[test]
fn recycling_arena_does_not_revive_payload_and_keeps_old_owner() {
    unsafe {
        let first = bork_arena_push();
        let p = bork_arena_alloc(first, 128, 8);
        bork_arena_pop(first);
        assert_eq!(state(p, 1), Some(MemoryState::UnusedInPool));
        let next = bork_arena_push();
        assert_eq!(state(p, 1), Some(MemoryState::UnusedInPool));
        let q = bork_arena_alloc(next, 16, 8);
        assert_eq!(p, q);
        assert_eq!(state(q, 16), None);
        assert!(diag::failure(q.addr(), 1, first.addr()).is_some());
        assert_eq!(
            diag::failure(p.byte_add(64).addr(), 1, 0)
                .unwrap()
                .current
                .owner,
            first.addr()
        );
        bork_arena_pop(next);
    }
}

#[test]
fn reset_and_moving_grow_retire_old_ranges_but_keep_provenance() {
    unsafe {
        let a = bork_arena_push();
        let p = bork_arena_alloc(a, 32, 8);
        diag::label(
            p.addr(),
            "numbers".into(),
            "[i32; 8]".into(),
            diag::Location::default(),
            false,
        );
        bork_arena_alloc(a, 32, 8);
        let q = bork_arena_grow(a, p, 32, 128, 8);
        assert_ne!(p, q);
        assert_eq!(state(p, 1), Some(MemoryState::Freed));
        assert_eq!(state(q, 128), None);
        let next = bork_arena_reset(a);
        let d = diag::failure(q.addr(), 1, 0).unwrap();
        assert_eq!(d.current.state, MemoryState::UnusedInPool);
        assert_eq!(d.current.name, "numbers");
        assert_eq!(d.current.type_name, "[i32; 8]");
        bork_arena_pop(next);
    }
}

#[test]
fn in_place_grow_extends_only_the_live_allocation() {
    unsafe {
        let a = bork_arena_push();
        let p = bork_arena_alloc(a, 32, 8);
        let q = bork_arena_grow(a, p, 32, 128, 8);
        assert_eq!(p, q);
        assert_eq!(state(p, 128), None);
        assert!(state(p, 129).is_some());
        bork_arena_pop(a);
    }
}

#[test]
fn allocations_in_older_pages_and_large_blocks_keep_their_bounds() {
    unsafe {
        let arena = bork_arena_push();
        bork_arena_alloc(arena, 2000, 8);
        bork_arena_alloc(arena, 2000, 8);
        bork_arena_alloc(arena, 2000, 8);
        bork_arena_alloc(arena, 2000, 8);
        bork_arena_alloc(arena, 80, 8);
        let older = bork_arena_alloc(arena, 64, 8);
        let large = bork_arena_alloc(arena, 8192, 8);
        assert_eq!(state(older, 64), None);
        assert!(state(older, 65).is_some());
        assert_eq!(state(large, 8192), None);
        let grown = bork_arena_grow(arena, large, 8192, 16384, 8);
        assert_eq!(state(grown, 16384), None);
        bork_arena_pop(arena);
        assert_eq!(state(older, 64), Some(MemoryState::UnusedInPool));
        assert_eq!(state(grown, 16384), Some(MemoryState::UnusedInPool));
    }
}

unsafe extern "C" fn checked_drop(p: *mut std::ffi::c_void) {
    assert!(diag::failure(p.addr(), 8, 0).is_none());
}

#[test]
fn destructor_reads_are_live_and_release_site_survives_callbacks() {
    unsafe {
        let a = bork_arena_push();
        let p = bork_arena_alloc(a, 8, 8);
        bork_arena_register_drop(a, p, Some(checked_drop));
        bork_arena_pop(a);
        assert_eq!(state(p, 8), Some(MemoryState::UnusedInPool));
    }
}

#[test]
fn history_keeps_exactly_one_previous_occupant() {
    unsafe {
        let a = bork_arena_push();
        for name in ["first", "second", "third"] {
            let p = bork_arena_alloc(a, 8, 8);
            diag::label(
                p.addr(),
                name.into(),
                "i64".into(),
                diag::Location::default(),
                false,
            );
            bork_arena_free(a, p, 8);
            let failure = diag::failure(p.addr(), 1, 0).unwrap();
            if name == "third" {
                assert_eq!(failure.previous.unwrap().name, "second");
            }
        }
        bork_arena_pop(a);
    }
}

#[test]
fn stack_and_constants_pass_without_becoming_arena_allocations() {
    let local = 42u64;
    assert!(diag::failure(&local as *const _ as usize, 8, 0).is_none());
    assert!(diag::failure(b"constant".as_ptr() as usize, 8, 0).is_none());
}

#[test]
fn zero_sized_objects_have_live_identity_without_readable_bytes() {
    unsafe {
        let arena = bork_arena_push();
        let p = bork_arena_alloc(arena, 0, 1);
        assert!(diag::failure(p.addr(), 1, arena.addr()).is_none());
        assert!(diag::failure(p.addr(), 1, 0).is_some());
        bork_arena_pop(arena);
        assert!(diag::failure(p.addr(), 1, arena.addr()).is_some());
    }
}

#[test]
fn larger_reuse_reports_the_previous_occupant_of_the_accessed_bytes() {
    unsafe {
        let a = bork_arena_push();
        let first = bork_arena_alloc(a, 32, 8);
        diag::label(
            first.addr(),
            "first".into(),
            "i64".into(),
            diag::Location::default(),
            false,
        );
        let second = bork_arena_alloc(a, 32, 8);
        diag::label(
            second.addr(),
            "second".into(),
            "i64".into(),
            diag::Location::default(),
            false,
        );
        let a = bork_arena_reset(a);
        let combined = bork_arena_alloc(a, 64, 8);
        assert_eq!(first, combined);
        assert_eq!(state(combined, 64), None);
        bork_arena_free(a, combined, 64);
        let failure = diag::failure(combined.byte_add(40).addr(), 1, 0).unwrap();
        assert_eq!(failure.previous.unwrap().name, "second");
        bork_arena_pop(a);
    }
}
