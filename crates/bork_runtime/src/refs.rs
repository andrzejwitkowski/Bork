use std::ffi::c_void;
use std::ptr;

use super::arena::{
    classify_internal, register_drop_internal, release_strong_internal, release_weak_internal,
    retain_strong_internal, retain_weak_internal, retain_weak_target_internal, state_of,
    weak_upgrade_internal, ArenaState, RefKind,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefHandle {
    /// Opaque arena ID token. The runtime never dereferences this pointer.
    pub control: *mut c_void,
    pub object: *mut c_void,
    pub kind: RefKind,
}

impl RefHandle {
    pub const fn none() -> Self {
        Self {
            control: ptr::null_mut(),
            object: ptr::null_mut(),
            kind: RefKind::None,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefObservation {
    pub object: *mut c_void,
    guard: *mut c_void,
}

impl RefObservation {
    const fn none() -> Self {
        Self {
            object: ptr::null_mut(),
            guard: ptr::null_mut(),
        }
    }
}

fn create_ref(source: *mut c_void, target: *mut c_void, object: *mut c_void) -> RefHandle {
    assert!(!object.is_null(), "live Ref requires an object pointer");
    assert_eq!(state_of(source), ArenaState::Alive);
    assert_eq!(state_of(target), ArenaState::Alive);
    let kind = classify_internal(source, target);
    match kind {
        RefKind::Strong => retain_strong_internal(target),
        RefKind::Weak => retain_weak_target_internal(target),
        RefKind::Arena | RefKind::Borrow => {}
        RefKind::None => unreachable!("live arenas never classify as None"),
    }
    RefHandle {
        control: target,
        object,
        kind,
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_create(
    source: *mut c_void,
    target: *mut c_void,
    object: *mut c_void,
) -> RefHandle {
    create_ref(source, target, object)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_create_out(
    output: *mut RefHandle,
    source: *mut c_void,
    target: *mut c_void,
    object: *mut c_void,
) {
    assert!(!output.is_null(), "Ref output must not be null");
    // SAFETY: The ABI caller provides writable storage for one RefHandle.
    unsafe { ptr::write(output, create_ref(source, target, object)) };
}

fn drop_ref_value(reference: RefHandle) {
    match reference.kind {
        RefKind::Strong => release_strong_internal(reference.control),
        RefKind::Weak => release_weak_internal(reference.control),
        RefKind::None | RefKind::Arena | RefKind::Borrow => {}
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_drop(reference: *mut RefHandle) {
    assert!(!reference.is_null(), "Ref drop place must not be null");
    // SAFETY: Caller gives exclusive access to one initialized Ref slot.
    let old = std::mem::replace(unsafe { &mut *reference }, RefHandle::none());
    drop_ref_value(old);
}

fn clone_same_owner(reference: RefHandle) -> RefHandle {
    match reference.kind {
        RefKind::Strong => retain_strong_internal(reference.control),
        // Expired weak handles remain clonable because the source Weak keeps metadata alive.
        RefKind::Weak => retain_weak_internal(reference.control),
        RefKind::None | RefKind::Arena | RefKind::Borrow => {}
    }
    reference
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_clone_same_owner(reference: RefHandle) -> RefHandle {
    clone_same_owner(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_clone_same_owner_out(
    output: *mut RefHandle,
    reference: *const RefHandle,
) {
    assert!(!output.is_null(), "Ref output must not be null");
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides readable input and writable output slots.
    unsafe { ptr::write(output, clone_same_owner(*reference)) };
}

fn move_ref(reference: &mut RefHandle) -> RefHandle {
    std::mem::replace(reference, RefHandle::none())
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_move(reference: *mut RefHandle) -> RefHandle {
    assert!(!reference.is_null(), "Ref move place must not be null");
    // SAFETY: Caller gives exclusive access and treats the source as moved/None afterwards.
    move_ref(unsafe { &mut *reference })
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_move_out(output: *mut RefHandle, reference: *mut RefHandle) {
    assert!(!output.is_null(), "Ref output must not be null");
    // SAFETY: The ABI caller provides writable output and source slots.
    unsafe { ptr::write(output, bork_ref_move(reference)) };
}

fn rehome(new_source: *mut c_void, reference: RefHandle) -> RefHandle {
    if reference.kind == RefKind::None {
        return RefHandle::none();
    }
    let pinned = if reference.kind == RefKind::Weak {
        if !weak_upgrade_internal(reference.control) {
            return RefHandle::none();
        }
        true
    } else {
        false
    };
    let rebased = create_ref(new_source, reference.control, reference.object);
    if pinned {
        release_strong_internal(reference.control);
    }
    rebased
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_rehome(
    new_source: *mut c_void,
    reference: RefHandle,
) -> RefHandle {
    rehome(new_source, reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_rehome_out(
    output: *mut RefHandle,
    new_source: *mut c_void,
    reference: *const RefHandle,
) {
    assert!(!output.is_null(), "Ref output must not be null");
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides readable input and writable output slots.
    unsafe { ptr::write(output, rehome(new_source, *reference)) };
}

fn store(destination: &mut RefHandle, destination_owner: *mut c_void, incoming: RefHandle) {
    // Retain/rebase first: self-assignment and old-last-owner stores stay live.
    let new_value = rehome(destination_owner, incoming);
    let old = std::mem::replace(destination, new_value);
    drop_ref_value(old);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_store(
    destination: *mut RefHandle,
    destination_owner: *mut c_void,
    incoming: RefHandle,
) {
    assert!(!destination.is_null(), "Ref destination must not be null");
    // SAFETY: Caller gives exclusive access to one initialized Ref slot.
    store(unsafe { &mut *destination }, destination_owner, incoming);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_store_take(
    destination: *mut RefHandle,
    destination_owner: *mut c_void,
    incoming: RefHandle,
) {
    assert!(!destination.is_null(), "Ref destination must not be null");
    // SAFETY: Caller gives exclusive access to one initialized Ref slot.
    store(unsafe { &mut *destination }, destination_owner, incoming);
    drop_ref_value(incoming);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_store_take_ptr(
    destination: *mut RefHandle,
    destination_owner: *mut c_void,
    incoming: *const RefHandle,
) {
    assert!(!incoming.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides a readable RefHandle input slot.
    unsafe { bork_ref_store_take(destination, destination_owner, *incoming) };
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_drop_value(reference: RefHandle) {
    drop_ref_value(reference);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_drop_value_ptr(reference: *const RefHandle) {
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides a readable RefHandle input slot.
    unsafe { drop_ref_value(*reference) };
}

fn observe(reference: RefHandle) -> RefObservation {
    match reference.kind {
        RefKind::None => RefObservation::none(),
        RefKind::Weak => {
            if !weak_upgrade_internal(reference.control) {
                return RefObservation::none();
            }
            RefObservation {
                object: reference.object,
                guard: reference.control,
            }
        }
        RefKind::Strong => {
            // Pin independently from the storage slot: the block may overwrite that slot.
            retain_strong_internal(reference.control);
            RefObservation {
                object: reference.object,
                guard: reference.control,
            }
        }
        RefKind::Arena | RefKind::Borrow => {
            assert_eq!(
                state_of(reference.control),
                ArenaState::Alive,
                "zero-cost Ref target is not Alive"
            );
            RefObservation {
                object: reference.object,
                guard: ptr::null_mut(),
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_observe(reference: RefHandle) -> RefObservation {
    observe(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_observe_out(
    output: *mut RefObservation,
    reference: *const RefHandle,
) {
    assert!(!output.is_null(), "Observation output must not be null");
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides readable input and writable output slots.
    unsafe { ptr::write(output, observe(*reference)) };
}

fn release_observation(observation: RefObservation) {
    if !observation.guard.is_null() {
        release_strong_internal(observation.guard);
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_release_observation(observation: RefObservation) {
    release_observation(observation);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_release_observation_ptr(observation: *const RefObservation) {
    assert!(!observation.is_null(), "Observation input must not be null");
    // SAFETY: The ABI caller provides a readable observation slot.
    unsafe { release_observation(*observation) };
}

fn ref_is_none(reference: RefHandle) -> bool {
    reference.kind == RefKind::None
        || (reference.kind == RefKind::Weak && state_of(reference.control) != ArenaState::Alive)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_is_none(reference: RefHandle) -> bool {
    ref_is_none(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_is_none_ptr(reference: *const RefHandle) -> bool {
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides a readable RefHandle input slot.
    unsafe { ref_is_none(*reference) }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_equal(lhs: RefHandle, rhs: RefHandle) -> bool {
    let lhs_none = ref_is_none(lhs);
    let rhs_none = ref_is_none(rhs);
    if lhs_none || rhs_none {
        return lhs_none == rhs_none;
    }
    lhs.control == rhs.control && lhs.object == rhs.object
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_equal_ptr(lhs: *const RefHandle, rhs: *const RefHandle) -> bool {
    assert!(
        !lhs.is_null() && !rhs.is_null(),
        "Ref inputs must not be null"
    );
    // SAFETY: The ABI caller provides readable RefHandle input slots.
    unsafe { bork_ref_equal(*lhs, *rhs) }
}

unsafe extern "C" fn drop_ref_slot_runtime(value: *mut c_void) {
    // SAFETY: The drop registry contains a pointer to an initialized RefHandle slot.
    unsafe { bork_ref_drop(value.cast()) };
}

unsafe extern "C" fn drop_observation_runtime(value: *mut c_void) {
    // SAFETY: The drop registry contains a pointer to an initialized observation slot.
    let observation = unsafe { *value.cast::<RefObservation>() };
    release_observation(observation);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_ref_drop(
    owner: *mut c_void,
    reference: *mut RefHandle,
) {
    register_drop_internal(owner, reference.cast(), drop_ref_slot_runtime);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_observation_drop(
    owner: *mut c_void,
    observation: *mut RefObservation,
) {
    register_drop_internal(owner, observation.cast(), drop_observation_runtime);
}
