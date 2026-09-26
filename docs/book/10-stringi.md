# Rozdział 9. Jak Bork przechowuje napisy w pamięci

## Ten rozdział obejmuje

- czym w gotowym programie jest wartość napisu
- gdzie powstają znaki, gdy zapisujesz wynik do zmiennej
- kiedy kompilator buduje napis od razu w arenie zmiennej docelowej
- jak czytać przykłady z dokumentacji, które kompilator odrzuca
- czego jeszcze nie da się zwrócić z funkcji

## Adres i długość, a osobno znaki

W kodzie maszynowym wartość typu `String` jest parą. Pierwszy element to adres znaków, drugi to długość zapisana na 64 bitach. Same znaki leżą albo w stałej programu, albo w buforze areny. Koniec regionu unieważnia bufor, a nie parę leżącą na stosie. Dlatego kompilator musi odrzucić parę, której adres wskazywałby w już zwolniony bufor, zanim generator kodu ją wyemituje. Robi to analiza ucieczki, funkcja `place` w `src/escape.rs`.

Głębokość zero oznacza region funkcji. Każdy region zagnieżdżony, który analiza ucieczki naprawdę otwiera, zwiększa głębokość. Funkcja `place` odpowiada na pytanie, w jakiej głębokości leżą znaki danego wyrażenia.

## Znaki powstają tam, gdzie stoisz

Bez miejsca przeznaczenia literał i wynik `concat` biorą bieżącą arenę. Wyjście z bloku czyści tę arenę. Nazwa zadeklarowana w bloku i tak nie jest widoczna na zewnątrz. Problem zaczyna się wtedy, gdy parę adresu i długości wynosisz przypisaniem albo instrukcją `return`.

## Zapis do zmiennej wybiera jej arenę

Gdy prawa strona jest zapisywana do zmiennej, znaki wyniku trafiają do areny tej zmiennej. Nie dotyczy to każdego podwyrażenia. Argumenty wywołania są liczone tak, jakby miejsca przeznaczenia chwilowo nie było, więc wartości tymczasowe zostają w bloku.

**Listing 9.1.** Literał zapisany wprost do zmiennej z zewnątrz. Program został zbudowany. Na wyjściu jest `b` oraz nowy wiersz.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

Nie potrzebujesz zmiennej pośredniej. Zapis `outer = concat(lewy, prawy)` kładzie wynik w arenie zmiennej `outer`. Napisy `lewy` i `prawy` są czytane tam, gdzie już żyją.

## Dwie sąsiednie instrukcje mogą uniknąć drugiej kopii

Gdy w bloku jest najpierw `var piece = literał albo wywołanie`, a zaraz potem `outer = move piece`, i nazwa `outer` jest już widoczna, kompilator ustawia przy deklaracji informację, że treść napisu ma powstać w buforze nazwy `outer`. W reprezentacji pośredniej to pole nazywa się `alloc_in_binding`. Druga instrukcja nie kopiuje wtedy znaków drugi raz.

Rozpoznanie jest celowo wąskie. Leży w `src/hoist.rs` i obejmuje tylko następną instrukcję, bez niczego pomiędzy. Inicjalizator musi być literałem napisu albo wywołaniem, nie tablicą i nie dowolnym wyrażeniem. Prawa strona przypisania musi być przeniesieniem tej samej nazwy. Cel musi być nazwą, nie elementem tablicy. Nazwa celu musi być parametrem albo zmienną zadeklarowaną wcześniej.

Pętla, warunek, funkcja dopisana na końcu wywołania i jakakolwiek instrukcja między tymi dwiema wyłączają to rozpoznanie. Zostaje kopia przy `move` albo jawne `promote`.

> **NOTA.** Kompilator nie dowodzi, że nazwa pośrednia istnieje tylko po to, żeby nakarmić zmienną zewnętrzną. Patrzy na dwie sąsiednie instrukcje. Gdy je rozdzielisz, płacisz kopię.

## Promote, gdy znaki już powstały za głęboko

Listing 8.3 pokazuje `promote`. Nazwa `held` powstała w arenie bloku. `promote` kopiuje znaki do areny zmiennej zewnętrznej i oznacza źródło jako przeniesione. Nie pomija kopii. Kopii unikasz wtedy, gdy wynik od razu zapisujesz do zmiennej zewnętrznej, albo gdy dwie sąsiednie instrukcje pozwolą zbudować wartość od razu w jej arenie.

## Czego analiza ucieczki nie przepuści

Komunikaty z `src/escape.rs` sprawdzone na przykładach są takie. Zwrot wartości, której znaki żyją głębiej niż region funkcji, mówi, że zwracany napis żyje w regionie wewnętrznym i że arena zostanie zwolniona przed powrotem. Zwrot wyniku `concat` mówi, że nie jest to jeszcze obsługiwane, bo znaki zwróconego napisu musiałyby przeżyć arenę wywoływanej funkcji. Zwrot napisu przeniesionego albo użytego ze słowem `promote` radzi użyć zwykłej nazwy albo literału. Przypisanie wartości, której pamięć leży głębiej niż zmienna docelowa, mówi, że taki zapis nie jest obsługiwany. Wartość warunku, która jest napisem przeniesionym tylko w gałęzi, jest odrzucana, bo arena gałęzi ginie razem z gałęzią.

Zapis `return if (c > 0) { concat("a", "b") } else { "x" }` wpada w komunikat o `concat`, nie w komunikat o gałęzi. Sprawdziłem to na osobnym pliku. Pozycja w pliku bywa pusta.

## Przykład z dokumentacji, który się nie kompiluje

Plik `docs/language.md` pokazuje w bloku wywołanie `concat(left, right)`, gdy `left` i `right` są zmiennymi. Taki program nie przechodzi sprawdzenia. Obie nazwy są zmienne, więc w bloku wewnętrznym trzeba je przenieść. Komunikaty, które dostałem, mówią, że `left` nie jest kopiowane i trzeba je przenieść do bloku, i to samo o `right`.

Działają dwie poprawki. W obu wynik na wyjściu to `LR` oraz nowy wiersz, a nie `prefixLR`. `concat` nie dopisuje znaków do istniejącego bufora. Buduje nowy napis, a przypisanie zastępuje parę adresu i długości trzymaną w zmiennej. Stary napis `"prefix"` zostaje w arenie aż do końca regionu funkcji, ale nazwa już na niego nie wskazuje.

**Listing 9.2.** Stałe napisy są w bloku tylko oglądane. Program został zbudowany.

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

W drzewie regionów widać, że `piece` zostało przeniesione, a `left` i `right` są współdzielone z regionu funkcji. Dwie sąsiednie instrukcje pasują do rozpoznania z `hoist.rs`, więc znaki wyniku mogą powstać od razu w arenie `out`. Dla programisty skutek jest ten sam niezależnie od tego, czy kopiowanie zostało pominięte. Po bloku `out` da się wypisać.

**Listing 9.3.** Zmienne napisy trzeba przenieść jawnie. Program został zbudowany. Po bloku `left` i `right` są martwe.

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

## Czego jeszcze nie ma

Kompilator nie zgaduje więcej układów niż te dwie sąsiednie instrukcje. Nie ma bufora wyniku, który należałby do funkcji wywołującej. Nie ma `promote` przy `return`. Nawet `concat` stojący w jednej gałęzi warunku nie może być zwrócony.

Osobno, przy budowaniu, poprawne sprawdzenie nie wystarcza, żeby przekazać napis do funkcji użytkownika. Taki program przerywa kompilator. `concat` i `println` są wyjątkiem, bo mają własne fragmenty generowania kodu.

## Podsumowanie

- Wartość napisu jest adresem i długością. Znaki leżą w stałej albo w buforze areny o pojemności 4096 bajtów.
- Przypisanie do zmiennej alokuje wynik w arenie tej zmiennej.
- Dwie sąsiednie instrukcje, deklaracja i zaraz potem przeniesienie do zmiennej zewnętrznej, mogą zbudować znaki od razu w arenie celu.
- `promote` kopiuje, gdy znaki już powstały w zbyt krótkim regionie.
- Przykład `concat` na dwóch zmiennych napisowych z dokumentacji języka nie kompiluje się. Trzeba użyć stałych albo słowa `move`.
- `concat` zastępuje wartość zmiennej. Nie dopisuje znaków do starego bufora.
