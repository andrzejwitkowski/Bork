# Rozdział 6. Kontrola przepływu

## Ten rozdział obejmuje

- `if` jako wyrażenie i jako sterowanie
- `for` po zakresie `lo..hi`
- `while`, `break`, `continue`
- Zwieranie `&&` i `||`
- Jak sema scala flagi `moved` po gałęziach

## `if`

Warunek ma mieć typ `bool`. Obie gałęzie, jeśli `else` jest, są blokami.
Typ wyniku zależy od tego, czy typeck **oczekuje** typu, i od tego, czy
`else` istnieje. To jest w `typeck/expr/control.rs`, funkcja `check_if`.

| `else` | Oczekiwany typ | Typ wyniku |
|---|---|---|
| brak | brak | `unit` |
| brak | jest (np. adnotacja `val x: i32`) | błąd: `value-producing if expression requires an else branch` |
| jest, typy gałęzi równe | dowolny | ten typ |
| jest, typy różne, oczekiwany typ jest | | błąd: `else branch has type …, expected …` |
| jest, ale wywołanie nie oczekuje typu | | `unit` |

Ostatni wiersz zaskakuje. `if` użyty tam, gdzie nikt nie prosi o wartość,
dostaje typ `unit` nawet z `else`. Wartość gałęzi jest sprawdzana, ale nie
wypływa. Dlatego nie ma „niejawnego wyniku bloku” w stylu Rusta poza
ostatnim wyrażeniem gałęzi, i tylko gdy ktoś tego typu oczekuje.

**Listing 6.1.** Wyrażenie `if`. Uruchomione: kod 42.

```bork
fun pick(flag: i32): i32 {
    val v = if (flag == 1) { 40 } else { 7 }
    return v + 2
}

fun main(): i32 {
    return pick(1)
}
```

Tu `val v` nie ma adnotacji, ale prawa strona jest w kontekście, w którym
obie gałęzie mają ten sam typ `i32`, więc wynik `if` to `i32`. Dump ma
osobno `IfThen` i `IfElse`.

**Listing 6.2.** Brak `else` przy oczekiwaniu `i32`. Uruchomione.

```bork
fun main(): i32 {
    val x: i32 = if (1 > 0) { 1 }
    return x
}
```

```text
if_ann.bork: error: type: value-producing if expression requires an else branch
```

Bez adnotacji, `val x = if (n > 0) { n }`, błąd jest inny: `if` staje się
`unit` (nikt nie oczekiwał typu), a dopiero `return x` mówi `return value
has type unit, expected i32`. Komunikat o brakującym `else` pojawia się
wtedy, gdy oczekiwany typ jest `Some`.

Gałąź `if` jest regionem. Napis zbudowany tylko w jednej gałęzi nie może
być wartością tego `if`, bo arena gałęzi ginie przy jej końcu. Escape
mówi:

```text
a `String` moved inside an `if` branch cannot be its value: the branch's arena is freed when the branch ends
```

Tekst mówi `String`, choć warunek w kodzie patrzy na `uses_arena_storage`
(także tablica). To drobny rozjazd komunikatu z warunkiem.

Sema scala stan `moved` po `if`: nazwa jest przeniesiona po całym `if`
wtedy, gdy była przeniesiona przed nim, albo gdy przeniosły ją **obie**
gałęzie. Samo `then` bez `else` nie zostawia nazwy martwej po `if`. Test
`if_then_only_move_without_else_does_not_stick` to zamyka. Inaczej gałąź,
która może nie pobiec, zabijałaby zmienną na stałe.

## `for`

`for (nazwa in lo..hi) { ciało }`. Zakres jest prawostronnie otwarty.
Granice są typu `i32` i są liczone raz. Nazwa pętli jest `i32` w typecku.
W semie wiązanie indeksu jest zakładane jako `Int`, czyli też `i32`, ale
wpisane przez `Type::from_ident("Int")`, nie przez typ elementu zakresu z
HIR. Przy poprawnym programie to ten sam typ.

**Listing 6.3.** Suma `0+1+2+3+4`. Uruchomione: kod 10.

```bork
fun main(): i32 {
    var total = 0
    for (i in 0..5) {
        total = total + i
    }
    return total
}
```

Dump:

```text
└── fun main
    ├── total [Local]
    └── ForLoop (i)
        ├── i [Local]
        └── total [Copy]
```

`total` w ciele jest Copy, bo `i32`. Przypisanie `total = ...` jest
mutacją zewnętrznego `var`, nie przeniesieniem. Codegen emituje jeden
`region_enter` dla ciała, a na zatrzasku pętli `bork_arena_reset`, nie
`pop` i ponowny `push`. Płyta zostaje. To jest kontrakt opisany w
`TODO.md`: jeden enter, wiele resetów, jeden exit.

Zakres `i64..i64` odpada:

```text
range_bad.bork:4:15: error: type: range bounds must have type i32, got i64 and i64
```

Iterator, który nie jest zakresem (`for (i in n)`), daje `for-loop iterator
must be a range`. Operator `..` poza `for` tworzy wartość typu zakresu;
typeck i tak wymaga `i32`. Nie ma `for` po tablicy.

## `while`, `break`, `continue`

**Listing 6.4.** `break`. Uruchomione: kod 3.

```bork
fun main(): i32 {
    var i: i32 = 0
    while (i < 10) {
        if (i == 3) {
            break
        }
        i = i + 1
    }
    return i
}
```

Warunek `while` musi być `bool` (`while condition has type …, expected
bool`). `break` i `continue` poza pętlą:

```text
err_break.bork:2:5: error: type: `break` outside of a loop
```

To błąd fazy `type`, nie osobnej fazy sterowania. Typeck trzyma
`loop_depth` i podbija go na `for` oraz `while`.

**Listing 6.5.** `continue` pomija `i == 2`. Suma `0+1+3+4 = 8`. Uruchomione: kod 8.

```bork
fun main(): i32 {
    var s = 0
    for (i in 0..5) {
        if (i == 2) {
            continue
        }
        s = s + i
    }
    return s
}
```

`continue` w `while` też jest. Sema oznacza odczyt `i` oraz `s` w ciele
jako `Copy`. Nie ma `for` z klauzulą `else`. Nie ma pętli nieskończonej
innej niż `while (true)`.

## Zwieranie

**Listing 6.6.** Prawa strona nie powinna podbić `n`. Uruchomione: kod 0.

```bork
fun side(): i32 {
    return 1
}

fun main(): i32 {
    var n: i32 = 0
    if (false && side() == 1) {
        n = 1
    }
    if (true || side() == 1) {
        return n
    }
    return 99
}
```

`side` nie ma efektu poza wynikiem, więc listing nie udowadnia zwierania
obserwowalnym printem. Udawadnia je zgodność z testem
`builds_logical_short_circuit` oraz kształt LLVM: `&&` i `||` dostają
osobne bloki, a `region_walk` nie schodzi w prawy operand, gdy flaga
`short_circuit_logical_operands` jest włączona. Dzięki temu stempel
`codegen_push` nie rozjeżdża się z emisją. To była osobna poprawka
(`fix: walk logical RHS for stamp/schedule only`).

`!` jest jednoargumentowe i wymaga `bool`. Program `val flag = !false`
oraz `if (flag) { return 1 } else { return 0 }` daje kod 1.

## Wczesny `return` a regiony

`return` w zagnieżdżonym bloku musi zdjąć areny, które funkcja otworzyła.
Codegen robi `unwind` uchwytów i nie wykonuje `region_exit` drugi raz po
ścieżce, która już wróciła (seria poprawek „skip region exit after
return”, „sync region stack”, „clear arena handles”). Test
`builds_nested_loops_with_regions_and_early_return` zwraca 63: zagnieżdżone
`for` liczą pary `j > i`, potem pętla do 100 zwraca `total * 10 + k` przy
`k == 3`.

To jest szczegół backendu, ale ma skutek językowy: `return` w środku bloku
jest legalny dla wartości Copy i dla napisu na głębokości 0. Nie jest
legalny dla napisu, którego bajty żyją głębiej.

**Listing 6.7.** Uruchomione checker, kod 1. `bork build` też kończy się 1 i nie zostawia binarki.

```bork
fun mk(): String {
    var s = "esc"
    {
        val x = move s
        return x
    }
}

fun main() {
    println(mk())
}
```

```text
err_ret_inner.bork:5:16: error: ownership: returning a `String` whose bytes live in an inner region is not supported: that arena is freed before the value is returned
```

Faza to `ownership`, bo escape dokłada diagnostykę do tej samej fazy co
sema. HIR przy błędzie escape jest wyrzucany: `CheckResult.hir` jest
`None`, raport aren zostaje.

## Podsumowanie

- `if` bez `else` w kontekście bez oczekiwanego typu ma typ `unit`.
- Z adnotacją brak `else` jest osobnym błędem.
- Move w jednej gałęzi nie zabija nazwy po `if`, jeśli druga gałąź jej nie ruszyła.
- `for` idzie po `i32..i32`, zakres prawostronnie otwarty, arena ciała resetowana co obrót.
- `break` i `continue` są błędami typu poza pętlą.
- `&&` i `||` zwierają. Backend i spacer regionów są co do tego zgodne.
- `return` napisu z wewnętrznego bloku jest odrzucany, zanim powstanie binarka.
