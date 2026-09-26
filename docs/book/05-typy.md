# Rozdział 4. Typy danych i ich znaczenie

## Ten rozdział obejmuje

- jakie typy wolno napisać i które nazwy są tylko skrótami
- które wartości kompilator kopiuje, a których nie
- jak literał dostaje typ
- jak zapisuje się brak wartości i typ funkcji
- jakie komunikaty pojawiają się, gdy typy do siebie nie pasują

## Nazwy typów

Typ zapisuje się po dwukropku, przy parametrze, przy wyniku funkcji albo przy deklaracji nazwy. Część nazw jest skrótem. Kompilator zamienia skrót na typ właściwy już przy budowie drzewa składni, w funkcji `Type::from_ident`. W dalszych fazach nie ma osobnego typu o nazwie `Int`. Jest trzydziestodwubitowa liczba całkowita ze znakiem.

| Zapis w programie | Typ po zamianie |
|---|---|
| `i8`, `i16`, `i32`, `i64` | liczba całkowita ze znakiem o podanej szerokości |
| `u8`, `u16`, `u32`, `u64` | liczba całkowita bez znaku o podanej szerokości |
| `Int` | `i32` |
| `Long` | `i64` |
| `Byte` | `u8` |
| `Float` | `f32` |
| `Double` | `f64` |
| `f32`, `f64` | liczba zmiennoprzecinkowa |
| `bool` | wartość logiczna, literały `true` i `false` |
| `unit` | typ pusty |
| `String` | napis |
| `[T; N]` | tablica o długości `N` |
| `T?` | wartość, która może być pusta |
| `(A, B) -> R` | typ funkcji |

**Listing 4.1.** Skróty typów są przyjmowane i dają się zbudować, dopóki wartości zmiennoprzecinkowych tylko przechowujesz.

```bork
fun main(): i32 {
    val n: Int = 1
    val wide: Long = 2
    val byte: Byte = 3
    val f: Float = 1.5
    val d: Double = 2.25
    val flag: bool = true
    return n
}
```

Program został zbudowany i kończy się kodem 1, bo zwracane jest `n`. Samo zapisanie liczb zmiennoprzecinkowych w lokalnych nazwach generator kodu obsługuje. Użycie ich w dodawaniu albo w porównaniu jest osobną, niedokończoną ścieżką. Ten listing ich nie używa, dlatego plik wykonywalny powstaje. Rozdział 17 opisuje, co dzieje się przy porównaniu.

Każda inna nazwa typu, na przykład `Point` albo `Unit` zapisane wielką literą, daje błąd `unknown named type`. Słowo `unit` małymi literami jest typem pustym. Wielka litera `Unit` nie jest skrótem.

## Które wartości są kopiowane

Wartość jest kopiowana tylko wtedy, gdy jest typem prostym i nie jest pusta. Poza kopiowaniem zostają napis, każda tablica, każdy typ funkcji oraz cokolwiek z pytajnikiem, także `i32?`. Ta sama odpowiedź obowiązuje przy sprawdzaniu typów i przy analizie własności. Typ, którego nie udało się ustalić, też nie jest traktowany jako kopiowany. Dlatego błąd typu potrafi pociągnąć dodatkowy błąd własności. Drugiego komunikatu nie należy ignorować tylko dlatego, że pierwszy już coś zgłosił.

## Literały

Literał całkowity jest w drzewie składni liczbą ze znakiem o szerokości 64 bitów. Typ, który dostanie w programie, zależy od kontekstu. Jeśli kontekst jest typem całkowitym, literał przyjmuje ten typ. Bez kontekstu staje się `i32`.

**Listing 4.2.** Literał dopasowany do `i64`. Sprawdzenie przechodzi. Budowanie odrzuca wynik funkcji `main`.

```bork
fun main(): i64 {
    val n: i64 = 1
    return n
}
```

Komunikat budowania brzmi `main returning i64 is not supported by codegen yet`. Sprawdzanie typów jest zadowolone. Ograniczenie dotyczy tylko tego, co wolno zwrócić z `main`, a nie samego typu `i64` w funkcji pomocniczej. Funkcja, która odejmuje dwie wartości `i64` i porównuje wynik, została zbudowana. Proces kończy się kodem 7. Przykład jest w zestawie programów do rozdziału 17.

Literał `1.5` bez adnotacji staje się `f64`. Z adnotacją `Float` albo `f32` zostaje liczbą o pojedynczej precyzji. Nie ma przyrostka w rodzaju `1.5f32`.

Literał napisu używa cudzysłowu. Sekwencje `\n`, `\r`, `\t`, `\\` i `\"` są zamieniane na znak nowej linii, powrót karetki, tabulację, ukośnik i cudzysłów.

**Listing 4.3.** Napis ze znakami specjalnymi.

```bork
fun main() {
    println("a\nb\t\"c\"")
}
```

Po uruchomieniu na wyjściu jest litera `a`, nowy wiersz, litera `b`, tabulacja, cudzysłów, litera `c`, cudzysłów i jeszcze jeden nowy wiersz.

Operator `+` nie skleja napisów. Wyrażenie `"a" + "b"` daje błąd fazy `type`: operandy arytmetyczne muszą mieć ten sam typ liczbowy, a kompilator widzi dwa napisy. Do sklejania służy funkcja wbudowana `concat`. Jej zasady pamięci są w rozdziale 9.

## Działania na liczbach i porównania

Operatory `+`, `-`, `*` i `/` wymagają dwóch operandów tego samego typu liczbowego. Nie ma automatycznego rozszerzenia `i32` do `i64`.

**Listing 4.4.** Dodawanie `i64` i `i32` jest błędem, i to błędem podwójnym.

```bork
fun main(): i32 {
    val a: i64 = 1
    val b: i32 = 2
    return a + b
}
```

Pierwszy komunikat mówi, że operandy mają typy `i64` i `i32`. Drugi, na tym samym miejscu, mówi, że zwracana wartość ma typ `i64`, a oczekiwano `i32`. Drugi komunikat jest skutkiem pierwszego. Przy niezgodnych operandach sprawdzanie typów zostawia typ lewej strony jako typ całego wyrażenia, a ten typ nie pasuje do wyniku funkcji.

Porównania `>`, `<`, `>=` i `<=` wymagają tego samego niepustego typu liczbowego. Dwie wartości `bool` dają komunikat, że uporządkowane porównanie wymaga typu liczbowego. Operatory `==` i `!=` wymagają identycznego typu, ale typ może dopuszczać brak wartości. Zapis `val n: i32? = None` oraz porównanie `n == None` przechodzi sprawdzenie. Budowanie odrzuca `None`.

Operatory `&&`, `||` i `!` działają na wartościach logicznych. Koniunkcja i alternatywa nie liczą prawej strony, gdy lewa już rozstrzyga wynik. Widać to w kodzie maszynowym i w teście `builds_logical_short_circuit`. Listing w rozdziale 6 uruchamia taki program.

Dzielenie całkowite przez zero przerywa program. Program z `return 1 / 0` został zbudowany: budowanie kończy się sukcesem, a uruchomiony proces dostaje sygnał przerwania i w powłoce widać kod 134, bo na standardowym wyjściu błędów nie ma komunikatu Borka, tylko wywołanie funkcji `abort` z biblioteki języka C. W kodzie generatora jest także strażnik przed dzieleniem najmniejszej liczby typu ze znakiem przez minus jeden. Literału minus jeden nie da się napisać, więc tej drugiej ścieżki nie uruchamiano z poziomu programu w Borku, choć strażnik w funkcji `guard_int_div` w kodzie jest.

> **NOTA.** Liczby zmiennoprzecinkowe wolno dodawać i porównywać na etapie sprawdzania typów, ale przy budowaniu dodawanie `f32` daje komunikat, że działanie zmiennoprzecinkowe nie jest jeszcze obsługiwane. Porównanie `f64`, uruchomione na tej ścieżce, nie daje komunikatu, bo proces kompilatora przerywa się awaryjnie: wartość w reprezentacji LLVM jest liczbą zmiennoprzecinkową, a fragment kodu oczekuje liczby całkowitej. Arytmetykę zmiennoprzecinkową traktuj jako sprawdzaną, a nie jako tłumaczoną na program.

## Brak wartości

Zapis `T?` oznacza, że wartość typu `T` może nie istnieć. Dotyczy to typów prostych, napisu i typu funkcji. Tablica z pytajnikiem daje się zapisać w składni i jest odrzucana komunikatem `nullable array types [T]? are not supported`.

Wartość obecną buduje `Some(x)`, a brak wartości zapisuje `None`. Samo `None`, bez oczekiwanego typu, nie przechodzi. Komunikat brzmi `cannot infer type of None`.

Operator `?:` wymaga, żeby lewa strona mogła być pusta. Prawa strona musi pasować do typu po zdjęciu pytajnika. Zapis `1 ?: 0` przy `n` typu `i32` daje `left operand of ?: must be nullable, got i32`. Operator `!!` wymaga wartości, która może być pusta, i daje typ bez pytajnika. Na zwykłym `i32` dostaniesz `operand of !! must be nullable, got i32`.

**Listing 4.5.** Napis, który może nie istnieć, oraz długość odczytana tylko wtedy, gdy napis jest. Sprawdzenie przechodzi. Budowanie odrzuca każdy `?:`, `Some` i `None`.

```bork
fun processUser(name: String?, score: i32): i32 {
    val fallbackName: String = name ?: "Guest"
    val verifiedUser: String? = Some(fallbackName)
    val emptyMiddle: String? = None
    val verifiedLength: i32 = verifiedUser?.length ?: 0
    if (verifiedLength > 0) {
        return score
    }
    return 0
}
```

Zapis `?.` czyta pole tylko wtedy, gdy wartość po lewej nie jest pusta. Na typie, który pusty być nie może, sprawdzanie typów każe użyć zwykłej kropki. Jedyne pole napisu i tablicy nazywa się `length` i ma typ `i32`. Dla tablicy ta długość jest liczbą wpisaną w typ, a nie osobnym licznikiem trzymanym obok danych. Próba odczytu pola `foo` z napisu daje `unknown field foo on type String`.

> **OSTRZEŻENIE.** Budowanie listingu 4.5 wypisuje osobny błąd fazy `codegen` dla każdego `?:`, `Some` i `None`. Nie ma częściowej obsługi braku wartości w gotowym programie. Kontrola przed generowaniem kodu odcina te konstrukcje, zanim powstanie moduł LLVM.

Typ funkcji też może dopuszczać brak wartości. Zapis `val f: ((i32) -> i32)? = None` przechodzi sprawdzenie. Wywołanie takiej nazwy bez `!!` nie jest tym, co generator kodu umie przetłumaczyć na instrukcje. Po `!!` kontrola przed generowaniem kodu i tak odrzuca asercję.

## Typ funkcji i nawiasy

Typ funkcji zapisuje się w nawiasach, na przykład `(i32, i32) -> i32`. Wariant, który może być pusty, wymaga dodatkowych nawiasów: `((i32) -> i32)?`. Gramatyka odrzuca pytajnik przy nawiasach, które nie są typem funkcji. Komunikaty brzmią `nullable parentheses require one function type` oraz `only function types may be parenthesized for nullability`.

Nie ma krotek jako wartości. Nawias przy typie albo grupuje typ funkcji, albo w wyrażeniu grupuje działanie. Nie napiszesz `(1, 2)` jako pary liczb.

## Typ nieznany

Gdy wyrażenie jest już błędne, sprawdzanie typów podstawia typ nieznany. Kolejne niezgodności z tym typem są pomijane, żeby jeden błąd nie produkował długiej kaskady. Analiza własności tego pominięcia nie dziedziczy. Typ, którego nie da się przenieść do jej środowiska, zachowuje się jak typ niekopiowany. Własność nadal może zgłosić błąd.

## Podsumowanie

- Skróty `Int`, `Long`, `Byte`, `Float` i `Double` znikają już przy czytaniu programu. Dalej kompilator widzi typy `i32`, `i64`, `u8`, `f32` i `f64`.
- Kopiowane są niepuste typy proste. Napis, tablica, typ funkcji i każda wartość z pytajnikiem kopiowane nie są.
- Nie ma automatycznego rozszerzania liczb i nie ma operatora `+` dla napisów.
- `None` potrzebuje oczekiwanego typu. Operatory `?:` i `!!` wymagają wartości, która może być pusta.
- Jedyne pole napisu i tablicy nazywa się `length`.
- Brak wartości i liczby zmiennoprzecinkowe są sprawdzane. Generator kodu albo je odrzuca komunikatem, albo przy porównaniu `f64` przerywa się awaryjnie.
