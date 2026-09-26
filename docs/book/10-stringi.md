# Rozdział 9. Gdzie żyją bajty `String`

## Ten rozdział obejmuje

- Deskryptor kontra bajty
- Pięć reguł z `docs/language.md`, skonfrontowanych z kompilatorem
- Hoist sąsiednich dwóch linii
- `concat` i to, czego przykładowy rozdział dokumentacji nie kompiluje
- Czego jeszcze nie ma: arena wyniku, `promote` na `return`

## Deskryptor

Wartość `String` w LLVM to struktura `{ ptr, i64 }`: adres bajtów i
długość. Bajty są albo w globalu (literał), albo w płycie areny. Koniec
regionu unieważnia płytę, nie deskryptor na stosie. Dlatego checker musi
odrzucić deskryptor, którego `ptr` wskazywałby w martwą płytę, zanim
codegen go wyemituje. To robi `escape::place`.

Głębokość 0 to region funkcji. Każdy zagnieżdżony region, który naprawdę
otwiera scope w escape, zwiększa głębokość. `place` zwraca głębokość puli,
w której leżą bajty wyrażenia.

## Reguła 1. Alokujesz tam, gdzie stoisz

Bez sinku literał i `concat` biorą bieżącą pulę. Wyjście z bloku czyści
pulę. Nazwa zadeklarowana w bloku i tak nie jest widoczna na zewnątrz.
Problem zaczyna się, gdy deskryptor wynosisz przypisaniem albo `return`.

## Reguła 2. Sink przypisania

Gdy prawa strona jest zapisywana do `var`, bajty **wyniku** (nie każdego
podwyrażenia) idą do areny tego `var`. Argumenty wywołania są liczone przy
wyczyszczonym `alloc_sink`, więc tymczasowe zostają w bloku.

**Listing 9.1.** Uruchomione: stdout `b\n`. Literał nie potrzebuje zmiennej pośredniej.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

`outer = concat(left, right)` kładzie **wynik** w puli `outer`. Napisy
`left` i `right` są czytane tam, gdzie już żyją.

## Reguła 3. Hoist dwóch sąsiednich linii

Gdy w bloku jest

```text
var piece = <literał albo wywołanie>
outer = move piece
```

i `outer` jest już w zasięgu, `hoist::annotate` ustawia na deklaracji
`alloc_in_binding = Some(outer)`. Codegen buduje `piece` od razu w puli
`outer`. Druga linia nie kopiuje znaków drugi raz.

Wzór jest celowo wąski (`src/hoist.rs`):

- tylko następna instrukcja, nic pomiędzy,
- inicjalizator to `Str` albo `Call`, nie tablica i nie inne wyrażenie,
- prawa strona przypisania to `move` tej samej nazwy,
- cel to nazwa, nie `a[i]`,
- `outer` jest parametrem albo wcześniejszym `var` w tym bloku lub w
  `outer` przekazanym przy zejściu.

Pętla, `if`, domknięcie i luka między liniami wyłączają hoist. Zostaje
kopia przy `move` albo jawny `promote`.

> **NOTE.** Hoist nie dowodzi, że „`inner` istnieje tylko po to, żeby
> nakarmić `outer`”. Patrzy na dwie sąsiednie instrukcje. Rozdziel je, a
> płacisz kopię.

## Reguła 4. `promote`, gdy bajty już leżą źle

Listing 8.3. `held` powstał w wewnętrznej puli. `promote` kopiuje do puli
`outer` i oznacza źródło jako moved. Nie omija kopii. Omija ją hoist albo
przypisanie świeżego wyniku wprost do `outer` (reguła 2).

## Reguła 5. Escape odrzuca resztę

Komunikaty z `escape.rs`, sprawdzone tam, gdzie dało się je wywołać:

| Komunikat | Kiedy |
|---|---|
| `` returning a `String` whose bytes live in an inner region... `` | `return` wartości o głębokości > 0 |
| `returning the result of concat is not supported yet...` | `return concat(...)` |
| `returning a moved or promoted String is not supported yet...` | `return move` / `return promote` |
| `assigning a value whose bytes live in an inner region to nazwa...` | przypisanie w górę, którego sink nie uratował |
| `a String moved inside an if branch cannot be its value...` | wartość `if` z bajtami gałęzi |

`return if (c > 0) { concat("a", "b") } else { "x" }` wpada w komunikat o
`concat`, nie w komunikat o gałęzi. Sprawdzone na `err_if_string.bork`.
Span znowu bywa pusty.

## `concat` i błąd w podręczniku

`docs/language.md` pokazuje:

```bork
var left = "L"
var right = "R"
{
    var piece = concat(left, right)
    out = move piece
}
```

Ten program **nie** przechodzi checku. `left` i `right` są `var`. W bloku
trzeba je przenieść. Sprawdzone:

```text
concat.bork:6:28: error: ownership: `left` is not Copy; move it into `Block` with `move`
concat.bork:6:34: error: ownership: `right` is not Copy; move it into `Block` with `move`
```

Dwie wersje, które przechodzą i drukują `LR\n`:

**Listing 9.2.** `val` jest Shared. Uruchomione.

```bork
fun main() {
    var out = "prefix"
    val left = "L"
    val right = "R"
    {
        var piece = concat(left, right)
        out = move piece
    }
    println(out)
}
```

**Listing 9.3.** `var` z jawnym `move`. Uruchomione. Po bloku `left` i `right` są martwe.

```bork
fun main() {
    var out = "prefix"
    var left = "L"
    var right = "R"
    {
        var piece = concat(move left, move right)
        out = move piece
    }
    println(out)
}
```

W obu wypadkach stdout to `LR`, nie `prefixLR`. `concat` nie dokleja do
istniejącego bufora. Buduje nowy napis i przypisanie **zastępuje**
deskryptor `out`. Stary `"prefix"` zostaje śmieciem w puli aż do końca
regionu funkcji.

Dump listingu 9.2: w bloku `piece [Moved ← Block]`, `left [Shared ← fun main]`,
`right [Shared ← fun main]`. Hoist może ustawić `alloc_in_binding` na
`piece`, bo następna linia to `out = move piece`, a inicjalizator jest
wywołaniem. Efekt widoczny dla programisty jest ten sam: `out` po bloku
da się wydrukować.

## Co jeszcze nie działa

Z `docs/language.md` i z kodu, bez dorabiania:

- hoist tylko dla wzorca dwóch linii, nie dla ogólnego „ta zmienna ucieka”,
- brak bufora zwracanego, który należałby do wołającego,
- `promote` nie występuje na `return`,
- `return` świeżego `concat` nawet w jednej gałęzi `if` jest odrzucany.

Dodatkowo codegen: nawet poprawny checkerowo przekaz `String` do funkcji
użytkownika panikuje. `concat` i `println` są wyjątkami, bo mają własne
emitory (`emit_concat`, `emit_print`), nie `coerce_value_to_ty`.

## Podsumowanie

- `String` to deskryptor. Bajty leżą w globalu albo w płycie 4 KiB.
- Przypisanie do `var` alokuje wynik w puli tego `var`.
- Dwie sąsiednie linie `var piece = ...` / `outer = move piece` mogą zbudować bajty od razu u celu.
- `promote` kopiuje, gdy bajty już powstały za głęboko.
- Przykład `concat(left, right)` na dwóch `var` z dokumentacji języka nie kompiluje się. Trzeba `val` albo `move`.
- `concat` zastępuje deskryptor. Nie dokleja do starego bufora w miejscu.
