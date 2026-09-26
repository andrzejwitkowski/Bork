# Wyniki uruchomień

Kompilator: `cargo build --features codegen --bin bork --no-default-features`
przy rustc 1.98.1 i LLVM 23.1.2. `check` to `bork plik.bork` (kod 0 albo 1).
`build` to `bork build`, potem uruchomienie binarki, jeśli powstała.

| Plik | check | build / proces |
|---|---|---|
| `01-hello.bork` | 0 | stdout `hi\n`, kod 0 |
| `02-add.bork` | 0 | kod 42 |
| `03-forsum.bork` | 0 | kod 10 |
| `04-while-break.bork` | 0 | kod 3 |
| `05-continue.bork` | 0 | kod 8 |
| `06-if-expr.bork` | 0 | kod 42 |
| `07-logic.bork` | 0 | kod 0 |
| `08-arrays.bork` | 0 | kod 12 |
| `09-slice-print.bork` | 0 | stdout `20\n2\n` |
| `10-shared.bork` | 0 | stdout `x\n` |
| `11-move-print.bork` | 0 | stdout `ab\n42\n` |
| `12-concat-val.bork` | 0 | stdout `LR\n` |
| `13-concat-move.bork` | 0 | stdout `LR\n` |
| `14-promote.bork` | 0 | stdout `temp\n` |
| `15-assign-up.bork` | 0 | stdout `b\n` |
| `16-return-local.bork` | 0 | stdout `hi\n` |
| `17-escapes.bork` | 0 | stdout `a`, NL, `b`, tabulacja, `"c"`, NL |
| `18-trace.bork` | 0 | stdout `sum\n`, kod 3 |
| `19-regional-move.bork` | 0 | stdout `A\nB\n` |
| `20-div-zero.bork` | 0 | build 0, proces SIGABRT (134) |
| `21-index-oob.bork` | 0 | build 0, proces SIGABRT (134) |
| `22-closure.bork` | 0 | build 1, `trailing closures are not supported by codegen` |
| `23-none.bork` | 0 (z adnotacją w `nullable`) | build 1, `` `None` is not supported by codegen `` |
| `24-main-i64.bork` | 0 | build 1, `` `main` returning `i64` `` |
| `25-no-main.bork` | 0 | build 1, `` `fun main` is required `` |
| `26-pass-string.bork` | 0 | panic kompilatora, kod 101, `into_int_value` |

Pliki błędów checkera są w treści rozdziałów; nie dubluję tu źródeł, które
mają kończyć się kodem 1, poza tymi, które łatwo pomylić z sukcesem buildu.
