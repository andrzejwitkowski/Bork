# Expression Move + Call-Site Move Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add expression `move x`, always-explicit ownership at assign/call, and `val`/`var` function params, while keeping regional `move (…) { }` as a secondary form.

**Architecture:** `Expr::Move` marks a binding Moved without opening an arena. Calls look up callee formals from a signature map built at analyze start. Regional captures stay Local in the child (`from_capture`) so a second `move` on that name errors. Handbook leads with expression/call examples.

**Tech Stack:** Rust, LALRPOP (`src/parser.lalrpop`), existing `sema` / `tests/parser.rs` / `src/sema/tests.rs`, handbook `docs/memory-model.md`.

**Spec:** [docs/superpowers/specs/2026-09-23-expression-move-design.md](../specs/2026-09-23-expression-move-design.md)

## Global Constraints

- Non-Copy **`var` sources** at assign/call: always require explicit `move` (no auto-consume).
- Brace / trailing regional `move` **kept**; captures already Local — no second `move` inside for those names.
- Bare param `name: Type` = **`val`**; optional `val` / `var` prefix allowed.
- MVP operand: **`move Ident` only** (not `move foo()`).
- Compile-time ownership only; no LLVM.
- Loop rule unchanged: cannot move an **outer** binding inside a `for` body (existing post-body check covers expression moves that set `moved`).

---

## File map

| File | Role |
|------|------|
| `src/ast.rs` | `Expr::Move`, `Param.kind: BindingKind` |
| `src/parser.lalrpop` | Atom `move Ident`; Param optional Binding |
| `tests/parser.rs` | Parse tests; flip old negatives to positives |
| `src/sema/env.rs` | `EnvBinding.from_capture`; optional `FunSig` map on `Analyzer` |
| `src/sema/region.rs` | `RegionParam.kind`; `bind_param` uses it; captures set `from_capture` |
| `src/sema/walk.rs` | Walk `Move`; assign/call explicit-move checks |
| `src/sema/analyze.rs` | Prefill function signatures; pass `kind` into `RegionParam` |
| `src/sema/free_vars.rs` | Visit `Expr::Move` as free use of name |
| `src/sema/tests.rs` | Sema acceptance + replace NYI gap tests |
| `docs/memory-model.md` | Lead with expression/call; regional secondary |

---

### Task 1: AST + parser for `move Ident` and `val`/`var` params

**Files:**
- Modify: `src/ast.rs`
- Modify: `src/parser.lalrpop`
- Modify: `tests/parser.rs`
- Test: `tests/parser.rs`

**Interfaces:**
- Produces: `Expr::Move { name: String, span: Span }`; `Param { kind: BindingKind, name: SpannedName, ty: Type }` (default `BindingKind::Val` when omitted)

- [ ] **Step 1: Write failing parser tests**

In `tests/parser.rs`, replace `val_or_var_function_params_not_supported_yet` with positives and add expression-move tests:

```rust
#[test]
fn parses_val_var_function_params() {
    let prog = parse("fun f(val x: Int, var y: String): Int { return x }").expect("parse");
    assert_eq!(prog.functions[0].params[0].kind, BindingKind::Val);
    assert_eq!(prog.functions[0].params[1].kind, BindingKind::Var);
}

#[test]
fn bare_param_defaults_to_val() {
    let prog = parse("fun f(x: Int): Int { return x }").expect("parse");
    assert_eq!(prog.functions[0].params[0].kind, BindingKind::Val);
}

#[test]
fn parses_move_expression_in_var_decl() {
    let prog = parse(
        r#"fun main() {
    var s: String = "hi"
    var x = move s
}"#,
    )
    .expect("parse");
    let Stmt::VarDecl { value, .. } = &prog.functions[0].body.stmts[1] else {
        panic!("expected second var decl");
    };
    assert!(matches!(value, Expr::Move { name, .. } if name == "s"));
}

#[test]
fn parses_move_expression_as_call_arg() {
    let prog = parse(
        r#"fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(move s)
}"#,
    )
    .expect("parse");
    let Stmt::Expr(Expr::Call { args, .. }) = &prog.functions[1].body.stmts[1] else {
        panic!("expected call");
    };
    assert!(matches!(&args[0], Expr::Move { name, .. } if name == "s"));
}
```

- [ ] **Step 2: Run tests — expect fail**

Run: `cargo test --test parser parses_move_expression_in_var_decl parses_val_var_function_params -- --nocapture`

Expected: compile and/or parse failures (`Expr::Move` / `Param.kind` missing).

- [ ] **Step 3: AST changes**

In `src/ast.rs`:

```rust
pub struct Param {
    pub kind: BindingKind,
    pub name: crate::span::SpannedName,
    pub ty: Type,
}

// in enum Expr, after Ident:
    Move {
        name: String,
        span: crate::span::Span,
    },
```

- [ ] **Step 4: Grammar**

In `src/parser.lalrpop` Param:

```rust
Param: Param = {
    <kind:Binding> <name:SpannedIdent> ":" <ty:Type> => Param { kind, name, ty },
    <name:SpannedIdent> ":" <ty:Type> => Param { kind: BindingKind::Val, name, ty },
};
```

In `Atom:` (before SpannedIdent):

```rust
    "move" <id:SpannedIdent> => Expr::Move {
        name: id.name,
        span: id.span,
    },
```

Keep Stmt `move` blocks unchanged. Regional `move (a) { }` still uses the Stmt production (keyword + optional `(…)` + Block), not Atom.

- [ ] **Step 5: Fix all `Param { name, ty }` construction sites**

Search for `Param {` and add `kind: BindingKind::Val` (or the parsed kind). Rebuild until the crate compiles.

- [ ] **Step 6: Run parser tests — expect pass**

Run: `cargo test --test parser`

Expected: PASS (including new tests; old negative `val_or_var_…` removed).

- [ ] **Step 7: Commit**

```bash
git add src/ast.rs src/parser.lalrpop tests/parser.rs
git commit -m "$(cat <<'EOF'
feat(parser): expression move and val/var params

EOF
)"
```

---

### Task 2: Sema — expression `move` + assign / same-arena rebinding

**Files:**
- Modify: `src/sema/env.rs`
- Modify: `src/sema/region.rs`
- Modify: `src/sema/walk.rs`
- Modify: `src/sema/free_vars.rs`
- Modify: `src/sema/tests.rs`
- Test: `src/sema/tests.rs`

**Interfaces:**
- Consumes: `Expr::Move { name, span }`
- Produces: `EnvBinding { from_capture: bool, … }`; `fn apply_expr_move(az, name, span, node)` marks Moved (errors if unknown / already moved / `from_capture`); VarDecl/Assign reject bare non-Copy `var` (and `var` dest from non-Copy without `move`)

- [ ] **Step 1: Write failing sema tests**

Add to `src/sema/tests.rs`:

```rust
#[test]
fn expr_move_rebinding_marks_source_moved() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = move s
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "s must be moved: {errs:?}"
    );
}

#[test]
fn bare_var_assign_requires_move() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "bare var RHS must error: {errs:?}"
    );
}

#[test]
fn move_of_regional_capture_errors() {
    let src = r#"
fun main() {
    var a: String = "A"
    move (a) {
        var t = move a
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("capture") || e.message.contains("already")),
        "second move of capture must error: {errs:?}"
    );
}

#[test]
fn regional_capture_use_without_inner_move_ok() {
    let src = r#"
fun main() {
    var a: String = "A"
    move (a) {
        val t = a
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}
```

- [ ] **Step 2: Run tests — expect fail**

Run: `cargo test --lib expr_move_rebinding_marks_source_moved bare_var_assign_requires_move -- --nocapture`

Expected: FAIL (Move not walked / no assign check).

- [ ] **Step 3: `from_capture` on env + bind helpers**

In `src/sema/env.rs`, add `from_capture: bool` to `EnvBinding` (default `false` in `bind`).

Extend `bind` or add:

```rust
pub(super) fn bind_with(
    az: &mut Analyzer,
    name: &str,
    arena_id: usize,
    arena_label: &str,
    ty: Ty,
    kind: BindingKind,
    from_capture: bool,
) -> Shadow { /* … */ }
```

In `region.rs` `bind_capture`, bind with `from_capture: true` (keep `BindingKind::Val` for captures as today, or preserve source kind — Val is fine for MVP).

- [ ] **Step 4: Walk `Expr::Move` + free_vars**

In `walk.rs`:

```rust
Expr::Move { name, span } => apply_expr_move(az, name, Some(*span), node),
```

```rust
fn apply_expr_move(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name).cloned() else {
        az.error(format!("cannot move unknown name `{name}`"), Some(name.into()), span);
        return;
    };
    if b.moved {
        az.error(/* already moved */, …);
        return;
    }
    if b.from_capture {
        az.error(
            format!("cannot move `{name}`: it was already moved into this region as a capture"),
            Some(name.into()),
            span,
        );
        return;
    }
    // Mark moved; record observation Moved if useful for dump
    if let Some(b) = az.env.get_mut(name) {
        b.moved = true;
    }
}
```

In `free_vars.rs` `collect_expr`, treat `Expr::Move { name, .. }` like Ident (add to free if not bound).

- [ ] **Step 5: VarDecl / Assign explicit-move gate**

Before/after walking the RHS of `VarDecl` / `Assign`:

```rust
fn check_transfer_rhs(az: &mut Analyzer, value: &Expr, dest_kind: BindingKind, span: Option<Span>) {
    let Expr::Ident { name, span: ispan } = value else { return };
    let Some(b) = az.env.get(name) else { return };
    if b.ty.is_copy() || b.moved { return; }
    let src_var = matches!(b.kind, BindingKind::Var);
    let dest_var = matches!(dest_kind, BindingKind::Var);
    if src_var || dest_var {
        az.error(
            format!("use `move {name}` to transfer ownership"),
            Some(name.clone()),
            Some(*ispan),
        );
    }
}
```

For `Assign`, treat dest as `Var`. Walk order: `check_transfer_rhs` then `walk_expr` (so `Move` still marks moved; bare Ident still `note_use`).

- [ ] **Step 6: Run sema tests — expect pass**

Run: `cargo test --lib expr_move_rebinding bare_var_assign move_of_regional regional_capture_use`

Expected: PASS. Also run full `cargo test --lib` and fix any match exhaustiveness on `Expr`.

- [ ] **Step 7: Commit**

```bash
git add src/sema/
git commit -m "$(cat <<'EOF'
feat(sema): expression move and explicit assign transfer

EOF
)"
```

---

### Task 3: Sema — call-site move + `val`/`var` formals

**Files:**
- Modify: `src/sema/env.rs` (signature map)
- Modify: `src/sema/analyze.rs`
- Modify: `src/sema/region.rs` (`RegionParam.kind`)
- Modify: `src/sema/walk.rs` (Call args)
- Modify: `src/sema/tests.rs`
- Test: `src/sema/tests.rs`

**Interfaces:**
- Consumes: `Param.kind`; `Analyzer.fun_sigs: HashMap<String, Vec<(BindingKind, Type)>>`
- Produces: call checks per design matrix; replace NYI tests `call_does_not_yet_*`

- [ ] **Step 1: Write failing call-site tests; replace NYI gaps**

```rust
#[test]
fn call_move_into_var_param_consumes() {
    let src = r#"
fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(move s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}

#[test]
fn call_bare_var_into_var_param_errors() {
    let src = r#"
fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "{errs:?}"
    );
}

#[test]
fn call_bare_val_into_val_param_ok() {
    let src = r#"
fun sink(s: String): Int { return 0 }
fun main() {
    val s: String = "hi"
    sink(s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}
```

Delete or rewrite `call_does_not_yet_move_non_copy_argument` and `call_does_not_yet_record_copy_crossing_into_callee` to match the new rules (val→val Shared/Local OK without consume; Copy args still OK).

- [ ] **Step 2: Run — expect fail**

Run: `cargo test --lib call_move_into_var_param call_bare_var_into_var_param -- --nocapture`

Expected: FAIL (no call formal checks).

- [ ] **Step 3: Signature map + RegionParam.kind**

```rust
// env.rs Analyzer
pub(super) fun_sigs: HashMap<String, Vec<(BindingKind, Type)>>,

// analyze.rs
pub fn analyze(program: &Program) -> … {
    let mut az = Analyzer::new();
    for f in &program.functions {
        az.fun_sigs.insert(
            f.name.clone(),
            f.params.iter().map(|p| (p.kind, p.ty.clone())).collect(),
        );
    }
    …
}

// RegionParam
pub(super) kind: BindingKind,

// bind_param uses param.kind instead of hard-coded Val
```

- [ ] **Step 4: Call argument checking**

In `walk_expr` for `Expr::Call`, after walking callee:

```rust
let formals: Option<Vec<(BindingKind, Type)>> = match callee.as_ref() {
    Expr::Ident { name, .. } => az.fun_sigs.get(name).cloned(),
    _ => None,
};
for (i, a) in args.iter().enumerate() {
    let formal = formals.as_ref().and_then(|f| f.get(i));
    check_call_arg(az, a, formal, node);
}
```

`check_call_arg` logic (non-Copy):

| Arg \ Formal | `val` / unknown | `var` |
|--------------|-----------------|-------|
| `move name` | `apply_expr_move` | `apply_expr_move` |
| bare Ident Copy | `note_use` | `note_use` |
| bare Ident `val` | `note_use` (Shared/Local) | error: need `move` |
| bare Ident `var` | error: need `move` | error: need `move` |
| other expr | `walk_expr` | if formal `var` and not Move → error after walk if needed |

Unknown callee: keep today’s `walk_expr` per arg (no new errors).

- [ ] **Step 5: Run all tests**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/sema/ tests/
git commit -m "$(cat <<'EOF'
feat(sema): call-site move and var params

EOF
)"
```

---

### Task 4: Handbook

**Files:**
- Modify: `docs/memory-model.md`

**Interfaces:**
- Consumes: shipped syntax from Tasks 1–3
- Produces: handbook leading with expression/call; regional secondary; remove “NYI” call/param rows

- [ ] **Step 1: Rewrite lead of “How to transfer ownership”**

Put this block first (before regional forms):

```markdown
## How to transfer ownership (`move`)

Everyday style — **expression** and **call-site** `move` (no extra braces):

```bork
var s: String = "xxx"
var x = move s

fun f(var a: String, var b: String) {
    // a and b owned here
}

fun main() {
    var x: String = "X"
    var y: String = "Y"
    f(move x, move y)
}
```

Rules:

- Non-Copy **`var`**: always write `move name` on assign RHS and call args.
- Bare `name: Type` params are **`val`**; use `var` when the callee should own.
- `f(x)` with non-Copy `var x` is an error.

Regional `move (a, b) { … }` remains for multi-capture / trailing closures. Names in the capture list are already Local inside — do **not** write `var t = move a` for those.
```

Keep subsections for explicit/empty/inferred regional forms; delete the “What is *not* a move (today)” rows that claimed call/params NYI (or rewrite to the new rules).

- [ ] **Step 2: Skim for contradictions**

Ensure no leftover “calls do not consume” / “params Val only”.

- [ ] **Step 3: Commit**

```bash
git add docs/memory-model.md
git commit -m "$(cat <<'EOF'
docs: handbook expression and call-site move

EOF
)"
```

---

### Task 5: Smoke + acceptance gate

**Files:** none new (verify only)

- [ ] **Step 1: Run full suite**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 2: Manual acceptance snippets**

```bash
cargo run --quiet -- /tmp/expr_move.bork --dump-arenas
```

With file contents covering: `var x = move s`; `f(move x)`; regional capture use; bare `sink(s)` error (check diagnostics via LSP or a tiny `analyze` unit already covered).

- [ ] **Step 3: Spec checklist**

Confirm each Acceptance bullet in the design spec has a passing test or handbook line. If any gap, add a test in `src/sema/tests.rs` and commit:

```bash
git add src/sema/tests.rs
git commit -m "$(cat <<'EOF'
test(sema): cover remaining expression-move acceptance

EOF
)"
```

---

## Spec coverage (self-review)

| Spec item | Task |
|-----------|------|
| `Expr::Move` / `move Ident` | 1 |
| Always explicit assign | 2 |
| Regional keep + no second move on capture | 2 |
| `val`/`var` params, bare = val | 1, 3 |
| Call matrix | 3 |
| Same-arena `var y = move x` | 2 |
| Loop outer rule | existing + Task 2 (moved flags) |
| Handbook lead expression/call | 4 |
| Non-goals (no desugar, no auto-move, no `move expr`) | Global Constraints |

No placeholders left in task steps. Types aligned: `Param.kind`, `Expr::Move`, `from_capture`, `fun_sigs`.
