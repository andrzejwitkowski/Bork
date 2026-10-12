//! Persistent Ref slot registration, source labels and activation scope cleanup.
use super::super::{state_of, ArenaState};
use super::{
    abi::DebugSlot,
    registry::{Mutation, DEBUG},
};
use crate::refs::RefHandle;
use std::ffi::{c_char, c_void};

pub(crate) fn remove_owner(owner: usize) {
    DEBUG.with(|state| state.borrow_mut().slots.retain(|s| s.owner != owner));
}

pub(crate) fn register_generic(owner: *mut c_void, slot: *mut RefHandle) {
    let _mutation = Mutation::begin();
    DEBUG.with(|state| {
        let mut state = state.borrow_mut();
        if !state.slots.iter().any(|s| s.address == slot as usize) {
            state.slots.push(DebugSlot {
                owner: owner as usize,
                address: slot as usize,
                label: 0,
                type_name: 0,
                scope: 0,
                index: usize::MAX,
            });
        }
    });
}

#[no_mangle]
pub extern "C" fn bork_debug_scope_enter() -> usize {
    let _mutation = Mutation::begin();
    DEBUG.with(|state| {
        let mut state = state.borrow_mut();
        let scope = state.next_scope;
        state.next_scope = scope.checked_add(1).expect("debug scope overflow");
        scope
    })
}

#[no_mangle]
pub extern "C" fn bork_debug_scope_exit(scope: usize) {
    let _mutation = Mutation::begin();
    assert_ne!(scope, 0, "debug local scope must be nonzero");
    DEBUG.with(|state| state.borrow_mut().slots.retain(|s| s.scope != scope));
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_register_slot(
    owner: *mut c_void,
    slot: *mut RefHandle,
    label: *const c_char,
    type_name: *const c_char,
    scope: usize,
    index: usize,
) {
    let _mutation = Mutation::begin();
    assert!(!slot.is_null(), "debug slot must not be null");
    assert_eq!(
        state_of(owner),
        ArenaState::Alive,
        "debug slot owner is not Alive"
    );
    DEBUG.with(|state| {
        let mut state = state.borrow_mut();
        let record = DebugSlot {
            owner: owner as usize,
            address: slot as usize,
            label: label as usize,
            type_name: type_name as usize,
            scope,
            index,
        };
        if let Some(previous) = state.slots.iter_mut().find(|s| s.address == slot as usize) {
            *previous = record;
        } else {
            state.slots.push(record);
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_label_slot(
    slot: *mut RefHandle,
    label: *const c_char,
    type_name: *const c_char,
    index: usize,
) {
    let _mutation = Mutation::begin();
    assert!(!slot.is_null(), "debug slot must not be null");
    DEBUG.with(|state| {
        if let Some(record) = state
            .borrow_mut()
            .slots
            .iter_mut()
            .find(|s| s.address == slot as usize)
        {
            record.label = label as usize;
            record.type_name = type_name as usize;
            record.index = index;
        }
    });
}
