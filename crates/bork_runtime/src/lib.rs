#![allow(clippy::missing_safety_doc)]

mod arena;
mod refs;

use std::io::{self, Write};
use std::slice;

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
    use std::ffi::c_void;

    use super::arena::*;
    use super::refs::*;

    unsafe extern "C" fn drop_ref_slot(value: *mut c_void) {
        unsafe { bork_ref_drop(value.cast()) };
    }

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
            let arena = bork_arena_reset(arena);
            let after = bork_arena_alloc(arena, 64, 8);
            bork_arena_pop(arena);
            (before, after)
        };
        assert_eq!(before, after);
    }

    #[test]
    fn arena_ids_are_monotonic_and_follow_the_parent_tree() {
        let root = bork_arena_root();
        let parent = unsafe { bork_arena_push_child(root) };
        let child = unsafe { bork_arena_push_child(parent) };
        assert!(unsafe { bork_arena_id(root) } < unsafe { bork_arena_id(parent) });
        assert!(unsafe { bork_arena_id(parent) } < unsafe { bork_arena_id(child) });
        assert_eq!(unsafe { bork_arena_depth(child) }, unsafe {
            bork_arena_depth(parent)
        } + 1);
        unsafe {
            bork_arena_pop(child);
            bork_arena_pop(parent);
        }
    }

    #[test]
    fn arena_handles_are_non_dereferenced_id_tokens() {
        let root = bork_arena_root();
        let arena = unsafe { bork_arena_create_dynamic(root, 0) };
        assert_eq!(root.addr(), unsafe { bork_arena_id(root) } as usize);
        assert_eq!(arena.addr(), unsafe { bork_arena_id(arena) } as usize);
        unsafe {
            bork_arena_release_strong(arena);
        }
    }

    #[test]
    #[should_panic(expected = "unknown arena handle")]
    fn released_metadata_rejects_stale_arena_tokens() {
        let root = bork_arena_root();
        let arena = unsafe { bork_arena_create_dynamic(root, 0) };
        unsafe {
            bork_arena_retain_weak(arena);
            bork_arena_release_strong(arena);
            bork_arena_release_weak(arena);
        }
        state_of(arena);
    }

    #[test]
    fn recycled_payload_and_reset_get_fresh_generation_ids() {
        let first = bork_arena_push();
        let first_id = unsafe { bork_arena_id(first) };
        let first_payload = unsafe { bork_arena_alloc(first, 8, 8) };
        unsafe { bork_arena_pop(first) };

        let second = bork_arena_push();
        let second_id = unsafe { bork_arena_id(second) };
        let second_payload = unsafe { bork_arena_alloc(second, 8, 8) };
        assert!(first_id < second_id);
        assert_eq!(first_payload, second_payload, "test expects pool reuse");

        let reset = unsafe { bork_arena_reset(second) };
        assert!(second_id < unsafe { bork_arena_id(reset) });
        unsafe { bork_arena_pop(reset) };
    }

    #[test]
    fn classifier_and_arena_level_rc_cover_strong_weak_and_pinning() {
        let root = bork_arena_root();
        let older = unsafe { bork_arena_create_dynamic(root, 0) };
        let newer = unsafe { bork_arena_create_dynamic(root, 0) };
        assert_eq!(
            unsafe { bork_arena_classify(root, older) },
            RefKind::Strong
        );
        assert_eq!(
            unsafe { bork_arena_classify(newer, older) },
            RefKind::Weak
        );

        unsafe { bork_arena_retain_weak(older) };
        assert!(unsafe { bork_arena_weak_upgrade(older) });
        assert_eq!(unsafe { bork_arena_strong_count(older) }, 2);
        unsafe { bork_arena_release_strong(older) };
        assert_eq!(unsafe { bork_arena_state(older) }, ArenaState::Alive);
        unsafe { bork_arena_release_strong(older) };
        assert_eq!(unsafe { bork_arena_state(older) }, ArenaState::Dead);
        assert!(!unsafe { bork_arena_weak_upgrade(older) });
        unsafe {
            bork_arena_release_weak(older);
            bork_arena_release_strong(newer);
        }
    }

    #[test]
    fn ref_move_has_no_rc_and_overwrite_releases_the_old_value() {
        let root = bork_arena_root();
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let mut original = unsafe { bork_ref_create(root, target, object) };
        assert_eq!(original.kind, RefKind::Strong);
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 2);

        let mut moved = unsafe { bork_ref_move(&mut original) };
        assert_eq!(original.kind, RefKind::None);
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 2);

        unsafe { bork_ref_store(&mut moved, root, RefHandle::none()) };
        assert_eq!(moved.kind, RefKind::None);
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 1);
        unsafe { bork_arena_release_strong(target) };
    }

    #[test]
    fn expired_weak_observes_as_none_and_live_observation_pins_payload() {
        let root = bork_arena_root();
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        let newer_source = unsafe { bork_arena_create_dynamic(root, 0) };
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let mut weak = unsafe { bork_ref_create(newer_source, target, object) };
        assert_eq!(weak.kind, RefKind::Weak);

        let observation = unsafe { bork_ref_observe(weak) };
        assert_eq!(observation.object, object);
        unsafe { bork_arena_release_strong(target) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Alive);
        unsafe { bork_ref_release_observation(observation) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Dead);

        let expired = unsafe { bork_ref_observe(weak) };
        assert!(expired.object.is_null());
        assert!(unsafe { bork_ref_equal(weak, RefHandle::none()) });
        unsafe {
            bork_ref_drop(&mut weak);
            bork_arena_release_strong(newer_source);
        }
    }

    #[test]
    fn observation_pin_survives_temporary_ref_drop() {
        let root = bork_arena_root();
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let handle = unsafe { bork_ref_create(root, target, object) };
        unsafe { bork_arena_retain_weak(target) };
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 2);

        let observation = unsafe { bork_ref_observe(handle) };
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 3);
        unsafe { bork_ref_drop_value(handle) };
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 2);

        unsafe { bork_arena_release_strong(target) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Alive);
        unsafe { bork_ref_release_observation(observation) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Dead);
        unsafe { bork_arena_release_weak(target) };
    }

    #[test]
    fn weak_observation_pin_survives_temporary_ref_drop() {
        let root = bork_arena_root();
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        let source = unsafe { bork_arena_create_dynamic(root, 0) };
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let weak = unsafe { bork_ref_create(source, target, object) };
        unsafe { bork_arena_retain_weak(target) };
        assert_eq!(weak.kind, RefKind::Weak);

        let observation = unsafe { bork_ref_observe(weak) };
        assert_eq!(unsafe { bork_arena_strong_count(target) }, 2);
        unsafe { bork_ref_drop_value(weak) };
        unsafe { bork_arena_release_strong(target) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Alive);
        unsafe { bork_ref_release_observation(observation) };
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Dead);
        unsafe {
            bork_arena_release_weak(target);
            bork_arena_release_strong(source);
        }
    }

    #[test]
    fn arena_destruction_drops_managed_fields_before_recycling_payload() {
        let root = bork_arena_root();
        let owner = unsafe { bork_arena_create_dynamic(root, 0) };
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        unsafe {
            bork_arena_retain_weak(owner);
            bork_arena_retain_weak(target);
        }
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let slot = unsafe { bork_arena_alloc(owner, size_of::<RefHandle>(), align_of::<RefHandle>()) }
            .cast::<RefHandle>();
        unsafe {
            slot.write(bork_ref_create(owner, target, object));
            bork_arena_register_drop(owner, slot.cast(), Some(drop_ref_slot));
            bork_arena_release_strong(target);
        }
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Alive);

        unsafe { bork_arena_release_strong(owner) };
        assert_eq!(unsafe { bork_arena_state(owner) }, ArenaState::Dead);
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Dead);
        unsafe {
            bork_arena_release_weak(owner);
            bork_arena_release_weak(target);
        }
    }

    #[test]
    fn cache_graph_drops_dynamic_arenas_but_keeps_domain_root() {
        let domain = bork_arena_root();
        let cache = unsafe { bork_arena_create_dynamic(domain, 0) };
        let root_node = unsafe { bork_arena_create_dynamic(domain, 0) };
        let child_node = unsafe { bork_arena_create_dynamic(domain, 0) };
        for arena in [cache, root_node, child_node] {
            unsafe { bork_arena_retain_weak(arena) };
        }

        unsafe fn register_ref(
            owner: *mut c_void,
            target: *mut c_void,
            object: *mut c_void,
        ) -> RefKind {
            let slot = bork_arena_alloc(owner, size_of::<RefHandle>(), align_of::<RefHandle>())
                .cast::<RefHandle>();
            let reference = bork_ref_create(owner, target, object);
            let kind = reference.kind;
            slot.write(reference);
            bork_arena_register_drop(owner, slot.cast(), Some(drop_ref_slot));
            kind
        }

        let root_object = unsafe { bork_arena_alloc(root_node, 1, 1) };
        let child_object = unsafe { bork_arena_alloc(child_node, 1, 1) };
        let cache_object = unsafe { bork_arena_alloc(cache, 1, 1) };
        let cache_ref = unsafe { bork_ref_create(domain, cache, cache_object) };
        assert_eq!(cache_ref.kind, RefKind::Strong);
        unsafe {
            // Cache owns strong root/hot edges; root owns a strong child edge.
            assert_eq!(
                register_ref(cache, root_node, root_object),
                RefKind::Strong
            );
            assert_eq!(
                register_ref(cache, child_node, child_object),
                RefKind::Strong
            );
            assert_eq!(
                register_ref(root_node, child_node, child_object),
                RefKind::Strong
            );
            // The child-to-parent back edge is weak because child_node is newer.
            assert_eq!(
                register_ref(child_node, root_node, root_object),
                RefKind::Weak
            );
        }

        unsafe {
            bork_ref_drop_value(cache_ref);
            bork_arena_release_strong(cache);
            bork_arena_release_strong(root_node);
            bork_arena_release_strong(child_node);
        }
        assert_eq!(unsafe { bork_arena_state(cache) }, ArenaState::Dead);
        assert_eq!(unsafe { bork_arena_state(root_node) }, ArenaState::Dead);
        assert_eq!(unsafe { bork_arena_state(child_node) }, ArenaState::Dead);
        assert_eq!(unsafe { bork_arena_state(domain) }, ArenaState::Alive);

        unsafe {
            bork_arena_release_weak(cache);
            bork_arena_release_weak(root_node);
            bork_arena_release_weak(child_node);
        }
        assert_eq!(control_count(), 1, "only the domain root may remain");
    }

    #[test]
    fn ref_handle_matches_the_stable_c_layout() {
        assert_eq!(size_of::<RefHandle>(), 24);
        assert_eq!(align_of::<RefHandle>(), 8);
        assert_eq!(size_of::<RefKind>(), 1);
    }

    #[test]
    fn self_store_keeps_the_last_owner_alive() {
        let root = bork_arena_root();
        let target = unsafe { bork_arena_create_dynamic(root, 0) };
        let object = unsafe { bork_arena_alloc(target, 8, 8) };
        let mut slot = unsafe { bork_ref_create(root, target, object) };
        let before = unsafe { bork_arena_strong_count(target) };
        let incoming = slot;
        unsafe { bork_ref_store(&mut slot, root, incoming) };
        assert_eq!(unsafe { bork_arena_strong_count(target) }, before);
        assert_eq!(unsafe { bork_arena_state(target) }, ArenaState::Alive);
        assert!(!unsafe { bork_ref_is_none(slot) });
        unsafe {
            bork_ref_drop(&mut slot);
            bork_arena_release_strong(target);
        }
    }

    #[test]
    fn same_arena_ref_is_arena_kind_without_extra_rc() {
        let root = bork_arena_root();
        let arena = unsafe { bork_arena_create_dynamic(root, 0) };
        unsafe { bork_arena_retain_weak(arena) };
        let before = unsafe { bork_arena_strong_count(arena) };
        let object = unsafe { bork_arena_alloc(arena, 8, 8) };
        let handle = unsafe { bork_ref_create(arena, arena, object) };
        assert_eq!(handle.kind, RefKind::Arena);
        assert_eq!(unsafe { bork_arena_strong_count(arena) }, before);
        unsafe {
            bork_ref_drop_value(handle);
            bork_arena_release_strong(arena);
        }
        assert_eq!(unsafe { bork_arena_state(arena) }, ArenaState::Dead);
        unsafe { bork_arena_release_weak(arena) };
    }

    #[test]
    fn scoped_child_borrow_does_not_outlive_its_lexical_parent() {
        let parent = bork_arena_push();
        let child = unsafe { bork_arena_create_dynamic(parent, 1) };
        unsafe { bork_arena_retain_weak(child) };
        assert_eq!(
            unsafe { bork_arena_classify(child, parent) },
            RefKind::Borrow
        );
        assert_eq!(
            unsafe { bork_arena_classify(parent, child) },
            RefKind::Strong
        );
        assert!(unsafe { bork_arena_id(parent) } < unsafe { bork_arena_id(child) });
        unsafe { bork_arena_release_strong(child) };
        assert_eq!(unsafe { bork_arena_state(child) }, ArenaState::Dead);
        unsafe { bork_arena_release_weak(child) };
        unsafe { bork_arena_pop(parent) };
    }

    #[test]
    fn scoped_child_registered_root_drops_before_parent_recycle() {
        let parent = bork_arena_push();
        let child = unsafe { bork_arena_create_dynamic(parent, 1) };
        unsafe {
            bork_arena_retain_weak(child);
            bork_arena_register_strong_root(parent, child);
            bork_arena_pop(parent);
        }
        assert_eq!(unsafe { bork_arena_state(child) }, ArenaState::Dead);
        unsafe { bork_arena_release_weak(child) };
    }

    #[test]
    fn scoped_child_registered_root_drops_before_parent_reset() {
        let parent = bork_arena_push();
        let child = unsafe { bork_arena_create_dynamic(parent, 1) };
        unsafe {
            bork_arena_retain_weak(child);
            bork_arena_register_strong_root(parent, child);
        }
        let replacement = unsafe { bork_arena_reset(parent) };
        assert_eq!(unsafe { bork_arena_state(child) }, ArenaState::Dead);
        unsafe {
            bork_arena_release_weak(child);
            bork_arena_pop(replacement);
        }
    }

    #[test]
    fn sibling_edges_follow_creation_order_and_drop_with_roots() {
        let root = bork_arena_root();
        let mut arenas = Vec::new();
        for _ in 0..6 {
            arenas.push(unsafe { bork_arena_create_dynamic(root, 0) });
        }
        for arena in &arenas {
            unsafe { bork_arena_retain_weak(*arena) };
        }
        for i in 0..arenas.len() {
            for j in 0..arenas.len() {
                let kind = unsafe { bork_arena_classify(arenas[i], arenas[j]) };
                let expected = if i == j {
                    RefKind::Arena
                } else if i < j {
                    RefKind::Strong
                } else {
                    RefKind::Weak
                };
                assert_eq!(kind, expected);
            }
        }
        for arena in &arenas {
            let object = unsafe { bork_arena_alloc(*arena, 8, 8) };
            let handle = unsafe { bork_ref_create(*arena, *arena, object) };
            assert_eq!(handle.kind, RefKind::Arena);
            unsafe { bork_ref_drop_value(handle) };
        }
        for arena in arenas {
            unsafe { bork_arena_release_strong(arena) };
            assert_eq!(unsafe { bork_arena_state(arena) }, ArenaState::Dead);
            unsafe { bork_arena_release_weak(arena) };
        }
    }
}
