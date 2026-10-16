# Semantics inventory and regression baseline (TS-P0-01, issue #28)

Snapshot of what the compiler does **today**, taken before any type-representation change.
Behaviour is not changed by this document. `docs/language.md` is partly stale; this file is
derived from `src/` and the test corpus, not from `language.md`.

Cell legend: **V:<test>** = verified by that test; **UNVERIFIED** = no test found that pins it
(the cell records what the code appears to do, from reading `src/`). `codegen-implemented` cells
point at `programs/build/*` which only run with `--features codegen` (LLVM 23) and were
**NOT RUN** for this baseline (see [Test runs](#test-runs)).

Test locations: parser `tests/parser.rs`; typeck `src/typeck/tests/*.rs`; corpus `programs/{check,check_fail,build,build_fail}`
(`tests/programs.rs`); gate `src/codegen_gate.rs` (`#[cfg(test)]`); arrays `tests/arrays.rs`.

## Feature matrix

| Feature | parsed | typechecked | sema/escape | memory plan | codegen-gated | codegen-implemented |
|---|---|---|---|---|---|---|
| prim types (`i32`, `i64`, `f64`, `bool`, ...) | V:corpus (programs/check, build) | V:src/typeck/tests/mod.rs | UNVERIFIED | UNVERIFIED | V:programs/build/basics (passes gate) | V:programs/build/basics (NOT RUN) |
| `T?` nullable | V:tests/parser.rs `nullable_function_*` | V:src/typeck/tests/nullable.rs | UNVERIFIED | UNVERIFIED | V:src/codegen_gate.rs `codegen_lowers_nullable` checks; rejects non-lowerable nullable types | V:programs/build/conditionals (NOT RUN) |
| `Ref<T>` (`ManagedRef`) | V:tests/parser.rs `parses_class_with_managed_ref_fields_and_field_assignment` | V:src/typeck/tests/managed_ref.rs | V:programs/check_fail/managed_refs | UNVERIFIED | V:src/codegen_gate.rs `Ref<Point> ?.x` test | V:programs/build/managed_refs (NOT RUN) |
| `&T` (`Ref`) | V:corpus (programs/check/borrow) | V:src/typeck/tests/reference.rs | V:programs/check_fail/borrow | UNVERIFIED | UNVERIFIED | V:programs/build/borrow (NOT RUN) |
| `[T; N]` | V:tests/arrays.rs `parses_array_type_and_literal` | V:tests/arrays.rs | V:programs/check/borrow | UNVERIFIED | V:src/codegen_gate.rs (`.length`, array-of-Ref move rejected) | V:programs/build/basics (NOT RUN) |
| `Vec<T>` | V:corpus (programs/build/vec) | V:programs/check_fail/vec; src/typeck/expr/vec.rs (3 unit tests) | UNVERIFIED | UNVERIFIED | UNVERIFIED | V:programs/build/vec (NOT RUN) |
| `Range` (`lo..hi`, `for`) | V:corpus (programs/check/loops) | V:programs/check/loops | UNVERIFIED | UNVERIFIED | UNVERIFIED | V:programs/build/loops (NOT RUN) |
| `String` | V:corpus (programs/build/text) | V:src/typeck/tests/text.rs | UNVERIFIED | UNVERIFIED | UNVERIFIED | V:programs/build/text (NOT RUN) |
| classes / records | V:tests/parser.rs `parses_class_decl_and_constructor` | V:programs/check_fail/classes | UNVERIFIED | UNVERIFIED | V:src/codegen_gate.rs rejects moving class objects, owned nested `?.` field | V:programs/build/classes (NOT RUN) |
| methods | V:tests/parser.rs `parses_class_with_interleaved_fields_and_methods` | V:src/typeck/tests/methods.rs | V:programs/check_fail/methods | UNVERIFIED | UNVERIFIED | V:programs/build/methods (NOT RUN) |
| consts | V:tests/parser.rs `parses_global_constants_among_items` | V:src/typeck/tests/constants.rs | UNVERIFIED | UNVERIFIED | UNVERIFIED | V:programs/build/constants (NOT RUN) |
| closures (trailing, `move (s) {}`) | V:tests/parser.rs `parses_trailing_move_closure` | V:src/typeck/tests/closures.rs (arg count only) | UNVERIFIED | UNVERIFIED | V:`gate_rejects_trailing_closure` (rejected: "trailing closures are not supported by codegen") | none: gated off |
| `move` / `promote` | V:tests/parser.rs `parses_move_expression_in_var_decl` | V:programs/check_fail/move_and_promote | V:programs/check/move_and_promote | UNVERIFIED | V:src/codegen_gate.rs (array-of-Ref move rejected) | V:programs/build/move_and_promote (NOT RUN) |

## Variant tables

Every variant must appear below in backticks (enforced by `inventory_mentions_every_variant` in
`tests/hir_types_golden.rs`).

**`ast::Type`:** `Primitive`, `Named`, `Array`, `Func`, `Ref`, `ManagedRef`, `Vec`.

**`hir::TyKind`:** `Prim`, `Named`, `Array`, `Func`, `Ref`, `ManagedRef`, `Vec`, `Range`, `Unknown`.
`Unknown` is an error-recovery type; it must not appear in the golden snapshot.

**`ast::Expr`:** `Int`, `Float`, `Bool`, `Str`, `ArrayLit`, `Index`, `Slice`, `VecNew`, `Ident`, `Move`,
`Promote`, `None`, `Some`, `Binary`, `Unary`, `Field`, `Call`, `If`, `IfVal`.

**`ast::Stmt`:** `Block`, `VarDecl`, `Assign`, `For`, `While`, `Break`, `Continue`, `MoveBlock`, `Return`, `Expr`.

Per-variant verification is by the corpus as a whole (`tests/programs.rs` parses and checks every
sample; HIR output is frozen by `tests/golden/hir_types.txt`). Variants without a dedicated
negative test: `Unknown`, `Func` (only via closures/calls), `MoveBlock`: **UNVERIFIED** individually.

## Compatibility matrix

"Frozen" = must stay byte-identical: `tests/golden/hir_types.txt` and every `programs/` expectation
(`exit`, `stdout`, `diag` directives) unless the owning phase explicitly re-blesses it in its PR.

| Row | May change in | Must stay identical until then |
|---|---|---|
| prim types, `Range` | any phase that touches inference (Phase 1+) | golden lines for prim params/returns |
| `T?` | nullability/ADT work | `codegen_lowers_nullable` gate behaviour; programs/check_fail diag text |
| `Ref<T>`, `&T` | representation/borrow phases | golden `&[T; N]` / `Ref<..>` param lines; check_fail/borrow, managed_refs diags |
| `[T; N]`, `Vec<T>` | generics phases | `Vec<T>` stays built-in, no general generics (see below) |
| `String`, classes, methods, consts | ADT/representation phases | corpus exit codes and stdout |
| closures | closure phase | gate rejection of trailing closures |
| `move` / `promote` | ownership phases | check_fail/move_and_promote diags |

## Decisions already made (do not reopen silently)

From `docs/superpowers/specs/2026-10-16-vec-design.md`:
- `Vec<T>` is **built-in**; there are **no general generics**.
- Non-goals: `Vec<Vec<T>>`, `Vec<T?>`, `Vec<Node>`.
- Element-borrow rule: **not enforced yet**.

## Orphan / unwired tests

- `TODO.md`: no test asserts that every `check()`-ok sample either passes the codegen gate or is listed as intentionally rejected.
- `TODO.md`: `tests/build.rs` quicksort/smoke binaries are not committed fixtures; the repo root holds untracked `quicksort*.bork`, `repro_*.bork` that no suite runs.
- `programs/build_fail/` has 7 files; `build_fail` is only run under `--features codegen` (`tests/programs.rs`), never in default `cargo test`.
- `src/typeck/tests/closures.rs` has a single test (argument count).

## Known stale spots in `docs/language.md` (not fixed here)

- `Vec<T>` is implemented (`src/typeck/expr/vec.rs`, `TyKind::Vec`) but mentioned only briefly (line ~104).
- Trailing closures (`f(a) { x -> ... }`) are documented as syntax but rejected by codegen (`src/codegen_gate.rs`); the "Rejected by codegen" paragraph lists only that and nested-record `?.`.
- The gate rejects "this binary operator", "field access" and nullable types in forms `language.md` does not describe.

## Running the suites

Mirrors `.github/workflows/ci.yml`.

```
cargo test --workspace                          # unit, parser, arrays, programs (check/check_fail), golden
cargo test programs_corpus_check                # corpus frontend suites only
cargo test --workspace --no-default-features    # without the lsp feature
cargo test --test hir_types_golden              # golden HIR types; BORK_BLESS=1 to refresh
# needs LLVM 23 (apt.llvm.org llvm-23-dev + clang); inkwell links libLLVM dynamically
LLVM_SYS_231_PREFIX=/usr/lib/llvm-23 cargo test --features codegen   # tests/build.rs, tests/debug_build.rs, build + build_fail corpus
```

## Test runs

(Filled in below.)
