# Bork program corpus

Runnable `.bork` sources grouped by language feature. They complement `tests/*.rs` and are meant as readable examples.

## Suites (top level)

| Directory | Meaning |
|-----------|---------|
| [`build/`](build/) | `bork build` + run; use `// exit:` and optional `// stdout:` |
| [`check/`](check/) | `bork <file>` must succeed (frontend only) |
| [`check_fail/`](check_fail/) | Frontend must reject (`// diag:` substring in stderr) |
| [`build_fail/`](build_fail/) | Frontend OK, `bork build` must fail |

## Topics (under each suite)

| Topic | Covers |
|-------|--------|
| [`conditionals/`](build/conditionals/) | `if` / `else`, expression `if`, `&&` `\|\|` short-circuit |
| [`loops/`](build/loops/) | `for (i in lo..hi)`, `while`, `break`, accumulation |
| [`recursions/`](build/recursions/) | Direct recursion, calls inside `if` |
| [`borrow/`](build/borrow/) | `&T` parameters, index/slice, re-borrow in `if`/`while`/`for` |
| [`move_and_promote/`](build/move_and_promote/) | `move`, `move (…) { }`, `promote`, `concat`, assign-up |
| [`basics/`](build/basics/) | Smoke: `return`, `println`, arrays, `i64` blocks |

The same topic names exist under `check/` and `check_fail/` where it makes sense.

## Directives

```bork
// exit: 42
// stdout: line\n
// diag: substring expected in stderr (fail suites)
```

## Run

```bash
cargo test programs_corpus_check          # check + check_fail (no LLVM)
cargo test programs_corpus --features codegen   # full corpus
```

CI runs both on every push and pull request (see `.github/workflows/ci.yml`).

`check` / `check_fail` run without codegen; `build` / `build_fail` need `--features codegen`.
