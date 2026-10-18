# RFC 0002: typeck / ownership / region contract and compiler invariants (TS-P0-03, issue #30)

- **Status:** Proposed
- **Date:** 2026-10-18
- **Issue:** #30 (epic: core-types, Phase 0)
- **Baseline:** [`docs/type-system/inventory.md`](../type-system/inventory.md) (#28) and [RFC 0001](0001-type-system-core.md) (#29).
- **Confirms RFC 0001 section 4:** generic bodies are type-checked and ownership-checked once, before monomorphization, with an opaque `T` treated as move-only. This RFC adds what must wait for concrete types and who reports errors there.

Generic syntax in the examples below is illustrative. Bork has no generics yet (#42).

## 1. Motivation and current pipeline

Three things make the ownership contract urgent:

1. Monomorphization (RFC 0001 section 4) gives several instances the same spans, while side tables are keyed by `Span`.
2. Copy and borrow logic is written more than once, in different type systems.
3. No cross-pass assumption is numbered, so no `debug_assert!` can cite one (there are none in the compiler today).

The pipeline as it is today (`src/frontend.rs`, `check`):

1. `parse`
2. `typeck::check_with_members` returns `(hir, span_tys, members, diagnostics)`
3. `sema::analyze_with_members(&program, ...)` walks the **AST** again (not HIR) and builds the `ArenaReport`. Typeck types reach it through `SpanTypes` and `hir_to_ast` (`src/sema/walk.rs:950-985`).
4. Everything below runs only with zero diagnostics so far:
   `hoist::annotate` (mutates HIR), `escape::check_function`, `memory::plan`, `region_walk::stamp_codegen_push` (mutates `ArenaNode.codegen_push`)
5. `codegen_gate::gate` runs inside `codegen::build_with_options` (`src/codegen/mod.rs:56`) and in the LSP (`src/lsp.rs:178`), not in the frontend.
6. `llvm::emit_module(hir, report, memory_plan)`

## 2. Fact dimensions

Each dimension is a separate question with one owner. Code must not answer one by asking another.

| Dimension | Question |
|---|---|
| identity | Which type is this, which definition? |
| ownership | How is this value used here: move, copy, promote, borrow, share, ref-move, ref-copy (`UseKind`)? |
| copyability | Can a value of this type be duplicated bitwise? |
| handle clone | Can a `Ref<T>` handle be duplicated, with what refcount effect? |
| borrow permission | May a `&T` be created, stored, returned, or moved? |
| needs-drop | Does destroying a value run code (drop glue)? |
| layout | Size, alignment, field offsets, nullable representation. |
| region | Which arena does an allocation live in, and which arena tree node is it in? |
| sink / result ownership | Does a call result get allocated into the caller's sink arena? |

## 3. Contract table

"Target" is what this RFC decides. "Today" lists the current producers, with duplicates called out. Issue numbers were checked against the epic.

| Fact | Target producer | Consumers | Today | Removed by |
|---|---|---|---|---|
| identity, per-expression type | typeck, into HIR | sema, escape, memory, codegen | HIR `ty`; `SpanTypes` (3 write sites: `typeck/stmt/mod.rs:133`, `stmt/assign.rs:132`, `expr/control.rs:146`) read only by sema | #35, #36, #37 |
| copyability | one capability query (`is_copy`) | typeck, mono, codegen | `hir::Ty::is_copy` (`src/hir/ty.rs:224`) **and** `ast::Type::is_copy` (`src/ast.rs:163`); codegen decides again at `llvm/presence.rs:175-182` | #60 |
| ownership (`UseKind`), pre-mono | typeck | sema (consume), escape, codegen, region_walk, hoist, gate | typeck sets `UseKind` (`typeck/expr/mod.rs:258-273`); sema **recomputes** it from AST types (`sema/policy.rs:14-43`, `walk.rs:673,757,891`); `Ownership` (`sema/report.rs:7`) is read only by `dump.rs` and `lsp.rs` | #37, #46 |
| ownership (`UseKind`), post-mono | monomorphizer (#47), via the same capability query | escape sink check, memory, codegen | n/a | #47, #60 |
| handle clone / `Ref` move or copy | typeck picks `RefMove` / `RefCopy` / `RefCreate`; strength is a runtime fact | codegen (`llvm/managed_ref.rs`), runtime (`RefHandle`, `classify_internal`) | sema re-decides "managed ref is always shared" (`policy.rs:35,62`) | #37, #62 |
| borrow permission, type-shape rules | typeck | none (rejects) | no `&T` field (`typeck/mod.rs:172`), no `&T?` (`lower.rs:94`), ref param must be `val` (`mod.rs:353`), no assign through borrow (`stmt/assign.rs:173-188`) | stays |
| borrow permission, flow rules | ownership pass (sema) | none (rejects) | moving a borrow (`policy.rs:77`); `var` bound to a borrow is checked in **both** `typeck/stmt/mod.rs:117` and `sema/walk.rs:143` | #37 (delete one) |
| escape of references | ownership/escape pre-mono | none (rejects) | "returning a reference" (`escape.rs:127`), guard escape (`escape.rs:427`) | #46 |
| needs-drop | one capability query (`needs_drop`) | codegen, memory | no predicate; ad hoc `ty.is_managed_ref()` at about 6 codegen sites (`emit_fn.rs:683`, `managed_ref.rs:136,192`, `array/alloc.rs:58`, `vec/alloc.rs:50`, `region_emit.rs:311`) | #60, #62 |
| layout | one layout query over concrete types | codegen, memory | `Codegen::basic_type` (`llvm/context.rs:103-133`), `array/layout.rs`, `vec/layout.rs`; typeck checks only recursive owned layout (`typeck/mod.rs:383`) | #66 |
| region tree | sema | memory, region_walk, codegen | `ArenaReport` built from AST; matched to HIR by walk order and labels (`region_walk.rs:32-45,206-246`); `peel_blocks` written twice (`sema/mod.rs:21`, `hir/mod.rs:314`) | #35 (NodeId); #74 (`peel_blocks`) |
| sink / result ownership | one query (`owns_result`) | escape, memory, region_emit, call_abi | the rule `is_managed_ref() \|\| record_name().is_some()` is repeated in `region_emit.rs:51,168`, `memory.rs:466`, `call_abi.rs:66`; `escape.rs` mirrors the codegen region model by hand (its header says so) | #77 |
| `codegen_push` | `region_walk::stamp_codegen_push` | codegen schedule | mutates `ArenaNode` in place | per-instance table, #47 |
| allocation plan | `memory::plan` | codegen | keyed by `Span` (`memory.rs:26-44`); a record allocation without a span is a planner error (`memory.rs:136`); `expression_origins` uses `HirExpr` pointer addresses | #35 |
| codegen support | `codegen_gate` | codegen | filter over HIR; not a source of semantic facts | #67 |

Per-pass rules that follow from the table:

- **Post-mono producers.** The monomorphizer produces the post-mono `UseKind` by calling the same capability query as typeck. It is a separate row so it is not a second copy of the logic.
- **`escape` is split in two.** The user-facing rules (returned references, guard escape) run pre-mono and treat an opaque `T` conservatively. The sink-placement consistency check runs post-mono and can only raise an ICE.
- **`memory::plan` diagnostics** become ICE-only post-mono.
- **The gate is a stated exception** (see section 4): codegen-support limitations are reported at the instantiation site, with a note on the parameter. They are not language errors and shrink as #67 lands.
- **`stamp_codegen_push`** needs a per-instance copy of the region tree, keyed by `(InstanceId, NodeId)`.

## 4. Generics: pre-mono vs post-mono

**Decision.** Typeck and the ownership pass run once on the generic body. An opaque `T` is:

- move-only, never copy;
- possibly droppable, so the check assumes it may need to be dropped;
- not handle-clonable, unless a bound grants it (#56, #60).

A body accepted this way cannot fail ownership for any instantiation (INV-10).

**Waits for concrete types (post-mono):**

- layout and nullable representation (`T?` storage);
- `needs_drop` and drop glue (#62);
- `Ref` clone and drop glue;
- sink / result ownership (`owns_result`);
- `codegen_push` and the per-instance region tree;
- the memory plan and allocation sites;
- the codegen gate.

**Post-mono passes produce no user diagnostics** (INV-2b). A failure there is an internal compiler error. The one exception is the gate, which reports at the instantiation site.

**Refinement never rejects.** A `T`-typed use is stored pre-mono as `Move`. At mono it may refine to `Copy` (for `i32`) or `RefMove` (for `Ref<Node>`). Refinement can only change how the use is lowered, never turn an accepted body into a rejected one.

**Examples (illustrative syntax):**

1. `fun id<T>(x: T): T { return x }` passes pre-mono (one move). Instantiating with `i32` or `Ref<Node>` needs no further check.
2. `fun dup<T>(x: T): Pair<T> { return Pair(x, x) }` is rejected at the definition: `x` is moved twice and `T` has no `Copy` bound. It is rejected even if every call site uses `i32`.
3. `Vec<T>.push` instantiated with `T = i32` vs `T = Ref<Node>`: ownership is identical pre-mono. Drop glue differs only post-mono (nothing vs a registered ref drop).
4. `fun first<T>(v: Vec<T>): T?` instantiated with a class `T` that the gate cannot yet lower as nullable: reported at the instantiation site, under the gate exception, with a note pointing at `T`. It is not a language error.

**Rejected.**

- Ownership checking after mono: accepts generic code that breaks for some instantiations (RFC 0001 section 4).
- Post-mono user diagnostics from `memory::plan` or `escape`: errors would depend on the caller.

**Depends on.** #46, #47, #60, #62.

## 5. Stable identity: `NodeId`

**Decision.** Every HIR expression, statement and binding gets a `NodeId`, assigned at lowering, dense per item and stored on the node.

- Pre-mono side tables are keyed by `(DefId, NodeId)`.
- Post-mono tables (memory plan, region schedule, `codegen_push`) are keyed by `(InstanceId, NodeId)`.
- `Span` is for diagnostics only.

**Rationale.** `Span` is `{start, end}` (`src/span.rs:4`). Two expressions with the same range collide, and all instances of a generic function share every span.

**Rejected.**

- `(Span, instance)` pairs: still collides within one body.
- Pointer address (as `memory.rs:94` does today): unstable across clones and mono substitution.

Implementation is out of scope (#35).

## 6. Compiler invariants

Each invariant has a number and, where it can be checked, a check site. Tests and `debug_assert!`s cite them as `INV-n`.

| # | Invariant | Holds at | Today |
|---|---|---|---|
| INV-1 | After typeck succeeds, HIR contains no `Unknown` or inference type. **Target**, owned by #33 and #41. | typeck -> sema | Violated: builtin callee idents get `Ty::unknown()` (`typeck/expr/call.rs:354,419`). |
| INV-2 | Every HIR expression has a `UseKind` consistent with its type's capabilities (`Copy` implies copyable). Holds after typeck and again after mono. | typeck -> sema; mono -> codegen | Unchecked. |
| INV-2b | Post-mono passes (escape sink check, memory plan, stamp, codegen) emit no user diagnostics; failure is an ICE. The gate is the exception. | mono -> codegen | `memory::plan` returns diagnostics (`memory.rs:61-68`). |
| INV-3 | A `&T` never appears in a class field, a return type, or a `var` binding, until region parameters exist (#61). | typeck, sema | Enforced in three places (see section 3). |
| INV-4 | No `T??` (RFC 0001 section 3). | type lowering | Unrepresentable today. |
| INV-5 | HIR function and method order matches the sema roots. | typeck -> sema | Comment only (`typeck/mod.rs:63`); only the count is checked (`region_walk.rs:745,773`). |
| INV-6 | Every sema region node matches exactly one HIR region site. Failure is an ICE. | sema -> region_walk | `stamp.rs:47` panics with `.expect`, while `memory::plan` turns the same failure into a diagnostic. |
| INV-7 | Every record allocation has an allocation site in the memory plan. | memory -> codegen | Planner error (`memory.rs:136`), `managed_ref.rs:93`. |
| INV-8 | HIR given to codegen has passed `codegen_gate::gate`. | gate -> codegen | Enforced by call order in `codegen/mod.rs`. |
| INV-9 | The LLVM `Ref` handle struct has the layout of the runtime `RefHandle`. | codegen / runtime | Two hand-written definitions (`llvm/context.rs:151`, `crates/bork_runtime/src/refs.rs:12`); nothing asserts equality. |
| INV-10 | A generic body accepted pre-mono passes ownership for every instance. Checkable: debug builds re-run the ownership check per instance and assert acceptance. | mono | n/a until #46/#47. |
| INV-11 | Arena and sink are open inside emitted function bodies. | codegen | Asserted ad hoc (`emit_fn.rs:107,114,186,236`, `llvm/arena.rs:68`). |

### Principles

These are enforced in review and cannot be asserted:

- Every fact in section 2 has exactly one producer (the section 3 table). New code calls the query; it does not re-derive the answer.
- Sema consumes typeck's results. It does not re-derive types or `UseKind` from the AST.
- Walks over the region tree use `region_walk`. A pass does not keep a private copy of the codegen region model.

## 7. Migration

| Duplicate or gap | Removed by |
|---|---|
| `Span`-keyed tables | #35 (NodeId), #37 |
| two `is_copy`, sema re-deriving `UseKind` | #37, #60 |
| pre-mono ownership of generic bodies | #46 |
| mono collector and instance tables | #47 |
| drop and clone glue | #62 |
| type layout | #66 |
| gate modernization | #67 |
| `peel_blocks` written twice | #74 (implement first) |
| `escape` mirroring the codegen region model | #77 (implement last, before #37) |
| `stamp.rs:47` panic vs diagnostic (INV-6) | #75 |
| `Ref` handle layout assertion (INV-9) | #76 |

Implementation order of the hardening issues: #74, then #75, with #76 in parallel, then #77. All four land before #35 and #37.

## Open questions

- Should INV-10's per-instance re-check stay in release builds, or debug only?
- Is `Ownership` (`sema/report.rs:7`) kept as a report-only view derived from `UseKind`, or deleted?

## Reviewer checklist

- [ ] The section 3 table gives every fact one target producer and lists its consumers.
- [ ] Section 4 documents the pre-mono vs post-mono split with examples, including the gate exception.
- [ ] Invariants in section 6 are numbered and each says where it is (or will be) checked.
- [ ] Section 5 accepts `NodeId` over `Span` as the key for side tables.
