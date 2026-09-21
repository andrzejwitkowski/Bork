# Task 4 Report: Full LALRPOP Grammar

## Status

GREEN. The built-in-lexer LALRPOP grammar parses the MVP sample and preserves
the empty-program behavior.

## Implementation

- Replaced the parser stub with grammar for functions, parameters, named and
  function types, nullable types, blocks, declarations, assignment, loops,
  returns, expressions, calls, required-arrow trailing closures, and `if/else`.
- Implemented operator precedence and maps `..` to `BinOp::RangeTo`.
- Omitted function return types become non-nullable `Unit`, per the conflict
  resolution supplied after the brief.
- Used the brief's quoted-keyword/regex-terminal fallback because LALRPOP 0.23
  rejected overlapping named keyword and identifier regex terminals.
- Kept newlines as statement separators. Skipping all whitespace makes the
  brief's `Stmt*` grammar intrinsically ambiguous (for example, `f (x)` can be
  either one call statement or two expression statements).
- Added MVP assertions for the default `Unit` return, function-typed parameter,
  and `for` statement.

## TDD Evidence

RED/baseline: `src/parser.lalrpop` was the supplied stub, which only accepted
empty input and could not parse `MVP_SAMPLE`.

GREEN smoke test:

```text
cargo test parses_mvp_sample -- --nocapture
test tests::parses_mvp_sample ... ok
test result: ok. 1 passed; 0 failed
```

GREEN full suite after final changes:

```text
cargo test
test tests::empty_input_parses ... ok
test tests::parses_mvp_sample ... ok
test result: ok. 2 passed; 0 failed
Doc-tests bork: 0 passed; 0 failed
```

## Self-review

- `cargo build`, the required smoke test, full tests, and `git diff --check`
  succeeded.
- Confirmed no `null` keyword, trailing closures require `->`, nested bare
  blocks remain AST nodes, and binding kinds remain `Val`/`Var`.
- The parser treats a trailing `?` on a function type as function nullability;
  this removes the ambiguity in the brief's recursive type production.

## Commit

`7c029b9 Implement LALRPOP grammar for MVP 0.1 Bork syntax.`

## Important Review Fixes

- Removed the `NL` token and now skip `r"\s+"`, so newlines have no syntactic
  significance.
- Structured block bodies around non-final declarations/assignments/loops and
  final return/expression statements. This avoids inventing separators while
  choosing the longest expression for otherwise ambiguous adjacency such as
  `f (x)`.
- Function-type returns now parse as `Type`, so `() -> Int?` makes the named
  return type nullable rather than the function type.
- Added regressions for compact `val`/`var` statements and nullable function
  return types.

Review verification:

```text
cargo test
running 4 tests
test tests::empty_input_parses ... ok
test tests::nullable_function_return_belongs_to_return_type ... ok
test tests::parses_statements_without_newline_separators ... ok
test tests::parses_mvp_sample ... ok
test result: ok. 4 passed; 0 failed
src/main.rs: 0 passed; 0 failed
Doc-tests bork: 0 passed; 0 failed
```

Remaining grammar constraint: without explicit separators, adjacent arbitrary
expression statements are intrinsically ambiguous. Expression statements and
bare blocks are therefore accepted as sole/final block statements; structured
statements and a final `return` can still be sequenced without newlines.

## Important Task 4 fix: nullable function types

- Moved `?` off named-only `TypePrimary` onto `Type`/`TypeParam`: named types
  use `Ident?`; function returns stay nullable via `TypeRet` (`() -> Int?`).
- Nullable function types use parenthesized form plus fused `)?` terminal
  (e.g. `((Int) -> Int)?`) to avoid LR ambiguity with return-nullability.
- Added `nullable_function_type_as_param` regression.

Verification:

```text
cargo test
running 5 tests
test tests::nullable_function_type_as_param ... ok
test tests::nullable_function_return_belongs_to_return_type ... ok
test result: ok. 5 passed; 0 failed
```

Commit: `33dec3c Allow nullable function types via parenthesized )? syntax.`

## Remaining Important Findings (2026-09-21)

Status: **BLOCKED** on restoring the brief's literal `Stmt*`.

Completed nullable-type fixes:

- Removed the fused `)?` lexer token; `)` and `?` are independent tokens, so
  whitespace before function nullability is insignificant.
- Right-factored parenthesized types so nullable function types compose in
  parameter lists and return types while `() -> Int?` still applies `?` to
  `Int`.
- Replaced the `(Int)?` `unreachable!()` with a fallible grammar action that
  returns a clean `ParseError::User`.

The direct brief grammar, `Block = "{" <Stmt*> "}"`, is not LR(1) for this
separator-free syntax. LALRPOP's first local ambiguity is:

```text
observed: "return" ExprCall
lookahead: "("
reduce: return f, followed by parenthesized expression statement (x)
shift:  return f(x)
```

It also reports the corresponding assignment/expression conflict after
`Ident` with lookahead `=`, and block/trailing-closure conflicts with lookahead
`{`. LALRPOP 0.23 documents `#[precedence]`/`#[assoc]` as expression-grammar
rewrites, not a general shift preference, so those attributes cannot resolve
the statement-boundary ambiguity. The prior final-expression compromise was
restored only to leave the branch buildable; finding 4 remains unresolved and
is not claimed as fixed.

Verification of completed fixes:

```text
cargo test
running 8 tests
test result: ok. 8 passed; 0 failed
```
Controller decision 2026-09-21T10:27:36+02:00: Block bodies use option A — bare Expr/Block stmts only in final position (separator-free LALR).

## Refined Option A follow-up

The conceptual `NonFinalStmt* FinalStmt?` grammar remains ambiguous. For
`{ val x = f (y) }`, LALRPOP cannot choose between a single declaration whose
value is `f(y)` and a declaration whose value is `f` followed by final `(y)`.
It reports the corresponding reduce/shift conflict at `ExprCall` followed by
`(`. A final bare block after a non-final statement similarly needs more than
one token of lookahead to distinguish it from a required-arrow trailing
closure. The attempted grammar and regression tests were reverted, leaving
the worktree buildable pending a disambiguation rule.

## Option C: newline-sensitive statements

Status: **GREEN**.

- The lexer emits `NL` for line feeds and skips only horizontal whitespace and
  comments.
- Programs and blocks use newline-separated function/statement lists. The
  A-lite final-statement restrictions are removed.
- A parenthesis-aware layout pass makes line feeds insignificant inside
  parameter/type lists, conditions, for headers, and call arguments while
  preserving statement newlines in nested braces.
- `else` works on the same or following line; newline before `(` remains a
  statement boundary.
- Added regressions for all requested behavior; nullable-type tests stay green.

Verification:

```text
cargo test
running 14 tests
test result: ok. 14 passed; 0 failed

cargo clippy --all-targets -- -D warnings
Finished successfully

git diff --check
Passed
```
