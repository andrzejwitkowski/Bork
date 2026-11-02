# RFC 0003: Borrows, regions and generics — the v1 restriction list (TS-P6-02, issue #61)

- **Status:** Proposed
- **Date:** 2026-11-02
- **Issue:** #61 (epic: ownership, Phase 6)
- **Builds on:** [RFC 0001](0001-type-system-core.md) section 2 (invariance), [RFC 0002](0002-ownership-typeck-contract.md) section 4 and INV-3.
- **Unblocks:** #62 (generic glue), #63 (generic regions), #64 (negative tests).

This RFC decides where a borrow `&T` may appear. It changes no code. Where it says "rejected in v1" the check either exists today (cited) or is a follow-up issue (section 7).

## 1. Motivation

`&T` is allowed only on parameters and `val` locals. The restriction is enforced by scattered checks that cite "until region parameters exist (#61)", and two gaps were found while writing this RFC (both reproduced through `frontend::check`, which reported no diagnostics):

1. `val a: [i32; 2] = ...; fill(&a)` where `fill(buf: &[i32; 2])` writes `buf[0]` mutates a `val`.
2. `id::<&P>(&p)` is accepted: a written type argument can be a borrow, although an inferred one is rejected (`check_argument`, `typeck/expr/call.rs`: "parameter expects owned").

#62-#64 need a stable list of what v1 accepts.

## 2. Borrows as type arguments

**Decision.** A type argument is never a borrow. `T = &U` is rejected for inferred and written arguments, for class/enum arguments, and after instantiation.

**Rationale.** A generic body is ownership-checked once, with `T` opaque and move-only (RFC 0002 section 4, INV-10). A borrow bound to `T` could be moved, stored or returned inside the body, which the borrow rules forbid.

**Rejected.** Allowing `T = &U` with a "borrow-carrying instance" marker: needs region parameters on every generic.

**Consequences.** `TyCtx::ill_formed` (`src/ty/query.rs`) gains the rule `contains_borrow(arg)` for instance arguments (`TyCtx::contains_borrow`, added by #60 / PR #192). Follow-up A.

## 3. Borrows in fields, containers and returns

**Decision.** Rejected in v1: class fields, enum payloads, `Vec<&T>`, `Ref<&T>`, `[&T; N]`, `&T?`, return types and returned values, `var` bindings.

**Rationale.** A borrow lives until the end of its region. A value that holds one has to carry that region (a region parameter on its type), and nothing in `TyData` can.

**Rejected.**
- Full Rust lifetimes (`'a`, outlives bounds, variance): too much surface for the language.
- Making `&T` a counted handle like `Ref<T>`: defeats region arenas.
- Region parameters inferred per call without annotations: not decidable for fields.

**Consequences.** The existing messages stay the contract (section 6). Future work, not designed here: region-parameterised class types, region-annotated signatures.

## 4. Region parameters

**Decision.** Regions stay inferred from `{}` blocks. A `&T` parameter has one anonymous region that outlives the callee (`REF_PARAM_BORROW_FROM = "call site"`, `src/sema/region.rs`). There are no written region parameters in v1.

**Consequences.** Reborrows (`val d = b`) share the view of `b` and end with the region (`docs/memory-model.md`).

## 5. Mutation through a borrow

**Decision.** `&` is the only borrow form. There is no `&mut` and no shared/mutable split, now or as a planned follow-up. A borrow of an array or `Vec` allows element writes in the callee (`docs/language.md`; `typeck/stmt/assign.rs`). To keep `val` immutable, **`&name` is rejected when `name` is a `val` array or `Vec`**; only a `var` can be borrowed.

**Rationale.** The user's constraint is one borrow form. Element writes through `&` are an existing, documented feature. `val` is documented as immutable, so a `val` must not be borrowable by something that can write.

**Rejected.**
- `&mut` or a read-only `&`: a second borrow form.
- Forbidding element writes through `&`: removes a documented feature.
- Documenting `val` as shallowly immutable: weakens `val`.

**Consequences.** Breaking for programs that borrow a `val` array or `Vec`, including read-only ones. No program in `programs/` does (checked by script on 2026-11-02). Follow-up D.

## 6. v1 restriction list

| Position | v1 | Diagnostic (today) | Enforced at | Follow-up |
|---|---|---|---|---|
| parameter, `val` local | allowed | n/a | typeck | none |
| class field | rejected | `borrowed references cannot be stored in class fields` | `typeck/mod.rs` | region-parameterised types |
| enum payload | rejected | `borrowed references cannot be stored in enum payloads` | `typeck/mod.rs` | same |
| return type or returned value | rejected | `returning a reference is not supported` | `escape.rs` | region-annotated signatures |
| `var` binding | rejected | `cannot bind var ... to a reference type` / `... to a borrow` | `typeck/stmt/mod.rs` and `sema/walk.rs` (duplicate) | B |
| `Vec<&T>`, `Ref<&T>`, `[&T; N]` | rejected | `vec_elem_message`, `managed_ref_target_message` | `typeck/lower.rs`, `ill_formed` | with fields |
| `&T?` | rejected | `nullable reference types ... are not supported` | `typeck/lower.rs` | none |
| type argument `T = &U` | rejected | inferred: `... parameter expects owned ...`; written: **not enforced** | `check_argument`; none | A |
| `&name`, `name` a `val` array/`Vec` | rejected | **not enforced** | none | D |
| element write through `&[T; N]` / `&Vec` | allowed on `var` | `cannot index-assign through ...` otherwise | `typeck/stmt/assign.rs` | none |

## 7. Follow-up issues

- **A.** Reject a borrow as an explicit type argument and in `ill_formed`; note pointing at the generic definition (#63, #64).
- **B.** Collapse the duplicated `var`-binding borrow checks (with #37).
- **C.** Region-parameterised class types (borrow fields): design after v1.
- **D.** Reject `&name` of a `val` array/`Vec`; add `programs/check_fail/borrow/borrow_val_array.bork`; update `docs/language.md`.
- **E.** Region-annotated signatures for returned borrows: design after v1.

## 8. Threads

No `Send`/`Sync` exists. Constraint for any future thread RFC: a borrow never crosses a region boundary, so a Send-like rule can be "owned types only". Threads are out of scope for this epic.

## Reviewer checklist

- Every cited diagnostic and path matches the source.
- Section 5 breakage is acceptable (no corpus program is affected).
- Follow-up issues A-E are opened after this RFC is approved.
