//! Flat versioned records and exported ABI symbols read by debuggers.
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

pub(super) const EMPTY: DebugRegistry = DebugRegistry {
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
