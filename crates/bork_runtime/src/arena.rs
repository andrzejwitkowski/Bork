use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroUsize;

const ARENA_CAPACITY: usize = 4096;

struct Arena {
    buf: [u8; ARENA_CAPACITY],
    offset: usize,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaState {
    Alive,
    Destroying,
    Dead,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    None,
    Arena,
    Borrow,
    Strong,
    Weak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArenaMode {
    DomainRoot,
    Lexical,
    DynamicFree,
    DynamicScoped,
}

pub type DropFn = unsafe extern "C" fn(*mut c_void);

struct DropEntry {
    value: *mut c_void,
    drop_fn: DropFn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ArenaId(NonZeroUsize);

impl ArenaId {
    fn new(raw: usize) -> Self {
        Self(NonZeroUsize::new(raw).expect("arena id must be non-zero"))
    }

    fn from_token(token: *mut c_void) -> Self {
        assert!(!token.is_null(), "arena handle must not be null");
        Self::new(token.addr())
    }

    fn token(self) -> *mut c_void {
        std::ptr::without_provenance_mut(self.0.get())
    }

    fn as_usize(self) -> usize {
        self.0.get()
    }

    fn as_u64(self) -> u64 {
        self.as_usize() as u64
    }
}

struct ArenaControl {
    arena_id: ArenaId,
    parent: Option<ArenaId>,
    depth: usize,
    strong_count: usize,
    weak_count: usize,
    live_scoped_children: usize,
    state: ArenaState,
    mode: ArenaMode,
    payload: Option<Box<Arena>>,
    drops: Vec<DropEntry>,
}

impl Arena {
    fn new() -> Self {
        Self {
            buf: [0; ARENA_CAPACITY],
            offset: 0,
        }
    }

    fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        assert!(align.is_power_of_two(), "align must be a power of two");
        let base = self.buf.as_mut_ptr() as usize;
        let addr = base
            .checked_add(self.offset)
            .expect("allocation address overflow");
        let aligned_addr = addr
            .checked_add(align - 1)
            .expect("aligned address overflow")
            & !(align - 1);
        let aligned = aligned_addr
            .checked_sub(base)
            .expect("aligned offset underflow");
        let end = aligned.checked_add(size).expect("allocation size overflow");
        if end > ARENA_CAPACITY {
            panic!("arena overflow: need {end} bytes, capacity {ARENA_CAPACITY}");
        }
        self.offset = end;
        aligned_addr as *mut u8
    }

    fn reset(&mut self) {
        self.offset = 0;
    }
}

#[derive(Default)]
struct ArenaPool {
    free: Vec<Box<Arena>>,
}

impl ArenaPool {
    fn acquire(&mut self) -> Box<Arena> {
        self.free.pop().unwrap_or_else(|| Box::new(Arena::new()))
    }

    fn release(&mut self, mut arena: Box<Arena>) {
        arena.reset();
        self.free.push(arena);
    }
}

struct RuntimeState {
    pool: ArenaPool,
    next_arena_id: usize,
    controls: HashMap<ArenaId, ArenaControl>,
}

impl RuntimeState {
    fn new() -> Self {
        let root_id = ArenaId::new(1);
        let mut controls = HashMap::new();
        controls.insert(
            root_id,
            ArenaControl {
                arena_id: root_id,
                parent: None,
                depth: 0,
                strong_count: 0,
                weak_count: 0,
                live_scoped_children: 0,
                state: ArenaState::Alive,
                mode: ArenaMode::DomainRoot,
                payload: None,
                drops: Vec::new(),
            },
        );
        Self {
            pool: ArenaPool::default(),
            next_arena_id: 2,
            controls,
        }
    }

    fn next_id(&mut self) -> ArenaId {
        let id = self.next_arena_id;
        self.next_arena_id = id.checked_add(1).expect("arena_id overflow");
        ArenaId::new(id)
    }
}

thread_local! {
    static RUNTIME: RefCell<RuntimeState> = RefCell::new(RuntimeState::new());
}

fn id_from_token(token: *mut c_void) -> ArenaId {
    ArenaId::from_token(token)
}

fn with_control<R>(token: *mut c_void, f: impl FnOnce(&ArenaControl) -> R) -> R {
    let id = id_from_token(token);
    RUNTIME.with(|runtime| {
        let runtime = runtime.borrow();
        let control = runtime
            .controls
            .get(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        f(control)
    })
}

fn with_control_mut<R>(token: *mut c_void, f: impl FnOnce(&mut ArenaControl) -> R) -> R {
    let id = id_from_token(token);
    RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let control = runtime
            .controls
            .get_mut(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        f(control)
    })
}

fn root_token() -> *mut c_void {
    ArenaId::new(1).token()
}

pub(crate) fn state_of(arena: *mut c_void) -> ArenaState {
    with_control(arena, |control| control.state)
}

#[cfg(test)]
pub(crate) fn control_count() -> usize {
    RUNTIME.with(|runtime| runtime.borrow().controls.len())
}

pub(crate) fn retain_weak_internal(arena: *mut c_void) {
    if arena.is_null() {
        return;
    }
    with_control_mut(arena, |control| {
        control.weak_count = control
            .weak_count
            .checked_add(1)
            .expect("arena weak_count overflow");
    });
}

pub(crate) fn retain_weak_target_internal(arena: *mut c_void) {
    with_control(arena, |control| {
        assert!(
            matches!(
                control.mode,
                ArenaMode::DynamicFree | ArenaMode::DynamicScoped
            ),
            "weak reference target must be dynamic"
        );
        assert_eq!(control.state, ArenaState::Alive);
    });
    retain_weak_internal(arena);
}

fn free_control(id: ArenaId) {
    let parent = RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let control = runtime
            .controls
            .remove(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        assert_ne!(
            control.mode,
            ArenaMode::DomainRoot,
            "cannot free domain root"
        );
        assert_eq!(control.state, ArenaState::Dead, "arena is not Dead");
        assert_eq!(control.weak_count, 0, "arena still has weak leases");
        control.parent
    });
    if let Some(parent) = parent {
        release_weak_id(parent);
    }
}

fn release_weak_id(id: ArenaId) {
    let should_free = RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let control = runtime
            .controls
            .get_mut(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        assert!(control.weak_count > 0, "arena weak_count underflow");
        control.weak_count -= 1;
        control.weak_count == 0
            && control.state == ArenaState::Dead
            && control.mode != ArenaMode::DomainRoot
    });
    if should_free {
        free_control(id);
    }
}

pub(crate) fn release_weak_internal(arena: *mut c_void) {
    release_weak_id(id_from_token(arena));
}

fn new_control(parent: ArenaId, mode: ArenaMode, strong_count: usize) -> *mut c_void {
    RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let depth = {
            let parent_control = runtime
                .controls
                .get_mut(&parent)
                .unwrap_or_else(|| panic!("unknown arena handle {}", parent.as_u64()));
            assert_eq!(
                parent_control.state,
                ArenaState::Alive,
                "parent arena is not Alive"
            );
            if mode == ArenaMode::DynamicScoped {
                parent_control.live_scoped_children = parent_control
                    .live_scoped_children
                    .checked_add(1)
                    .expect("scoped child count overflow");
            }
            parent_control.weak_count = parent_control
                .weak_count
                .checked_add(1)
                .expect("arena weak_count overflow");
            parent_control
                .depth
                .checked_add(1)
                .expect("arena depth overflow")
        };
        let id = runtime.next_id();
        assert!(
            parent.as_usize() < id.as_usize(),
            "parent arena_id must be smaller than child arena_id"
        );
        let payload = runtime.pool.acquire();
        runtime.controls.insert(
            id,
            ArenaControl {
                arena_id: id,
                parent: Some(parent),
                depth,
                strong_count,
                weak_count: 0,
                live_scoped_children: 0,
                state: ArenaState::Alive,
                mode,
                payload: Some(payload),
                drops: Vec::new(),
            },
        );
        id.token()
    })
}

fn destroy_control(id: ArenaId) {
    let mut drops = RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let control = runtime
            .controls
            .get_mut(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        assert_eq!(control.state, ArenaState::Alive, "arena is not Alive");
        control.state = ArenaState::Destroying;
        std::mem::take(&mut control.drops)
    });

    while let Some(entry) = drops.pop() {
        // SAFETY: Entries are compiler-registered initialized values inside live payload bytes.
        unsafe { (entry.drop_fn)(entry.value) };
    }

    let should_free = RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let (payload, parent, mode, weak_count) = {
            let control = runtime
                .controls
                .get_mut(&id)
                .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
            assert_eq!(
                control.live_scoped_children, 0,
                "scoped dynamic child outlived its parent"
            );
            let payload = control.payload.take().expect("Alive arena has no payload");
            let parent = control.parent;
            let mode = control.mode;
            let weak_count = control.weak_count;
            control.state = ArenaState::Dead;
            (payload, parent, mode, weak_count)
        };
        if mode == ArenaMode::DynamicScoped {
            let parent = parent.expect("scoped arena has no parent");
            let parent_control = runtime
                .controls
                .get_mut(&parent)
                .unwrap_or_else(|| panic!("unknown arena handle {}", parent.as_u64()));
            assert!(
                parent_control.live_scoped_children > 0,
                "scoped child count underflow"
            );
            parent_control.live_scoped_children -= 1;
        }
        runtime.pool.release(payload);
        weak_count == 0
    });

    if should_free {
        free_control(id);
    }
}

pub(crate) fn register_drop_internal(arena: *mut c_void, value: *mut c_void, drop_fn: DropFn) {
    assert!(!value.is_null(), "drop value must not be null");
    with_control_mut(arena, |control| {
        assert_eq!(control.state, ArenaState::Alive, "drop owner is not Alive");
        control.drops.push(DropEntry { value, drop_fn });
    });
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_drop(
    arena: *mut c_void,
    value: *mut c_void,
    drop_fn: Option<DropFn>,
) {
    let drop_fn = drop_fn.expect("drop function must not be null");
    register_drop_internal(arena, value, drop_fn);
}

unsafe extern "C" fn drop_strong_root(value: *mut c_void) {
    release_strong_internal(value);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_register_strong_root(owner: *mut c_void, target: *mut c_void) {
    register_drop_internal(owner, target, drop_strong_root);
}

#[no_mangle]
pub extern "C" fn bork_arena_root() -> *mut c_void {
    root_token()
}

#[no_mangle]
pub extern "C" fn bork_arena_push() -> *mut c_void {
    new_control(ArenaId::new(1), ArenaMode::Lexical, 0)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_push_child(parent: *mut c_void) -> *mut c_void {
    let parent = if parent.is_null() {
        ArenaId::new(1)
    } else {
        id_from_token(parent)
    };
    new_control(parent, ArenaMode::Lexical, 0)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_create_dynamic(parent: *mut c_void, scoped: u8) -> *mut c_void {
    let parent = if scoped == 0 {
        ArenaId::new(1)
    } else {
        assert!(!parent.is_null(), "scoped dynamic arena requires a parent");
        id_from_token(parent)
    };
    let mode = if scoped == 0 {
        ArenaMode::DynamicFree
    } else {
        with_control(parent.token(), |control| {
            assert_eq!(control.mode, ArenaMode::Lexical);
        });
        ArenaMode::DynamicScoped
    };
    new_control(parent, mode, 1)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_reset(arena: *mut c_void) -> *mut c_void {
    let id = id_from_token(arena);
    let parent = with_control(arena, |control| {
        assert_eq!(
            control.mode,
            ArenaMode::Lexical,
            "only lexical arenas reset"
        );
        control.parent.expect("lexical arena has no parent")
    });
    destroy_control(id);
    new_control(parent, ArenaMode::Lexical, 0)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_pop(arena: *mut c_void) {
    let id = id_from_token(arena);
    with_control(arena, |control| {
        assert_eq!(control.mode, ArenaMode::Lexical);
    });
    destroy_control(id);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_id(arena: *mut c_void) -> u64 {
    with_control(arena, |control| control.arena_id.as_u64())
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_depth(arena: *mut c_void) -> usize {
    with_control(arena, |control| control.depth)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_state(arena: *mut c_void) -> ArenaState {
    state_of(arena)
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_strong_count(arena: *mut c_void) -> usize {
    with_control(arena, |control| control.strong_count)
}

pub(crate) fn retain_strong_internal(arena: *mut c_void) {
    with_control_mut(arena, |control| {
        assert!(
            matches!(
                control.mode,
                ArenaMode::DynamicFree | ArenaMode::DynamicScoped
            ),
            "strong reference target must be dynamic"
        );
        assert_eq!(
            control.state,
            ArenaState::Alive,
            "strong target is not Alive"
        );
        assert!(control.strong_count > 0, "cannot resurrect an arena");
        control.strong_count = control
            .strong_count
            .checked_add(1)
            .expect("arena strong_count overflow");
    });
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_retain_strong(arena: *mut c_void) {
    retain_strong_internal(arena);
}

pub(crate) fn release_strong_internal(arena: *mut c_void) {
    let id = id_from_token(arena);
    let destroy = RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let control = runtime
            .controls
            .get_mut(&id)
            .unwrap_or_else(|| panic!("unknown arena handle {}", id.as_u64()));
        assert!(control.strong_count > 0, "arena strong_count underflow");
        control.strong_count -= 1;
        control.strong_count == 0
    });
    if destroy {
        destroy_control(id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_release_strong(arena: *mut c_void) {
    release_strong_internal(arena);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_retain_weak(arena: *mut c_void) {
    retain_weak_target_internal(arena);
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_release_weak(arena: *mut c_void) {
    release_weak_internal(arena);
}

pub(crate) fn weak_upgrade_internal(arena: *mut c_void) -> bool {
    let can_upgrade = with_control(arena, |control| {
        control.state == ArenaState::Alive && control.strong_count > 0
    });
    if can_upgrade {
        retain_strong_internal(arena);
    }
    can_upgrade
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_weak_upgrade(arena: *mut c_void) -> bool {
    weak_upgrade_internal(arena)
}

fn is_ancestor(runtime: &RuntimeState, ancestor: ArenaId, source: ArenaId) -> bool {
    let ancestor_depth = runtime
        .controls
        .get(&ancestor)
        .unwrap_or_else(|| panic!("unknown arena handle {}", ancestor.as_u64()))
        .depth;
    let mut current = source;
    let source_depth = runtime
        .controls
        .get(&source)
        .unwrap_or_else(|| panic!("unknown arena handle {}", source.as_u64()))
        .depth;
    if ancestor_depth >= source_depth {
        return false;
    }
    while runtime
        .controls
        .get(&current)
        .unwrap_or_else(|| panic!("unknown arena handle {}", current.as_u64()))
        .depth
        > ancestor_depth
    {
        current = runtime
            .controls
            .get(&current)
            .and_then(|control| control.parent)
            .expect("broken arena parent chain");
    }
    current == ancestor
}

pub(crate) fn classify_internal(source: *mut c_void, target: *mut c_void) -> RefKind {
    let source = id_from_token(source);
    let target = id_from_token(target);
    if source == target {
        return RefKind::Arena;
    }
    if RUNTIME.with(|runtime| is_ancestor(&runtime.borrow(), target, source)) {
        return RefKind::Borrow;
    }
    if source.as_u64() < target.as_u64() {
        RefKind::Strong
    } else {
        RefKind::Weak
    }
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_classify(source: *mut c_void, target: *mut c_void) -> RefKind {
    classify_internal(source, target)
}

pub(crate) fn alloc_internal(arena: *mut c_void, size: usize, align: usize) -> *mut c_void {
    with_control_mut(arena, |control| {
        assert_eq!(
            control.state,
            ArenaState::Alive,
            "allocation arena is not Alive"
        );
        control
            .payload
            .as_mut()
            .expect("arena has no payload")
            .alloc(size, align)
            .cast()
    })
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_alloc(
    arena: *mut c_void,
    size: usize,
    align: usize,
) -> *mut c_void {
    alloc_internal(arena, size, align)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "arena overflow")]
    fn overflow_panics() {
        let mut arena = Arena::new();
        arena.alloc(ARENA_CAPACITY, 1);
        arena.alloc(1, 1);
    }

    #[test]
    #[should_panic(expected = "weak reference target must be dynamic")]
    fn weak_reference_target_must_be_dynamic() {
        let lexical = bork_arena_push();
        retain_weak_target_internal(lexical);
    }
}
