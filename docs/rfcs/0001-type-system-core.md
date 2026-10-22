# RFC 0001: Core type-system decisions (TS-P0-02, issue #29)

- **Status:** Proposed
- **Date:** 2026-10-17
- **Issue:** #29 (epic: core-types, Phase 0)
- **Baseline:** [`docs/type-system/inventory.md`](../type-system/inventory.md) (#28) describes what the compiler does today.
- **Supersedes:** the line "no general generics" in `docs/type-system/inventory.md` and in `docs/superpowers/specs/2026-10-16-vec-design.md`. `Vec<T>` stays built-in until the generics phase (#45, #68) replaces it with a regular generic.

## Motivation

The type-system work in #31 to #69 fixes representation choices that are expensive to undo
(`DefId`/`TyId`, interning, generics, enums, traits, `Option`). This RFC decides the
semantic questions up front so those issues implement them instead of choosing them silently.

Today: types are nominal by class name string (`TyKind::Named(String)`, `src/hir/ty.rs`); there is no
subtyping; `Ty { kind, nullable: bool }` makes `T??` unrepresentable; `Ref<T>` is a separate
`TyKind::ManagedRef` that is "presence-capable" like `T?` (`Ty::is_presence_capable`); `Unknown`
doubles as the error type; `Vec` and `Range` are hardcoded kinds; compatibility is structural `Ty`
equality (there is no central assignability function).

Each section below has: Decision, Rationale, Rejected alternatives, Consequences, Depends on.

## 1. Nominal typing only; no class subtyping

**Decision.** Two class types are the same type only if they are the same definition. A class is never
a subtype of another class. Shared behaviour is expressed with traits (#55). `&T` is **not** a subtype
of `T` and `T` is not a subtype of `&T`. Moving between them is an explicit operation (borrow `&x`,
read through the reference, copy or clone as allowed by the capability queries in #60).

**Rationale.** Subtyping interacts with variance, layout (arena field offsets), ownership and drop.
Without it, assignability is type equality, which keeps inference (section 5) simple and matches how
typeck already works.

**Rejected.**
- Class inheritance / structural subtyping: needs variance rules and a uniform object layout, and fights
  arena allocation by value.
- Implicit `&T` to `T` (auto-deref on assignment): hides moves and copies, which the ownership model
  must keep visible.

**Consequences.** Assignability is `ty_a == ty_b` after substitution (sole exception: the `T` to `Ref<T>`
creation in section 3). Method lookup does not walk a
class hierarchy. Dynamic dispatch, if ever wanted, arrives as trait objects (#69).

**Depends on.** #55, #60, #69.

## 2. Variance: invariant everywhere in v1

**Decision.** Every type constructor is invariant in each of its parameters: generic parameters,
`Ref<T>`, `&T`, `Vec<T>`, `[T; N]`, `Range<T>`, and function types. There is no way to declare variance.
Changing this requires a new RFC (see #69).

**Rationale.** Without subtyping (section 1) variance has nothing to act on except lifetimes/regions,
which are deferred to the region RFC (#61). Invariance is the only choice that is sound without
that machinery.

**Rejected.**
- Covariant `&T` / `Vec<T>`: only meaningful with subtyping or regions; unsound for mutable containers.
- Per-parameter variance annotations: surface area with no user in v1.

**Consequences.** `Vec<A>` and `Vec<B>` are unrelated types for any `A != B`. No variance
computation pass is needed in HIR.

**Depends on.** #43, #45, #61, #69.

## 3. Nullability

**Decision.**
- `T?` stays a **distinct type constructor**, not sugar for `Option<T>`. `nullable` becomes a first-class
  part of the type (#33), not an orthogonal flag that other code can forget to check.
- **`T??` is rejected** with a diagnostic at type lowering. It is not collapsed to `T?`: collapsing hides
  which layer is absent and breaks generic code (`T` instantiated with `U?`).
- `T?`, `Ref<T>` and `Option<T>` are three different types. Conversions between them are **explicit
  only**; there are no implicit coercions in either direction. `T` to `T?` needs `Some(..)` (current
  behaviour: `val a: i32? = 5` is rejected).
- **One exception, kept permanently as a deliberate shorthand:** in checking mode, a value of class type `T` where `Ref<T>` is
  expected is wrapped in a ref creation (`check_managed_ref`, `src/typeck/expr/mod.rs`; e.g.
  `val r: Ref<Node> = Node("live")`, used across `programs/*/managed_refs`). This is an allocation into
  a managed arena driven by the expected type, not subtyping: it never applies in synthesis mode, never
  to `T?` or `Option<T>`, and never in the other direction. No new coercion may be added without an RFC.
- Generic instantiation that would produce `T??` (for example `T?` with `T = U?`) is an error at the
  instantiation site, with a note pointing at the parameter.

**Rationale.** `T?` is the cheap, presence-niche representation the codegen gate already lowers
(`codegen_lowers_nullable`). `Option<T>` is the general ADT (#54) and needs patterns and enums first.
Keeping them separate avoids blocking nullability work on ADTs while leaving a clear conversion story.

**Rejected.**
- `T?` as alias of `Option<T>`: couples nullable lowering to enums and exhaustiveness (#49 to #51).
- Collapse `T??` to `T?`: loses information; `Some(None)` vs `None` become indistinguishable.
- Implicit `T` to `T?` widening everywhere: the compiler already accepts `T` where `T?` is expected only
  through explicit `Some` / `None` expressions; this RFC does not widen that.

**Consequences.** Today's `is_presence_capable` (nullable or `Ref<T>`) remains a representation query, not
a subtyping relation. A generic signature that mentions `T?` cannot be instantiated with a nullable type
(e.g. `fun first<T>(v: Vec<T>): T?` with `T = U?`); lifting that needs a "non-nullable" bound or
`Option<T>`, decided with traits (#55) and #54. Existing `check_fail` diagnostics for nullable types are unchanged until the
nullability phase re-blesses them.

**Depends on.** #33, #54, and the nullability/ADT work (#49).

## 4. Generics: monomorphization only

**Decision.** Generic items are compiled by **monomorphization**: one specialised copy per distinct set of
concrete type arguments. There is no dictionary passing and no boxed/erased representation.
A generic body is **type-checked once**, against its declared bounds, not once per instantiation.
It is **ownership-checked once, before monomorphization, conservatively**: a parameter `T` is treated as
move-only with no known capabilities unless a bound says otherwise, so a generic body that passes
cannot fail ownership checks for any instantiation.

**Rationale.** Layout and region handling in Bork are known statically; monomorphization gives direct
calls and by-value layout with no runtime representation of types. Checking once against bounds gives
errors at the definition, not at some distant use.

**Rejected.**
- Dictionary passing / type erasure: needs uniform boxed values; conflicts with arena by-value layout.
- Check-per-instantiation (C++ templates style): errors appear at use sites and depend on the caller.
- Ownership-check after monomorphization: accepts generic code that breaks for some instantiations.

**Consequences.** Binary size grows with instantiation count; a later pass may dedupe identical
instances. `Vec<T>` becomes an ordinary generic class once #45 and #68 land; the vec design spec's
"no general generics" non-goal ends then. Recursive generic layout is validated at declaration (#45).

**Ownership contract.** The precise typeck / ownership / region contract is RFC #30 (TS-P0-03); the
conservative pre-mono check is implemented in #46. If #30 disagrees with the rule above ("`T` is
move-only without bounds"), #30 wins and this section is amended.

**Depends on.** #30, #34, #42, #43, #44, #45, #46, #47, #48, #60, #68.

## 5. Inference scope: bidirectional with local unification

**Decision.** Type inference is **bidirectional** (synthesize / check) plus **unification of inference
variables local to one function body**. Every item signature (function parameters and return, class fields,
constants) is **explicit**. There is no inference across item boundaries.

**Rationale.** This is what typeck does today (`expected: Option<&Ty>` passed down) and it extends
naturally to generics via local inference variables (#38, #39). Explicit item signatures keep
checking modular and make incremental/LSP re-checking cheap.

**Implementation (#38).** `src/infer` holds `InferCtxt`: union-find over `TyData::Infer` variables of kind
General / Int / Float (literals), `unify` (Array, Func, Ref, ManagedRef, Vec, Range, Nullable; `Error` absorbs),
an occurs check, snapshots with `rollback_to`/`commit`, and `resolve_shallow`/`resolve`. A failed `unify` leaves
the context unchanged. Failures are a structured `TypeError` (mismatch, occurs, arity, kind, nested nullable)
that callers turn into spanned diagnostics. It is not called by typeck yet (#39, #40).

**Rejected.**
- Whole-program Hindley-Milner: non-local errors, defeats modular checking.
- Inferring return types of items: makes signatures depend on bodies, breaks recursion and forward use.

**Consequences.** Local variables may omit their type; `[]` and `None` without an annotation get an
inference variable resolved by later uses in the same body (#40). Inference variables never escape a
body; unresolved ones are an error (#41), not a silent `Unknown`.

**Depends on.** #38, #39, #40, #41.

**Implementation (#39).** `typeck::expr` exposes `synthesize(expr)` and `check(expr, expected)`; the
mode of every `Expr` kind is listed in the module doc of `src/typeck/expr/mod.rs`. A synthesized
expression met with an expected type is compared by its caller through `Env::mismatch`, the single
place that words `X has type A, expected B`. Accepted programs are unchanged; `src/infer` is wired in
by #40.

## 6. Identity: `DefId` / `TyId` in a side interner

**Decision.** Definitions are identified by `DefId` (an index into a definition table) and types by `TyId`
(an index into a canonical type interner). Both live in a **side table owned by the compilation
session**. They are never tied to arena allocation, region lifetimes or addresses of compiler data.
`TyKind::Named(String)` is replaced by a `DefId` reference. Equal types intern to the same `TyId`.

**Rationale.** Type identity must outlive any user-program arena notion, be cheap to compare and hash,
and be stable under renaming and module moves.

**Rejected.**
- Names as identity (today's `Named(String)`): breaks with modules, generics and shadowing.
- Pointer identity into an arena: couples type lifetime to allocation and blocks serialization/debug tooling.

**Implementation (#32).** `src/ty` holds `TyId`, `TyData` and the `TyCtx` hash-consing interner, and the
constructors and queries every pass uses. `T?` is `Nullable(TyId)` (#33; `T??` is rejected, `Error?` is `Error`), `Error` replaces `Unknown`,
`Param(ParamIdx)` / `Infer(InferVar)` are `u32` newtypes for generics (Phase 3), `String` is its own variant (it has no `DefId`), and a class is `Class(DefId)`. Ids are only comparable within one `TyCtx`.

**Folding, printing, signatures (#34).** `TypeVisitor`/`TypeFolder`, `subst` (`Param(i)` to `args[i]`; an
instantiation that would make `T??` is an error, `Error?` stays `Error`), `normalize` (identity today: ids from
`TyCtx` are already canonical), `TyCtx::display` (`(i32)->i32` without spaces) and `FnSig<T>`. Typeck (#36) checks in `TyId`/`DefId`: signatures are `FnSigs` keyed by `DefId`, `ClassTable` is keyed by class `DefId`, and `lower_type` interns through `TyCtx`. `builtins::signatures` builds `FnSig<TyId>` directly.

**Consequences.** Typeck (#36) and every consumer (#37: sema, escape, memory, region_walk, codegen gate and the
LLVM backend, builtins, constants, the LSP) use `TyId` through the `TyCtx` that `frontend::CheckResult` keeps; the
legacy `hir::Ty` tree is gone (`hir::Prim` stays). The golden HIR-type snapshot
(`tests/golden/hir_types.txt`) prints names via `TyCtx::display`, not ids.

**Depends on.** #31, #32, #33, #35, #36, #37.

## 7. Numeric policy: no implicit coercion

**Decision.**
- There is **no implicit numeric conversion** between typed values: `i32` to `i64`, integer to float,
  signed to unsigned all require an explicit conversion.
- A literal takes the **expected type** when one is available and it is a compatible numeric type.
  Otherwise an integer literal defaults to **`i32`** and a float literal to **`f64`**.
- In a binary operation a literal adopts its peer operand's type; two typed operands must be equal.
- A literal that does not fit its resulting type is an error. (**New rule, not current behaviour:** no
  range check exists today; the owning phase adds it and re-blesses any affected diagnostics.)

**Rationale.** This documents what `src/typeck/expr/mod.rs` and `binary.rs` already do, and fixes it as the
contract before inference variables arrive (#38). Once they do, a literal without an expected type gets
an integer (or float) inference variable that any use in the same body may fix, e.g.
`val x = 5` then `takes_i64(x)` makes `x: i64`; if nothing fixes it, it defaults to `i32` / `f64` at the
end of the body (#40).

**Rejected.**
- Implicit widening (`i32` to `i64`): hides cost and overflow semantics, adds a second compatibility relation.
- Literal types chosen from outside the enclosing body: non-local, conflicts with section 5.

**Consequences.** Mixed-width arithmetic needs explicit conversions; an explicit conversion syntax does not
exist yet and is owned by #40. The range check may break corpus programs that rely on silent truncation
(today `val c: u8 = 300` is accepted); the owning phase must run the corpus and re-bless.

**Depends on.** #38, #39, #40.

## Migration impact on the current `TyKind`

This RFC adds no `TyKind` variants itself. Follow-up issues change representation (`Named(String)` to a
`DefId`, `Unknown` split into error/infer kinds, `nullable` made first-class). Any change to the variants in `hir::TyKind` or `ast::Type` must update
`docs/type-system/inventory.md`, which `inventory_mentions_every_variant` enforces.

## Open questions

- Explicit numeric conversion syntax (`as`, `i64(x)`, methods) is not chosen here (section 7).

## Reviewer checklist

- [ ] Each of the 7 sections states a decision, rationale and rejected alternatives.
- [ ] Section 3 rejects `T??` and keeps `T?`, `Ref<T>`, `Option<T>` as explicit conversions only.
- [ ] Section 3's `T` to `Ref<T>` creation is accepted as the only implicit conversion.
- [ ] Section 7 defaults literals to `i32` / `f64` and marks the range check as new.
- [ ] Section 4's supersession of "no general generics" is accepted.
- [ ] Downstream issues are linked to the sections they depend on (see PR description).
