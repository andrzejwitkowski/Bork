use super::*;
use crate::arena::*;
use crate::refs::*;

#[test]
fn slot_labels_scopes_and_counts() {
    let owner = bork_arena_push();
    let target = unsafe { bork_arena_create_dynamic(std::ptr::null_mut(), 0) };
    let scope = bork_debug_scope_enter();
    let mut slot = unsafe { bork_ref_create(owner, target, target) };
    let count = unsafe { bork_arena_strong_count(target) };
    unsafe {
        bork_arena_register_ref_drop(owner, &mut slot);
        bork_debug_register_slot(
            owner,
            &mut slot,
            c"local".as_ptr(),
            c"Ref".as_ptr(),
            scope,
            usize::MAX,
        );
        bork_arena_register_ref_drop(owner, &mut slot);
    }
    DEBUG.with(|state| {
        let state = state.borrow();
        assert_eq!(state.header.sequence % 2, 0);
        assert_eq!(state.slots.len(), 1);
        assert_eq!(state.slots[0].label, c"local".as_ptr() as usize);
    });
    assert_eq!(unsafe { bork_arena_strong_count(target) }, count);
    // Avoid duplicate runtime drops from registering twice; both see a valid None slot.
    unsafe {
        bork_ref_drop(&mut slot);
    }
    DEBUG.with(|state| assert_eq!(state.borrow().slots.len(), 1));
    bork_debug_scope_exit(scope);
    DEBUG.with(|state| assert!(state.borrow().slots.is_empty()));
    unsafe {
        bork_arena_pop(owner);
        bork_arena_release_strong(target);
    }
}

#[test]
fn alias_labels_preserve_slot_ownership_and_do_not_register_absent_slots() {
    let owner = bork_arena_push();
    let target = unsafe { bork_arena_create_dynamic(std::ptr::null_mut(), 0) };
    let scope = bork_debug_scope_enter();
    let mut slot = unsafe { bork_ref_create(owner, target, target) };
    let mut absent = RefHandle::none();
    unsafe {
        bork_arena_register_ref_drop(owner, &mut slot);
        bork_debug_register_slot(
            owner,
            &mut slot,
            std::ptr::null(),
            std::ptr::null(),
            scope,
            usize::MAX,
        );
    }
    let count = unsafe { bork_arena_strong_count(target) };
    unsafe {
        bork_debug_label_slot(&mut slot, c"alias".as_ptr(), c"Ref".as_ptr(), 3);
        bork_debug_label_slot(&mut absent, c"absent".as_ptr(), std::ptr::null(), 4);
    }
    DEBUG.with(|state| {
        let state = state.borrow();
        assert_eq!(state.slots.len(), 1);
        let record = state.slots[0];
        assert_eq!((record.owner, record.scope), (owner as usize, scope));
        assert_eq!(
            (record.label, record.type_name, record.index),
            (c"alias".as_ptr() as usize, c"Ref".as_ptr() as usize, 3)
        );
        assert_eq!(state.header.sequence % 2, 0);
    });
    assert_eq!(unsafe { bork_arena_strong_count(target) }, count);
    bork_debug_scope_exit(scope);
    DEBUG.with(|state| assert!(state.borrow().slots.is_empty()));
    unsafe {
        bork_arena_pop(owner);
        bork_arena_release_strong(target);
    }
}

unsafe extern "C" fn inspect_destruction(_: *mut c_void) {
    DEBUG.with(|state| {
        let state = state.borrow();
        assert_eq!(state.header.sequence % 2, 1);
        assert!(state.slots.is_empty());
    });
    // Reentrant arena creation and destruction must not publish an even sequence.
    let temporary = bork_arena_push();
    unsafe {
        bork_arena_pop(temporary);
    }
    DEBUG.with(|state| assert_eq!(state.borrow().header.sequence % 2, 1));
}

#[test]
fn destruction_removes_slots_before_reentrant_callbacks() {
    let owner = bork_arena_push();
    let mut slot = RefHandle::none();
    unsafe {
        bork_arena_register_ref_drop(owner, &mut slot);
        bork_arena_register_drop(owner, owner, Some(inspect_destruction));
        bork_arena_pop(owner);
    }
    DEBUG.with(|state| assert_eq!(state.borrow().header.sequence % 2, 0));
}

#[test]
fn nested_mutations_and_abi_word_layout() {
    assert_eq!(
        std::mem::size_of::<DebugRegistry>(),
        6 * std::mem::size_of::<usize>()
    );
    assert_eq!(
        std::mem::size_of::<DebugArena>(),
        13 * std::mem::size_of::<usize>()
    );
    assert_eq!(
        std::mem::size_of::<DebugSlot>(),
        6 * std::mem::size_of::<usize>()
    );
    let outer = Mutation::begin();
    let inner = Mutation::begin();
    drop(inner);
    DEBUG.with(|state| assert_eq!(state.borrow().header.sequence % 2, 1));
    drop(outer);
    DEBUG.with(|state| assert_eq!(state.borrow().header.sequence % 2, 0));
}
