# Rozdział 22. Ćwiczenia

## Ten rozdział obejmuje

- ćwiczenia na samo sprawdzenie programu, bez generowania kodu
- ćwiczenia na `bork build` i na kod wyjścia procesu
- ćwiczenia czytania kodu kompilatora
- odpowiedzi, które da się zestawić z komunikatem albo z plikiem

Odpowiedzi są w tym samym rozdziale, pod treścią zadań. Najpierw uruchom kompilator. Komunikaty z części pierwszej wolno potraktować jako ściągę.

## Część A. Samo sprawdzenie

**A1.** Napisz funkcję `main`, która zwraca `i32` i nie ma ani jednego `return`. Jaki jest dokładny początek komunikatu, czyli faza i pierwsze słowa?

**A2.** Zadeklaruj `var s = "hi"` i `var t = s`. Popraw program najmniejszą zmianą tak, żeby sprawdzenie przeszło, a nazwy `s` nie dało się użyć niżej.

**A3.** Poniższy program jest poprawny i odejmuje trzy od zera. Zamień `0 - 3` na `-3` i nazwij fazę błędu.

```bork
fun main(): i32 {
    val n = 0 - 3
    return n
}
```

**A4.** Program `val a: [i32; 2] = [1, 2, 3]` daje więcej niż jeden komunikat. Ile ich jest, jakiej są fazy i czego dotyczą? Nie poprawiaj programu.

**A5.** Napisz pętlę `for` po zakresie typu `i64`. Jakiego typu granic wymaga sprawdzanie?

**A6.** Weź fragment z dokumentacji języka, dopisz brakującą deklarację `out` i spraw, żeby sprawdzenie przeszło na dwa sposoby: przez stałe `val` oraz przez `move`.

```bork
var left = "L"
var right = "R"
{
    var piece = concat(left, right)
    out = move piece
}
```

**A7.** W funkcji zwracającej `String` napisz `return concat("a", "b")`. Czy komunikat ma numer linii?

## Część B. Zbudowany program

Uruchom `bork build` i powstały proces.

**B1.** Ile wynosi kod wyjścia sumy `for (i in 0..5)`, gdy w ciele jest `total = total + i`, a `total` zaczyna się od zera?

**B2.** Pętla `while` podbija `i` od zera i robi `break` przy `i == 3`, zanim zwiększy licznik. Co zwraca `main`?

**B3.** W funkcji `main` bez zadeklarowanego typu wyniku jest samo `println("hi")`. Co jest na standardowym wyjściu i jaki jest kod wyjścia?

**B4.** Program zwraca `1 / 0`. Czy `bork build` kończy się błędem? Co robi proces?

**B5.** Program ma `val a = [10, 20, 30]` i odczyt `a[9]`. Czy sprawdzenie przechodzi?

**B6.** Weź listing 20.1 i zmień zakres na `0..4`. Jaki kod wyjścia przewidujesz, zanim uruchomisz program, i dlaczego?

**B7.** Program to `fun main(): i64 { return 1 }`. Czy pada sprawdzenie, czy dopiero budowanie, i jaka jest faza?

## Część C. Kod kompilatora

**C1.** Wskaż plik i funkcję, które ustawiają kolejność wypisu: najpierw komunikaty analizy własności, potem komunikaty typów.

**C2.** Gdzie jest pojemność 4096? Podaj dwa pliki. Który wykonuje się w procesie użytkownika?

**C3.** Dlaczego `--dump-arenas` przy urwanym `fun oops(` nie drukuje drzewa?

**C4.** Pole przechwycenia bloku `move` bywa puste albo bywa pustą listą. Który wariant zgaduje nazwy?

**C5.** Dlaczego reprezentacja pośrednia nie ma pola z numerem regionu? Skąd generator kodu bierze region?

**C6.** Która funkcja w `src/sema/policy.rs` zwraca współdzielenie?

**C7.** Co trzeba zmienić, żeby przekazanie napisu do funkcji użytkownika dało komunikat zamiast awarii kompilatora? Nie musisz pisać poprawki. Nazwij funkcję, która się wywraca, i powiedz, czemu kontrola przed generowaniem kodu tego nie łapie.

**C8.** Masz nowy operator dwuargumentowy, którego LLVM jeszcze nie tłumaczy. Wymień warstwy z rozdziału 21 w kolejności i zaznacz, która jest obowiązkowa, żeby nie było awarii kompilatora.

## Odpowiedzi do części A

**A1.** Początek komunikatu to `type: function must return a value of type i32 on all paths`. Zakres bywa pusty, więc numer linii może zniknąć z prefiksu.

**A2.** Zapis `var t = move s` przenosi własność. Przy gołym `s` komunikat każe użyć `move s`, żeby przenieść własność.

**A3.** Zapis `return -3` jest fazą `parse` i mówi o nieoczekiwanym tokenie. Zapis `0 - 3` jest odejmowaniem. Dla `main` zwracającego `i32` sprawdzenie jest zadowolone. Kod wyjścia procesu dla wartości ujemnej owija się w zakresie `i32`.

**A4.** Są dwie diagnostyki fazy `type`. Literał ma 3 elementy, a oczekiwano 2. Inicjalizator ma typ `[i32; 3]`, a oczekiwano `[i32; 2]`.

**A5.** Komunikat mówi, że granice zakresu muszą mieć typ `i32`.

**A6.** Na zewnątrz bloku deklarujesz `var out` z jakimś napisem początkowym, na przykład `"prefix"`. Albo `left` i `right` są stałymi `val`, albo wywołanie ma postać `concat(move left, move right)`. Wynikowy napis to `LR`, a nie `prefixLR`, bo `concat` zastępuje deskryptor, a nie dokleja znaki do starego bufora zmiennej `out`.

**A7.** Nie. Analiza ucieczki stawia pusty zakres. Zostaje komunikat fazy `ownership` o zwracaniu wyniku `concat`, bez numeru linii.

## Odpowiedzi do części B

**B1.** Kod wyjścia to 10.

**B2.** Kod wyjścia to 3.

**B3.** Standardowe wyjście to `hi` i nowa linia. Kod to 0, bo `main` bez typu zwraca `unit`, a generowanie kodu zamienia to na `i32` równe zero.

**B4.** Budowanie się udaje. Proces kończy się sygnałem przerwania, w powłoce kodem 134.

**B5.** Sprawdzenie przechodzi, ale proces jest przerywany, bo indeks nie jest częścią typu, natomiast częścią typu jest długość.

**B6.** Suma `0 + 1 + 2 + 3` wynosi 6, bo zakres `0..4` jest otwarty z prawej strony.

**B7.** Sprawdzenie przechodzi. Budowanie zgłasza fazę `codegen` i mówi, że `main` zwracające `i64` nie jest jeszcze obsługiwane.

## Odpowiedzi do części C

**C1.** Robi to `frontend::check` w `src/frontend.rs`. Sprawdzanie typów wykonuje się wcześniej i zwraca komunikaty. Analiza własności zwraca własne błędy. Wektor buduje się najpierw z nich, a dopiero potem dopisuje błędy typów.

**C2.** Stała `ARENA_CAPACITY` jest w `crates/bork_runtime/src/lib.rs` i ten plik wykonuje się w procesie użytkownika. Tę samą pojemność opisuje `src/arena.rs`, ale to tylko model w kompilatorze, którego analiza własności nie woła.

**C3.** Przy błędzie składni pole raportu w wyniku sprawdzenia jest puste. Wiersz poleceń drukuje drzewo tylko wtedy, gdy raport jest obecny.

**C4.** Brak listy zgaduje nazwy. Pusta lista to jawne `move ()` i oznacza, że nic nie jest przenoszone.

**C5.** Komentarz w `src/hir/mod.rs` mówi, że regiony żyją w raporcie z analizy własności, a generator kodu bierze je we wspólnym przejściu `region_walk`, zsynchronizowanym z reprezentacją pośrednią.

**C6.** Współdzielenie zwraca `classify_use`, w gałęzi wiązania stałego, po sprawdzeniu, że typ nie jest kopiowalny i że region nie jest ten sam.

**C7.** Wywraca się `value_as_int`, przez `into_int_value`, wołane z `coerce_value_to_ty` przy emisji wywołania po przejściu `region_walk`. Kontrola przed generowaniem kodu ogląda kształt reprezentacji pośredniej: czy jest `Some`, funkcja na końcu wywołania, niedozwolony operator. Nie pyta, czy argument jest strukturą LLVM. Wywołanie z argumentem napisowym wygląda jak zwykłe wywołanie, więc kontrola milczy.

**C8.** Warstwy z rozdziału 21 idą w tej kolejności. Najpierw jest gramatyka i drzewo składni, potem sprawdzanie typów, potem analiza własności, gdy operator przenosi nazwy albo je współdzieli, potem kontrola czasu życia napisu i wyniesienie alokacji, gdy operator obchodzi się z napisem albo tablicą, potem wspólne przejście reprezentacji pośredniej i raportu regionów, gdy operator otwiera region, a na końcu kontrola przed generowaniem kodu, emisja i test w `tests/build.rs`. Obowiązkowa, dopóki emisja nie ma gałęzi dla tego operatora, jest właśnie kontrola przed generowaniem kodu. Bez niej dopasowanie wpadnie albo w komunikat o braku wsparcia, albo w ścieżkę liczby i w złe rzutowanie wartości LLVM, czyli w awarię kompilatora zamiast w diagnostykę.

## Podsumowanie

- Ćwiczenia A kończą się na tekście komunikatu. Ćwiczenia B kończą się na kodzie procesu.
- Ćwiczenia C wskazują pliki, a nie ogólniki.
- Zakres `0..n` jest otwarty z prawej strony. Suma `0..5` to 10. Suma `0..4` to 6.
- Kontrola przed generowaniem kodu nie zastępuje rzutowania argumentu wywołania. Stąd osobna odpowiedź C7.
