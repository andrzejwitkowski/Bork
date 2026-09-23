# Bork — Typed HIR, full typecheck, and LLVM native codegen

## Goal

Make the current language **run**: a compiler that checks programs against the existing grammar (syntax, ownership, types) and, for a defined subset, emits a **native Linux x86_64 binary** via **Inkwell/LLVM**, linked with a bump-arena runtime.

## Decisions (locked)

| Topic | Choice |
|-------|--------|
| End product v1 | Native binary: `bork build` → executable |
| Language subset for **codegen** | Ownership + arenas: primitives, `val`/`var`, `move`, regional `move`, `if`, `for`, `fun` calls, `String`, `print`/`println` |
| Typecheck scope | **Full grammar** (including nullable/`?:`/`!!`, trailing closures) even when codegen does not emit them |
| Unsupported at `build` | Hard error `not yet implemented in codegen` with span — no binary |
| Codegen stack | Inkwell over pinned LLVM; link with `clang` |
| IR boundary | **Typed HIR** between frontend and LLVM (not ad-hoc AST annotations; not MIR yet) |
| Work order | **Phase 1:** typecheck + `bork check` / LSP. **Phase 2:** codegen + runtime + `bork build` |
| I/O | Builtins `print` / `println` for `i64`/`i32` and `String`; `main` returns `i32` exit code |
| String representation | Fat pointer `{ptr, len}`; Shared looks at parent arena / `.rodata`; `move` copies payload into destination arena |
| Target | Linux x86_64 only in v1 |
| Default LLVM pin | LLVM **18** via matching Inkwell feature; confirm `llvm-config` at implement time (bump to 19 only if 18 unavailable) |

## Pipeline

```text
.source
  → layout normalize
  → parse (AST)                         # syntax errors
  → ownership / regions (sema)          # existing rules
  → typecheck → Typed HIR               # full grammar
  → [bork check / LSP stop here]
  → codegen capability gate (subset B)
  → Inkwell LLVM module
  → link runtime (arenas + print) → native binary
```

## CLI

| Command | Behavior |
|---------|----------|
| `bork check <file>` | Parse + ownership + typecheck; print diagnostics; no LLVM |
| `bork build <file> [-o out]` | Same as check, then subset-B codegen + link; default `-o` from source stem |
| `bork <file>` | Same as `check` (keep current habit / scripts); `--dump-arenas` still works on this path |
| `bork-lsp` | Diagnostics from the same `frontend::check` as `check` |

Exit codes: `0` success; `1` user program errors (parse / ownership / type / codegen NYI); `2` toolchain failure (missing LLVM, clang, link).

## Phase 1 — Typed HIR and full typecheck

### HIR

- `HirProgram` / `HirFunction` / stmt & expr nodes carrying `Ty` and `Span`
- `RegionId` on regions aligned with arena nesting (`fun`, blocks, `for`, `if` branches, closures)
- Binding uses tagged with use-kind from ownership: `Copy` | `Shared` | `Move` | `Local`
- Resolved callees where possible (`Fun` by name); unresolved / unsupported forms still typed for check

### Types

- Primitives and aliases as today; non-null primitives are **Copy**
- `String` is **non-Copy**; string literals have type `String`
- Nullable `T?`, `Some` / `None`, `?:`, `!!` — fully checked
- Function types; trailing closures — infer param types from call context when possible, else clear type error
- Binary operators on primitives; reject ill-typed ops
- No assign to `val`; arity and argument types vs formals; `return` matches function result
- Unknown named types → type error (no user `type` decls in v1)

### Ownership vs typecheck

- Existing `sema` remains source of truth for move / Shared / region rules
- Public entry: `frontend::check(source) -> Result<HirProgram, Diagnostics>`
- Typecheck must not re-implement move policy; it consumes ownership results when lowering to HIR
- Prefer: ownership pass, then typecheck pass producing HIR (single pipeline API)

### Diagnostics

```text
Diagnostic { span, severity, phase: Parse | Ownership | Type | Codegen, message }
```

Report ownership and type errors together when both apply; do not require fixing ownership before seeing type errors when both passes can run.

### Tests (phase 1)

String-program unit tests (same style as `src/sema/tests/`): good/bad cases for nullable, closures, assign-to-`val`, arity, `String` vs numeric, use-after-move still caught.

## Phase 2 — Codegen subset B, runtime, build

### Codegen gate

Walk HIR; any node outside subset B → `Codegen` diagnostic with span; abort before writing objects.

### Subset B (emitted)

- Top-level `fun`, including `main(): i32` (or equivalent return type mapped to exit code)
- `val` / `var`, assign to `var`, nested blocks, `if` / `else`, `return`
- Arithmetic and comparisons on primitives
- Calls to known `fun` **without** trailing closures
- `for` with **body arena reset each iteration** (per `docs/memory-model.md`)
- Expression `move` and regional `move { }` / `move (caps) { }`
- `String` and `print` / `println`

### Explicitly NYI at build (check may still succeed)

Trailing closures / lambdas, `Some` / `None` / `?:` / `!!`, and any other HIR outside the list above.

### Runtime

- Bump slabs of **4096** bytes; pool recycle for sequential siblings — behavior matching `src/arena.rs` / `docs/memory-model.md`
- Delivered as a small **`bork_runtime` static library** (Rust, evolved from `arena.rs`) exposing C ABI: arena push/reset/alloc, `print`/`println` for ints and strings
- Codegen emits push on region entry, reset on exit; loop body reset each iteration

### String lowering

| Situation | Behavior |
|-----------|----------|
| Literał | Descriptor points at `.rodata` |
| Shared (child reads parent `val`) | Copy `{ptr,len}` only; bytes stay in parent / rodata |
| `move` | `arena_alloc(len)` in **destination** arena, `memcpy`, new descriptor; source binding already dead in HIR |

Scalars and control stay on stack / SSA; region payloads use the bump allocator.

### Inkwell / link

- Feature `codegen` on the `bork` crate pulls Inkwell (LLVM 18)
- `bork build`: emit object, link runtime + libc via `clang`
- Host: Linux x86_64

### Tests (phase 2)

- Golden: small `.bork` fixtures → `build` → run → assert stdout + exit code (Shared string, `move` string, `for` + arena, prints)
- Negative: program with trailing closure → `check` OK (if types OK) → `build` fails with NYI span
- Ownership errors fail at check before LLVM

## Module layout

```text
src/ast.rs, parser, layout     # existing
src/sema/                      # ownership / regions
src/hir/                       # HIR + Ty
src/typeck/                    # AST + ownership → HirProgram
src/frontend.rs                # check()
src/codegen/                   # gate + Inkwell (feature codegen)
runtime/ or crates/bork_runtime/
```

LSP calls `frontend::check` for diagnostics. Phase 1 does not require type hover; phase 1 follow-up may expose HIR types on hover without changing the check pipeline.

## Non-goals (v1)

- JIT `bork run`, Cranelift, textual `.ll`-only workflow as primary
- MIR layer (may come after subset B grows)
- Windows / macOS, packages/modules, GC, debugger, self-hosting
- Aggressive LLVM optimization productization (optional `-O` later)
- Emitting codegen for nullable / closures in this milestone

## Success criteria

1. `bork check` reports type and ownership errors across the full grammar without crashing.
2. A subset-B program with `move`, Shared/`move` strings, `for`, and `print` builds and runs correctly on Linux x86_64.
3. The same program plus a trailing closure: `build` returns NYI with a span; no binary.
4. Use-after-move / illegal ownership fails at check before any LLVM invocation.
5. Spec/README notes LLVM 18 + `clang` prerequisites for `build`.

## Implementation planning note

This spec is **two sequential implementation plans** (phase 1, then phase 2). Do not start Inkwell work until phase 1 `frontend::check` and tests are in place.
