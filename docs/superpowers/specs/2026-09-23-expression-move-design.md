# Bork — Expression `move`, call-site move, and `val`/`var` params

## Goal

Make everyday ownership transfer look like linear rebinding and call arguments, not nested brace regions:

```bork
var s: String = "xxx"
var x = move s
fun f(var a: String, var b: String) { /* a, b owned here */ }
f(move x, move y)
```

Brace `move` stays as the **regional** form. Capture lists already transfer ownership — no second `move` inside for those names.

## Decisions

| Topic | Choice |
|-------|--------|
| Non-Copy `var` at assign / call | **Always explicit `move`** — bare `var y = x` or `f(x)` is an error |
| Brace `move (…​) { }` / trailing | **Keep** as regional form (multi-capture, closures) |
| Names in capture list | Already **Local** in the child; no `var t = move a` needed inside |
| Param mutability / ownership | Grammar: `val` \| `var` before param name; bare `name: Type` = **`val`** (compat) |
| `move` expression operand | Simple name for MVP (`move x`); general expr later if needed |
| Call without `move` | Copy args OK; Shared if formal is `val` and actual is parent `val` non-Copy; `var` formal requires `move` actual |
| Same-arena rebinding | `var y = move x` marks `x` Moved; `y` is Local in the current arena (no new region) |
| Loop rule | Unchanged: cannot move an **outer** binding inside a loop body |

## Two mechanisms (do not unify by desugar)

### 1. Expression `move` (primary ergonomics)

```text
Expr ::= … | "move" Ident
```

- Evaluates to the value of `Ident`, marks that binding **Moved** in the current env.
- Legal contexts (MVP): RHS of `val`/`var` init, and call arguments.
- Does **not** open an arena by itself.
- Use-after-move of the source name is an error (same as today).

```bork
var s: String = "hi"
var x = move s      // s Moved; x Local here
// val z = s        → error: use after move
f(move x)           // x Moved; callee receives ownership if formal is `var`
```

### 2. Regional `move` (existing)

```bork
move (a, b) {
    // a, b are Local (captured) — already moved from parent
    val t = a       // OK: use, not a second move
    // var u = move a  → error: already Local / or redundant; prefer forbid move of Local that is not escaping
}
```

Trailing closures unchanged: `action() move (acc) { x -> … }` / `move { … }` (inferred).

**Inside a regional move:** reading a capture is ordinary use. Writing `move a` when `a` is already a Local capture of this region is an error (nothing left to transfer from a parent). Moving a *different* outer name via expression `move` inside the region is allowed subject to the loop rule and nest rules.

## Parameters

```text
Param ::= ("val" | "var")? Ident ":" Type
```

| Formal | Meaning |
|--------|---------|
| `val x: T` or `x: T` | Callee observes; non-Copy caller `val` may **Share**; caller `var` still needs `move` into a `var` formal (or error if passed bare into `val` formal — see below) |
| `var x: T` | Callee **owns** (and may mutate). Caller must pass `move name` for non-Copy. |

### Call-site matrix (non-Copy)

| Actual \ Formal | `val` param | `var` param |
|-----------------|-------------|-------------|
| bare `v` (`val`) | Shared (if nested/live) | Error — need `move v` |
| bare `v` (`var`) | Error — need `move v` (explicit always) | Error — need `move v` |
| `move v` | Allowed (consume into callee; rare for `val`) | Allowed — ownership transfer |
| Copy type | Always OK without `move` | Always OK without `move` |

Locked rule from product: **always explicit `move` for non-Copy `var` sources** at assign and call. Passing a `val` into a `var` formal also requires `move` (consume the val).

## What changes in the compiler

1. **Parser** — `move Ident` as expression; optional `val`/`var` on params (reject unknown forms already covered by tests flipping to allow).
2. **AST** — `Expr::Move { name, span }`; `Param { kind: Val \| Var, … }` (default Val).
3. **Sema**
   - Walk `Expr::Move`: resolve name, apply move policy, yield type; mark Moved.
   - Assign: if RHS is not `move` and both sides non-Copy `var` path would transfer — error (“use `move`”).
   - Call: for each arg, if formal is `var` (or non-Copy consume) require `Expr::Move` (or Copy); mark caller binding Moved when `move` used.
   - Regional captures: unchanged Local binding; expression `move` of a capture name in the same region = error.
4. **Dump / LSP** — Moved from expression/call shows like today’s Moved; hover notes `move` at use site when useful.
5. **Handbook** — lead with expression/call examples; regional section secondary; remove “NYI” rows for call/params once implemented.

## Non-goals (this spec)

- Desugaring regional `move` into expression moves
- Removing brace / trailing `move`
- Auto-move / implicit consume of bare `var`
- `move` of arbitrary expressions (`move foo()`)
- LLVM / runtime relocation (still compile-time ownership only)

## Acceptance

- `var x = move s` analyzes; `s` unusable after; dump shows `x` Local, `s` Moved.
- `f(move x, move y)` with `fun f(var a: T, var b: T)` marks `x`,`y` Moved; formals Local/`var` in callee.
- `f(x)` with non-Copy `var x` errors asking for `move`.
- `move (a) { val t = a }` still OK without inner `move`.
- `move (a) { var t = move a }` errors (already local capture).
- Existing regional / inferred / empty-list / loop-outer tests still pass.
- Handbook documents expression form first.

## Pipeline order

1. Grammar + AST + parser tests  
2. Sema expression `move` + assign  
3. Sema call + `val`/`var` params  
4. Handbook update  
5. Dump/LSP polish if needed  
