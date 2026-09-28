# Codegen thermo four Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear the four thermonuclear blockers (`ChildPlan::If` payload, `NullableRepr`, `IfKind`, combine-`Binary` dispatch) so a deslop + re-review can approve the nullable/unify/`If` work as merge-ready.

**Architecture:** `child_plan` is the only fetch policy: `If` carries cond/blocks, never stack arity. LLVM `if` emission takes `IfKind::{Unit,Value}` instead of `Option<&Ty>`. `T?` layout is one `NullableRepr`. `combine_expr` matches `BinOp` so CFG/nullable ops are not an if-ladder on the ALU path.

**Tech Stack:** Rust, Inkwell/LLVM 23 (`codegen` feature), HIR `HirExprKind` / `Ty`.

## Global Constraints

- Do **not** edit the optionals codegen plan file (or any other existing plan except this one).
- Behavior unchanged except classification bugs where `nullable_is_tagged_scalar` said true for types `nullable_storage_type` cannot lower (`f32?` / `f64?`): those stay `None` from `nullable_repr`, matching storage.
- Do not invent work outside the four issues plus the final deslop/thermo gate (no `String?` identity-eq rewrite, no `emit_skipped_as` ty-lie unless deslop finds a one-line deletion).
- Do not commit unless the user explicitly asks. Skip every Commit step until then.
- Tests: `LD_LIBRARY_PATH=/home/enrju/llvm23-prefix/usr/lib/x86_64-linux-gnu cargo test --lib --features codegen`
- Keep `src/codegen/llvm/expr.rs` under 1000 lines.

---

## File map

| File | Responsibility |
| --- | --- |
| `src/codegen/llvm/expr.rs` | `ChildPlan` (including `If {..}`), `emit_expr` / `emit_plan` / `combine_expr` Binary match, child_plan tests |
| `src/codegen/llvm/region_emit.rs` | `IfKind`, `emit_if_with_driver`; delete `emit_if_expr` |
| `src/codegen/llvm/nullable.rs` | `NullableRepr` + `nullable_repr`; all `T?` helpers match on it; delete `nullable_is_buffer` / `nullable_is_tagged_scalar` |
| `src/codegen/llvm/context.rs` | `nullable_storage_type` uses `nullable_repr` (one layout story) |

---

### Task 1: `IfKind` — unit vs value `if`

**Files:**
- Modify: `src/codegen/llvm/region_emit.rs`
- Test: existing `cargo test --lib --features codegen` (value-`if` + statement-`if` already covered)

**Interfaces:**
- Consumes: `Ty::unit()`, current `emit_if_with_driver(..., ty: Option<&Ty>)`
- Produces: `pub(super) enum IfKind<'a> { Unit, Value(&'a Ty) }` with `IfKind::from_ty(ty: &Ty) -> IfKind<'_>` (`Unit` iff `*ty == Ty::unit()`, else `Value(ty)`). `emit_if_with_driver(..., kind: IfKind<'_>)`.

- [ ] **Step 1: Add `IfKind` and switch `emit_if_with_driver`**

```rust
pub(super) enum IfKind<'a> {
    Unit,
    Value(&'a Ty),
}

impl IfKind<'_> {
    pub(super) fn from_ty(ty: &Ty) -> IfKind<'_> {
        if *ty == Ty::unit() {
            IfKind::Unit
        } else {
            IfKind::Value(ty)
        }
    }
}
```

Replace the `ty: Option<&Ty>` parameter. Inside, do **not** `unwrap` an `Option<&Ty>`. Split on `kind`:

```rust
let (value_ty, llvm_ty) = match kind {
    IfKind::Unit => (None, None),
    IfKind::Value(ty) => {
        let llvm = self.cx.basic_type(ty);
        (llvm.is_some().then_some(ty), llvm)
    }
};
```

Use `value_ty` for `emit_region_with_driver` / missing-trailing error (same as today’s `value_ty`). Use `llvm_ty` for the phi (`result_ty`). Empty incoming + `IfKind::Value(_)` still `const_zero` (today’s `else if ty.is_some()`). `IfKind::Unit` never builds a phi.

Call sites:

- `emit_if_expr` (still exists until Task 2): `IfKind::from_ty(&expr.ty)`
- `if_expr` visitor: `IfKind::from_ty(result_ty)`

- [ ] **Step 2: Run lib tests**

Run: `LD_LIBRARY_PATH=/home/enrju/llvm23-prefix/usr/lib/x86_64-linux-gnu cargo test --lib --features codegen`

Expected: PASS (204+ tests). No `ty.unwrap()` left on this path.

- [ ] **Step 3: Commit** (skip unless user asked)

---

### Task 2: `ChildPlan::If` payload; delete `Region` and `emit_if_expr`

**Files:**
- Modify: `src/codegen/llvm/expr.rs`
- Modify: `src/codegen/llvm/region_emit.rs` (delete `emit_if_expr`)
- Test: `src/codegen/llvm/expr.rs` `child_plan_tests`

**Interfaces:**
- Consumes: `IfKind::from_ty` from Task 1; `emit_if_with_driver`
- Produces: `ChildPlan::If { cond: &'a HirExpr, then_block: &'a HirBlock, else_block: Option<&'a HirBlock> }`. No `ChildPlan::Region`. `len()` / `emit_plan` do not treat `If` as a leaf (`unreachable!` on `If`). `emit_expr` calls `emit_if_with_driver` from the `If` arm. `emit_if_expr` gone.

- [ ] **Step 1: Write the failing test**

Rename `child_plan_value_if_is_region` → `child_plan_value_if_carries_blocks`. Replace `ChildPlan::Region` / `len() == 0` with:

```rust
#[test]
fn child_plan_value_if_carries_blocks() {
    let program =
        return_expr("fun main(c: bool): i32 { return if (c) { 1 } else { 2 } }\n");
    let expr = main_return(&program);
    let ChildPlan::If {
        cond,
        then_block,
        else_block,
    } = child_plan(expr)
    else {
        panic!("expected ChildPlan::If, got {:?}", /* keep matches! if Debug not derived */);
    };
    assert!(matches!(cond.kind, HirExprKind::Ident { .. }));
    assert!(else_block.is_some());
    let _ = then_block;
}
```

Do **not** call `.len()` on `If`. Derive `Debug` on `ChildPlan` if the panic message needs it; otherwise use `matches!` only:

```rust
assert!(matches!(
    child_plan(expr),
    ChildPlan::If {
        else_block: Some(_),
        ..
    }
));
```

- [ ] **Step 2: Run test to verify it fails**

Run: `LD_LIBRARY_PATH=... cargo test --lib --features codegen child_plan_value_if -- --nocapture`

Expected: FAIL (still `ChildPlan::Region` or test name missing).

- [ ] **Step 3: Implement `ChildPlan::If` and fetchers**

```rust
enum ChildPlan<'a> {
    Zero,
    If {
        cond: &'a HirExpr,
        then_block: &'a HirBlock,
        else_block: Option<&'a HirBlock>,
    },
    One(&'a HirExpr),
    Two(&'a HirExpr, &'a HirExpr),
    Three(&'a HirExpr, &'a HirExpr, &'a HirExpr),
    Many(&'a [HirExpr]),
}

impl ChildPlan<'_> {
    fn len(self) -> usize {
        match self {
            ChildPlan::If { .. } => unreachable!("If is not stacked"),
            ChildPlan::Zero => 0,
            ChildPlan::One(_) => 1,
            ChildPlan::Two(_, _) => 2,
            ChildPlan::Three(_, _, _) => 3,
            ChildPlan::Many(children) => children.len(),
        }
    }
}
```

`child_plan`: HIR is `else_block: Option<HirBlock>` (not boxed). Map with `else_block.as_ref()`:

```rust
HirExprKind::If {
    cond,
    then_block,
    else_block,
} => ChildPlan::If {
    cond,
    then_block,
    else_block: else_block.as_ref(),
},
```

`emit_expr`:

```rust
pub fn emit_expr(...) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
    match child_plan(expr) {
        ChildPlan::If {
            cond,
            then_block,
            else_block,
        } => {
            self.emit_if_with_driver(cond, then_block, else_block, IfKind::from_ty(&expr.ty))?;
            Ok(self.walk.trailing.take())
        }
        plan => {
            let ops = if call_gathers_without_sink(expr) {
                self.without_alloc_sink(|e| e.emit_plan(plan))?
            } else {
                self.emit_plan(plan)?
            };
            self.combine_expr(expr, &ops)
        }
    }
}
```

`emit_plan`: `ChildPlan::If { .. } => unreachable!("If is not combined")` — **not** `Vec::new()`.

`combine_expr` If arm: `unreachable!("If is ChildPlan::If")`.

Delete `emit_if_expr` from `region_emit.rs`. Import `IfKind` in `expr.rs` (`use super::region_emit::IfKind` or re-export from the llvm module if `region_emit` is private — use `super::region_emit::IfKind` only if the module is `pub(super)`; otherwise add `pub(super) use` in `region_emit` and import from `super::region_emit`). If the module is private, `expr.rs` can still `use super::region_emit::IfKind` as a sibling.

Add `HirBlock` to `expr.rs` hir import.

- [ ] **Step 4: Run tests**

Run: full `--lib --features codegen`

Expected: PASS. Grep: no `ChildPlan::Region`, no `emit_if_expr`.

- [ ] **Step 5: Commit** (skip unless user asked)

---

### Task 3: `NullableRepr` — one layout type

**Files:**
- Modify: `src/codegen/llvm/nullable.rs`
- Modify: `src/codegen/llvm/context.rs` (`nullable_storage_type`)
- Test: unit tests in `nullable.rs` (no LLVM)

**Interfaces:**
- Consumes: `Ty::is_nullable`, `with_nullable(false)`, `is_string` / `is_array`, `TyKind::Prim` (same prims as `int_type`: integers + `Bool`, not `F32`/`F64`/`Unit`)
- Produces: `pub(super) enum NullableRepr { Buffer, TaggedScalar }` and `pub(super) fn nullable_repr(ty: &Ty) -> Option<NullableRepr>`. Delete `nullable_is_buffer` / `nullable_is_tagged_scalar`. Helpers match `nullable_repr(ty)` once. `nullable_storage_type` matches the same function.

- [ ] **Step 1: Write failing tests**

In `nullable.rs`:

```rust
#[cfg(test)]
mod nullable_repr_tests {
    use super::{nullable_repr, NullableRepr};
    use crate::hir::{Prim, Ty, TyKind};

    #[test]
    fn string_optional_is_buffer() {
        assert_eq!(
            nullable_repr(&Ty::string(true)),
            Some(NullableRepr::Buffer)
        );
    }

    #[test]
    fn i32_optional_is_tagged() {
        assert_eq!(
            nullable_repr(&Ty::i32().with_nullable(true)),
            Some(NullableRepr::TaggedScalar)
        );
    }

    #[test]
    fn f32_optional_is_none() {
        let ty = Ty::new(TyKind::Prim(Prim::F32), true);
        assert_eq!(nullable_repr(&ty), None);
    }

    #[test]
    fn non_nullable_is_none() {
        assert_eq!(nullable_repr(&Ty::i32()), None);
    }
}
```

`NullableRepr` needs `PartialEq` + `Debug` (and `Clone`/`Copy`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib --features codegen nullable_repr_tests -- --nocapture`

Expected: FAIL (items missing).

- [ ] **Step 3: Implement `nullable_repr` and rewrite helpers**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NullableRepr {
    Buffer,
    TaggedScalar,
}

pub(super) fn nullable_repr(ty: &Ty) -> Option<NullableRepr> {
    if !ty.is_nullable() {
        return None;
    }
    let inner = ty.with_nullable(false);
    if inner.is_string() || inner.is_array() {
        return Some(NullableRepr::Buffer);
    }
    match inner.kind {
        TyKind::Prim(Prim::F32 | Prim::F64 | Prim::Unit) => None,
        TyKind::Prim(_) => Some(NullableRepr::TaggedScalar),
        _ => None,
    }
}
```

`nullable_storage_type`:

```rust
pub fn nullable_storage_type(&self, ty: &Ty) -> Option<BasicTypeEnum<'ctx>> {
    match crate::codegen::llvm::nullable::nullable_repr(ty)? {
        NullableRepr::Buffer => Some(self.buffer_descriptor_type().into()),
        NullableRepr::TaggedScalar => {
            let inner = ty.with_nullable(false);
            let value_ty = self.int_type(&inner)?;
            let tag = self.context.bool_type();
            Some(self.context.struct_type(&[tag.into(), value_ty.into()], false).into())
        }
    }
}
```

If `nullable` is a private submodule, `context.rs` uses `super::nullable::{nullable_repr, NullableRepr}`.

Rewrite `emit_nullable_is_null` / `some` / `unwrap` and the payload_eq arm to `match nullable_repr(ty)` with `_ => Err(not_yet_supported(...))`. No remaining `if cx.nullable_is_*`.

- [ ] **Step 4: Run tests**

Run: full `--lib --features codegen`

Expected: PASS. Grep: no `nullable_is_buffer`, no `nullable_is_tagged_scalar`.

- [ ] **Step 5: Commit** (skip unless user asked)

---

### Task 4: `combine_expr` Binary — match `op`, not an if-ladder

**Files:**
- Modify: `src/codegen/llvm/expr.rs` (`combine_expr` `HirExprKind::Binary` arm only)
- Test: existing elvis / `&&` / `||` / nullable `==` tests (no new corpus required)

**Interfaces:**
- Consumes: `emit_elvis`, `emit_logical_from_lhs`, `is_nullable_equality`, `combine_binary_values`
- Produces: a single `match op` (match-guard for nullable equality is allowed). No sequential `if *op == Elvis` / `if And|Or` / `if is_nullable_equality`.

- [ ] **Step 1: Replace the Binary arm**

```rust
HirExprKind::Binary { op, lhs, rhs } => match op {
    BinOp::Elvis => self.emit_elvis(lhs, rhs, ops[0], expr).map(Some),
    BinOp::And | BinOp::Or => {
        let lhs_bool = self.value_as_bool(ops[0], expr.span)?;
        self.emit_logical_from_lhs(op, lhs_bool, rhs, expr).map(Some)
    }
    BinOp::Eq | BinOp::Ne if is_nullable_equality(op, lhs, rhs) => self
        .emit_nullable_eq(op, lhs, rhs, ops[0], ops[1], expr)
        .map(Some),
    _ => self
        .combine_binary_values(op, lhs, rhs, ops[0], ops[1], expr)
        .map(Some),
},
```

Integer `==` still hits `_` → `combine_binary_values` → `comparison()`. Nullable `==` hits the guard. Do not change `is_nullable_equality`.

- [ ] **Step 2: Run tests**

Run: full `--lib --features codegen`

Expected: PASS.

- [ ] **Step 3: Commit** (skip unless user asked)

---

### Task 5: Deslop + thermonuclear iteration (merge gate)

**Files:** whatever Task 1–4 touched; no new features.

- [x] **Step 1: Deslop vs this plan’s diff**

Remove comments that only restated variant names, dead `Region` leftovers, `pub(super)` that nothing outside the module uses, duplicate `child_plan(expr)` calls, and defensive `Option`/`unwrap` that `IfKind` already made impossible. Keep behavior.

- [x] **Step 2: Thermonuclear re-review — all four must be gone**

Approve only if **all** of these are true:

1. **If payload:** `ChildPlan::If { cond, then_block, else_block }` exists; `Region` does not; `emit_if_expr` does not; `emit_expr` does not re-match `HirExprKind::If`; `len()`/`emit_plan` do not treat `If` as `Zero`.
2. **NullableRepr:** `nullable_is_buffer` / `nullable_is_tagged_scalar` gone; is_null/some/unwrap/eq payload and `nullable_storage_type` all go through `nullable_repr`.
3. **IfKind:** `emit_if_with_driver` takes `IfKind`, not `Option<&Ty>`; no `ty.unwrap()` on that path.
4. **Binary dispatch:** `combine_expr` Binary is a `match op`, not a ladder of `if`s.

Also: `expr.rs` still under 1000 lines; no new spaghetti in `after_expr` / `emit_after_walk` for `If` (walker still must not `after_expr` `If`).

If any blocker remains, fix it in this task and repeat Step 1–2. Stop when this review would approve.

- [x] **Step 3: Verify**

Run: `LD_LIBRARY_PATH=/home/enrju/llvm23-prefix/usr/lib/x86_64-linux-gnu cargo test --lib --features codegen`

Expected: PASS.

- [ ] **Step 4: Commit** (skip unless user asked)

---

## Self-review

- Spec coverage: issues 1–4 map to Tasks 2, 3, 1, 4; merge gate is Task 5.
- No placeholders. `else_block` is `Option<HirBlock>`; plan uses `as_ref()`.
- Names: `IfKind`, `NullableRepr`, `nullable_repr`, `ChildPlan::If` used consistently.
- Out of scope: `String?` pointer identity `==`, `emit_skipped_as` ty on the walk path.
