//! Debugger ABI: flat target-word records, independent of Rust collection layout.
use super::*;
use crate::refs::RefHandle;
use std::ffi::c_char;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugRegistry {
    pub version: usize,
    pub sequence: usize,
    pub arenas: usize,
    pub arena_count: usize,
    pub slots: usize,
    pub slot_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugArena {
    pub id: usize,
    pub parent: usize,
    pub depth: usize,
    pub strong_count: usize,
    pub weak_count: usize,
    pub live_scoped_children: usize,
    pub state: usize,
    pub mode: usize,
    pub payload: usize,
    pub used: usize,
    pub capacity: usize,
    pub source: usize,
    pub line: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DebugSlot {
    pub owner: usize,
    pub address: usize,
    pub label: usize,
    pub type_name: usize,
    pub scope: usize,
    pub index: usize,
}

const EMPTY: DebugRegistry = DebugRegistry {
    version: 1,
    sequence: 0,
    arenas: 0,
    arena_count: 0,
    slots: 0,
    slot_count: 0,
};

#[no_mangle]
pub static mut bork_debug_registry: DebugRegistry = EMPTY;

#[no_mangle]
pub extern "C" fn bork_debug_abi_v1() {}

struct DebugState {
    header: DebugRegistry,
    depth: usize,
    next_scope: usize,
    arenas: Vec<DebugArena>,
    slots: Vec<DebugSlot>,
    labels: HashMap<usize, (usize, usize)>,
}

impl Drop for DebugState {
    fn drop(&mut self) {
        invalidate();
    }
}

pub(super) fn invalidate() {
    #[cfg(not(test))]
    unsafe {
        // Either TLS owner can drop first; invalidate before either releases memory.
        let sequence = std::ptr::read_volatile(&raw const bork_debug_registry.sequence);
        std::ptr::write_volatile(&raw mut bork_debug_registry.sequence, sequence | 1);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        std::ptr::write_volatile(&raw mut bork_debug_registry.arenas, 0);
        std::ptr::write_volatile(&raw mut bork_debug_registry.arena_count, 0);
        std::ptr::write_volatile(&raw mut bork_debug_registry.slots, 0);
        std::ptr::write_volatile(&raw mut bork_debug_registry.slot_count, 0);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

thread_local! {
    static DEBUG: RefCell<DebugState> = RefCell::new(DebugState {
        header: EMPTY, depth: 0, next_scope: 1,
        arenas: Vec::new(), slots: Vec::new(), labels: HashMap::new(),
    });
}

fn publish(state: &DebugState) {
    // Unit tests have independent TLS runtimes on concurrent harness threads.
    #[cfg(not(test))]
    unsafe {
        // Publish the sequence last: an even header must never expose old array pointers.
        std::ptr::write_volatile(&raw mut bork_debug_registry.arenas, state.header.arenas);
        std::ptr::write_volatile(
            &raw mut bork_debug_registry.arena_count,
            state.header.arena_count,
        );
        std::ptr::write_volatile(&raw mut bork_debug_registry.slots, state.header.slots);
        std::ptr::write_volatile(
            &raw mut bork_debug_registry.slot_count,
            state.header.slot_count,
        );
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
        std::ptr::write_volatile(&raw mut bork_debug_registry.sequence, state.header.sequence);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(test)]
    let _ = state;
}

pub(crate) struct Mutation;
impl Mutation {
    pub(crate) fn begin() -> Self {
        #[cfg(not(test))]
        {
            static OWNER: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();
            let current = std::thread::current().id();
            assert_eq!(
                *OWNER.get_or_init(|| current),
                current,
                "debug-runtime supports one Bork execution thread"
            );
        }
        DEBUG.with(|state| {
            let mut state = state.borrow_mut();
            if state.depth == 0 {
                state.header.sequence = state.header.sequence.wrapping_add(1);
                publish(&state);
            }
            state.depth += 1;
        });
        Self
    }
}

impl Drop for Mutation {
    fn drop(&mut self) {
        DEBUG.with(|state| {
            let mut state = state.borrow_mut();
            state.depth -= 1;
            if state.depth != 0 || std::thread::panicking() {
                return;
            }
            RUNTIME.with(|runtime| {
                let runtime = runtime.borrow();
                state
                    .labels
                    .retain(|id, _| runtime.controls.contains_key(&ArenaId::new(*id)));
                state.arenas = runtime
                    .controls
                    .values()
                    .map(|c| {
                        let (source, line) = state
                            .labels
                            .get(&c.arena_id.as_usize())
                            .copied()
                            .unwrap_or_default();
                        DebugArena {
                            id: c.arena_id.as_usize(),
                            parent: c.parent.map_or(0, ArenaId::as_usize),
                            depth: c.depth,
                            strong_count: c.strong_count,
                            weak_count: c.weak_count,
                            live_scoped_children: c.live_scoped_children,
                            state: c.state as usize,
                            mode: c.mode as usize,
                            payload: c.payload.as_ref().map_or(0, |a| a.buf.as_ptr() as usize),
                            used: c.payload.as_ref().map_or(0, |a| a.offset),
                            capacity: c.payload.as_ref().map_or(0, |_| ARENA_CAPACITY),
                            source,
                            line,
                        }
                    })
                    .collect();
            });
            state.arenas.sort_unstable_by_key(|a| a.id);
            state.header.arenas = state.arenas.as_ptr() as usize;
            state.header.arena_count = state.arenas.len();
            state.header.slots = state.slots.as_ptr() as usize;
            state.header.slot_count = state.slots.len();
            state.header.sequence = state.header.sequence.wrapping_add(1);
            publish(&state);
        });
    }
}

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

#[no_mangle]
pub unsafe extern "C" fn bork_debug_label_arena(
    arena: *mut c_void,
    source: *const c_char,
    line: usize,
) {
    let _mutation = Mutation::begin();
    state_of(arena);
    DEBUG.with(|state| {
        state
            .borrow_mut()
            .labels
            .insert(arena as usize, (source as usize, line));
    });
}

#[cfg(test)]
pub(super) fn test_arenas() -> Vec<DebugArena> {
    DEBUG.with(|state| state.borrow().arenas.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
