# Rozdział 7. Tablice

## Ten rozdział obejmuje

- Typ `[T; N]` i literał `[1, 2, 3]`
- Indeks, przypisanie elementu, wycinek o stałych granicach
- `.length`
- Własność całej tablicy i współdzielenie elementu `String`
- Co robi runtime, gdy indeks nie mieści się w `N`

## Długość jest typem

`N` w `[T; N]` jest literałem całkowitym w typie, nie wyrażeniem. Za duże
`N` dla `u32` daje błąd gramatyki `array length is too large`. Pusta
tablica nie ma skąd wziąć `N` ani `T`:

```text
err_empty.bork:2:13: error: type: empty array literal requires an explicit type, e.g. `val a: [i32; 0] = []`
```

Z adnotacją działa. **Listing 7.1.** Uruchomione: kod 0, bo `length` zera
zwrócone jako kod wyjścia.

```bork
fun main(): i32 {
    val a: [i32; 0] = []
    return a.length
}
```

Elementy literału muszą mieć jeden typ. Liczba elementów musi równać się
`N`.

**Listing 7.2.** Uruchomione checker.

```text
array_mismatch.bork:2:23: error: type: array literal has 3 elements, expected 2
array_mismatch.bork:2:9: error: type: initializer for `a` has type [i32; 3], expected [i32; 2]
```

dla `val a: [i32; 2] = [1, 2, 3]`.

Dozwolony element to typ Copy albo nienullowalny `String`. `String?` jako
element jest odrzucany. Nullowalna tablica jako całość też.

Tablica nie jest Copy, nawet gdy elementy są. Przypisanie całych tablic
wymaga identycznego `T` i `N`. Nie ma `[i32; 3]` w miejsce `[i32; 2]`.

## Indeks

Indeks jest `i32` i jest liczony w runtime. Nie jest częścią typu, więc
typeck **nie** odrzuca `a[9]` na tablicy trzyelementowej. Sprawdzone:
checker mówi tak, `bork build` produkuje binarkę, uruchomienie kończy się
SIGABRT (134). `guard_index` woła `abort`.

**Listing 7.3.** Uruchomione: kod 12. Po zapisie `a[1] = 9` wycinek widzi ten sam bufor, więc `b[1]` to 9, a `a.length` to 3.

```bork
fun main(): i32 {
    var a: [i32; 3] = [1, 2, 3]
    a[1] = 9
    val b = a[0..2]
    return b[1] + a.length
}
```

`val` nie przyjmuje przypisania elementu:

```text
val_index.bork:3:5: error: type: cannot assign to immutable `val` binding `a`
```

Indeks złego typu: `array index must be i32` albo, przy przypisaniu,
`array index has type …, expected i32`.

## Wycinek

`a[lo..hi]` wymaga, żeby `lo` i `hi` były literałami `i32`. Typ wyniku to
`[T; hi-lo]`. Wycinek nie kopiuje bufora. Deskryptor wskazuje w środek
tablicy, a długość bierze z typu.

Poza zakresem:

```text
err_slice.bork:3:13: error: type: slice [1..9] is out of bounds for `[i32; 3]`
```

Granice, które nie są literałami: `slice bounds must be integer literals so
the result type is [T; N]`. Codegen **nie** wstawia drugiego sprawdzenia
wycinka w runtime. Ufa typeckowi. Indeks w runtime sprawdzany jest, bo nie
da się go włożyć do typu.

**Listing 7.4.** Uruchomione: stdout `20\n2\n`.

```bork
fun main() {
    val a = [10, 20, 30]
    val b = a[0..2]
    println(b[1])
    println(b.length)
}
```

Escape traktuje wycinek jak odbiorcę: nie może przeżyć areny, która trzyma
bufor. Test `rejects_slice_escaping_inner_region` oczekuje fazy
`ownership` i tekstu `inner region`.

## Napis jako element

Odczyt `a[i]`, gdy element jest `String`, jest widokiem Shared na bufor
tablicy, niezależnie od `val` czy `var` tablicy. Nie ma `move` jednego
elementu. `move` i `promote` dotyczą całej tablicy.

Zapis elementu nie-Copy używa areny tablicy jako sinku przypisania i
wymaga `move` albo `promote` po prawej.

**Listing 7.5.** Uruchomione: stdout `z\n`.

```bork
fun main() {
    var a: [String; 2] = ["a", "b"]
    var s = "z"
    a[0] = move s
    println(a[0])
}
```

Bez `move` checker mówi, że trzeba `move` (test
`index_assign_string_requires_move`). Po `move` nazwa `s` jest martwa.
Dump listingu 7.5: `s [Moved ← fun main]`.

## Reprezentacja

W runtime tablica i napis mają ten sam kształt deskryptora LLVM:
`{ ptr, i64 }`. Dla tablicy `len` równa się `N`. Bajty elementów leżą w
arenie domu tej wartości: arena aktywna w miejscu utworzenia albo arena
sinku, gdy tablica jest od razu zapisywana do zewnętrznego `var`.

`length` w codegenie jest wycięciem pola `i64` i obcięciem do `i32`.
Dlatego typ pola w języku to `i32`, mimo że w deskryptorze długość jest
64-bitowa. Przy `N`, które mieści się w gramatyce jako `u32`, obcięcie do
`i32` jest prawdziwe tylko dopóki `N` mieści się w `i32`. Gramatyka
przyjmuje `u32`. To jest krawędź, której testy nie spinają dużym `N`. Nie
udawaj, że `length` tablicy o `N > 2^31-1` jest dobrze określone.

## Podsumowanie

- `[T; N]` niesie długość w typie. Literał musi mieć dokładnie `N` elementów.
- `[]` wymaga adnotacji.
- Indeks spoza zakresu aboruje proces. Zły wycinek jest błędem typu.
- Wycinek jest widokiem, nie kopią, i ma typ `[T; hi-lo]`.
- Element `String` czyta się jako Shared. Zapis elementu wymaga `move`.
- Całą tablicę wolno `move` / `promote` tylko na identyczny `[T; N]`.
