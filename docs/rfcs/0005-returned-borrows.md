# RFC 0005: Returned borrows, without lifetime annotations (RFC0003-E, issue #202)

- **Status:** Proposed
- **Date:** 2026-11-03
- **Issue:** #202 (epic: RFC 0003 follow-ups)
- **Builds on:** [RFC 0004](0004-borrow-carrying-types.md) (view types, borrow sets, section 3.6 locking), [RFC 0003](0003-borrows-regions-generics.md) sections 3 and 5, [RFC 0002](0002-ownership-typeck-contract.md) INV-3 and INV-10.
- **Related:** #63 (generic regions), #64 (negative tests), #69 (trait objects).

Design only. No code changes. Borrow forms are `&` (read-only) and `&rw` (editable), as in RFC 0003 section 5.

## 1. Motivation

A function cannot return a borrow today. `src/escape.rs` rejects every `return` whose type is a reference ("returning a reference is not supported"), and RFC 0004 section 3.3 extends that to view types. So accessors and slicing helpers must copy:

```bork
fun first(v: &Vec<i32>): &i32 { return &v[0] }       // rejected today
fun window(b: &rw Buf, lo: i32): Window { ... }      // Window holds `&rw`; rejected today
```

The obstacle is not memory safety for these cases. The result points into data owned by the caller, so it cannot dangle. The obstacle is that nothing says *which* argument the result points into. RFC 0003 section 3 rejected inferring that at the call site from the callee's body, and rejected Rust lifetimes. This RFC picks the rule that needs neither.

## 2. Decision: the signature is the contract, and it names no region

> A function may return a borrow or view type when every borrow it can return is rooted in a **borrow or view parameter** (including `this`). The tie from result to arguments is derived from the **signature alone**: the result borrows from **all** borrow and view arguments of the call.

No lifetime names, no outlives clauses, no new syntax, no new field in `TyData`. The vocabulary is the borrow set of RFC 0004 section 3.2: the result of a call is a view whose borrow set is the union of the owners of its borrow and view arguments.

## 3. Rules

**3.1 Body check (callee).** For each `return e`, compute the borrow set of `e` with the machinery of RFC 0004 section 3.2. It is accepted when every member is a borrow or view parameter of this function. Rejected, each with the existing message plus a note at the offending expression:
- `return &local` or a view of a local (`returning a reference is not supported`, note: `local` dies with the function);
- a view built in an inner region of the body, or from a `var`/owned parameter (the owner is not the caller's);
- mixed branches, one returning a parameter's view and another a local's. Each `return` is checked on its own, so the mixed case is the second one failing.
Allowed shapes: `return p`, `return &p[i]`, `return p[lo..hi]`, `return p.field` where the field is a view, and `return Window(buf: p, lo: 0)` (constructor from a parameter's borrow).

**3.2 Result at the call site (caller).** The call expression has the declared return type and a borrow set equal to the union of the owners of all its borrow and view arguments (the receiver counts, as in #199). The result is a view like any other (RFC 0004 section 3.3): `val` only, not returned past its own function's rules (3.1 applies again), dies with its region.

**3.3 Over-locking is accepted.** With two borrow parameters the result is tied to both, even if the body only ever returns one:

```bork
fun pick(a: &[i32; 3], b: &[i32; 3], left: bool): &[i32; 3] { if left { return a } else { return b } }
val r = pick(&x, &y, true)      // r locks both x and y (RFC 0004 section 3.6 locking) while it is in scope
```

This is stricter than needed but never unsafe, needs no syntax, and the body check (3.1) already guarantees the result comes from some parameter. A clause naming parameters (`... from a`) would remove the over-locking. It names a *parameter*, not a region, but it is new syntax, so it is **not** part of this RFC; it can be added later without breaking any program that compiles under 3.2, because it only narrows the borrow set.

**3.4 `&` and `&rw` in the result.** A result is `&rw` only if every parameter it can come from is `&rw` (or a view holding `&rw`). A `&rw` parameter may be returned as `&rw` or `&` (reborrow, RFC 0003 section 5); a `&` parameter only as `&`. At the call site a `&rw` result keeps the exclusive lock of RFC 0004 section 3.6 on every owner in its borrow set, so `val w = window(&rw buf, 0)` locks `buf` until `w`'s region ends; a `&` result locks only writes.

**3.5 Methods and traits.** `this` is a parameter, so `fun get(i: i32): &i32` on a class returns a borrow tied to the receiver. The rule is checked on the `impl` method body. A trait method signature declaring a borrow return gets the same tie to all borrow and view parameters, so a call through the trait needs no body. Trait objects are out of scope (#69).

**3.6 Generics (#63).** `T` is never a borrow, so `fun id<T>(x: T): T` is unaffected. A generic function may return `&Pair<T>` or a view, tied to its borrow parameters by 3.2. INV-10 holds: the return rule depends only on parameter kinds, which are the same in every instance, and the monomorphiser re-runs `escape` on each instance as today.

**3.7 Recursion.** Call sites use only the callee's signature, so recursive and mutually recursive functions need no fixpoint. The body check is local to each function.

**3.8 Codegen.** A returned borrow is the caller's pointer passed back; no allocation, no drop glue, no change to arenas. Arena dumps and LSP show `Borrow <- x, y` on the receiving binding, as for views.

## 4. Examples

| Program | Result | Reason |
|---|---|---|
| `fun first(v: &Vec<i32>): &i32 { return &v[0] }` | accepted | rooted in parameter `v` |
| `fun pick(a: &T, b: &T, l: bool): &T` returning `a` or `b` | accepted | result locks both args at the call site (3.3) |
| `fun f(v: &rw Buf): &rw Buf { return v }` | accepted | `&rw` passed through (3.4) |
| `fun f(v: &Buf): &rw Buf { return v }` | rejected | cannot widen `&` to `&rw` |
| `fun g(): &String { val s = "x"; return &s }` | rejected | borrow of a local (3.1) |
| `fun h(p: String): &String { return &p }` | rejected | owned parameter dies with the call (3.1) |

## 5. Rejected

- **Lifetime names or `'a` on signatures.** Rust's model; against the purpose of the language. Also rejected in RFC 0003 section 3 and RFC 0004 section 2.
- **Inferring the tie from the callee body at each call site.** Call sites would depend on another function's body; unclear for trait methods (RFC 0003 section 5, rejected there).
- **Promoting the returned value to an outer arena** so a borrow of a local could be returned. A borrow would then own memory; that is `Ref<T>` or a copy, not a borrow.
- **A `from a, b` clause in v1.** Useful, not required; see 3.3.
- **Liveness (NLL-style) tracking** to shorten the lock. Same reasoning as RFC 0004 section 5.

## 6. Consequences

- INV-3 (RFC 0002) loses its "no `&T` in a return type" half; the class-field half was already lifted by RFC 0004.
- `docs/language.md` and `docs/memory-model.md` drop "not allowed on ... return types" and gain a returns table (section 4 above).
- Programs that currently copy to avoid returning a borrow keep compiling; nothing that compiles today is rejected.
- Open point for review: no known open question blocks the design. The one deliberate cost is over-locking with several borrow parameters (3.3).

## 7. Follow-up issues

1. **Typeck:** result borrow set at calls (3.2), `FnSig` return-type check, trait-method elision (3.5). `src/typeck/expr/call.rs`, `src/ty/sig.rs`.
2. **Escape/sema:** body check for returns rooted in parameters (3.1), reuse RFC 0004 borrow sets. `src/escape.rs` (the `Return` arm and the second reference check), `src/sema/`.
3. **`&rw` returns (3.4):** widening rules and the exclusive lock on every owner in the result's borrow set.
4. **Generics and traits (3.6):** instantiation re-check, notes pointing at the generic definition.
5. **Cleanup, docs, tests:** remove the "returning a reference" shortcut, update INV-3, `docs/language.md`, `docs/memory-model.md`, LSP/dump labels, negative suite (#64) from the table in section 4.

Dependencies: RFC 0004 follow-up issues 3 (typeck), 4 (sema borrow sets) and 5 (locking).

## Reviewer checklist

- Every cited path and diagnostic matches source.
- "Result borrows from all borrow and view arguments" (3.2, 3.3) is acceptable for v1, with a parameter-naming clause left as a later, backwards-compatible narrowing.
- Section 4 table matches intended behaviour, in particular the rejected rows.
- Nothing here needs a lifetime name or a type-level region; if review finds a case that does, revisit RFC 0004 instead of adding one.
