# Wyniki historycznych uruchomień

Poniższa tabela opisuje przebieg sprzed synchronizacji z `main` (`cd8fee4`). Nie jest aktualnym wynikiem bieżącej gałęzi książki. Po zmianach w parserze, pożyczkach i codegenie trzeba ją ponownie wygenerować, zanim wyniki zostaną oznaczone jako aktualne. Korpus obecnej gałęzi jest w `programs/` i obsługiwany przez `tests/programs.rs`.

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
| `26-pass-string.bork` | 0 | historyczny snapshot: panic 101, `into_int_value`; ponowny build i wykonanie bieżącego listingu przechodzą, patrz weryfikacja poniżej |

## Ponowna weryfikacja na gałęzi z nullable

Na `feat/nullable-codegen` (`da21b65`) ponownie zbudowałem i uruchomiłem wszystkie sześć przykładów `programs/build/conditionals/nullable_*.bork`. Każde budowanie zakończyło się kodem 0, a standardowe wyjście i kody zakończenia zgadzały się z adnotacjami w plikach: pięć programów zakończyło się kodem 0, a `nullable_eq.bork` kodem 1. `nullable_f32.bork` z `val n: f32? = None` buduje się i kończy kodem 0.

Dodatkowy program sprawdził `Some`, `?:`, `!!` oraz `==`/`!=` dla `f32?` i `f64?`; sprawdzenie, budowanie i wykonanie zakończyły się kodem 0. To potwierdza nullable operacje, nie zwykłe porównania floatów — te nadal wywołują panikę codegenu. Wartości w tabeli wyżej pozostają historyczną migawką; nie są wynikami tej gałęzi.

Pliki błędów checkera są w treści rozdziałów; nie dubluję tu źródeł, które
mają kończyć się kodem 1, poza tymi, które łatwo pomylić z sukcesem buildu.
