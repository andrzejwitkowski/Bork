# Bork — follow-ups after `feat/fixed-arrays-slices`

Tracked work after fixed arrays/slices, `while`/`break`/`continue`, logical ops, and unified `region_walk` codegen.

## Refactor

- [ ] **Codegen visitor split** — extract a dedicated `CodegenRegionVisitor` (or similar) wrapping `FnEmitter` so LLVM emission and `RegionVisitor` hooks are not on the same type; avoid `codegen_walk` ↔ `expr` module cycles.
- [ ] **Region cursor** — if `arena_cursor_shared!` grows, consider a single cursor type with explicit mut/ref modes instead of macro expansion.
- [ ] **`RegionEmitter` vs walk** — document (or assert in tests) the contract for loops: one `region_enter`, many `latch` resets in LLVM, one `region_exit`; keep schedule visitor and codegen visitor aligned.
- [ ] **Array codegen** — `src/codegen/llvm/array/` is split from monolithic `array.rs`; optional further split (emit vs runtime helpers) if files grow.

## Fix / harden

- [ ] **Codegen gate vs frontend** — keep `gate.rs` in sync when adding surface syntax; add a test that every `check()`-ok MVP sample either passes gate or is listed as intentionally rejected.
- [ ] **Walk errors** — ensure all codegen failures that originate as `Diagnostic` go through `WalkError::from_diagnostic` (no stray `schedule_error` wrappers on user messages).
- [ ] **Float / wider integers in arrays** — verify runtime abort paths and codegen for non-i32 element types where supported.
- [ ] **Slice bounds** — typeck rejects bad literal bounds; document or test runtime index abort messaging if we add user-facing traces later.

## Tests & tooling

- [ ] Extend **`codegen_arena_push_pop_counts_match_schedule`** to more samples (nested `if`, `move` blocks, string sinks) once those paths are stable in CI LLVM env.
- [ ] **Integration**: `tests/build.rs` — quicksort/smoke binaries only if committed as fixtures (avoid orphan `.bork` in repo root).

## Docs

- [ ] Sync **language.md** / **memory-model.md** with any gate changes after new features.
- [ ] Remove or relocate **`docs/superpowers/plans/`** entries once absorbed into this file or closed PRs.

## Not for main tree (local only)

- `smoke*.bork`, `quicksort.bork`, `qsort.brok`, `tools/gen_quicksort.py` — benchmarks/generators; keep out of default commits unless adding official perf fixtures.
