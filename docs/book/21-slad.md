# Rozdział 20. Jeden program od źródła do uruchomienia

## Ten rozdział obejmuje

- krótki program, który został zbudowany i uruchomiony
- co widzi czytanie składni
- co dopisuje sprawdzanie typów
- jakie drzewo regionów wypisuje analiza własności
- dlaczego wyniesienie alokacji i analiza ucieczki w tym programie milczą
- które funkcje wygenerowanego kodu i biblioteki wykonawczej naprawdę pracują

Program jest krótki celowo. Ma funkcję, pętlę, wywołanie, stały napis i blok, który ten napis tylko czyta. Polecenie `bork build` tłumaczy go na plik wykonywalny. Nie ma w nim napisu przekazywanego do funkcji użytkownika, bo ta ścieżka kończy się awarią kompilatora i ślad urwałby się w emisji.

## Program, który zwraca trzy i wypisuje sumę

**Listing 20.1.** Uruchomiony. Standardowe wyjście to `sum` i nowa linia. Kod wyjścia to 3, bo suma `0 + 1 + 2` wynosi 3.

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

Zakres `0..3` jest otwarty z prawej strony. Są trzy iteracje, a nie cztery. Indeks przyjmuje wartości 0, 1 i 2.

## Czytanie składni widzi dwie funkcje

W pliku nie ma nawiasu, który łamałby linię, więc funkcja normalizująca nowe linie zwraca brak zmiany i parser dostaje oryginał. Parser buduje dwie funkcje.

Dla `add` drzewo składni ma dwa parametry. Nie napisano przy nich `val`, a goła forma parametru jest stała. Typ obu to `i32`. Ciało ma jeden powrót i dodawanie.

Dla `main` jest deklaracja zmiennej `total` bez adnotacji typu, pętla `for` z zakresem, deklaracja stałej `label`, blok z wywołaniem `println` oraz powrót.

Gdyby w pliku był średnik, czytanie składni zwróciłoby nierozpoznany token. Sprawdzenie skończyłoby się jednym komunikatem fazy `parse`, bez drzewa regionów. Średnika nie ma.

Wejście jest w funkcji `bork::parse` w `src/lib.rs`. Reguły gramatyki to produkcje funkcji, instrukcji, zakresu i atomu.

## Sprawdzanie typów widzi liczby i napis

Sprawdzanie typów widzi dwie sygnatury. Funkcja `add` bierze dwie liczby `i32` i zwraca `i32`. Funkcja `main` zwraca `i32`. Nazwa `println` jest wbudowana i nie koliduje.

Zmienna `total` nie ma adnotacji. Inicjalizator `0` bez oczekiwanego typu jest literałem całkowitym, więc dostaje `i32`. Ten typ wpada do wektora typów deklaracji. Stała `label` ma typ `String`. Indeks `i` w pętli dostaje `i32`, bo granice zakresu są typu `i32`. Warunek pętli nie jest osobnym wyrażeniem logicznym. Zakres jest osobnym typem, który istnieje dopiero po sprawdzeniu typów.

Wywołanie `add(total, i)` ma nazwę jako rzecz wywoływaną i dwa argumenty typu `i32`. Odczyt `total` w ciele pętli dostaje rodzaj użycia oznaczający kopię, bo region jest inny, a typ jest kopiowalny. Wywołanie `println(label)` jest funkcją wbudowaną. Argument jest napisem. Odczyt `label` dostaje rodzaj użycia oznaczający współdzielenie.

Powrót `total` zgadza się z `i32`. Ścieżka funkcji zawsze wraca. Reprezentacja pośrednia funkcji `add` to jeden powrót z dodawaniem.

Pliki tego kroku to `src/typeck/mod.rs`, `src/typeck/stmt.rs`, `src/typeck/expr/call.rs` i `src/typeck/expr/binary.rs`.

## Analiza własności widzi kopię w pętli i współdzielenie w bloku

Analiza dostaje kolejkę typów deklaracji: najpierw `total`, potem `label`. Pamięta, że oba parametry `add` są stałe.

Korzeń `fun add` wiąże `a` i `b` jako nazwy lokalne. Wydruk nie pokazuje osobnych obserwacji kopiowania dla parametrów użytych w tym samym regionie. Reguła odczytu zwraca użycie lokalne, a wydruk pomija lokalne obserwacje przy deklaracji.

Korzeń `fun main` wygląda tak. Ten tekst pochodzi z uruchomienia ze znacznikiem `--dump-arenas`, a nie z rekonstrukcji na papierze.

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

Węzeł pętli powstał przy zejściu w instrukcję `for`. Indeks `i` jest parametrem tego regionu. Nazwa `total` w ciele przechodzi regułę odczytu: inny region i typ kopiowalny, więc obserwacja to kopia. Przypisanie `total = ...` idzie do zmiennej z regionu zewnętrznego. Cel jest zmienną, a numer jego regionu jest mniejszy, więc lewa strona nie jest błędem użycia zmiennej rodzica. Prawa strona to `i32`, więc i tak jest kopią.

Blok po pętli otwiera zwykły region o etykiecie `Block`. Stała `label` nie jest kopiowalna i żyje w innym regionie, więc obserwacja to współdzielenie z etykietą `fun main`.

Na czas pętli zbiór zakazu przeniesienia zawiera `total`. Nikt nie woła `move`, więc zakaz milczy. Zdjęcie opakowań nic nie skleja, bo żaden blok nie jest gołym opakowaniem drugiego bloku.

Pliki tego kroku to `src/sema/analyze.rs`, `src/sema/walk.rs`, `src/sema/policy.rs`, `src/sema/region.rs` i `src/dump.rs`.

## Oznaczenie buforów, wyniesienie i ucieczka nie mają tu pracy poza korzeniem funkcji

Komunikatów nie ma, więc oznaczanie schodzi wspólnym spacerem. Korzeń każdej funkcji dostaje flagę pobrania bufora.

Ciało pętli to przypisanie liczby `i32` i wywołanie `add`. Ani jedno, ani drugie nie alokuje napisu w miejscu przeznaczenia. Predykat `block_may_allocate_sink` dla ciała złożonego z samej arytmetyki jest fałszywy. Węzeł pętli zostaje w raporcie, ale nie prosi o `bork_arena_push`. Wydruk ma węzeł, a w czasie działania pętla nie dostaje własnego bufora.

Blok z `println(label)` nie tworzy nowego napisu. Czyta wartość współdzieloną. Samo wywołanie `println` nie jest ani `concat`, ani literałem napisu. Literał `"sum"` stoi przy deklaracji `label` w ciele funkcji, a nie w bloku wewnętrznym. Blok wewnętrzny też nie musi pobierać własnego bufora.

Wyniesienie alokacji nie znajduje pary: deklaracja zmiennej wewnętrznej i zaraz potem przeniesienie do zmiennej zewnętrznej. Pole miejsca alokacji zostaje puste.

Analiza ucieczki widzi powrót liczby `total`, a nie typu trzymanego w buforze. Stała `label` nie wraca z regionu wewnętrznego. Funkcja licząca głębokość nic nie zgłasza.

Reprezentacja pośrednia zostaje w wyniku sprawdzenia.

## Kontrola, moduł i wygenerowane wywołania

Kontrola przed generowaniem kodu nie znajduje funkcji dopisanej na końcu wywołania, słów `None` i `Some`, operatora `?:`, wykrzykników `!!` ani obcego pola. Przechodzi.

Emisja modułu deklaruje `main` jako funkcję zwracającą `i32` bez parametrów oraz `bork.add` jako funkcję wewnętrzną. Potem emituje ciała.

Dla `main` otwiera blok wejścia, rezerwuje slot na `total` i slot na deskryptor `label`. Region funkcji pobiera bufor, bo korzeń zawsze ma flagę pobrania. Literał `"sum"` jest stałą globalną. Deskryptor ze wskaźnikiem i długością 3 ląduje w slocie. Bufor funkcji w tym programie może zostać nietknięty przez alokację, bo znaki są w stałej globalnej. Pobranie bufora i tak jest.

Pętla trzyma indeks, porównuje go z 3, woła `bork.add` i przypisuje wynik. Zatrzask nie musi czyścić bufora pętli, jeśli wejście w pętlę w ogóle nie położyło uchwytu. Spacer i tak woła hak zatrzasku. Emiter na braku uchwytu tego regionu nie czyści cudzego bufora.

Wypisanie schodzi do `bork_println_str`. Po bloku, jeśli nie było pobrania, nie ma zwrotu. Powrót ładuje `total` i zwraca. Zdjęcie uchwytów zdejmuje uchwyt funkcji.

Funkcja `add` ładuje dwa parametry, dodaje je instrukcją całkowitą i wraca. Napisów w niej nie ma.

## Plik obiektowy, konsolidacja i proces

Zapis obiektu bierze natywny cel. Na maszynie, na której książka była sprawdzana, był to Linux x86-64. Ustawia trójkę docelową i układ danych, potem pisze plik obiektowy. Konsolidacja woła `clang` z archiwum biblioteki wykonawczej.

Proces zwraca 3. Wypisanie pisze trzy bajty słowa `sum` i nową linię, potem opróżnia bufor wyjścia. Pula buforów w bibliotece wykonawczej żyje w statycznym muteksie. Po zwrocie bufor funkcji wraca na listę. Proces się kończy, więc pula znika razem z pamięcią procesu.

## Czego ten ślad nie pokrywa

Nie było wyniesienia alokacji, promocji, przeniesienia napisu, wycinka, pętli `while`, zwierania ani błędu ucieczki. Każde z nich ma listing w części pierwszej i test w `tests/build.rs` albo w `src/escape.rs`. Ślad miał pokazać udaną drogę przez funkcje graniczne, a nie każdy hak.

Gdy będziesz powtarzał ślad na programie, który deklaruje kawałek napisu wynikiem `concat` i w następnym wierszu przenosi go do zmiennej zewnętrznej, zatrzymaj się dłużej przy wyniesieniu. Pole miejsca alokacji zostanie ustawione, a emisja deklaracji ustawi miejsce przeznaczenia na dom zmiennej zewnętrznej, zanim wyemituje `concat`. Reszta kroków jest ta sama.

## Podsumowanie

- Śledzony program zwraca 3 i drukuje `sum`. Sprawdzenie i `bork build` są zgodne.
- Parser widzi dwie funkcje i zakres. Sprawdzanie typów widzi `i32` i `String`.
- Analiza własności oznacza `total` jako kopię w pętli i `label` jako współdzielenie w bloku.
- Pętla bez alokacji nie dostaje własnego bufora, choć ma węzeł w wydruku.
- Wyniesienie alokacji i analiza ucieczki nie mają tu nic do zrobienia. To też jest wynik.
- Wygenerowany kod woła `bork.add` i `bork_println_str`. Biblioteka wykonawcza dostaje deskryptor literału ze stałej globalnej.
