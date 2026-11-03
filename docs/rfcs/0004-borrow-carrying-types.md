# RFC 0004: Borrow-carrying class and enum types, without region parameters (RFC0003-C, issue #201)

- **Status:** Proposed
- **Date:** 2026-11-03
- **Issue:** #201 (epic: RFC 0003 follow-ups)
- **Builds on:** [RFC 0003](0003-borrows-regions-generics.md) sections 3-6, [RFC 0002](0002-ownership-typeck-contract.md) INV-3 and INV-10.
- **Related:** #202 (returned borrows, RFC 0005), #63 (generic regions), #64 (negative tests).

Design only. No code changes. Borrow form names follow RFC 0003 section 5: `&` (read-only) and `&rw` (editable). Issue #201 says `&edit`; that spelling was replaced by `&rw`.

## 1. Motivation

A class or enum cannot hold a borrow today. Two shortcuts reject it (`src/typeck/mod.rs`: "borrowed references cannot be stored in class fields" and "... in enum payloads"), and INV-3 says "until region parameters exist (#61)". That rules out view-like types: a cursor over a `Vec`, a parser holding the text it reads, a `Window { buf: &rw [i32; 64], lo: i32 }`.

RFC 0003 section 5 also leaves a hole open: `val b = &a; f(&rw a, &b)` is not detected, because the call-site rule (`check_aliased_borrows`, #199) only sees the names in one call. It says closing it "needs region tracking (#201)".

## 2. Decision: no region parameters, no lifetime syntax

Bork gets **no** written lifetimes or region names (`'r`, `<'a>`, outlives bounds). That is the line between Bork and Rust, and this RFC keeps it. `TyData` gets **no** region slot either. Instead:

> A type is a **view type** when `TyCtx::contains_borrow` is true for it. A class or enum is a view type when any field or payload is a view type. Every v1 rule for `&T` applies to a view type unchanged.

So the region is not part of the type. It is a fact about a **binding**, tracked by sema, which already has `BindingOrigin::View` for `val b = &a` (`src/sema/env.rs`) and the anonymous region `REF_PARAM_BORROW_FROM = "call site"` for `&T` parameters (`src/sema/region.rs`).

```bork
class Window { buf: &rw [i32; 64], lo: i32 }   // field written as plain `&rw`
var data: [i32; 64] = ...
{
    val w = Window(buf: &rw data, lo: 0)       // w is a view of `data`; it dies with this region
    w.buf[w.lo] = 1                            // allowed: the field is &rw
}
```

## 3. Rules

**3.1 Fields and payloads.** A field or enum payload may be `&T` or `&rw T`, written plainly. `Rejected:` borrows inside `Vec`, `Ref`, arrays, nullable (`Window?`), and as type arguments (`Box<Window>`, `id::<Window>(w)`) stay rejected, because those checks already go through `contains_borrow` / `ill_formed` (#195). Making `contains_borrow` look through class and enum fields (with a cycle guard) is what extends them to view types. A generic class may hold a plain borrow (`Pair<T> { a: &[T; 3] }`); `T` itself is still never a borrow (RFC 0003 section 2).

**3.2 Construction.** `Window(buf: &rw data, ...)` is typed like a call. Each borrow argument records its owner name in the new binding's **borrow set**. A view binding's borrow set is the union of its borrow arguments' owners; `val d = w` copies it; passing a view into another view's constructor unions it. The borrow set is sema state, not type state.

**3.3 Where a view type may live.** Exactly where `&T` may today: `val` locals, parameters, and `this`. Not `var`, not returned, not assigned to an outer `var` or field, not `move`d or promoted out of its region. The existing diagnostics apply with the type printed (`cannot bind var ... to reference type`, `returning a reference is not supported`, ...). Because a view dies with the region of its binding and every owner it borrows lives in an enclosing region, it cannot outlive an owner. No outlives check is needed.

**3.4 Parameters.** A parameter of view type is a view parameter, with the same anonymous "call site" region as `&T`. Callers pass the view by name (`f(w)`, reborrow semantics). The callee cannot store it or return it (3.3). The call site does not write `f(&w)`: that would be a borrow of a borrow.

**3.5 Access through a field.** Reading `w.buf[i]` is allowed for `&` and `&rw` fields. Writing `w.buf[i] = v` requires the field to be `&rw`; a `&` field is read-only (same as RFC 0003 section 5). Reading `w.buf` as a value gives a reborrow of the field's form. A view with an `&rw` field is not copyable: `val d = w` is an error for it (exclusive access has one holder), but `f(w)` as a reborrow is fine.

**3.6 Closing the aliasing hole.** While a view whose borrow set contains `a` is in scope (to the end of its region), `a` is locked, by name and by region, with no liveness analysis (conservative; relax later if programs need it):
- a view through `&rw` excludes every other use of `a`: `a[i]`, `&a`, `&rw a`, `a = ...`, `move a`;
- a view through `&` excludes writes to `a`: `a[i] = v`, `a = ...`, `move a`, `&rw a`, and any `Vec` operation that can reallocate (`push`, ...). Reads and further `&a` stay allowed.

This covers `val b = &rw a; f(&rw a, &b)` and subsumes the call-site rule of #199, which stays as the cheaper early check. **Breaking:** programs that write `a` while a `val b = &a` is in scope stop compiling. The follow-up issue (section 7, item 5) starts by counting them in `programs/` and migrating them, as #198 did for `&`.

**3.7 Enums.** Same as classes. A `match` binding of a borrow payload is a view with the owner set of the scrutinee.

**3.8 Drop and codegen.** A borrow field owns nothing: no drop glue, no arena. It is a pointer to the owner's payload, as `&T` parameters are today. Arena dumps and LSP show `Borrow <- a` for such bindings, as for `&T` params.

## 4. Interaction with other work

- **Generics (#63).** Instantiating a generic class with an owned `T` never creates a view type from `T`. `ill_formed` already rejects `T = view type` (3.1). Diagnostics keep the note pointing at the generic definition (#195).
- **Returned borrows (#202).** Out of scope here. 3.3 keeps functions from returning views. RFC 0005 will add the signature-level rule that ties a returned view to a parameter. It can reuse the borrow set from 3.2 as its vocabulary, so E needs no new type-level concept either. If it turns out to need one, that is a reason to revisit this RFC, not to add lifetimes.
- **`&unique` (#200).** Stays reserved. If it ever arrives it is a third form of borrow in a field; 3.6 is already the exclusive case it would need.
- **Threads (RFC 0003 section 8).** View types are not Send-like: they hold a borrow that never crosses a region boundary.

## 5. Rejected

- **Region parameters on types (`View<'r>`), or a region slot in `TyData`.** Rust lifetimes in another spelling. Type equality, inference, variance (RFC 0001 section 2) and generic instantiation all grow a new dimension. Against the language's purpose.
- **Inferring a region per type from field uses.** Not decidable for fields without annotations (RFC 0003 section 3).
- **Views as counted handles (`Ref`).** Defeats region arenas (RFC 0003 section 3).
- **Liveness (NLL-style) tracking in v1 of this RFC.** More precise, more machinery. 3.6 can be loosened later without breaking programs that already pass.

## 6. Decisions taken in review

1. View arguments are passed as `f(w)` (3.4).
2. Plain `&` views also lock their owner against writes and `Vec` reallocation (3.6).
3. Views in `Vec<View>`, arrays and `View?` stay rejected. Allowing them makes the container's region the minimum of its elements', which is a separate RFC.

## 7. Follow-up issues

1. **Parser/AST:** allow `&`/`&rw` in field and payload positions; no new syntax.
2. **Types:** `contains_borrow` through class/enum fields; `ill_formed` and `lower.rs` use it; remove the two `typeck/mod.rs` shortcuts; update INV-3.
3. **Typeck:** construction (3.2), field access and write rules (3.5), view-type bindings/params (3.3, 3.4).
4. **Sema:** borrow sets on view bindings (3.2), escape/move/promote rules (3.3) in `sema/walk`, `region.rs`, `escape.rs`.
5. **Exclusivity (3.6):** by-name lock while a view is in scope, for `&rw` and `&` views; replaces the "not detected" note in RFC 0003 section 5. First count and migrate affected programs in `programs/`; check how `Vec` reallocation under a live `&` view behaves today.
6. **Generics (#63):** instantiation tests with view types, definition notes.
7. **Codegen, dump, LSP, docs:** pointer fields, `Borrow <- a`, `docs/language.md`, `docs/memory-model.md`, negative suite (#64).

## Reviewer checklist

- Every cited path and diagnostic matches source.
- "A view type is a type that contains a borrow, and the region is a binding fact" is acceptable as the whole mechanism (no region in `TyData`, no syntax).
- 3.6 by-name, to-end-of-region locking (including for plain `&` views) is the right v1 strictness, and its breakage is acceptable.
