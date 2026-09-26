# Rozdział 10. Gdy kompilator borks: diagnostyka i aborcja

## Ten rozdział obejmuje

- Cztery fazy i jeden format linii
- Co jest błędem kompilacji, a co śmiercią procesu
- Kolejność diagnostyk i to, że HIR znika
- Panikę kompilatora, która nie jest diagnostyką
- Czego język nie ma: wyjątków, `Result`, `try`

## Nie ma obsługi błędów w języku

Nie ma `throw`, `try`, `catch`, typu `Result`, typu `Option` poza
powierzchnią `T?`, ani makra `panic`. Program Bork albo przechodzi
kompilację, albo nie. W runtime są dokładnie trzy awarie, które kod
naprawdę wywołuje:

| Zdarzenie | Mechanizm | Co zobaczysz |
|---|---|---|
| Dzielenie przez zero, albo (w backendzie) `MIN / -1` | `abort` z libc | proces SIGABRT, kod powłoki 134, pusty stderr |
| Indeks tablicy poza `N` | ten sam `abort` | j.w. |
| Alokacja ponad 4096 bajtów płyty | `panic!` w `bork_runtime` | komunikat Rust `arena overflow: need … bytes, capacity 4096` |

`T?` nie jest błędem. Jest wartością, którą frontend umie sprawdzić, a
codegen umie odrzucić. Nie myl nullable z modelem błędów.

> **NOTE.** Składniowo nie zapiszesz literału ujemnego, więc ścieżki
> `MIN / -1` nie odpalisz literałem `-1`. Strażnik w `guard_int_div` i tak
> jest: porównuje dzielnik z zerem i, dla typów ze znakiem, parę
> (minimum, minus jeden).

## Format

`src/main.rs`, funkcja `print_diagnostics`:

```text
ścieżka:linia:kolumna: error: faza: treść
```

albo, gdy `span` jest `None`:

```text
ścieżka: error: faza: treść
```

Fazy to `parse`, `ownership`, `type`, `codegen`. `Severity` ma wariant
`Warning`, ale ścieżki, które książka przeszła, produkują `Error`. CLI i
tak drukuje słowo `error`.

Kolumna jest liczbą znaków (`chars`) od ostatniego `\n`, plus jeden. Nie
jest indeksem bajtu. Span w kompilatorze jest bajtowy. Przy ASCII, czyli
przy całym dzisiejszym lekserze, to jest to samo.

LSP dokłada prefiks fazy drugi raz w treści (`"parse: …"`) i przelicza
bajty na pozycje UTF-16, których wymaga protokół. Rozdział 18.

## Faza `parse`

Źródła komunikatów: `diag::from_parse`.

| Wariant LALRPOP | Tekst |
|---|---|
| `InvalidToken` | `invalid token` |
| `UnrecognizedEof` | `unexpected end of file; expected …` |
| `UnrecognizedToken` | `` unexpected token `…`; expected … `` |
| `ExtraToken` | `` unexpected extra token `…` `` |
| `User` | tekst z gramatyki, span na końcu pliku |

**Listing 10.1.** Uruchomione.

```text
err_parse.bork:1:10: error: parse: unexpected end of file; expected r#"[a-zA-Z_][a-zA-Z0-9_]*"#, ")", "val", "var"
```

dla `fun oops(`. Lista `expected` jest surowa, z regexami leksera. Przy
błędzie parsowania `CheckResult.report` i `hir` są `None`. `--dump-arenas`
nie drukuje drzewa.

Błąd użytkownika gramatyki, na przykład zły cel przypisania, nie wskazuje
miejsca tokenu. Wskazuje EOF. To warto wiedzieć, zanim zaczniesz szukać
błędu w ostatniej linii.

## Faza `type`

Komunikaty z `src/typeck`. Rozdziały 4–7 cytowały te, które uruchomiliśmy.
Pełniejszy katalog jest w dodatku A. Kilka, które warto znać z nazwy, bo
psują całe funkcje, nie pojedyncze wyrażenia:

- `function must return a value of type … on all paths`,
- `cannot redefine builtin function …`,
- `unknown binding …`,
- `unknown function …`,
- `function … expects N arguments, got M`.

Typeck nie zatrzymuje się na pierwszym błędzie. Zbiera wektor. Potem
frontend i tak uruchamia semę. Dlatego jeden plik potrafi mieć i `type`, i
`ownership`. Test `reports_ownership_and_type_together` używa `1 + "x"`
razem z użyciem nazwy po `move`.

## Faza `ownership`

Dwa autorzy: `diag::from_sema` oraz `escape::check_function`. Escape nie
idzie, gdy wcześniejsze diagnostyki są niepuste. Skutek praktyczny: przy
błędzie typu możesz nie zobaczyć błędu escape, który w czystym programie
by się pojawił. Najpierw napraw typy i gołe `move`, potem czytaj escape.

Sema przy błędzie nadal oddaje `ArenaReport`. Dump przy
`err_use_after` pokazuje `s [Moved ← fun main]` oraz lokalne `u`, mimo że
program jest zły. Drzewo jest przybliżeniem tego, co spacer zdążył
zbudować, nie certyfikatem poprawności.

## Faza `codegen`

Pojawia się tylko na `bork build`. Checker jej nie produkuje. Bramka
(`gate`) odrzuca konstrukcje, zanim powstanie moduł LLVM:

- `trailing closures are not supported by codegen`,
- `` `None` is not supported by codegen ``,
- `` `Some` is not supported by codegen ``,
- `` `!!` is not supported by codegen ``,
- `` `?:` is not supported by codegen ``,
- `field access is not supported by codegen` (wszystko poza `.length` na
  napisie lub tablicy),
- `this binary operator is not supported by codegen`.

Potem emisja dokłada między innymi:

- `` `fun main` is required to build an executable ``,
- `` `main` returning `f64` is not supported by codegen yet `` (analogicznie `i64`, `bool`),
- `indirect calls is not supported by codegen yet`,
- `float binary after walk is not supported by codegen yet`.

Ostatni tekst potrafi przyjść owinięty: `internal arena schedule mismatch: …`.
`resolve_walk_failure` dokleja ten prefiks, gdy błąd spaceru nie niesie
osobnej diagnostyki albo gdy niesie ją w treści `WalkError`. To jest
szorstkie. Treść po dwukropku jest tą, która mówi, czego brakuje.

Błąd codegen ma kod wyjścia 1 i **nie** zostawia binarki (testy
`gate_rejection_exits_one_without_binary` i brak `main`).

## Panic kompilatora

To nie jest diagnostyka. Proces `bork` umiera z kodem 101 i śladem Rusta.
Sprawdzone przypadki:

| Program | Miejsce |
|---|---|
| argument `String` do funkcji użytkownika | `expr.rs` `value_as_int` / `into_int_value` |
| porównanie `f64` na ścieżce walk | ten sam `into_int_value`, wartość jest `FloatValue` |

Przyczyna w kodzie: `coerce_value_to_ty` dla wszystkiego, co nie jest
`bool` ani floatem (a float i tak zwraca błąd „not supported” tylko na
części ścieżek), woła `value.into_int_value()`. Inkwell na złym wariancie
**panikuje**, zamiast zwrócić `Err`. Nie ma `match` na `BasicValueEnum`.

> **WARNING.** Panic kompilatora na poprawnym według frontendu programie
> jest błędem projektu backendu. Nie obchodź jej „bo język zabrania”.
> Język, w `typeck` i semie, tego nie zabrania. Dodatek C trzyma to przy
> innych długach.

Błąd narzędzi, nie programu, kończy się kodem 2: zły flag, brak pliku,
nakładające się ścieżki, brak `clang` albo brak feature `codegen`. Tekst
zaczyna się od `error:` bez fazy.

## Jak czytać kilka błędów naraz

Frontend nie implementuje severity „pierwszy błąd wygrywa”. Dostajesz
wszystkie. Heurystyka, która oszczędza czas:

1. Najpierw `parse`. Przy błędzie składni reszty nie ma.
2. Potem `type` o niezgodności podstawowej (`expected`, `unknown`).
3. Potem `ownership` o `move`.
4. `codegen` tylko gdy check jest czysty albo gdy build i tak doszedł do bramki. Build woła `frontend::check` i przy błędach frontendu w ogóle nie wchodzi w LLVM; wypisuje te diagnostyki i kończy się 1.

Sprawdzone na `bork build` programu z `None`: faza `codegen`, kod 1, pliku
wyjściowego nie ma. Program z błędem `ownership` na `return` z wewnętrznego
regionu też kończy build kodem 1, z tekstem `inner region`, bez binarki.
Escape jest częścią checku, więc build nawet nie udaje, że to problem LLVM.

## Podsumowanie

- Język nie ma wyjątków. Ma diagnostykę kompilacji i trzy awarie runtime.
- Linia diagnostyki niesie fazę. Pusty span pomija numer linii.
- `ownership` to sema albo escape. Escape milczy, gdy wcześniej są inne błędy.
- Bramka codegen mówi wprost `not supported`. Część emisji mówi `not supported yet`.
- Przekazanie `String` do funkcji użytkownika nie daje diagnostyki. Zabija kompilator.
- Kod 1 to zły program. Kod 2 to złe wywołanie albo brak narzędzia. Kod 101 to panic Rusta.
