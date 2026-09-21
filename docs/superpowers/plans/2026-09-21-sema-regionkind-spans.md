# Sema RegionKind, Single Visitor, Error Spans — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the three thermo follow-ups: replace `open_region` bool flags with `RegionKind`, eliminate the redundant free-var pre-scan where walk already covers it (single visitor for inferred-move collect), and give semantic diagnostics real identifier ranges instead of first-`str::find`.

**Architecture:** `RegionKind` encodes move vs check-cross policy. A small AST visitor supports `CollectFree` (only for inferred `move`) and the existing analyze walk. Parser attaches `Span` to `Expr::Ident` and binding/capture name sites that sema can put on `SemaError`; LSP maps that span to a diagnostic range.

**Tech Stack:** Existing LALRPOP (`@L`/`@R`), `sema.rs`, `lsp.rs`, parser tests.

## Global Constraints

- Behavior of Copy/Move/collapse/`--dump-arenas` unchanged except better diagnostic locations
- No full AST spanning of every node — only names that diagnostics need
- No new dependencies
- Keep `sema.rs` under ~1k lines (tests included); extract `src/span.rs` if spans clutter AST

---

## File map

| File | Change |
|------|--------|
| [src/span.rs](src/span.rs) (create) | `Span { start: usize, end: usize }` |
| [src/ast.rs](src/ast.rs) | `Expr::Ident { name, span }`; spanned names on `VarDecl` / `MoveBlock` captures / `Closure` captures+params as needed |
| [src/parser.lalrpop](src/parser.lalrpop) | Spanned `Ident` helper; wire into Ident exprs and capture lists |
| [src/sema.rs](src/sema.rs) | `RegionKind`; drop bool pair; visitor collect; `SemaError.span` |
| [src/lsp.rs](src/lsp.rs) | Use `error.span` → `byte_range`; delete first-`find` heuristic (keep as fallback only if span missing) |
| tests | RegionKind call sites compile; free-var only for inferred move; LSP range hits the use site under shadows |

---

### Task 1: `RegionKind` replaces bools

**Files:** Modify [src/sema.rs](src/sema.rs)

**Interfaces:**
- Produces:
```rust
enum RegionKind {
    /// Function / for / bare block / if / non-move closure: walk notes cross-arena uses.
    Checked { move_: bool }, // wait — cleaner:

    Ordinary,   // check cross-arena via walk (block, if, for, non-move closure, fun body)
    Move,       // capture + no cross-check beyond moved binding rules
}
```
- Prefer:
```rust
enum RegionKind {
    Ordinary, // check_cross via walk; not a move region
    Move,     // is_move; infer/explicit captures; no free pre-check loop
}
```
- `open_region(..., kind: RegionKind)` — label still passed separately (`"Block"`, `"Closure"`, …)
- `kind == Move` ⇒ append `" (move)"` to label; resolve captures; skip the old `if check_cross_arena { for name in free }` loop entirely (walk covers it for `Ordinary`)

- [ ] **Step 1: Write a compile-breaking rename test / fix call sites**

Replace every `open_region(..., false, true)` with `RegionKind::Ordinary` and `(..., true, false)` with `RegionKind::Move`. Function entry uses `Ordinary`.

- [ ] **Step 2: Implement enum + match in `open_region`**

```rust
enum RegionKind {
    Ordinary,
    Move,
}

fn open_region(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    explicit_captures: &[String], // later SpannedName — Task 3
    params: &[(String, Option<Type>)],
    kind: RegionKind,
) -> ArenaNode {
    let is_move = matches!(kind, RegionKind::Move);
    // ...
    let captures = if is_move { resolve_captures(...) } else { vec![] };
    // DELETE the check_cross_arena free-var note_use loop
    walk_block(...);
    // ...
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib sema --quiet`  
Expected: PASS (same ownership behavior; cross-arena errors still fire from `walk`/`note_use`)

- [ ] **Step 4: Commit**

```bash
git add src/sema.rs
git commit -m "$(cat <<'EOF'
refactor(sema): replace open_region bools with RegionKind.

EOF
)"
```

---

### Task 2: Single visitor — free vars only for inferred move

**Files:** Modify [src/sema.rs](src/sema.rs)

**Problem today:** `free_vars_in_block` + `collect_*` duplicate the AST walk. The `check_cross_arena` pre-scan is redundant with `note_use` on `Expr::Ident`. Only **inferred** `move` (empty capture list) needs a free-var set before walk.

**Interfaces:**
- Produces: `fn free_vars_in_block(block: &Block) -> HashSet<String>` kept but **only** called from `resolve_captures` when `explicit_captures.is_empty()`
- Delete any other call sites
- Optionally rename collect helpers to `visit_free` and keep them private next to `resolve_captures` (not a second public analysis path)

**Code judo (locked):** Do **not** invent a dual-mode Analyze/Collect enum for the whole analyzer unless collect stays >~80 lines after Task 1. Prefer: Task 1 deletes the redundant pre-check; Task 2 gates free-var collect behind inferred-move only and adds a unit test that inferred `move { use x }` still captures `x`.

- [ ] **Step 1: Failing test for inferred move capture**

```rust
#[test]
fn inferred_move_captures_free_parent() {
    let src = r#"
fun main() {
    val s: String = "hi"
    move {
        val t = s
    }
    val u = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.iter().any(|e| e.message.contains("after move")), "{errs:?}");
}
```

- [ ] **Step 2: Run test — should already PASS if inferred move works; if FAIL, fix `resolve_captures`**

Run: `cargo test inferred_move_captures_free_parent -- --nocapture`

- [ ] **Step 3: Ensure `free_vars_in_block` has exactly one call site**

```bash
rg -n "free_vars_in_block" src/sema.rs
```

Expected: definition + one call inside move-capture resolution.

- [ ] **Step 4: Run full sema + parser tests**

Run: `cargo test --quiet`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/sema.rs
git commit -m "$(cat <<'EOF'
refactor(sema): collect free vars only for inferred move captures.

EOF
)"
```

---

### Task 3: Diagnostic spans for identifier use/decl sites

**Files:** Create [src/span.rs](src/span.rs); Modify [src/ast.rs](src/ast.rs), [src/parser.lalrpop](src/parser.lalrpop), [src/sema.rs](src/sema.rs), [src/lsp.rs](src/lsp.rs), [src/lib.rs](src/lib.rs), tests

**Locked approach (minimal spans, not full AST):**

```rust
// span.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
```

AST changes (minimal set that fixes shadows):

```rust
// Expr::Ident becomes structured
Ident { name: String, span: Span },

// VarDecl keeps name: String but add name_span: Span
// MoveBlock captures: Vec<SpannedName> where SpannedName { name, span }
// Closure params/captures: Vec<SpannedName> (or parallel Vec<Span> — prefer SpannedName)
```

Update all `Expr::Ident(name)` matches to `Expr::Ident { name, .. }` / `Expr::Ident { name, span }`.

**Parser:** LALRPOP location markers:

```rust
SpannedIdent: (Span, String) = {
    <start:@L> <name:Ident> <end:@R> => (Span { start, end }, name),
};
```

Use `SpannedIdent` for atom idents, move captures, closure params/captures, `VarDecl` names. Assignments `name =` should also be spanned so use-after-move on assign LHS is accurate.

**Sema:**

```rust
pub struct SemaError {
    pub message: String,
    pub name: Option<String>,
    pub span: Option<Span>,
}

fn error(&mut self, message: impl Into<String>, name: Option<String>, span: Option<Span>) { ... }
```

`note_use` takes the span from `Expr::Ident` / assign LHS and passes it into `error`.

**LSP:**

```rust
fn sema_error_to_diagnostic(source: &str, error: &SemaError) -> Diagnostic {
    let range = error
        .span
        .map(|s| byte_range(source, s.start, s.end))
        .unwrap_or_else(|| byte_range(source, 0, 0));
    // ...
}
```

Delete `find_name_range` (or keep private behind `#[cfg(test)]` only if a test needs it — prefer delete).

- [ ] **Step 1: Add `span.rs` + `Expr::Ident { name, span }` with dummy `Span {0,0}` in any hand-built AST in tests — fix compile**

- [ ] **Step 2: Wire `SpannedIdent` in grammar for Idents / captures / decls / assign LHS**

- [ ] **Step 3: Failing LSP/sema location test**

```rust
#[test]
fn sema_diagnostic_points_at_use_not_decl() {
    let source = "fun main() {\n    val s: String = \"a\"\n    val s2: String = \"b\"\n    {\n        val t = s\n    }\n}\n";
    let diags = diagnostics_for_source(source);
    let d = diags.iter().find(|d| d.message.contains("not Copy")).expect("err");
    // use of `s` is on the line with `val t = s` (0-based line index)
    assert_eq!(d.range.start.line, 4, "{d:?}");
}
```

- [ ] **Step 4: Run test — expect FAIL (points at decl or line 0)**

- [ ] **Step 5: Thread spans through `note_use` / move errors; fix until PASS**

- [ ] **Step 6: `cargo test --quiet` full suite**

- [ ] **Step 7: Commit**

```bash
git add src/span.rs src/ast.rs src/parser.lalrpop src/sema.rs src/lsp.rs src/lib.rs tests
git commit -m "$(cat <<'EOF'
fix(lsp): attach parse spans to semantic errors for accurate ranges.

EOF
)"
```

---

## Self-review

| Thermo item | Task |
|-------------|------|
| 1 RegionKind | Task 1 |
| 2 Single visitor / no duplicate free scan | Task 1 deletes cross-check scan; Task 2 gates free collect to inferred move |
| 3 LSP ranges under shadows | Task 3 spans on idents + `SemaError.span` |

**Out of scope:** Spans on every expr/stmt; incremental reparse; renaming `Ownership::Copy` dump behavior.

---

## Execution handoff

Plan saved to `docs/superpowers/plans/2026-09-21-sema-regionkind-spans.md`.

**1. Subagent-Driven (recommended)** — fresh subagent per task  
**2. Inline Execution** — this session with checkpoints  

Which approach?
