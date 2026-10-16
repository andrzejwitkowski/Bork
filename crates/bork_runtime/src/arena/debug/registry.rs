//! Thread-local snapshots, nested mutation publication and teardown invalidation.
use super::super::{ArenaId, RUNTIME};
#[cfg(not(test))]
use super::abi::bork_debug_registry;
use super::abi::{DebugArena, DebugPage, DebugRegistry, DebugSlot, EMPTY};
use std::{cell::RefCell, collections::HashMap};

pub(super) struct DebugState {
    pub(super) header: DebugRegistry,
    pub(super) depth: usize,
    pub(super) next_scope: usize,
    pub(super) arenas: Vec<DebugArena>,
    pub(super) pages: Vec<DebugPage>,
    pub(super) slots: Vec<DebugSlot>,
    pub(super) labels: HashMap<usize, (usize, usize)>,
}

impl Drop for DebugState {
    fn drop(&mut self) {
        invalidate();
    }
}

pub(in crate::arena) fn invalidate() {
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
    pub(super) static DEBUG: RefCell<DebugState> = RefCell::new(DebugState {
        header: EMPTY, depth: 0, next_scope: 1,
        arenas: Vec::new(), pages: Vec::new(), slots: Vec::new(), labels: HashMap::new(),
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
                let mut pages = Vec::new();
                let mut arenas: Vec<(DebugArena, usize)> = runtime
                    .controls
                    .values()
                    .map(|c| {
                        let (source, line) = state
                            .labels
                            .get(&c.arena_id.as_usize())
                            .copied()
                            .unwrap_or_default();
                        let first = pages.len();
                        for (base, used, capacity) in c
                            .payload
                            .as_ref()
                            .map_or_else(Vec::new, |a| a.debug_pages())
                        {
                            pages.push(DebugPage {
                                base,
                                used,
                                capacity,
                            });
                        }
                        let own = &pages[first..];
                        (
                            DebugArena {
                                id: c.arena_id.as_usize(),
                                parent: c.parent.map_or(0, ArenaId::as_usize),
                                depth: c.depth,
                                strong_count: c.strong_count,
                                weak_count: c.weak_count,
                                live_scoped_children: c.live_scoped_children,
                                state: c.state as usize,
                                mode: c.mode as usize,
                                pages: 0,
                                page_count: own.len(),
                                used: own.iter().map(|p| p.used).sum(),
                                capacity: own.iter().map(|p| p.capacity).sum(),
                                source,
                                line,
                            },
                            first,
                        )
                    })
                    .collect();
                // The page vector is final here, so element addresses are stable.
                for (arena, first) in &mut arenas {
                    if arena.page_count != 0 {
                        arena.pages = &pages[*first] as *const DebugPage as usize;
                    }
                }
                state.arenas = arenas.into_iter().map(|(a, _)| a).collect();
                state.pages = pages;
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
