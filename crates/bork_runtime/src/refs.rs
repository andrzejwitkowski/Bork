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
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(!reference.is_null(), "Ref move place must not be null");
    // SAFETY: Caller gives exclusive access and treats the source as moved/None afterwards.
    move_ref(unsafe { &mut *reference })
}

#[no_mangle]
pub unsafe extern "C" fn bork_ref_move_out(output: *mut RefHandle, reference: *mut RefHandle) {
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
    let _mutation = super::arena::debug::Mutation::begin();
    assert!(
        !lhs.is_null() && !rhs.is_null(),
        "Ref inputs must not be null"
    );
    // SAFETY: The ABI caller provides readable RefHandle input slots.
    unsafe { bork_ref_equal(*lhs, *rhs) }
}

/// Header of a `Vec<Ref<T>>`: `{ buf, len, cap, arena }`. The field order is `BUF`/`LEN`/`CAP`/`ARENA` in `codegen/llvm/vec/mod.rs`.
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
struct VecHeader {
    buf: *mut RefHandle,
    len: usize,
    cap: usize,
    arena: *mut c_void,
}

/// Drop registered on a `Vec<Ref<T>>` header: releases every live slot. One drop per vector,
/// because the buffer moves on growth and a per-slot registration would dangle.
#[no_mangle]
pub unsafe extern "C" fn bork_vec_drop_refs(header: *mut c_void) {
    // SAFETY: The registry holds a pointer to an initialized header whose first `len` slots are live.
    let header = unsafe { &*header.cast::<VecHeader>() };
    for index in 0..header.len {
        // SAFETY: Slots below `len` are initialized RefHandles inside the live buffer.
        unsafe { bork_ref_drop(header.buf.add(index)) };
    }
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
    let _mutation = super::arena::debug::Mutation::begin();
    register_drop_internal(owner, observation.cast(), drop_observation_runtime);
}

#[cfg(test)]
mod vec_tests {
    //! `Vec<Ref<T>>` as `codegen/llvm/vec.rs` builds it, driven through the same runtime calls:
    //! header + buffer in the owner arena, `bork_arena_grow` on growth, `clone` + `store_take`
    //! per slot on promotion, one `bork_vec_drop_refs` per header.
    use super::*;
    use crate::arena::{
        bork_arena_alloc, bork_arena_create_dynamic, bork_arena_grow, bork_arena_pop,
        bork_arena_push, bork_arena_register_drop, bork_arena_release_strong,
        bork_arena_strong_count, control_count, weak_count_of,
    };

    const SLOT: usize = std::mem::size_of::<RefHandle>();

    fn object() -> *mut c_void {
        0x10 as *mut c_void
    }

    unsafe fn vec_new(arena: *mut c_void) -> *mut VecHeader {
        let header = bork_arena_alloc(arena, std::mem::size_of::<VecHeader>(), 8).cast::<VecHeader>();
        header.write(VecHeader { buf: ptr::null_mut(), len: 0, cap: 0, arena });
        bork_arena_register_drop(arena, header.cast(), Some(bork_vec_drop_refs));
        header
    }

    unsafe fn reserve(header: *mut VecHeader) {
        let h = &mut *header;
        if h.len < h.cap {
            return;
        }
        let new_cap = if h.cap == 0 { 4 } else { h.cap * 2 };
        h.buf = if h.cap == 0 {
            bork_arena_alloc(h.arena, new_cap * SLOT, 8)
        } else {
            bork_arena_grow(h.arena, h.buf.cast(), h.cap * SLOT, new_cap * SLOT, 8)
        }
        .cast();
        h.cap = new_cap;
    }

    /// `v.push(Node(..))`: a fresh handle made in the vector's arena, stored into a new slot.
    unsafe fn push(header: *mut VecHeader, target: *mut c_void) {
        reserve(header);
        let h = &mut *header;
        let fresh = bork_ref_create(h.arena, target, object());
        let slot = h.buf.add(h.len);
        slot.write(RefHandle::none());
        bork_ref_store_take(slot, h.arena, fresh);
        h.len += 1;
    }

    /// `promote v` into `dest`: new header, `cap = len` buffer, every slot cloned then rehomed.
    unsafe fn promote(source: *mut VecHeader, dest: *mut c_void) -> *mut VecHeader {
        let header = vec_new(dest);
        let len = (*source).len;
        if len > 0 {
            (*header).buf = bork_arena_alloc(dest, len * SLOT, 8).cast();
            (*header).cap = len;
        }
        for index in 0..len {
            let cloned = bork_ref_clone_same_owner(*(*source).buf.add(index));
            let slot = (*header).buf.add(index);
            slot.write(RefHandle::none());
            bork_ref_store_take(slot, dest, cloned);
            (*header).len += 1;
        }
        header
    }

    unsafe fn kinds(header: *mut VecHeader) -> Vec<RefKind> {
        (0..(*header).len).map(|i| (*(*header).buf.add(i)).kind).collect()
    }

    /// (strong, weak) counts of a target arena.
    unsafe fn counts(target: *mut c_void) -> (usize, usize) {
        (bork_arena_strong_count(target), weak_count_of(target))
    }

    #[test]
    fn grown_vec_keeps_counts_exact_across_older_and_newer_arenas() {
        unsafe {
            let base = control_count();
            // Ids ascend in creation order: low < t1 < a < t2 < b.
            let low = bork_arena_push();
            let t1 = bork_arena_create_dynamic(ptr::null_mut(), 0);
            let a = bork_arena_push();
            let t2 = bork_arena_create_dynamic(ptr::null_mut(), 0);
            let b = bork_arena_push();
            let (t1_base, t2_base) = (counts(t1), counts(t2));

            // 10 pushes grow the buffer 4 -> 8 -> 16. In `a`, t1 is a back-edge (weak), t2 strong.
            let in_a = vec_new(a);
            for i in 0..10 {
                push(in_a, if i % 2 == 0 { t1 } else { t2 });
            }
            assert_eq!((*in_a).cap, 16, "doubled twice past the first buffer");
            assert_eq!((*in_a).len, 10);
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 5), "t1: five weak slots");
            assert_eq!(counts(t2), (t2_base.0 + 5, t2_base.1), "t2: five strong slots");
            assert_eq!(kinds(in_a)[..2], [RefKind::Weak, RefKind::Strong]);

            // Promote to the newest arena: every edge is now a back-edge, so all become weak.
            let in_b = promote(in_a, b);
            assert!(kinds(in_b).iter().all(|k| *k == RefKind::Weak));
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 10), "both vectors alive");
            assert_eq!(counts(t2), (t2_base.0 + 5, t2_base.1 + 5));
            // Dropping the source vector (arena `a`) releases exactly its own slots.
            bork_arena_pop(a);
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 5));
            assert_eq!(counts(t2), (t2_base.0, t2_base.1 + 5), "t2 lost its strong slots");

            // Promote to the oldest arena: every edge points forward, so all become strong.
            let in_low = promote(in_b, low);
            assert!(kinds(in_low).iter().all(|k| *k == RefKind::Strong));
            assert_eq!(counts(t1), (t1_base.0 + 5, t1_base.1 + 5));
            assert_eq!(counts(t2), (t2_base.0 + 5, t2_base.1 + 5));
            bork_arena_pop(b);
            assert_eq!(counts(t1), (t1_base.0 + 5, t1_base.1));
            assert_eq!(counts(t2), (t2_base.0 + 5, t2_base.1));

            // Back to a newest arena, then let the strong holder go: the owners' own references
            // are the only thing left keeping t1 and t2 alive.
            let b2 = bork_arena_push();
            let in_b2 = promote(in_low, b2);
            assert!(kinds(in_b2).iter().all(|k| *k == RefKind::Weak));
            bork_arena_pop(low);
            assert_eq!(counts(t1), (t1_base.0, t1_base.1 + 5));
            assert_eq!(counts(t2), (t2_base.0, t2_base.1 + 5));

            // t1's owner releases it: weak slots in `b2` observe it as expired, t2 stays live.
            bork_arena_release_strong(t1);
            let dead = bork_ref_observe(*(*in_b2).buf);
            assert!(dead.object.is_null(), "weak slot to a released arena reads as absent");
            let live = bork_ref_observe(*(*in_b2).buf.add(1));
            assert!(!live.object.is_null(), "t2 is still owned, so its weak slot upgrades");
            bork_ref_release_observation(live);
            assert_eq!(bork_arena_strong_count(t2), t2_base.0);

            bork_arena_release_strong(t2);
            bork_arena_pop(b2);
            assert_eq!(control_count(), base, "every arena and weak lease is released");
        }
    }

    #[test]
    fn clear_and_drop_release_each_slot_once() {
        unsafe {
            let base = control_count();
            let owner = bork_arena_push();
            let target = bork_arena_create_dynamic(ptr::null_mut(), 0);
            let before = counts(target);
            let vec = vec_new(owner);
            for _ in 0..9 {
                push(vec, target);
            }
            // `owner` is older than `target`, so the slots are strong.
            assert_eq!(counts(target), (before.0 + 9, before.1));
            bork_arena_pop(owner);
            assert_eq!(counts(target), before, "one drop released all nine slots exactly once");
            bork_arena_release_strong(target);
            assert_eq!(control_count(), base);
        }
    }
}
