# Rozdział 20. Jeden program przez cały kompilator

## Ten rozdział obejmuje

- Program śledzony, uruchomiony od źródła do kodu wyjścia
- Co widzi parser i jaki jest AST w zarysie
- Co typeck dopisuje do HIR
- Jakie drzewo aren wypisuje sema
- Dlaczego hoist i escape milczą
- Które funkcje LLVM i runtime naprawdę pracują

Program jest krótki celowo. Ma funkcję, pętlę, wywołanie, napis `val` i
blok, który ten napis tylko czyta. Wszystko to `bork build` obniża.
Nie ma w nim `String` przekazywanego do funkcji użytkownika, bo ta ścieżka
wywraca kompilator i ślad by się urwał paniką.

## Program

**Listing 20.1.** Uruchomione. Stdout `sum\n`. Kod wyjścia 3, bo `0+1+2`.

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main(): i32 {
    var total = 0
    for (i in 0..3) {
        total = add(total, i)
    }
    val label = "sum"
    {
        println(label)
    }
    return total
}
```

Zakres `0..3` jest prawostronnie otwarty. Trzy iteracje, nie cztery.

## Krok 1. Layout i parse

W pliku nie ma nawiasu, który łamałby linię, więc
`normalize_parenthesized_newlines` zwraca `None` i parser dostaje oryginał.
`ProgramParser` buduje dwie `Function`.

Dla `add` AST ma dwa `Param` z `BindingKind::Val` (nie napisano `val`, a
goła forma jest `val`), typ `i32`, ciało z jednym `Return` i `Binary Add`.

Dla `main`: `VarDecl` `total` bez adnotacji, `For` z `Binary RangeTo`,
`VarDecl` `label`, `Stmt::Block` z `Expr` wywołania `println`, `Return`.

Gdyby tu był średnik, `parse` zwróciłby `UnrecognizedToken` i `check`
skończyłby się jednym `Diagnostic` fazy `parse`, bez raportu. Nie ma.

Wejście: `bork::parse` w `src/lib.rs`. Gramatyka: produkcje `Function`,
`Stmt`, `ExprRange`, `Atom`.

## Krok 2. Typeck

`typeck::check` widzi dwie sygnatury: `add (i32, i32) -> i32`, `main`
`-> i32`. `println` jest wbudowane, nie koliduje.

`total` bez adnotacji, inicjalizator `0` bez oczekiwanego typu poza tym,
że literał całkowity domyślnie jest `i32`. Do `decl_tys` wpada `i32`.
`label` to `String`. `i` w `for` dostaje `i32`, bo granice zakresu są
`i32`. Warunek nie jest osobnym wyrażeniem bool; zakres jest osobnym
typem `Range`.

Wywołanie `add(total, i)`: callee jest nazwą, arność 2, oba argumenty
`i32`. `UseKind` odczytu `total` w ciele pętli wychodzi `Copy` (inny
region, typ Copy). `println(label)`: builtin, argument `String`, w HIR
`UseKind` odczytu `label` to `Shared`.

`return total` zgadza się z `i32`. Ścieżka funkcji zawsze wraca. HIR
`add` to jeden `HirStmt::Return` z `Binary`.

Pliki: `src/typeck/mod.rs`, `stmt.rs`, `expr/call.rs`, `expr/binary.rs`.

## Krok 3. Sema

`analyze_with_decl_tys` dostaje kolejkę typów deklaracji: `total`, potem
`label`. `fun_sigs` pamięta, że oba parametry `add` są `Val`.

Korzeń `fun add` wiąże `a` i `b` jako `Local`. W ciele nie ma odczytu
spoza regionu, który warto obserwować poza parametrami. Dump nie pokazuje
osobnych obserwacji Copy dla parametrów użytych w tym samym regionie:
`classify_use` zwraca `Local`, a dump lokalne obserwacje przy deklaracji
pomija.

Korzeń `fun main`:

```text
Arenas
├── fun add
│   ├── a [Local]
│   └── b [Local]
└── fun main
    ├── total [Local]
    ├── label [Local]
    ├── ForLoop (i)
    │   ├── i [Local]
    │   └── total [Copy]
    └── Block
        └── label [Shared ← fun main]
```

To jest wyjście `--dump-arenas` z uruchomienia, nie rekonstrukcja.
`ForLoop` powstał w `walk` na `Stmt::For`. `i` jest `RegionParam`.
`total` w ciele przechodzi `classify_use`: inna arena, `is_copy` → `Copy`.
Przypisanie `total = ...` jest assign-up (cel `Var`, `arena_id` mniejszy),
więc lewa strona nie jest błędem „użyłeś var rodzica”. Prawa strona to
`i32`, więc i tak Copy.

Blok po pętli: `open_ordinary` z etykietą `Block`. `label` jest `Val` i
nie-Copy, inna arena → `Shared { from: "fun main" }`.

`loop_move_ban` na czas `for` zawiera `total`. Nikt nie woła `move`, więc
zakaz milczy. `peel_blocks` nic nie skleja: żaden blok nie jest gołym
opakowaniem.

Pliki: `src/sema/analyze.rs`, `walk.rs`, `policy.rs` (`classify_use`),
`region.rs` (`RegionFrame`), `dump.rs`.

## Krok 4. Stempel, hoist, escape

Diagnostyk nie ma, więc `stamp_codegen_push` schodzi `region_walk`.
Korzeń każdej funkcji dostaje `codegen_push = true`.

Ciało pętli: przypisanie `i32` i wywołanie `add` nie alokują sinku
napisowego. Predykat `block_may_allocate_sink` dla ciała złożonego z
samej arytmetyki jest fałszywy. Węzeł `ForLoop` zostaje w raporcie, ale
**nie** prosi o `bork_arena_push`. To jest ten przypadek z modelu pamięci:
„węzeł w dumpu, brak płyty”.

Blok z `println(label)` nie tworzy nowego napisu. Czyta Shared. Predykat
alokacji sinku na samym wywołaniu `println` nie jest `concat` ani
literałem. Literał `"sum"` stoi przy deklaracji `label` w ciele funkcji,
nie w bloku. Blok wewnętrzny też nie musi pchać własnej płyty.

Hoist: nigdzie nie ma pary `var inner` / `outer = move inner`.
`alloc_in_binding` zostaje puste.

Escape: `return total` nie jest typem z areną. `label` nie wraca z
wewnętrznego regionu. `place` nie zgłasza nic.

HIR zostaje w `CheckResult`.

Pliki: `src/region_walk.rs` (`stamp_codegen_push`,
`block_may_allocate_sink`), `src/hoist.rs`, `src/escape.rs`.

## Krok 5. Bramka i moduł

`gate` nie znajduje trailing closure, `None`, `Some`, `?:`, `!!` ani
obcego pola. Przechodzi.

`emit_module` deklaruje `main` jako `i32 ()` oraz `bork.add` jako funkcję
wewnętrzną. Potem emituje ciała.

`emit_function` dla `main` otwiera blok wejścia, `alloca` na `total` i na
deskryptor `label`. Region funkcji pcha płytę (`codegen_push` korzenia).
Literał `"sum"` jest globalem; deskryptor `{ ptr, len: 3 }` ląduje w
slocie. Płyta funkcji w tym programie może zostać nietknięta przez
`alloc`, bo znaki są w globalu. Push i tak jest.

Pętla: indeks w rejestrze albo w `alloca`, porównanie z 3, ciało woła
`bork.add`, dodawanie jest `build_int_add` po stronie `add` i przypisanie
w `main`. Zatrzask nie musi resetować płyty, jeśli enter pętli w ogóle nie
wypchnął uchwytu. Spacer i tak woła `loop_latch`; `ArenaCalls` na braku
uchwytu regionu nie resetuje cudzej płyty.

`println` obniża się do `bork_println_str`. Po bloku, jeśli nie było
pusha, nie ma popa. `return total` ładuje `i32` i zwraca. `unwind` zdejmuje
uchwyt funkcji.

`add` to `load` dwóch parametrów, `build_int_add`, `ret`. Bez napisów.

Pliki: `src/codegen/gate.rs`, `src/codegen/llvm/mod.rs`,
`src/codegen/llvm/emit_fn.rs`, `src/codegen/llvm/expr.rs`,
`src/codegen/llvm/arena.rs`, `src/codegen/regions.rs`.

## Krok 6. Obiekt, link, proces

`write_object` bierze natywny target (na maszynie, na której książka była
sprawdzana: Linux x86-64), ustawia triple i data layout, pisze plik
obiektowy. `link_executable` woła `clang` z `libbork_runtime.a`.

Proces `main` zwraca 3. `println` pisze trzy bajty `sum` i nową linię,
potem flush. Pula aren w runtime żyje w statycznym `Mutex`. Po `pop`
płyta funkcji wraca na listę. Proces się kończy, więc pula znika z
pamięcią procesu.

## Czego ten ślad nie pokrywa

Nie było hoista, `promote`, `move` napisu, wycinka, `while`, zwierania ani
błędu escape. Każde z nich ma listing w części I i test w `tests/build.rs`
albo w `src/escape.rs`. Ślad miał pokazać **szczęśliwą ścieżkę przez
wszystkie funkcje graniczne**, nie każdy hak.

Gdy będziesz powtarzał ślad na programie z `var piece = concat(...)` i
`out = move piece`, zatrzymaj się dłużej w kroku 4: `hoist::annotate`
ustawi `alloc_in_binding`, a w kroku 5 emisja deklaracji ustawi
`alloc_sink` na dom `out` zanim wyemituje `concat`. Reszta kroków jest ta
sama.

## Podsumowanie

- Śledzony program zwraca 3 i drukuje `sum`. Checker i `bork build` są zgodne.
- Parser widzi dwie funkcje i zakres `..`. Typeck widzi `i32` i `String`.
- Sema oznacza `total` jako Copy w pętli i `label` jako Shared w bloku.
- Pętla bez alokacji nie dostaje własnej płyty, choć ma węzeł w dumpu.
- Hoist i escape nie mają tu nic do roboty. To też jest wynik, nie pominięcie.
- LLVM woła `bork.add` i `bork_println_str`. Runtime dostaje deskryptor literału z globala.
