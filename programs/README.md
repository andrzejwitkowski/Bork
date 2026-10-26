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
| [`classes/`](build/classes/) | `class`, constructor calls, nested fields, mutable fields, `Ref` navigation |
| [`methods/`](build/methods/) | Class methods, `this`, implicit members, receivers |
| [`loops/`](build/loops/) | `for (i in lo..hi)`, `while`, `break`, accumulation |
| [`recursions/`](build/recursions/) | Direct recursion, calls inside `if` |
| [`borrow/`](build/borrow/) | `&T` parameters, index/slice, re-borrow in `if`/`while`/`for` |
| [`move_and_promote/`](build/move_and_promote/) | `move`, `move (…) { }`, `promote`, `concat`, assign-up |
| [`basics/`](build/basics/) | Smoke: `return`, `println`, arrays, `i64` blocks |
| [`constants/`](build/constants/) | File-level `const`, shadowing in functions and methods |
| [`vec/`](build/vec/) | Growable `Vec<T>`: push/pop/get/set/len/clear, doubling across pages, `String` and `Ref<T>` elements, `promote` between arenas, exact `strongCount` / `weakCount` across older and newer arenas |
| [`enums/`](build/enums/) | `enum` construction and `when`: every variant shape, mixed-size payloads, `Ref` payload counts, generic enums, literal/nested patterns, an expression evaluator, a linked list |
| [`option/`](build/option/) | prelude `Option` / `Result`: `Some` / `None` by expected type, `when` over them, conversions to and from `T?` and `Ref<T>`, a user `Option` shadowing the prelude |
| [`text/`](build/text/) | `String` bytes and content equality, `stringFromUtf8` (copy, `None`, sinks), a small HTTP-style example |

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
