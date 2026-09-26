# Rozdział 3. Z czego składa się plik źródłowy

## Ten rozdział obejmuje

- jak wygląda plik z programem i jak zapisuje się komentarz
- dlaczego nowa linia rozdziela instrukcje
- czym różni się nazwa stała od nazwy zmiennej
- kiedy kilka par nawiasów klamrowych staje się jednym regionem
- dlaczego wywołanie funkcji wolno zapisać w wielu wierszach

## Plik jest listą funkcji

Plik o rozszerzeniu `.bork` zawiera zero lub więcej funkcji. Pusty plik oraz plik złożony z samej pustej linii dają się odczytać jako program bez funkcji. Komentarz zaczyna się od `//` i trwa do końca wiersza. Nie ma komentarza blokowego, atrybutów ani polecenia `import`.

**Listing 3.1.** Komentarz nie zmienia wyniku funkcji.

```bork
fun main(): i32 {
    // komentarz
    val n = 1 // też komentarz
    return n
}
```

Program został zbudowany. Proces kończy się kodem wyjścia 1, bo funkcja zwraca liczbę 1.

Słowa rozpoznawane przez lekser jako osobne tokeny to `if`, `fun`, `val`, `var`, `for`, `in`, `return`, `Some`, `None`, `move` i `promote`. Słowa `while`, `break`, `continue`, `true` i `false` są w gramatyce zapisane wprost. Nie użyjesz ich jako nazw. Słowo `else` ma osobną regułę. Przed nim wolno złamać wiersz i wstawić komentarz, a parser i tak widzi `else`, nie osobny koniec instrukcji. Dzięki temu warunek może mieć drugą gałąź w następnym wierszu.

Nazwa składa się z liter alfabetu łacińskiego, cyfr i znaku podkreślenia, a nie może zaczynać się od cyfry. Polskie znaki w nazwach nie są przyjmowane.

## Nowa linia kończy instrukcję

**Listing 3.2.** Średnik nie jest częścią języka.

Źródło `val n = 1;` kończy się błędem fazy `parse`. Komunikat mówi, że token średnika jest nieoczekiwany i że w tym miejscu parser spodziewa się między innymi nowej linii albo końca bloku.

Pusta linia między instrukcjami jest w porządku, bo kilka przejść do nowego wiersza skleja się w jeden separator. Dwie instrukcje bez tego separatora nie są poprawne. Gdy komunikat wymienia token `NL`, prawie zawsze dwie rzeczy stoją w jednym wierszu albo w kodzie został średnik przyniesiony z C albo z Rusta.

## Nazwy stałe i nazwy zmienne

**Listing 3.3.** `val` wprowadza nazwę, której nie wolno przypisać ponownie. `var` wprowadza nazwę, którą wolno zmienić.

```bork
val n = 1
var count: i32 = 0
var s: String = "hi"
count = count + 1
```

Adnotacja typu po dwukropku jest opcjonalna, gdy wyrażenie po prawej stronie ma znany typ. Przypisanie do nazwy stałej jest błędem sprawdzania typów. Tekst tego błędu był w rozdziale 1. Nazwa wprowadzona w bloku przesłania nazwę z bloku zewnętrznego aż do końca bloku wewnętrznego. Inicjalizator nowej nazwy może jeszcze odczytać przesłanianą nazwę. Program, który na zewnątrz ma `val n = 1`, a w bloku `val n = n + 1` i zwraca to wewnętrzne `n`, został zbudowany i kończy się kodem 2. Drzewo regionów pokazuje w bloku lokalne `n` oraz osobną obserwację, że zewnętrzne `n` zostało skopiowane.

Przypisanie po lewej stronie może dotyczyć tylko nazwy albo elementu tablicy zapisanego jako `nazwa[indeks]`. Pole, wywołanie i dowolne inne wyrażenie po lewej stronie znaku równości nie przechodzą składni. Komunikat gramatyki brzmi `assignment target must be a name or name[index]`. Ten rodzaj błędu, zgłaszany przez samą gramatykę, dostaje pozycję na końcu pliku, a nie przy lewym składniku. To ograniczenie tłumaczenia błędu parsera na diagnostykę, opisane w `diag::from_parse`. Nie jest decyzją, że błąd „jest na końcu programu”.

Słowo `var` albo `val` przy lokalnej nazwie stoi na początku instrukcji i jest obowiązkowe. Nie ma zapisu `n := 1` ani samego `n = 1` w roli deklaracji. Przy parametrze funkcji te słowa są opcjonalne. Rozdział 5 wyjaśnia, co wtedy znaczą.

## Blok jest instrukcją i otwiera region

Samotna para nawiasów klamrowych w ciele funkcji jest instrukcją. Otwiera region o etykiecie `Block` w drzewie regionów. Kilka warstw nawiasów, które nie zawierają nic poza jednym wewnętrznym blokiem, analiza własności skleja w jeden region.

**Listing 3.4.** Trzy pary nawiasów, a w drzewie regionów jeden blok.

```bork
fun main() {
    val n = 1
    {
        {
            {
                val m = n
            }
        }
    }
}
```

Sprawdzenie przechodzi. Wypis drzewa zawiera węzeł `Block (compacted 2 braces)`. Zewnętrzna para nawiasów zostaje regionem. Dwie kolejne, które tylko owijają wewnętrzną treść, znikają z drzewa i zostają licznikiem. To nie jest ozdoba wypisu. Generator kodu i analiza własności muszą zdejmować takie opakowania w ten sam sposób. W przeciwnym razie drzewo regionów rozminie się z drzewem programu używanym przy tłumaczeniu na kod. Funkcja `peel_blocks` występuje w dwóch miejscach, w analizie własności i w reprezentacji pośredniej, a komentarz w kodzie każe trzymać je w zgodzie.

Warunek, pętla, funkcja i funkcja dopisana na końcu wywołania nie sklejają się z otoczeniem. Każde z nich ma w drzewie własną etykietę.

## Wyrażenie zapisane jako instrukcja

Każde wyrażenie może stać samodzielnie jako instrukcja. Jego wartość jest wtedy porzucana. Warunek bez drugiej gałęzi jest typowym przykładem. Służy do sterowania, a gdy nikt nie oczekuje od niego wartości, jego typem jest typ pusty. Typ pusty nazywa się `unit`. Funkcja, która nie deklaruje typu wyniku, też ma wynik `unit`.

**Listing 3.5.** Warunek, który tylko zmienia zmienną.

```bork
fun main(): i32 {
    var n = 0
    if (true) {
        n = 1
    }
    return n
}
```

Program został zbudowany i kończy się kodem 1. Składnia warunku wymaga nawiasów okrągłych wokół wyrażenia logicznego i wymaga bloków w gałęziach. Nie napiszesz `if n > 0 n else 0`. Gdy warunek ma dać wartość, obie gałęzie są blokami i potrzebna jest gałąź `else`. Szczegóły typów są w rozdziale 6.

## Łamanie wiersza wewnątrz nawiasów

Wywołanie i każda para nawiasów okrągłych może zająć wiele wierszy. Zanim parser zobaczy tekst, funkcja `normalize_parenthesized_newlines` w pliku `src/layout.rs` zamienia znak nowej linii na znak powrotu karetki wewnątrz nawiasów, na tej samej głębokości nawiasów klamrowych, na której nawias został otwarty. Pomija przy tym napisy i komentarze. Lekser traktuje powrót karetki jak zwykły odstęp, więc nowa linia w środku listy argumentów nie kończy instrukcji.

**Listing 3.6.** Argumenty funkcji w osobnych wierszach.

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main(): i32 {
    return add(
        1,
        2
    )
}
```

Program został zbudowany. Wynik wynosi 3, więc kod wyjścia też wynosi 3. Zamiana znaku nowej linii na powrót karetki nie zmienia długości pliku w bajtach, bo oba znaki zajmują jeden bajt. Pozycje błędów pozostają tam, gdzie były. Testy w `tests/parser.rs` sprawdzają wielowierszowe wywołanie i stabilność miejsca błędu.

Poza nawiasami okrągłymi nowa linia jest znacząca. Nie ma reguły, która ciągnęłaby wyrażenie do następnego wiersza tylko dlatego, że wiersz skończył się operatorem.

## Czego w składni nie ma

Sprawdziłem trzy zapisy, które wyglądają naturalnie, jeśli przychodzisz z innego języka. Wszystkie kończą się błędem fazy `parse` i kodem wyjścia jeden.

Słowo `struct` na początku pliku daje komunikat, że oczekiwano nowej linii albo słowa `fun`. Tak samo zachowuje się słowo `module`. Zapis `return -1` daje komunikat o nieoczekiwanym tokenie minusa. Minus jest tylko operatorem między dwiema wartościami. Liczbę ujemną da się uzyskać odejmowaniem, na przykład `0 - 1`. Nie ma literału ujemnego. Funkcji nie zagnieżdża się wewnątrz innych funkcji. Funkcje stoją wyłącznie na poziomie programu.

## Podsumowanie

- Plik źródłowy jest listą funkcji. Komentarz to tylko `//` do końca wiersza.
- Instrukcję kończy nowa linia. Średnik jest błędem składni.
- Nazwa wprowadzona przez `val` nie przyjmuje późniejszego przypisania. Nazwa wprowadzona przez `var` przyjmuje. Nazwa w bloku może przesłonić nazwę zewnętrzną i w swoim inicjalizatorze może ją jeszcze odczytać.
- Kilka par nawiasów, które tylko owijają jeden blok, składa się na jeden region.
- Wewnątrz nawiasów okrągłych nowa linia jest odstępem. Poza nimi rozdziela instrukcje.
- Nie ma struktur, modułów ani jednoargumentowego minusa.
