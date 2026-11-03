# RFC 0003: Borrows, regions and generics — the v1 restriction list (TS-P6-02, issue #61)

- **Status:** Proposed
- **Date:** 2026-11-02
- **Issue:** #61 (epic: ownership, Phase 6)
- **Builds on:** [RFC 0001](0001-type-system-core.md) section 2 (invariance), [RFC 0002](0002-ownership-typeck-contract.md) section 4 and INV-3.
- **Unblocks:** #62 (generic glue), #63 (generic regions), #64 (negative tests).

This RFC decides where a borrow `&T` may appear. It changes no code. Where it says "rejected in v1" the check either exists today (cited) or is a follow-up issue (section 7).

## 1. Motivation

`&T` is allowed only on parameters and `val` locals. The restriction is enforced by scattered checks that cite "until region parameters exist (#61)", and two gaps were found while writing this RFC (both reproduced through `frontend::check`, which reported no diagnostics):

1. `val a: [i32; 2] = ...; fill(&a)` where `fill(buf: &[i32; 2])` writes `buf[0]` mutates a `val`. Fixed by section 5 (`&` is read-only).
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

## 5. Borrow forms: `&`, `&rw`, `&unique`

**Decision.** Three borrow forms, written the same way in types and at call sites (`p: &rw [i32; 3]`, `f(&rw a)`):

| Form | Meaning | v1 |
|---|---|---|
| `&T` | immutable borrow: reads only. An element write through it is an error. | yes |
| `&rw T` | editable borrow: the callee may write elements. Only a `var` can be borrowed this way; `&rw` of a `val` is an error. | yes |
| `&unique T` | exclusively locked object: no other access while the borrow lives. | **reserved, not implemented** |

Rules for v1:
- `&name` is allowed for `val` and `var`; it never allows writes. This closes the `val`-mutated-through-a-borrow gap (section 1).
- `&rw name` requires `name` to be a `var` (array, `Vec`; further types when they gain element writes).
- A call cannot borrow the same name twice when one of the borrows is `&rw` (`f(&rw a, &a)`, `f(&rw a, &rw a)`). This is the only aliasing rule in v1.
- Inside a callee, a `&rw` parameter may be passed on as `&rw buf` or `&buf` (reborrow); a `&` parameter can only be passed on as `&`.
- `&rw` follows every `&` restriction in section 6 (not a type argument, not a field, not returned, not in containers, not nullable).
- `unique` is reserved: using `&unique` is an error ("not implemented"), so the syntax stays free. Its semantics are for a later RFC (candidate use: exclusive access across a region boundary, Send-like rules).

**Rationale.** The call site and the signature both show whether a function can change its argument. A `val` stays immutable. The `&rw`-only-on-`var` rule is the same rule `val` already follows for assignment.

**Rejected.**
- One borrow form where `&` may write (previous draft): the signature does not say whether the callee writes, so `val` arrays had to be unborrowable, even for reads.
- Inferring "callee writes" from its body: rule depends on another function's body; unclear for generic and trait methods.
- `&mut`: `rw` is the chosen spelling (`edit` was rejected because it is a natural variable name, and `rw` is a reserved word).

**Consequences.** Breaking. `&` no longer allows `p[i] = v` (documented today at `docs/language.md`, checked in `typeck/stmt/assign.rs`). 14 programs (8 under `programs/` and 6 scratch files at the repo root) wrote through a `&` parameter and moved to `&rw` in #198 (e.g. `programs/build/borrow/reference_in_while.bork`, `programs/build/recursions/recursive_qsort_small.bork`). Follow-ups D and F.

## 6. v1 restriction list

| Position | v1 | Diagnostic (today) | Enforced at | Follow-up |
|---|---|---|---|---|
| parameter, `val` local | allowed | n/a | typeck | none |
| class field | rejected | `borrowed references cannot be stored in class fields` | `typeck/mod.rs` | region-parameterised types |
| enum payload | rejected | `borrowed references cannot be stored in enum payloads` | `typeck/mod.rs` | same |
| return type or returned value | rejected | `returning a reference is not supported` | `escape.rs` | region-annotated signatures |
| `var` binding | rejected | `cannot bind var ... to a reference type; use val` | `typeck/stmt/mod.rs` (sema duplicates removed, #196) | done |
| `Vec<&T>`, `Ref<&T>`, `[&T; N]` | rejected | `vec_elem_message`, `managed_ref_target_message` | `typeck/lower.rs`, `ill_formed` | with fields |
| `&T?` | rejected | `nullable reference types ... are not supported` | `typeck/lower.rs` | none |
| type argument `T = &U` / `&rw U` | rejected | inferred: `... parameter expects owned ...`; written: **not enforced** | `check_argument`; none | A |
| element write through `&[T; N]` / `&Vec` | rejected (changed) | today allowed on explicit `&` | `typeck/stmt/assign.rs` | D |
| `&rw name`, `name` a `var` array/`Vec` | allowed (new) | n/a | new | D |
| `&rw name`, `name` a `val` | rejected (new) | new | new | D |
| `&rw` twice, or `&rw` plus `&`, of one name in one call | rejected (new) | new | new | D |
| `&unique` | reserved, error "not implemented" | new | parser | F |

## 7. Follow-up issues

- **A.** Reject a borrow as an explicit type argument and in `ill_formed`; note pointing at the generic definition (#63, #64).
- **B.** Collapse the duplicated `var`-binding borrow checks (#196, done: sema no longer reports it; `tests/restriction_sites.rs` guards one site per rule).
- **C.** Region-parameterised class types (borrow fields): design after v1.
- **D.** Introduce `&rw`; make `&` immutable (element writes through `&` become errors); `&rw` only on `var`; the one call-site aliasing rule; migrate the 14 programs (#198); add `programs/check_fail/borrow/{write_through_shared,rw_on_val,rw_aliased}.bork`; update `docs/language.md` and `docs/memory-model.md`.
- **E.** Region-annotated signatures for returned borrows: design after v1.
- **F.** Reserve `&unique` in the parser (error "not implemented"); its semantics get their own RFC.

## 8. Threads

No `Send`/`Sync` exists. Constraint for any future thread RFC: a borrow never crosses a region boundary, so a Send-like rule can be "owned types only". Threads are out of scope for this epic.

## Reviewer checklist

- Every cited diagnostic and path matches the source.
- Section 5 breakage is acceptable (14 programs moved to `&rw`).
- `&rw` spelling and the single call-site aliasing rule are the intended v1 scope.
- Follow-up issues A-E are opened after this RFC is approved.
