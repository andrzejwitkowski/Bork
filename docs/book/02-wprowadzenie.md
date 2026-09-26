# Rozdział 1. Pierwszy program i sposób jego uruchomienia

## Ten rozdział obejmuje

- czym jest program napisany w Borku
- jak sprawdzić plik, nie budując programu wykonywalnego
- jak zbudować program i jak odczytać jego kod wyjścia
- jak czytać pierwszy komunikat o błędzie
- które konstrukcje język już rozumie, a których generator kodu jeszcze nie tłumaczy

## Program, który wypisuje jeden wiersz

Bork jest językiem kompilowanym do kodu maszynowego. Kompilator czyta plik tekstowy, sprawdza, czy program jest poprawny, i na życzenie tłumaczy go na plik, który da się uruchomić. Składnia jest zbliżona do Kotlina. Funkcję rozpoczyna słowo `fun`, nazwę niezmienną słowo `val`, nazwę zmienną słowo `var`, a grupę instrukcji obejmuje się nawiasami klamrowymi. Pamięcią nie zarządza garbage collector. Sens tej decyzji wyjaśnia następny rozdział. Tutaj celem jest zobaczyć działający program i pierwszy błąd.

**Listing 1.1.** Funkcja `main` wypisuje słowo i przechodzi do nowego wiersza.

```bork
fun main() {
    println("hi")
}
```

Program składa się z funkcji. Nie ma instrukcji zapisanych luzem na poziomie pliku. `main` jest zwykłą funkcją, która przy budowaniu programu wykonywalnego dostaje szczególną rolę. Punkt wejścia procesu nazywa się właśnie tak. Jeśli nie podasz typu wyniku, funkcja nie zwraca wartości. Taki `main` kończy się kodem wyjścia zero. `println` jest funkcją wbudowaną. Wypisuje argument i dodaje znak nowego wiersza. Ten program został zbudowany i uruchomiony. Na standardowym wyjściu pojawił się tekst `hi` oraz nowy wiersz, a proces zakończył się kodem zero.

**Listing 1.2.** Wynik funkcji `main` staje się kodem wyjścia procesu.

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main(): i32 {
    return add(40, 2)
}
```

Funkcja `add` przyjmuje dwie liczby typu `i32` i zwraca ich sumę. Parametr zapisany jako `nazwa: Typ`, bez słowa `val` albo `var`, jest nazwą niezmienną. `main` z zadeklarowanym wynikiem `i32` przekazuje tę liczbę systemowi jako kod wyjścia. Po zbudowaniu i uruchomieniu proces kończy się kodem 42. Innych typów wyniku funkcji `main` generator kodu nie obsługuje. Próba zwrócenia `i64`, `f64` albo `bool` przechodzi sprawdzenie typów, a przy budowaniu dostaje komunikat, że taki wynik nie jest jeszcze obsługiwany.

> **NOTA.** Sprawdzenie pliku bez funkcji `main` może się udać, jeśli reszta programu jest poprawna. Budowanie programu wykonywalnego wymaga `main`. Brak tej funkcji jest błędem fazy generowania kodu, a nie błędem typu.

## Sprawdzenie i zbudowanie

Samą poprawność programu sprawdzisz bez biblioteki LLVM. LLVM to biblioteka, która później tłumaczy sprawdzony program na kod maszynowy. Na tym etapie nie jest potrzebna. Z katalogu repozytorium:

```text
cargo run --bin bork -- sciezka/do/pliku.bork
cargo run --bin bork -- --dump-arenas sciezka/do/pliku.bork
```

Pierwsze polecenie uruchamia sprawdzenie. Obejmuje ono składnię, typy i własność nazw. Drugie polecenie dopisuje na standardowe wyjście drzewo regionów. Komunikaty o błędach idą na standardowe wyjście błędów. Kod zero oznacza brak błędów. Kod jeden oznacza błąd w programie. Kod dwa oznacza złe wywołanie albo problem z odczytem pliku.

Zbudowanie programu wymaga opcji kompilacji `codegen`, biblioteki LLVM 23, kompilatora `clang` i bibliotek, z którymi ta wersja LLVM jest powiązana. Szczegóły instalacji są w pliku `README` repozytorium. Ubuntu 24.04 nie ma LLVM 23 w domyślnym archiwum pakietów. Po złożeniu kompilatora budujesz program tak:

```text
cargo run --features codegen -- build plik.bork
cargo run --features codegen -- build -o wyjscie plik.bork
```

Jeśli nie podasz `-o`, nazwa pliku wynikowego powstaje przez odcięcie rozszerzenia `.bork`. Kompilator odmawia pracy, gdy plik wynikowy nadpisałby plik źródłowy. Komunikat brzmi wtedy `error: output path would overwrite input file`, a kod wyjścia wynosi dwa. Bez opcji `codegen` podpolecenie `build` kończy się informacją, że ta opcja jest wymagana, również kodem dwa.

**Listing 1.3.** Próba zbudowania pliku, w którym nie ma funkcji `main`.

Źródło to jedna funkcja pomocnicza zwracająca liczbę 7. Budowanie kończy się kodem jeden i nie zostawia pliku wykonywalnego. Komunikat, przepisany z uruchomienia, brzmi:

```text
nomain.bork: error: codegen: `fun main` is required to build an executable
```

Słowo `codegen` w komunikacie jest nazwą fazy. Sprawdzenie tego samego pliku, bez budowania, kończy się sukcesem. Język nie wymaga, żeby każdy plik miał `main`. Wymaga tego dopiero tłumaczenie na program wykonywalny.

## Dwa błędy, które pojawiają się na początku

Instrukcje rozdziela się nową linią, nie średnikiem. Dwie instrukcje w jednym wierszu nie są programem.

**Listing 1.4.** Parser odrzuca dwie deklaracje zapisane bez przejścia do nowego wiersza.

```bork
fun main() { val a = 1 val b = 2 }
```

Komunikat zaczyna się od fazy `parse` i mówi, że token `val` jest nieoczekiwany, a wśród rzeczy oczekiwanych jest `NL`, czyli znak nowej linii. Lista oczekiwanych tokenów jest surowa. Pochodzi wprost z generatora parserów i nie została przepisana na zdanie dla człowieka. Średnik jest traktowany tak samo. W wierszu `val n = 1;` kompilator zgłasza nieoczekiwany token średnika.

Drugi częsty błąd dotyczy zmiany nazwy, która miała pozostać stała.

**Listing 1.5.** Przypisanie do nazwy wprowadzonej przez `val`.

```bork
fun main() {
    val n = 1
    n = 2
}
```

Komunikat, z kolumną wskazującą przypisanie, brzmi:

```text
err_val.bork:3:5: error: type: cannot assign to immutable `val` binding `n`
```

Faza `type` pochodzi ze sprawdzania typów. Faza `ownership` pochodzi z analizy własności. Obie mogą pojawić się w jednym uruchomieniu. W wydruku błędy własności stoją przed błędami typów, chociaż sprawdzanie typów wykonuje się wcześniej. Powód tej kolejności jest techniczny i wraca w rozdziale 12. Tutaj wystarczy czytać fazę w komunikacie i nie zakładać, że pierwsza linia jest zawsze pierwszym błędem, który kompilator znalazł w czasie.

## Co język przyjmuje, a czego nie przetłumaczy na kod maszynowy

Poniższy program przechodzi sprawdzenie. Jest to próbka zapisana w kodzie kompilatora jako `MVP_SAMPLE`. Budowanie kończy się kodem jeden. Test `build_rejects_mvp_sample_at_codegen_gate` pilnuje, żeby w komunikacie były słowa `codegen` i `not supported`.

**Listing 1.6.** Funkcja przekazana na końcu wywołania. Sprawdzenie kończy się sukcesem. Budowanie jest odrzucane.

```bork
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main(): i32 {
    val threshold = 5
    var accumulator = 0
    for (i in 0..10) {
        accumulator = action(accumulator, i) { acc, current ->
            if (current > threshold) {
                acc + current
            } else {
                acc
            }
        }
    }
    return accumulator
}
```

To jest prawdziwy fragment języka, a nie martwa reguła gramatyki. Sprawdzanie typów kontroluje liczbę parametrów funkcji dopisanej na końcu i typ jej wyniku. Analiza własności buduje dla niej osobny region. Generator kodu zatrzymuje się wcześniej i wypisuje `trailing closures are not supported by codegen`.

> **OSTRZEŻENIE.** Nie każdy program odrzucony przy budowaniu dostaje taki komunikat. Przekazanie napisu do funkcji napisanej przez programistę przechodzi sprawdzenie, a potem proces kompilatora przerywa się awaryjnie, z kodem 101 i śladem stosu Rusta. Funkcje wbudowane `println` i `concat` tego nie robią. Rozdział 5 i rozdział 17 opisują tę różnicę na konkretnych plikach.

## Drzewo regionów, zanim wyjaśnimy regiony

Dla listingu 1.2 polecenie z `--dump-arenas` wypisuje drzewo. Węzeł `fun add` ma dwie nazwy lokalne, `a` oraz `b`. Węzeł `fun main` nie ma nazw lokalnych, bo jedyną instrukcją jest `return`. Znacznik `Local` przy nazwie oznacza, że nazwa powstała w tym regionie. W rozdziale 8 pojawią się pozostałe znaczniki: kopiowanie, współdzielenie i przeniesienie. Ten sam tekst drzewa pokazuje w edytorze polecenie Bork: Dump Arenas.

## Podsumowanie

- Program w Borku jest listą funkcji, a instrukcje rozdziela nowa linia.
- Polecenie `bork plik.bork` sprawdza program. Polecenie `bork build` tłumaczy obsługiwany podzbiór na plik wykonywalny.
- Funkcja `main` z wynikiem `i32` przekazuje ten wynik jako kod wyjścia. Funkcja `main` bez typu wyniku kończy się zerem.
- Komunikat błędu ma fazę. Na tym etapie spotkasz `parse`, `type` i `codegen`. Faza `ownership` dojdzie w rozdziałach o własności.
- Funkcja dopisana na końcu wywołania jest częścią języka sprawdzanego przez kompilator i nie jest jeszcze tłumaczona na kod maszynowy.
