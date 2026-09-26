# Rozdział 5. Funkcje, parametry i wywołania

## Ten rozdział obejmuje

- jak zadeklarować funkcję, parametr i typ wyniku
- kiedy przy argumentie trzeba napisać `move`
- co robią trzy funkcje wbudowane
- jak dopisać funkcję na końcu wywołania
- dlaczego napis przekazany do własnej funkcji wywraca budowanie

## Kształt funkcji

Funkcję zapisuje się jako `fun nazwa(parametry): TypWyniku`, a potem blok ciała. Pominięty typ wyniku oznacza typ pusty `unit`, a nie nazwę `Unit`. Parametr ma jedną z trzech postaci: `nazwa: Typ`, `val nazwa: Typ` albo `var nazwa: Typ`. Postać bez słowa jest stała, tak jak `val`.

**Listing 5.1.** Dwa rodzaje parametrów. Sprawdzenie przechodzi. W reprezentacji pośredniej pierwszy parametr jest stały, drugi zmienny, a wynik ma typ `i32`.

```bork
fun add(val a: Int, var b: Int): Int {
    return a
}
```

Słowo `var` przy parametrze znaczy, że wywołujący, który przekazuje wartość niekopiowaną, musi ją oddać słowem `move`. Dla `i32` różnica nie boli, bo liczba jest kopiowana. Dla napisu różnica jest sprawdzana.

Ciało funkcji jest blokiem. Ostatnie wyrażenie w bloku nie staje się samo wynikiem funkcji. Wynik przekazuje się słowem `return`. Jeśli funkcja deklaruje wynik inny niż `unit`, każda ścieżka musi wykonać `return`.

**Listing 5.2.** Funkcja z zadeklarowanym wynikiem, która do niego nie wraca.

```bork
fun f(): i32 {
    val n = 1
}

fun main(): i32 {
    return f()
}
```

Komunikat nie ma numeru linii, bo błąd dotyczy całej funkcji:

```text
missing_ret.bork: error: type: function must return a value of type i32 on all paths
```

Samotne `return` bez wartości ma typ `unit`. W funkcji o wyniku `i32` dostaniesz informację, że pusty powrót ma typ `unit`, a oczekiwano `i32`. `return` z wartością sprawdza zgodność z zadeklarowanym wynikiem.

Nie ma przeciążania. Nazwy funkcji są trzymane w mapie po samym identyfikatorze. Komentarz w `src/sema/env.rs` mówi wprost, że to jest uproszczenie obecnej wersji i że lokalne przesłonięcie wywoływanej funkcji nie jest obsłużone. Wywołanie funkcji zdefiniowanej niżej w pliku działa, bo sygnatury są zbierane z całego programu, zanim sprawdzane są ciała.

## Własność na granicy wywołania

Dla wartości, która nie jest kopiowana, analiza własności stosuje prostą tabelę. Stałą nazwę wolno przekazać do parametru stałego. Przekazanie jej do parametru zmiennego wymaga `move`. Zmienną nazwę trzeba przenieść zarówno do parametru stałego, jak i do zmiennego. Literał, wywołanie `concat` i inne wyrażenie, które dopiero tworzy wartość, nie wymaga `move`.

**Listing 5.3.** Literał napisu przekazany do parametru zmiennego. Sprawdzenie przechodzi. Budowanie przerywa kompilator.

```bork
fun f(var a: String) {
    println(a)
}

fun main() {
    f("hello")
}
```

Brak słowa `move` jest zgodny z regułą sprawdzania. Świeży literał nie jest nazwą, którą trzeba przenieść. Generator kodu tej reguły nie dotrzymuje. Proces `bork build` kończy się kodem 101. Ślad wskazuje `src/codegen/llvm/expr.rs`, funkcję `value_as_int`, i mówi, że znaleziono strukturę, a oczekiwano liczby całkowitej. Ta sama awaria występuje przy `f(move s)` oraz przy przekazaniu stałego napisu do parametru funkcji użytkownika. `println` i `concat` działają, bo mają własne fragmenty generatora, a nie ogólną ścieżkę argumentu. Dopóki ta dziura istnieje, funkcja, która ma przejść przez `bork build`, powinna przyjmować i zwracać liczby. Napisy zostawiaj w `main` i przekazuj je do `print`, `println` albo `concat`.

Zwrócenie napisu, który już żyje na poziomie funkcji, sprawdzenie i budowanie akceptują.

**Listing 5.4.** Zwrot lokalnego napisu i jego wypisanie. Program został uruchomiony. Na wyjściu jest `hi` oraz nowy wiersz.

```bork
fun shout(): String {
    var s = "hi"
    return s
}

fun main() {
    println(shout())
}
```

Zapis `return "hi"` też działa i daje ten sam wydruk. Zapis `return name` dla parametru przechodzi sprawdzenie. Wywołanie `println(greet("Ada"))`, w którym `greet` przyjmuje napis, znowu przerywa kompilator na argumencie, nie na instrukcji `return`.

Zapis `return concat("a", "b")` jest odrzucany przy sprawdzaniu. Komunikat nie ma numeru linii:

```text
err_ret_concat.bork: error: ownership: returning the result of `concat` is not supported yet: returned `String` bytes must outlive the callee arena
```

Podobny komunikat dotyczy `return move`. Tekst radzi użyć `return` ze zwykłą nazwą parametru albo nazwy lokalnej, albo zwrócić literał.

## Funkcje wbudowane

Trzy nazwy nie są funkcjami, które wolno zdefiniować. Próba deklaracji jest błędem typu, zanim ciało w ogóle ma znaczenie. Dla `fun println(n: i32)` komunikat brzmi `cannot redefine builtin function println`.

`print` wypisuje jeden argument i opróżnia bufor wyjścia. `println` robi to samo i dodaje nowy wiersz. Argumentem może być `i32`, `i64` albo napis, chociaż w wewnętrznej tabeli sygnatura nominalna mówi o jednym `i32`. To jest specjalny przypadek, nie ogólna zasada, że każdy parametr `i32` przyjmie napis. `concat` przyjmuje dwa napisy i zwraca jeden nowy. Bufor wyniku powstaje w arenie miejsca, do którego wynik jest zapisywany. Gdy takiego miejsca nie ma, powstaje w arenie bieżącego bloku.

**Listing 5.5.** Kolejność wypisywania. Program został uruchomiony. Na wyjściu jest dokładnie cyfra 1, nowy wiersz, słowo `tail` i cyfra 7, bez końcowego nowego wiersza.

```bork
fun main() {
    println(1)
    print("tail")
    print(7)
}
```

## Funkcja dopisana na końcu wywołania

Ostatni argument może być blokiem stojącym po nawiasie wywołania. Parametry tego bloku zapisuje się przed strzałką `->`.

**Listing 5.6.** Ostatni parametr ma typ funkcji. Sprawdzenie przechodzi. Budowanie odrzuca konstrukcję.

```bork
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main(): i32 {
    val threshold = 5
    return action(1, 2) { acc, current ->
        if (current > threshold) { acc + current } else { acc }
    }
}
```

Sprawdzanie typów pilnuje trzech rzeczy. Ostatni parametr wywoływanej funkcji musi być typem funkcji. Liczba parametrów bloku musi się zgadzać. Typ wyniku bloku musi pasować do typu wyniku tego parametru. Komunikaty mówią o tym wprost, po angielsku, słowami `trailing closure`. Wywołanie, którego celem nie jest nazwa funkcji, dostaje komunikat `only named functions can be called`. Generator kodu i tak odrzuca wywołanie pośrednie osobnym tekstem.

Analiza własności nie dostaje typów parametrów takiego bloku. Wpisuje im typ nieznany. Skutek widać w drzewie regionów. Parametry bloku są lokalne, a ich odczyt w gałęzi warunku bywa oznaczony jako współdzielenie, mimo że `Int` jest kopiowany. Program mimo to przechodzi sprawdzenie. Drzewo jest w tym miejscu mylące. Nie jest to błąd twojego programu.

Bloku funkcji nie zapiszesz w stałej poza wywołaniem. Gramatyka ma taką funkcję tylko jako argument końcowy albo jako blok `move`. Typ funkcji istnieje, ale nie ma osobnego literału funkcji.

## Trzy formy przeniesienia przy bloku

Zwykły blok po wywołaniu niczego nie przenosi. Odczyt zewnętrznej zmiennej napisowej jest wtedy błędem. Zapis `move ()` przed blokiem też niczego nie przenosi, ale robi to jawnie. Zapis `move (a, b)` przenosi dokładnie wymienione nazwy. Zapis `move` bez nawiasu przenosi każdą żywą, niekopiowaną nazwę z regionu zewnętrznego, której blok używa.

To samo da się napisać jako instrukcję, bez wywołania. Rozdział 8 ma zbudowane programy dla tych instrukcji. Formy stojące po wywołaniu odpadają przy budowaniu komunikatem `trailing closures are not supported by codegen`.

Wnioskowanie, które nazwy przenieść, nie patrzy na wyrażenia `move nazwa` i `promote nazwa` jak na zwykłe użycie nazwy. W przeciwnym razie przeniesienie zapisane w wyrażeniu byłoby liczone drugi raz jako przeniesienie całego bloku.

## Co z tego wynika dla programów, które mają się zbudować

Ciało wywołanej funkcji ma własne regiony. Argumenty są liczone w regionie wywołującego. Wynik liczbowy wraca w rejestrze. Wynik napisowy nie dostaje dziś bufora należącego do wywołującego, dlatego analiza ucieczki zabrania zwrócić napis utworzony w regionie tej funkcji. Listing 5.4 działa, bo napis `"hi"` jest literałem albo wartością na poziomie funkcji, a `println` tylko czyta adres i długość.

> **WSKAZÓWKA.** W programie, który ma przejść przez `bork build`, trzymaj funkcje przy liczbach, a napisy wypisuj z `main`. To nie jest zalecenie na zawsze, tylko obejście błędu w funkcji `coerce_value_to_ty`, która wartość inną niż `bool` próbuje potraktować jako liczbę całkowitą. Liczba zmiennoprzecinkowa na części ścieżek dostaje komunikat, że nie jest obsługiwana, a napis kończy się awarią kompilatora.

## Podsumowanie

- Wynik funkcji zapisuje się słowem `return`. Brak powrotu na którejś ścieżce jest błędem typu.
- Parametr bez `val` i bez `var` jest stały. Parametr zmienny wymaga `move`, gdy przekazuje się istniejącą nazwę wartości niekopiowanej.
- Świeży literał nie wymaga `move`, ale napis jako argument funkcji użytkownika wywraca budowanie.
- `print`, `println` i `concat` są wbudowane. Nie wolno ich zadeklarować ponownie.
- Funkcja dopisana na końcu wywołania jest sprawdzana i nie jest tłumaczona na kod maszynowy.
- Analiza własności nie zna typów parametrów takiej funkcji, więc drzewo regionów potrafi pokazać współdzielenie tam, gdzie typ jest kopiowany.
