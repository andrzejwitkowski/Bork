//! Allocation diagnostics live outside payload pages and the debugger's stable ABI.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void, CStr},
    fmt,
    io::Write,
    rc::Rc,
};

#[repr(C)]
pub struct SourceLocation {
    pub path: *const c_char,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Location {
    path: Rc<str>,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            return f.write_str("<runtime>");
        }
        write!(
            f,
            "{}:{}:{} [bytes {}..{}]",
            self.path, self.line, self.column, self.start, self.end
        )
    }
}

unsafe fn string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

unsafe fn location(p: *const SourceLocation) -> Location {
    if p.is_null() {
        return current_location();
    }
    let p = &*p;
    Location {
        path: string(p.path).into(),
        start: p.start,
        end: p.end,
        line: p.line,
        column: p.column,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MemoryState {
    Live,
    Freed,
    UnusedInPool,
}

#[derive(Clone, Debug)]
pub(super) struct DebugDiag {
    pub start: usize,
    pub size: usize,
    pub id: usize,
    pub owner: usize,
    arena: Option<Rc<Arena>>,
    pub state: MemoryState,
    pub name: String,
    pub type_name: String,
    created: Location,
    declared: Option<Location>,
    released: Option<Location>,
}

#[derive(Clone)]
struct Range {
    end: usize,
    current: DebugDiag,
    // A flat description deliberately cannot own another history entry.
    previous: Option<DebugDiag>,
}

#[derive(Debug)]
struct Arena {
    id: usize,
    parent: Option<Rc<Arena>>,
    created: Location,
}

struct Frame {
    name: String,
    site: Location,
}

#[derive(Default)]
struct Diagnostics {
    next_id: usize,
    ranges: BTreeMap<usize, Range>,
    covered: BTreeMap<usize, usize>,
    owned_ranges: BTreeMap<usize, BTreeSet<usize>>,
    arenas: BTreeMap<usize, Rc<Arena>>,
    stack: Vec<Frame>,
    site: Location,
    #[cfg(test)]
    gap_range_visits: usize,
}

thread_local! { static DIAG: RefCell<Diagnostics> = RefCell::new(Diagnostics::default()); }

pub(crate) fn current_location() -> Location {
    DIAG.with(|d| {
        let d = d.borrow();
        d.stack
            .last()
            .map_or_else(|| d.site.clone(), |f| f.site.clone())
    })
}

pub(crate) fn arena_created(id: usize, parent: usize) {
    let created = current_location();
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        let parent = d.arenas.get(&parent).cloned().unwrap_or_else(|| {
            Rc::new(Arena {
                id: parent,
                parent: None,
                created: Location::default(),
            })
        });
        d.arenas.insert(
            id,
            Rc::new(Arena {
                id,
                parent: Some(parent),
                created,
            }),
        );
    });
}

impl Diagnostics {
    fn register_page(&mut self, owner: usize, page: (usize, usize), created: &Location) {
        let (base, capacity) = page;
        let end = base + capacity;
        let preceding = self.covered.range(..=base).next_back();
        if preceding.is_some_and(|(_, &covered_end)| covered_end >= end) {
            return;
        }
        let merged_start = preceding
            .filter(|(_, &covered_end)| covered_end >= base)
            .map_or(base, |(&start, _)| start);

        // Range replacement preserves coverage, including unused tails and pooled pages.
        let mut cursor = base;
        let occupied: Vec<_> = self
            .covered
            .range(merged_start..=end)
            .map(|(&start, &end)| (start, end))
            .collect();
        #[cfg(test)]
        {
            self.gap_range_visits += occupied.len();
        }
        let mut merged_end = end;
        for (key, range_end) in occupied.into_iter().chain(std::iter::once((end, end))) {
            if cursor < key {
                let empty = DebugDiag {
                    start: cursor,
                    size: key - cursor,
                    id: 0,
                    owner,
                    arena: self.arenas.get(&owner).cloned(),
                    state: MemoryState::UnusedInPool,
                    name: String::new(),
                    type_name: String::new(),
                    created: created.clone(),
                    declared: None,
                    released: None,
                };
                self.insert_range(
                    cursor,
                    Range {
                        end: key,
                        current: empty,
                        previous: None,
                    },
                );
            }
            cursor = cursor.max(range_end);
            merged_end = merged_end.max(range_end);
            self.covered.remove(&key);
        }
        self.covered.insert(merged_start, merged_end);
    }

    fn insert_range(&mut self, start: usize, range: Range) {
        let owner = range.current.owner;
        if let Some(old) = self.ranges.insert(start, range) {
            if let Some(keys) = self.owned_ranges.get_mut(&old.current.owner) {
                keys.remove(&start);
            }
        }
        if self.arenas.contains_key(&owner) {
            self.owned_ranges.entry(owner).or_default().insert(start);
        }
    }

    fn remove_range(&mut self, start: usize) -> Range {
        let old = self.ranges.remove(&start).unwrap();
        if let Some(keys) = self.owned_ranges.get_mut(&old.current.owner) {
            keys.remove(&start);
        }
        old
    }

    fn overlaps(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, &Range)> {
        let first = self
            .ranges
            .range(..=start)
            .next_back()
            .map_or(start, |(&a, _)| a);
        self.ranges
            .range(first..end)
            .filter_map(move |(&a, r)| (r.end > start).then_some((a, r)))
    }

    fn replace(&mut self, start: usize, end: usize, current: DebugDiag) {
        let mut cursor = start;
        let keys: Vec<_> = self.overlaps(start, end).map(|(key, _)| key).collect();
        for key in keys {
            let old = self.remove_range(key);
            if key < start {
                self.insert_range(
                    key,
                    Range {
                        end: start,
                        ..old.clone()
                    },
                );
            }
            let lo = key.max(start);
            let hi = old.end.min(end);
            if cursor < lo {
                self.insert_range(
                    cursor,
                    Range {
                        end: lo,
                        current: current.clone(),
                        previous: None,
                    },
                );
            }
            let previous = if old.current.id == current.id {
                old.previous.clone()
            } else if old.current.id != 0 {
                Some(old.current.clone())
            } else {
                None
            };
            self.insert_range(
                lo,
                Range {
                    end: hi,
                    current: current.clone(),
                    previous,
                },
            );
            cursor = hi;
            if old.end > end {
                self.insert_range(end, old);
            }
        }
        if cursor < end {
            self.insert_range(
                cursor,
                Range {
                    end,
                    current,
                    previous: None,
                },
            );
        }
    }

    fn edit_allocation(&mut self, address: usize, mut edit: impl FnMut(&mut DebugDiag)) {
        let Some(r) = self.ranges.get(&address) else {
            return;
        };
        let (start, end, id) = (
            r.current.start,
            r.current.start + r.current.size.max(1),
            r.current.id,
        );
        for (_, r) in self.ranges.range_mut(start..end) {
            if r.current.id == id && r.current.state == MemoryState::Live {
                edit(&mut r.current);
            }
        }
    }
}

pub(crate) fn allocated(owner: usize, start: usize, size: usize, page: (usize, usize)) {
    track(owner, start, size, page, None);
}

fn track(owner: usize, start: usize, size: usize, page: (usize, usize), origin: Option<DebugDiag>) {
    let created = current_location();
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        d.register_page(owner, page, &created);
        d.next_id += 1;
        let mut current = origin.unwrap_or_else(|| DebugDiag {
            start,
            size,
            id: d.next_id,
            owner,
            arena: d.arenas.get(&owner).cloned(),
            state: MemoryState::Live,
            name: String::new(),
            type_name: String::new(),
            created,
            declared: None,
            released: None,
        });
        current.start = start;
        current.size = size;
        current.owner = owner;
        current.arena = d.arenas.get(&owner).cloned();
        current.state = MemoryState::Live;
        current.released = None;
        d.replace(
            start,
            start
                .checked_add(size.max(1))
                .expect("allocation range overflow"),
            current,
        );
    });
}

pub(crate) fn freed(start: usize) {
    let site = current_location();
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        d.edit_allocation(start, |a| {
            a.state = MemoryState::Freed;
            a.released = Some(site.clone());
        });
    });
}

pub(crate) fn retired(owner: usize, site: Location) {
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        d.arenas.remove(&owner);
        let keys = d.owned_ranges.remove(&owner).unwrap_or_default();
        for key in keys {
            let r = d.ranges.get_mut(&key).unwrap();
            r.current.state = MemoryState::UnusedInPool;
            if r.current.released.is_none() {
                r.current.released = Some(site.clone());
            }
        }
    });
}

pub(crate) fn grown(owner: usize, old: usize, new: usize, size: usize, page: (usize, usize)) {
    let origin = DIAG.with(|d| d.borrow().ranges.get(&old).map(|r| r.current.clone()));
    if old != new {
        freed(old);
    }
    track(owner, new, size, page, origin);
}

pub(super) fn label(
    address: usize,
    name: String,
    type_name: String,
    site: Location,
    declaration: bool,
) {
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        d.edit_allocation(address, |a| {
            // An alias cannot rename the original declaration or the concrete allocation type.
            if a.name.is_empty() && !name.is_empty() {
                a.name = name.clone();
                a.declared = Some(site.clone());
            }
            if a.type_name.is_empty() {
                a.type_name = type_name.clone();
            }
            if !declaration && a.created.path.is_empty() {
                a.created = site.clone();
            }
        });
    });
}

pub(super) struct Failure {
    pub current: DebugDiag,
    pub previous: Option<DebugDiag>,
}

pub(super) fn failure(start: usize, size: usize, owner: usize) -> Option<Failure> {
    // shortcut: raw pointers cannot identify reused live bytes; that requires a pointer ABI change.
    if size == 0 {
        return None;
    }
    DIAG.with(|d| {
        let d = d.borrow();
        let end = start.checked_add(size);
        let ranges = d.overlaps(start, end.unwrap_or(usize::MAX));
        let mut cursor = start;
        let mut allocation = None;
        let mut first = None;
        for (key, r) in ranges {
            first.get_or_insert(r);
            let valid = key <= cursor
                && end.is_some()
                && r.current.state == MemoryState::Live
                && (r.current.size != 0 || owner != 0)
                && (owner == 0 || r.current.owner == owner)
                && (owner == 0 || d.arenas.contains_key(&owner))
                && allocation.is_none_or(|id| id == r.current.id);
            if !valid {
                return Some(Failure {
                    current: r.current.clone(),
                    previous: r.previous.clone(),
                });
            }
            allocation = Some(r.current.id);
            cursor = r.end;
            if cursor >= end.unwrap() {
                return None;
            }
        }
        if let Some(r) = first {
            return Some(Failure {
                current: r.current.clone(),
                previous: r.previous.clone(),
            });
        }
        if start == 0 || end.is_none() || (owner != 0 && !d.arenas.contains_key(&owner)) {
            return Some(Failure {
                current: DebugDiag {
                    start,
                    size,
                    id: 0,
                    owner,
                    arena: d.arenas.get(&owner).cloned(),
                    state: MemoryState::UnusedInPool,
                    name: String::new(),
                    type_name: String::new(),
                    created: Location::default(),
                    declared: None,
                    released: None,
                },
                previous: None,
            });
        }
        None // Stack and program constants have no arena allocation record.
    })
}

fn report(
    start: usize,
    size: usize,
    op: &str,
    site: &Location,
    failure: &Failure,
    expected: usize,
) -> String {
    use fmt::Write;
    let a = &failure.current;
    let mut out = format!("Bork DebugDiag: invalid {op} at 0x{start:x}, size {size}, state {:?}\n  access: {site}\n  allocation #{} at 0x{:x}, size {}, arena #{} (handle arena #{expected})\n  name: {}\n  type: {}\n  created: {}\n", a.state, a.id, a.start, a.size, a.owner, a.name, a.type_name, a.created);
    if let Some(s) = &a.declared {
        let _ = writeln!(out, "  declared: {s}");
    }
    if let Some(s) = &a.released {
        let _ = writeln!(out, "  released: {s}");
    }
    if let Some(p) = &failure.previous {
        let _ = writeln!(out, "  previous occupancy (history): allocation #{}, arena #{}, {}: {}, created: {}, released: {}", p.id, p.owner, p.name, p.type_name, p.created, p.released.as_ref().unwrap_or(&p.created));
    }
    DIAG.with(|d| {
        let d = d.borrow();
        let _ = writeln!(out, "  Bork call stack:");
        for f in d.stack.iter().rev() {
            let _ = writeln!(out, "    {} at {}", f.name, f.site);
        }
        let _ = writeln!(out, "  arena chain:");
        let id = if expected == 0 { a.owner } else { expected };
        let mut arena = a
            .arena
            .as_ref()
            .filter(|a| a.id == id)
            .or_else(|| {
                failure
                    .previous
                    .as_ref()
                    .and_then(|p| p.arena.as_ref())
                    .filter(|a| a.id == id)
            })
            .or_else(|| d.arenas.get(&id));
        if arena.is_none() {
            let _ = writeln!(out, "    arena #{id}");
        }
        while let Some(a) = arena {
            let _ = writeln!(out, "    arena #{}, created: {}", a.id, a.created);
            arena = a.parent.as_ref();
        }
    });
    out
}

pub(crate) fn check(address: usize, size: usize, write: bool) {
    check_at(
        address,
        size,
        if write { "write" } else { "read" },
        current_location,
        0,
    );
}

fn check_at(address: usize, size: usize, op: &str, site: impl FnOnce() -> Location, owner: usize) {
    if let Some(f) = failure(address, size, owner) {
        let _ = std::io::stderr()
            .lock()
            .write_all(report(address, size, op, &site(), &f, owner).as_bytes());
        std::process::abort();
    }
}

pub(crate) fn check_handle(owner: usize, address: usize) {
    check_at(address, 1, "handle access", current_location, owner);
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_check(
    address: *const c_void,
    size: usize,
    operation: u8,
    site: *const SourceLocation,
) {
    check_at(
        address as usize,
        size,
        if operation == 0 { "read" } else { "write" },
        || location(site),
        0,
    );
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_check_handle(
    owner: *const c_void,
    address: *const c_void,
    site: *const SourceLocation,
) {
    check_at(
        address as usize,
        1,
        "handle access",
        || location(site),
        owner as usize,
    );
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_site(site: *const SourceLocation) {
    let site = location(site);
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        if let Some(frame) = d.stack.last_mut() {
            frame.site = site;
        } else {
            d.site = site;
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_frame_enter(name: *const c_char, site: *const SourceLocation) {
    let frame = Frame {
        name: string(name),
        site: location(site),
    };
    DIAG.with(|d| d.borrow_mut().stack.push(frame));
}

#[no_mangle]
pub extern "C" fn bork_debug_frame_exit() {
    DIAG.with(|d| {
        d.borrow_mut().stack.pop();
    });
}

#[no_mangle]
pub unsafe extern "C" fn bork_debug_label_allocation(
    address: *const c_void,
    name: *const c_char,
    type_name: *const c_char,
    site: *const SourceLocation,
    declaration: u8,
) {
    label(
        address as usize,
        string(name),
        string(type_name),
        location(site),
        declaration != 0,
    );
}

#[no_mangle]
pub extern "C" fn bork_debug_copy_origin(source: *const c_void, destination: *const c_void) {
    DIAG.with(|d| {
        let mut d = d.borrow_mut();
        let origin = d
            .ranges
            .range(..=source as usize)
            .next_back()
            .filter(|(_, r)| (source as usize) < r.end && r.current.state == MemoryState::Live)
            .map(|(_, r)| r.current.clone());
        if let Some(origin) = origin {
            d.edit_allocation(destination as usize, |dest| {
                dest.created = origin.created.clone();
                dest.declared = origin.declared.clone();
                dest.name = origin.name.clone();
                if dest.type_name.is_empty() {
                    dest.type_name = origin.type_name.clone();
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::*;

    #[test]
    fn source_paths_are_shared_by_location_snapshots() {
        std::thread::spawn(|| unsafe {
            let site = SourceLocation {
                path: c"fixture.bork".as_ptr(),
                start: 1,
                end: 2,
                line: 1,
                column: 2,
            };
            bork_debug_site(&site);
            let first = current_location();
            let second = current_location();
            assert_eq!(first.path.as_ptr(), second.path.as_ptr());
            arena_created(1, 0);
            allocated(1, 0x1000, 16, (0x1000, 4096));
            freed(0x1000);
            assert_eq!(
                failure(0x1000, 1, 0).unwrap().current.created.path.as_ptr(),
                first.path.as_ptr()
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn small_allocations_do_not_rescan_registered_page_ranges() {
        std::thread::spawn(|| {
            arena_created(1, 0);
            allocated(1, 0x1000, 16, (0x1000, 4096));
            let before = DIAG.with(|d| d.borrow().gap_range_visits);
            for i in 1..128 {
                allocated(1, 0x1000 + i * 16, 16, (0x1000, 4096));
            }
            assert_eq!(DIAG.with(|d| d.borrow().gap_range_visits), before);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn registered_pages_merge_and_extend_without_reviving_old_bytes() {
        std::thread::spawn(|| {
            arena_created(1, 0);
            for base in [0x1000, 0x1200, 0x1100] {
                allocated(1, base, 16, (base, 0x100));
                freed(base);
            }
            allocated(1, 0x1100, 32, (0x1100, 0x300));
            allocated(1, 0xf00, 16, (0xf00, 0x200));
            assert!(failure(0x1100, 32, 1).is_none());
            for address in [0x1000, 0x1200] {
                assert_eq!(
                    failure(address, 1, 0).unwrap().current.state,
                    MemoryState::Freed
                );
            }
            assert_eq!(
                failure(0x13ff, 1, 0).unwrap().current.state,
                MemoryState::UnusedInPool
            );
            assert!(failure(0x1400, 1, 0).is_none());
            DIAG.with(|d| {
                let d = d.borrow();
                assert_eq!(d.covered.len(), 1);
                assert_eq!(d.covered.get(&0xf00), Some(&0x1400));
            });
        })
        .join()
        .unwrap();
    }

    #[test]
    fn retiring_an_owner_preserves_another_owners_reused_subrange() {
        std::thread::spawn(|| {
            arena_created(1, 0);
            arena_created(2, 0);
            allocated(1, 0x1000, 64, (0x1000, 4096));
            freed(0x1000);
            allocated(2, 0x1010, 16, (0x1000, 4096));
            retired(1, Location::default());
            for address in [0x1000, 0x1020, 0x1040] {
                assert_eq!(
                    failure(address, 1, 0).unwrap().current.state,
                    MemoryState::UnusedInPool
                );
            }
            assert!(failure(0x1010, 16, 2).is_none());
            retired(2, Location::default());
            let failed = failure(0x1010, 16, 0).unwrap();
            assert_eq!(failed.current.state, MemoryState::UnusedInPool);
            assert_eq!(failed.previous.unwrap().owner, 1);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn recycling_keeps_only_live_arena_records_and_referenced_ancestors() {
        let before = DIAG.with(|d| d.borrow().arenas.len());
        let indexed_before = DIAG.with(|d| d.borrow().owned_ranges.len());
        let mut arena = bork_arena_push();
        for _ in 0..100 {
            unsafe {
                bork_arena_alloc(arena, 8, 8);
                arena = bork_arena_reset(arena);
            }
        }
        assert_eq!(DIAG.with(|d| d.borrow().arenas.len()), before + 1);
        assert_eq!(DIAG.with(|d| d.borrow().owned_ranges.len()), indexed_before);
        unsafe {
            bork_arena_pop(arena);
        }
        assert_eq!(DIAG.with(|d| d.borrow().arenas.len()), before);
    }

    #[test]
    fn retired_ancestors_remain_available_in_reports() {
        unsafe {
            let parent = bork_arena_push();
            let child = bork_arena_push_child(parent);
            let address = bork_arena_alloc(child, 8, 8).addr();
            bork_arena_pop(child);
            bork_arena_pop(parent);
            let failed = failure(address, 8, child.addr()).unwrap();
            let message = report(
                address,
                8,
                "read",
                &Location::default(),
                &failed,
                child.addr(),
            );
            assert!(
                message.contains(&format!("arena #{}", parent.addr())),
                "{message}"
            );
            assert!(
                message.contains(&format!("arena #{}", child.addr())),
                "{message}"
            );
        }
    }
}
