#[cfg(feature = "debug-runtime")]
pub(crate) mod debug;

use std::alloc::Layout;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroUsize;

const PAGE_SIZE: usize = 4096;
// Larger requests get their own system block; smaller ones live in pages.
const LARGE_THRESHOLD: usize = PAGE_SIZE / 2;
const MIN_CLASS: usize = 16;
// Free-list classes: 16, 32, ..., 2048 bytes.
const CLASSES: usize = 8;

struct Page {
    buf: [u8; PAGE_SIZE],
    offset: usize,
}

impl Page {
    fn new() -> Box<Self> {
        Box::new(Self {
            buf: [0; PAGE_SIZE],
            offset: 0,
        })
    }

    fn base(&self) -> usize {
        self.buf.as_ptr() as usize
    }

    fn contains(&self, ptr: *mut u8) -> bool {
        (self.base()..self.base() + PAGE_SIZE).contains(&(ptr as usize))
    }

    fn bump(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let base = self.base();
        let addr = base.checked_add(self.offset)?;
        let aligned_addr = addr.checked_add(align - 1)? & !(align - 1);
        let end = (aligned_addr - base).checked_add(size)?;
        if end > PAGE_SIZE {
            return None;
        }
        self.offset = end;
        Some(aligned_addr as *mut u8)
    }
}

/// Bump-allocated pages plus recycled blocks. Allocated blocks never move
/// unless the caller asks via `grow`.
struct Arena {
    pages: Vec<Box<Page>>,
    // Pages released by `reset`, reused before asking the system for more.
    spare: Vec<Box<Page>>,
    free_lists: [*mut u8; CLASSES],
    large: Vec<(*mut u8, Layout)>,
    // Upper bound on the free tail of any page but the newest: lets a miss skip the
    // older-page scan. Anything that grows an older tail must raise it.
    older_max_tail: usize,
    #[cfg(test)]
    probes: usize,
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

fn ceil_class(size: usize) -> usize {
    size.max(MIN_CLASS).next_power_of_two().trailing_zeros() as usize
        - MIN_CLASS.trailing_zeros() as usize
}

fn floor_class(size: usize) -> usize {
    (size.ilog2() as usize - MIN_CLASS.trailing_zeros() as usize).min(CLASSES - 1)
}

impl Arena {
    fn new() -> Self {
        Self {
            pages: vec![Page::new()],
            spare: Vec::new(),
            free_lists: [std::ptr::null_mut(); CLASSES],
            large: Vec::new(),
            older_max_tail: 0,
            #[cfg(test)]
            probes: 0,
        }
    }

    fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        assert!(align.is_power_of_two(), "align must be a power of two");
        if size > LARGE_THRESHOLD {
            return self.alloc_large(size, align);
        }
        if size >= MIN_CLASS {
            if let Some(ptr) = self.take_free_block(size, align) {
                return ptr;
            }
        }
        match self.bump_alloc(size, align) {
            Some(ptr) => ptr,
            // The alignment is too large for a page.
            None => self.alloc_large(size, align),
        }
    }

    /// Pops the first block of the size class that fits `size` and is aligned.
    /// The block may be larger than `size`; its slack is not tracked.
    fn take_free_block(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let mut link: *mut *mut u8 = &mut self.free_lists[ceil_class(size)];
        // SAFETY: every list node is a free block that stores its next pointer
        // in its first bytes, and `link` points at the list head or such a node.
        unsafe {
            loop {
                let block = link.read_unaligned();
                if block.is_null() {
                    return None;
                }
                let next = block.cast::<*mut u8>().read_unaligned();
                if block as usize % align == 0 {
                    link.write_unaligned(next);
                    return Some(block);
                }
                link = block.cast();
            }
        }
    }

    /// Bumps in the newest page, then in the tails of older pages, then in a new
    /// page. `None` means the alignment cannot be met inside a page.
    fn bump_alloc(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let last = self.pages.len() - 1;
        #[cfg(test)]
        {
            self.probes += 1;
        }
        if let Some(ptr) = self.pages[last].bump(size, align) {
            return Some(ptr);
        }
        // Tails of older pages stay usable after a miss, unless none can hold `size`.
        // shortcut: a hit still scans from page 0; add a first-fit cursor if that shows up.
        if size <= self.older_max_tail {
            let mut max_tail = 0;
            for page in &mut self.pages[..last] {
                #[cfg(test)]
                {
                    self.probes += 1;
                }
                if let Some(ptr) = page.bump(size, align) {
                    return Some(ptr);
                }
                max_tail = max_tail.max(PAGE_SIZE - page.offset);
            }
            self.older_max_tail = max_tail;
        }
        let mut page = self.spare.pop().unwrap_or_else(Page::new);
        let ptr = page.bump(size, align);
        if ptr.is_some() {
            let prev_tail = PAGE_SIZE - self.pages[last].offset;
            self.older_max_tail = self.older_max_tail.max(prev_tail);
            self.pages.push(page);
        } else {
            self.spare.push(page);
        }
        ptr
    }

    fn alloc_large(&mut self, size: usize, align: usize) -> *mut u8 {
        // `max(1)`: a size-0 request with a page-exceeding align lands here and must not become a zero-size layout.
        let layout =
            Layout::from_size_align(size.max(1).next_multiple_of(PAGE_SIZE), align.max(MIN_CLASS))
                .expect("invalid large allocation layout");
        // SAFETY: layout has non-zero size.
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        self.large.push((ptr, layout));
        ptr
    }

    fn owns(&self, ptr: *mut u8) -> bool {
        self.large.iter().any(|(p, _)| *p == ptr) || self.pages.iter().any(|p| p.contains(ptr))
    }

    /// If the block is the newest one on its page (it ends where the bump offset
    /// is), moves the offset back to its start so the space becomes page tail again.
    fn rewind_if_last(&mut self, ptr: *mut u8, size: usize) -> bool {
        let Some(page) = self.pages.iter_mut().find(|p| p.contains(ptr)) else {
            return false;
        };
        let start = ptr as usize - page.base();
        if start + size != page.offset {
            return false;
        }
        page.offset = start;
        self.older_max_tail = PAGE_SIZE;
        true
    }

    /// Returns a block of `size` bytes obtained from this arena for reuse.
    /// `size` must be what the block was allocated or last grown to.
    fn free(&mut self, ptr: *mut u8, size: usize) {
        if size == 0 {
            return;
        }
        debug_assert!(self.owns(ptr), "free of a block this arena does not own");
        // Membership, not size, picks the container: blocks grown in place or
        // over-aligned ones do not match their size class.
        if let Some(i) = self.large.iter().position(|(p, _)| *p == ptr) {
            let (p, layout) = self.large.swap_remove(i);
            // SAFETY: p/layout come from alloc_large.
            unsafe { std::alloc::dealloc(p, layout) };
            return;
        }
        if self.rewind_if_last(ptr, size) {
            return;
        }
        // shortcut: blocks under 16 bytes are not recycled, until a collection needs it.
        if size >= MIN_CLASS {
            let class = floor_class(size);
            // SAFETY: the block is at least MIN_CLASS bytes, enough for a pointer.
            unsafe {
                ptr.cast::<*mut u8>()
                    .write_unaligned(self.free_lists[class])
            };
            self.free_lists[class] = ptr;
        }
    }

    /// Resizes a block, in place when it is the last one on its page. Otherwise
    /// the block moves: callers must not keep the old address.
    fn grow(&mut self, ptr: *mut u8, old: usize, new: usize, align: usize) -> *mut u8 {
        if new <= old {
            return ptr;
        }
        debug_assert!(self.owns(ptr), "grow of a block this arena does not own");
        if let Some(i) = self.large.iter().position(|(p, _)| *p == ptr) {
            let (_, layout) = self.large[i];
            // SAFETY: ptr/layout come from alloc_large.
            let grown =
                unsafe { std::alloc::realloc(ptr, layout, new.next_multiple_of(PAGE_SIZE)) };
            if grown.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            let new_layout =
                Layout::from_size_align(new.next_multiple_of(PAGE_SIZE), layout.align()).unwrap();
            self.large[i] = (grown, new_layout);
            return grown;
        }
        if let Some(page) = self.pages.iter_mut().find(|p| p.contains(ptr)) {
            let start = ptr as usize - page.base();
            if start + old == page.offset && start + new <= PAGE_SIZE {
                page.offset = start + new;
                self.older_max_tail = PAGE_SIZE;
                return ptr;
            }
        }
        let moved = self.alloc(new, align);
        // SAFETY: both blocks are live, distinct and at least `old` bytes.
        unsafe { std::ptr::copy_nonoverlapping(ptr, moved, old) };
        self.free(ptr, old);
        moved
    }

    /// Keeps the first page so recycled arenas start with the same address.
    fn reset(&mut self) {
        self.pages[0].offset = 0;
        self.older_max_tail = 0;
        for mut page in self.pages.drain(1..) {
            page.offset = 0;
            self.spare.push(page);
        }
        self.free_lists = [std::ptr::null_mut(); CLASSES];
        for (p, layout) in self.large.drain(..) {
            // SAFETY: p/layout come from alloc_large.
            unsafe { std::alloc::dealloc(p, layout) };
        }
    }

    #[cfg(feature = "debug-runtime")]
    fn debug_pages(&self) -> Vec<(usize, usize, usize)> {
        let pages = self.pages.iter().map(|p| (p.base(), p.offset, PAGE_SIZE));
        let large = self
            .large
            .iter()
            .map(|(p, l)| (*p as usize, l.size(), l.size()));
        pages.chain(large).collect()
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        self.reset();
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

#[cfg(feature = "debug-runtime")]
impl Drop for RuntimeState {
    fn drop(&mut self) {
        debug::invalidate();
    }
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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

pub(crate) fn weak_count_of(arena: *mut c_void) -> usize {
    with_control(arena, |control| control.weak_count)
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
    #[cfg(feature = "debug-runtime")]
    debug::remove_owner(id.as_usize());
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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
    if arena.is_null() {
        return 0;
    }
    with_control(arena, |control| control.strong_count)
}

/// Pins `arena`'s control record while a count is read, so releasing the last reference cannot
/// free it first. Returns whether a lease was taken (a null `None` token has none). Unlike
/// `bork_arena_retain_weak` it also works on a dead arena.
#[no_mangle]
pub unsafe extern "C" fn bork_arena_count_lease_retain(arena: *mut c_void) -> bool {
    if arena.is_null() {
        return false;
    }
    retain_weak_internal(arena);
    true
}

/// Releases a lease taken by `bork_arena_count_lease_retain`; frees the control if it was the last.
#[no_mangle]
pub unsafe extern "C" fn bork_arena_count_lease_release(arena: *mut c_void) {
    if !arena.is_null() {
        release_weak_internal(arena);
    }
}

/// Diagnostic: weak leases on `arena`, which include every child arena created under it.
/// A null token (a `None` Ref) reads as 0.
#[no_mangle]
pub unsafe extern "C" fn bork_arena_weak_count(arena: *mut c_void) -> usize {
    if arena.is_null() {
        return 0;
    }
    weak_count_of(arena)
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
    #[cfg(feature = "debug-runtime")]
    let _mutation = debug::Mutation::begin();
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

fn with_payload<R>(arena: *mut c_void, f: impl FnOnce(&mut Arena) -> R) -> R {
    with_control_mut(arena, |control| {
        assert_eq!(control.state, ArenaState::Alive, "arena is not Alive");
        f(control.payload.as_mut().expect("arena has no payload"))
    })
}

pub(crate) fn alloc_internal(arena: *mut c_void, size: usize, align: usize) -> *mut c_void {
    with_payload(arena, |payload| payload.alloc(size, align).cast())
}

#[no_mangle]
pub unsafe extern "C" fn bork_arena_alloc(
    arena: *mut c_void,
    size: usize,
    align: usize,
) -> *mut c_void {
    alloc_internal(arena, size, align)
}

pub(crate) fn grow_internal(
    arena: *mut c_void,
    ptr: *mut c_void,
    old: usize,
    new: usize,
    align: usize,
) -> *mut c_void {
    with_payload(arena, |payload| {
        payload.grow(ptr.cast(), old, new, align).cast()
    })
}

/// Resizes a block from `arena`. The block may move; the old pointer is then dead.
#[no_mangle]
pub unsafe extern "C" fn bork_arena_grow(
    arena: *mut c_void,
    ptr: *mut c_void,
    old_size: usize,
    new_size: usize,
    align: usize,
) -> *mut c_void {
    grow_internal(arena, ptr, old_size, new_size, align)
}

/// Returns a block (with the size it was allocated or last grown to) for reuse.
#[no_mangle]
pub unsafe extern "C" fn bork_arena_free(arena: *mut c_void, ptr: *mut c_void, size: usize) {
    with_payload(arena, |payload| payload.free(ptr.cast(), size));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(ptr: *mut u8, n: usize, v: u8) {
        unsafe { std::ptr::write_bytes(ptr, v, n) };
    }

    fn all(ptr: *mut u8, n: usize, v: u8) -> bool {
        unsafe { std::slice::from_raw_parts(ptr, n) }
            .iter()
            .all(|b| *b == v)
    }

    #[test]
    fn exceeding_a_page_adds_a_page() {
        let mut arena = Arena::new();
        let a = arena.alloc(2000, 1);
        let b = arena.alloc(2000, 1);
        let c = arena.alloc(2000, 1);
        assert_eq!(arena.pages.len(), 2);
        assert!(a != b && b != c);
    }

    #[test]
    fn zero_size_overaligned_alloc_uses_a_nonzero_layout() {
        let mut arena = Arena::new();
        let ptr = arena.alloc_large(0, 8192); // direct: whether `alloc` reaches it depends on the page address
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 8192, 0);
        assert_eq!(arena.large.len(), 1);
        assert!(arena.large[0].1.size() >= PAGE_SIZE);
        arena.free(ptr, 0);
        arena.reset();
        assert!(arena.large.is_empty());
    }

    #[test]
    fn mid_size_allocs_do_not_rescan_old_pages() {
        let mut arena = Arena::new();
        let allocs = 4000;
        for _ in 0..allocs {
            arena.alloc(1800, 1);
        }
        assert!(arena.pages.len() >= allocs / 2);
        assert!(arena.probes <= 3 * allocs, "{} probes", arena.probes);
    }

    #[test]
    fn older_tail_grown_by_rewind_is_found() {
        let mut arena = Arena::new();
        let a = arena.alloc(2000, 1);
        let b = arena.alloc(2040, 1); // page 0 is now full
        arena.alloc(2000, 1);
        arena.alloc(2000, 1); // a miss records that older pages have no room
        arena.free(b, 2040); // rewinds page 0 by 2040 bytes
        let c = arena.alloc(2000, 1);
        assert!(arena.pages[0].contains(c), "rewound tail of page 0 is reusable");
        let _ = a;
    }

    #[test]
    fn tail_bound_survives_reset() {
        let mut arena = Arena::new();
        for _ in 0..6 {
            arena.alloc(1800, 1);
        }
        arena.reset();
        let p = arena.alloc(1800, 1);
        assert!(arena.pages[0].contains(p));
        let q = arena.alloc(1800, 1);
        assert!(arena.pages[0].contains(q), "page 0 still has room");
        let r = arena.alloc(1800, 1);
        assert!(arena.pages.len() == 2 && arena.pages[1].contains(r));
    }

    #[test]
    fn old_page_tail_is_reused() {
        let mut arena = Arena::new();
        arena.alloc(2000, 1);
        arena.alloc(1900, 1); // page 0 tail: 196 bytes
        arena.alloc(2000, 1);
        arena.alloc(2000, 1); // page 1 tail: 96 bytes
        let small = arena.alloc(150, 1);
        assert!(arena.pages[0].contains(small), "tail of page 0 has room");
    }

    #[test]
    fn large_block_is_contiguous_and_freed() {
        let mut arena = Arena::new();
        let p = arena.alloc(10_000, 8);
        fill(p, 10_000, 7);
        assert_eq!(arena.large.len(), 1);
        arena.free(p, 10_000);
        assert!(arena.large.is_empty());
    }

    #[test]
    fn freeing_last_block_rewinds() {
        let mut arena = Arena::new();
        arena.alloc(100, 1);
        let b = arena.alloc(100, 1);
        arena.free(b, 100);
        assert_eq!(arena.alloc(100, 1), b);
    }

    #[test]
    fn freed_hole_is_reused_by_same_class() {
        let mut arena = Arena::new();
        let a = arena.alloc(64, 8);
        arena.alloc(8, 8);
        arena.free(a, 64);
        assert_eq!(arena.alloc(64, 8), a);
    }

    #[test]
    fn aligned_block_behind_a_misaligned_head_is_found() {
        let mut arena = Arena::new();
        let a = arena.alloc(32, 16);
        arena.alloc(1, 1);
        let b = arena.alloc(32, 1);
        arena.alloc(1, 1);
        arena.free(a, 32);
        arena.free(b, 32); // list head is b
        let p = arena.alloc(32, 16);
        if b as usize % 16 != 0 {
            assert_eq!(p, a);
            assert_eq!(arena.alloc(32, 1), b, "b stays on the list");
        }
        assert_eq!(p as usize % 16, 0);
    }

    #[test]
    fn misaligned_free_block_is_skipped() {
        let mut arena = Arena::new();
        arena.alloc(1, 1);
        let a = arena.alloc(32, 1);
        arena.alloc(8, 8);
        arena.free(a, 32);
        let p = arena.alloc(32, 16);
        assert_eq!(p as usize % 16, 0);
    }

    #[test]
    fn grow_last_block_stays_in_place() {
        let mut arena = Arena::new();
        let p = arena.alloc(64, 8);
        fill(p, 64, 3);
        let g = arena.grow(p, 64, 256, 8);
        assert_eq!(g, p);
        assert!(all(g, 64, 3));
    }

    #[test]
    fn grow_blocked_block_moves_and_keeps_data() {
        let mut arena = Arena::new();
        let p = arena.alloc(64, 8);
        fill(p, 64, 5);
        arena.alloc(8, 8);
        let g = arena.grow(p, 64, 256, 8);
        assert_ne!(g, p);
        assert!(all(g, 64, 5));
        // the old hole is recycled
        assert_eq!(arena.alloc(64, 8), p);
    }

    #[test]
    fn block_grown_in_place_past_threshold_still_grows_and_frees() {
        let mut arena = Arena::new();
        let p = arena.alloc(100, 8);
        let g = arena.grow(p, 100, 3000, 8);
        assert_eq!(g, p);
        let g2 = arena.grow(g, 3000, 6000, 8);
        assert!(arena.large.len() == 1 && all(g2, 100, 0));
        arena.free(g2, 6000);
        assert!(arena.large.is_empty());
        let q = arena.alloc(100, 8);
        let h = arena.grow(q, 100, 3000, 8);
        arena.free(h, 3000);
        assert_eq!(arena.alloc(100, 8), q, "rewound");
    }

    #[test]
    fn small_block_in_large_storage_frees_its_system_allocation() {
        let mut arena = Arena::new();
        // What alloc falls back to when a page cannot satisfy the alignment.
        let p = arena.alloc_large(64, 8192);
        assert_eq!(arena.large.len(), 1);
        arena.free(p, 64);
        assert!(arena.large.is_empty());
    }

    #[test]
    fn grow_small_to_large_and_large_realloc() {
        let mut arena = Arena::new();
        let p = arena.alloc(1000, 8);
        fill(p, 1000, 9);
        arena.alloc(8, 8);
        let big = arena.grow(p, 1000, 5000, 8);
        assert_eq!(arena.large.len(), 1);
        assert!(all(big, 1000, 9));
        let bigger = arena.grow(big, 5000, 20_000, 8);
        assert!(all(bigger, 1000, 9));
        assert_eq!(arena.large.len(), 1);
        assert_eq!(arena.large[0].1.size(), 20_480);
    }

    #[test]
    fn reset_keeps_first_page_and_drops_the_rest() {
        let mut arena = Arena::new();
        let first = arena.alloc(8, 8);
        for _ in 0..3 {
            arena.alloc(1500, 1); // third one needs page 2
        }
        arena.alloc(10_000, 1);
        arena.reset();
        assert_eq!(arena.pages.len(), 1);
        assert!(arena.large.is_empty());
        assert_eq!(arena.spare.len(), 1);
        assert_eq!(arena.alloc(8, 8), first);
        for _ in 0..3 {
            arena.alloc(1500, 1);
        }
        assert!(arena.spare.is_empty(), "page 2 came from spare");
    }

    #[test]
    fn exported_grow_and_free_keep_data_and_recycle() {
        let arena = bork_arena_push();
        unsafe {
            let p = bork_arena_alloc(arena, 64, 8).cast::<u8>();
            fill(p, 64, 4);
            let wall = bork_arena_alloc(arena, 8, 8);
            let g = bork_arena_grow(arena, p.cast(), 64, 256, 8).cast::<u8>();
            assert_ne!(g, p);
            assert!(all(g, 64, 4));
            bork_arena_free(arena, g.cast(), 256);
            bork_arena_free(arena, wall, 8);
            assert_eq!(bork_arena_alloc(arena, 64, 8), p.cast(), "old hole reused");
            bork_arena_pop(arena);
        }
    }

    #[test]
    fn runtime_arena_survives_more_than_a_page() {
        let arena = bork_arena_push();
        unsafe {
            let p = bork_arena_alloc(arena, 10_000, 1).cast::<u8>();
            fill(p, 10_000, 1);
            let q = bork_arena_alloc(arena, 8, 8);
            let g = bork_arena_grow(arena, q, 8, 64, 8);
            assert!(!g.is_null());
            bork_arena_free(arena, g, 64);
            bork_arena_pop(arena);
        }
    }

    #[test]
    #[should_panic(expected = "weak reference target must be dynamic")]
    fn weak_reference_target_must_be_dynamic() {
        let lexical = bork_arena_push();
        retain_weak_target_internal(lexical);
    }
}

#[cfg(all(test, feature = "debug-runtime"))]
mod debug_tests {
    use super::*;
    #[test]
    fn mirrors_lifetime_reset_and_payload() {
        let owner = bork_arena_push();
        let before = debug::test_arenas();
        let record = before.iter().find(|a| a.id == owner.addr()).unwrap();
        assert_eq!(record.state, 0);
        assert_eq!(record.used, 0);
        let first_base = unsafe { (*(record.pages as *const debug::DebugPage)).base };
        unsafe {
            bork_arena_alloc(owner, 9, 1);
        }
        assert_eq!(
            debug::test_arenas()
                .iter()
                .find(|a| a.id == owner.addr())
                .unwrap()
                .used,
            9
        );
        let new = unsafe { bork_arena_reset(owner) };
        assert_ne!(new, owner);
        let records = debug::test_arenas();
        assert!(!records.iter().any(|a| a.id == owner.addr()));
        let reused = records.iter().find(|a| a.id == new.addr()).unwrap();
        assert_eq!(
            unsafe { (*(reused.pages as *const debug::DebugPage)).base },
            first_base
        );
        unsafe {
            bork_arena_pop(new);
        }
    }
    #[test]
    fn mirrors_every_page_and_large_block() {
        let owner = bork_arena_push();
        unsafe {
            bork_arena_alloc(owner, 1500, 1);
            bork_arena_alloc(owner, 1500, 1);
            bork_arena_alloc(owner, 1500, 1);
            bork_arena_alloc(owner, 10_000, 1);
        }
        let records = debug::test_arenas();
        let a = records.iter().find(|a| a.id == owner.addr()).unwrap();
        assert_eq!(a.page_count, 3);
        assert_eq!(a.used, 3000 + 1500 + 12_288);
        let pages = unsafe { std::slice::from_raw_parts(a.pages as *const debug::DebugPage, 3) };
        assert_eq!(pages[0].used, 3000);
        assert_eq!(pages[2].capacity, 12_288);
        unsafe {
            bork_arena_pop(owner);
        }
    }
    #[test]
    fn retains_dead_weak_metadata() {
        let arena = unsafe { bork_arena_create_dynamic(std::ptr::null_mut(), 0) };
        unsafe {
            bork_arena_retain_weak(arena);
            bork_arena_release_strong(arena);
        }
        let records = debug::test_arenas();
        let a = records.iter().find(|a| a.id == arena.addr()).unwrap();
        assert_eq!(
            (a.state, a.strong_count, a.weak_count, a.page_count),
            (2, 0, 1, 0)
        );
        unsafe {
            bork_arena_release_weak(arena);
        }
        assert!(!debug::test_arenas().iter().any(|a| a.id == arena.addr()));
    }
}
