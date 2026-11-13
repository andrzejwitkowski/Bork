# Bork — follow-ups after `feat/fixed-arrays-slices`

Tracked work after fixed arrays/slices, `while`/`break`/`continue`, logical ops, and MIR-based codegen.

## Refactor

> MIR migration (RFC 0006): steps 1-15 done (#215-#226, #67); step 16 is this documentation pass (#227). Open: #260 (delete `escape.rs` and `hoist.rs`; `move` of objects now lowers, deleting them is its second PR).

- [ ] **Array codegen** — `src/codegen/llvm/array/` is split from monolithic `array.rs`; optional further split (emit vs runtime helpers) if files grow.

## Fix / harden

- [ ] **Codegen gate vs build** — a program the gate accepts must not fail `bork build` with `MIR lowering does not cover` (#257).
- [ ] **Float / wider integers in arrays** — verify runtime abort paths and codegen for non-i32 element types where supported.
- [ ] **Slice bounds** — typeck rejects bad literal bounds; document or test runtime index abort messaging if we add user-facing traces later.

## Tests & tooling

- [ ] **Integration**: `tests/build.rs` — quicksort/smoke binaries only if committed as fixtures (avoid orphan `.bork` in repo root).

## Docs

- [ ] Sync **language.md** / **memory-model.md** with any gate changes after new features.
- [ ] Remove or relocate **`docs/superpowers/plans/`** entries once absorbed into this file or closed PRs.

## Not for main tree (local only)

- `smoke*.bork`, `quicksort.bork`, `qsort.brok`, `tools/gen_quicksort.py` — benchmarks/generators; keep out of default commits unless adding official perf fixtures.
