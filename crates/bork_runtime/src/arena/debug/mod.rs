//! Debug registry: ABI records, snapshot publication and persistent Ref slots.
mod abi;
mod registry;
mod slots;

pub(super) use registry::invalidate;
pub(crate) use registry::Mutation;
pub(crate) use slots::{register_generic, remove_owner};

use super::state_of;
use registry::DEBUG;
use std::ffi::{c_char, c_void};

#[cfg(test)]
use abi::{DebugArena, DebugRegistry, DebugSlot};
#[cfg(test)]
use slots::*;

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
mod tests;
