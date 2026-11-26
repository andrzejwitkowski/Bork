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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check_handle(target as usize, object as usize);
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    create_ref(source, target, object)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_create_out(
    output: *mut RefHandle,
    source: *mut c_void,
    target: *mut c_void,
    object: *mut c_void,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(output as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    clone_same_owner(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_clone_same_owner_out(
    output: *mut RefHandle,
    reference: *const RefHandle,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(output as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(!reference.is_null(), "Ref move place must not be null");
    // SAFETY: Caller gives exclusive access and treats the source as moved/None afterwards.
    move_ref(unsafe { &mut *reference })
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_move_out(output: *mut RefHandle, reference: *mut RefHandle) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(output as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    rehome(new_source, reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_rehome_out(
    output: *mut RefHandle,
    new_source: *mut c_void,
    reference: *const RefHandle,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(output as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(destination as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(destination as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(incoming as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(!incoming.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides a readable RefHandle input slot.
    unsafe { bork_ref_store_take(destination, destination_owner, *incoming) };
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_drop_value(reference: RefHandle) {
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    drop_ref_value(reference);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_drop_value_ptr(reference: *const RefHandle) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
            #[cfg(feature = "debug-runtime")]
            super::arena::debug::diag::check_handle(reference.control as usize, reference.object as usize);
            RefObservation {
                object: reference.object,
                guard: reference.control,
            }
        }
        RefKind::Strong => {
            #[cfg(feature = "debug-runtime")]
            super::arena::debug::diag::check_handle(reference.control as usize, reference.object as usize);
            // Pin independently from the storage slot: the block may overwrite that slot.
            retain_strong_internal(reference.control);
            RefObservation {
                object: reference.object,
                guard: reference.control,
            }
        }
        RefKind::Arena | RefKind::Borrow => {
            #[cfg(feature = "debug-runtime")]
            super::arena::debug::diag::check_handle(reference.control as usize, reference.object as usize);
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    observe(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_observe_out(
    output: *mut RefObservation,
    reference: *const RefHandle,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(output as usize, std::mem::size_of::<RefObservation>(), true);
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    release_observation(observation);
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_release_observation_ptr(observation: *const RefObservation) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(observation as usize, std::mem::size_of::<RefObservation>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    ref_is_none(reference)
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_is_none_ptr(reference: *const RefHandle) -> bool {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(!reference.is_null(), "Ref input must not be null");
    // SAFETY: The ABI caller provides a readable RefHandle input slot.
    unsafe { ref_is_none(*reference) }
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_equal(lhs: RefHandle, rhs: RefHandle) -> bool {
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    let lhs_none = ref_is_none(lhs);
    let rhs_none = ref_is_none(rhs);
    if lhs_none || rhs_none {
        return lhs_none == rhs_none;
    }
    lhs.control == rhs.control && lhs.object == rhs.object
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_equal_ptr(lhs: *const RefHandle, rhs: *const RefHandle) -> bool {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(lhs as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(rhs as usize, std::mem::size_of::<RefHandle>(), false);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(
        !lhs.is_null() && !rhs.is_null(),
        "Ref inputs must not be null"
    );
    // SAFETY: The ABI caller provides readable RefHandle input slots.
    unsafe { bork_ref_equal(*lhs, *rhs) }
}

/// Header of a `Buf<Ref<T>>`: `{ ptr, cap }`.
#[repr(C)]
struct BufHeader {
    ptr: *mut RefHandle,
    cap: usize,
}

/// Drop registered on a `Buf<Ref<T>>`: releases every slot. Unwritten and moved-out slots are zeroed `None` handles,
/// which release nothing, so no length is needed.
#[no_mangle]
pub unsafe extern "C" fn bork_buf_drop_refs(header: *mut c_void) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(header as usize, std::mem::size_of::<BufHeader>(), false);
    // SAFETY: The registry holds a pointer to an initialized header whose `cap` slots are all initialized handles.
    let header = unsafe { &*header.cast::<BufHeader>() };
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(header.ptr as usize, header.cap.checked_mul(std::mem::size_of::<RefHandle>()).unwrap_or(usize::MAX), true);
    for index in 0..header.cap {
        // SAFETY: Every slot of the live buffer is an initialized (possibly `None`) RefHandle.
        unsafe { bork_ref_drop(header.ptr.add(index)) };
    }
}

unsafe extern "C" fn drop_ref_slot_runtime(value: *mut c_void) {
    // SAFETY: The drop registry contains a pointer to an initialized RefHandle slot.
    unsafe { bork_ref_drop(value.cast()) };
}

unsafe extern "C" fn drop_observation_runtime(value: *mut c_void) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(value as usize, std::mem::size_of::<RefObservation>(), false);
    // SAFETY: The drop registry contains a pointer to an initialized observation slot.
    let observation = unsafe { *value.cast::<RefObservation>() };
    release_observation(observation);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_ref_drop(
    owner: *mut c_void,
    reference: *mut RefHandle,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(reference as usize, std::mem::size_of::<RefHandle>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    register_drop_internal(owner, reference.cast(), drop_ref_slot_runtime);
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::register_generic(owner, reference);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_observation_drop(
    owner: *mut c_void,
    observation: *mut RefObservation,
) {
    #[cfg(feature = "debug-runtime")]
    super::arena::debug::diag::check(observation as usize, std::mem::size_of::<RefObservation>(), true);
    #[cfg(feature = "debug-runtime")]
    let _mutation = super::arena::debug::Mutation::begin();
    register_drop_internal(owner, observation.cast(), drop_observation_runtime);
}

#[cfg(test)]
mod buf_tests {
    //! `Buf<Ref<T>>` as `codegen/llvm/buf.rs` builds it: a zeroed buffer plus one `{ptr, cap}` header carrying
    //! `bork_buf_drop_refs`, slots filled through `store_take`, growth by copy and zeroing the old slots.
    use super::*;
    use crate::arena::{
        bork_arena_alloc, bork_arena_create_dynamic, bork_arena_pop, bork_arena_push, bork_arena_register_drop,
        bork_arena_release_strong, bork_arena_strong_count, control_count, weak_count_of,
    };

    const SLOT: usize = std::mem::size_of::<RefHandle>();

    fn object() -> *mut c_void {
        0x10 as *mut c_void
    }

    /// A zeroed buffer of `cap` slots in `arena` with its drop registered.
    unsafe fn buf_alloc(arena: *mut c_void, cap: usize) -> *mut RefHandle {
        let ptr = bork_arena_alloc(arena, cap * SLOT, 8).cast::<RefHandle>();
        std::ptr::write_bytes(ptr.cast::<u8>(), 0, cap * SLOT);
        let header = bork_arena_alloc(arena, std::mem::size_of::<BufHeader>(), 8).cast::<BufHeader>();
        header.write(BufHeader { ptr, cap });
        bork_arena_register_drop(arena, header.cast(), Some(bork_buf_drop_refs));
        ptr
    }

    /// `bufGrow`: a bigger buffer holding the same handles; the old slots are zeroed so its drop releases nothing.
    unsafe fn buf_grow(arena: *mut c_void, old: *mut RefHandle, cap: usize, new_cap: usize) -> *mut RefHandle {
        let new = buf_alloc(arena, new_cap);
        std::ptr::copy_nonoverlapping(old, new, cap);
        std::ptr::write_bytes(old.cast::<u8>(), 0, cap * SLOT);
        new
    }

    unsafe fn store(arena: *mut c_void, slot: *mut RefHandle, target: *mut c_void) {
        bork_ref_store_take(slot, arena, bork_ref_create(arena, target, object()));
    }

    unsafe fn counts(target: *mut c_void) -> (usize, usize) {
        (bork_arena_strong_count(target), weak_count_of(target))
    }

    #[test]
    fn grown_buffer_keeps_counts_exact_and_releases_each_slot_once() {
        unsafe {
            let base = control_count();
            // Ids ascend in creation order: t1 < a < t2. From `a`, t1 is a back-edge (weak) and t2 a forward edge (strong).
            let t1 = bork_arena_create_dynamic(ptr::null_mut(), 0);
            let a = bork_arena_push();
            let t2 = bork_arena_create_dynamic(ptr::null_mut(), 0);
            let (t1_base, t2_base) = (counts(t1), counts(t2));

            let mut cap = 4;
            let mut buf = buf_alloc(a, cap);
            for i in 0..6 {
                if i == cap {
                    buf = buf_grow(a, buf, cap, cap * 2);
                    cap *= 2;
                }
                store(a, buf.add(i), if i % 2 == 0 { t1 } else { t2 });
            }
            assert_eq!(cap, 8, "one growth past the first buffer");
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 3), "t1: three weak slots, none doubled by the growth");
            assert_eq!(counts(t2), (t2_base.0 + 3, t2_base.1), "t2: three strong slots");

            // Overwriting a live slot releases its old value.
            store(a, buf, t2);
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 2));
            assert_eq!(counts(t2), (t2_base.0 + 4, t2_base.1));

            // Taking a handle out leaves `None`: the buffer's drop must not release it again.
            let taken = std::ptr::read(buf.add(1));
            buf.add(1).write(RefHandle::none());
            bork_ref_drop_value(taken);
            assert_eq!(counts(t2), (t2_base.0 + 3, t2_base.1));

            // Both buffers' drops run with the arena: the old one holds only zeroed slots, the new one the rest.
            bork_arena_pop(a);
            assert_eq!(counts(t1), t1_base, "every weak slot released exactly once");
            assert_eq!(counts(t2), t2_base, "every strong slot released exactly once");

            bork_arena_release_strong(t1);
            bork_arena_release_strong(t2);
            assert_eq!(control_count(), base, "every arena and lease is released");
        }
    }
}
