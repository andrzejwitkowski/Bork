# Rozdział 10. Komunikaty kompilatora i błędy podczas działania

## Ten rozdział obejmuje

- jak czytać linię błędu i co znaczą cztery fazy
- czego język nie oferuje w miejsce wyjątków
- które błędy przerywają kompilację, a które przerywają gotowy program
- jak odróżnić aktualne usterki codegenu od wyników historycznych
- w jakiej kolejności warto naprawiać kilka błędów naraz

## Język nie ma instrukcji obsługi błędu

Nie ma `throw`, `try`, `catch` ani typu `Result`. Pytajnik przy typie nie jest modelem błędu, tylko wartością nullable. Codegen obsługuje wybrane typy i operacje nullable; ograniczenia reprezentacji są w rozdziale 4 i dodatku C. W czasie działania sprawdzenia kończą program m.in. przy dzieleniu przez zero, indeksie poza tablicą oraz `!!` na `None`.

Dzielenie przez zero woła `abort` z biblioteki języka C. Proces kończy się sygnałem przerwania. W powłoce widać kod 134, a na wyjściu błędów nie ma tekstu Borka. Ten sam mechanizm przerywa program, gdy indeks tablicy nie mieści się w długości zapisanej w typie. Trzecia awaria dotyczy alokacji większej niż 4096 bajtów w jednym buforze areny. Biblioteka wykonawcza, napisana w Ruście, przerywa się wtedy komunikatem `arena overflow`, z liczbą potrzebnych bajtów i pojemnością 4096. Program w Borku nie może tego złapać.

> **NOTA.** W kodzie generatora jest także przerwanie przy dzieleniu najmniejszej liczby ze znakiem przez minus jeden. Literału ujemnego nie da się zapisać, więc tej ścieżki nie uruchamiano z pliku `.bork`. Strażnik w funkcji `guard_int_div` porównuje dzielnik z zerem, a dla typów ze znakiem także parę złożoną z minimum typu i z minus jeden.

## Jak wygląda linia błędu

Funkcja `print_diagnostics` w `src/main.rs` drukuje ścieżkę, numer linii, numer kolumny, słowo `error`, nazwę fazy i treść. Gdy błąd nie ma zakresu w pliku źródłowym, numer linii i kolumny znikają. Zostaje `ścieżka: error: faza: treść`.

Zakres źródłowy, po angielsku span, jest parą pozycji bajtowych w pliku. Kolumna na wydruku liczy znaki od ostatniego przejścia do nowego wiersza, plus jeden. Przy tekście ASCII, a taki jest cały dzisiejszy lekser, znak i bajt wypadają w tym samym miejscu.

Fazy nazywają się `parse`, `ownership`, `type` i `codegen`. W strukturze błędu jest też poziom ostrzeżenia, ale programy uruchomione przy pisaniu książki produkują wyłącznie błędy, a wiersz poleceń i tak drukuje słowo `error`.

Serwer edytora dokłada nazwę fazy jeszcze raz, na początku treści, i przelicza bajty na pozycje wymagane przez protokół. Rozdział 18 opisuje różnicę.

## Błędy składni

Tłumaczenie błędu parsera jest w `diag::from_parse`. Niepoprawny znak daje `invalid token`. Nagły koniec pliku daje `unexpected end of file` i listę rzeczy oczekiwanych. Nieoczekiwany token daje `unexpected token` oraz tę samą listę. Dodatkowy token na końcu daje `unexpected extra token`. Błąd zgłoszony wprost przez gramatykę, na przykład zły cel przypisania, dostaje pozycję na końcu pliku.

**Listing 10.1.** Niedomknięta lista parametrów.

Źródło `fun oops(` daje:

```text
err_parse.bork:1:10: error: parse: unexpected end of file; expected r#"[a-zA-Z_][a-zA-Z0-9_]*"#, ")", "val", "var"
```

Lista oczekiwanych rzeczy jest surowa i zawiera wyrażenie regularne nazwy. Przy błędzie składni nie ma drzewa regionów ani dalszych faz. Polecenie `--dump-arenas` nie wypisuje wtedy drzewa.

## Błędy typów

Komunikaty powstają w katalogu `src/typeck`. Rozdziały 4–7 cytowały te, które pojawiły się przy uruchomieniu kompilatora, a kilka dotyczy całej funkcji, nie jednego wyrażenia. Brak powrotu na wszystkich ścieżkach, ponowna deklaracja funkcji wbudowanej, nieznana nazwa, nieznana funkcja oraz zła liczba argumentów należą do tej grupy.

Sprawdzanie typów nie zatrzymuje się na pierwszym błędzie, tylko zbiera całą listę, a potem i tak uruchamia się analiza własności. Dlatego jeden plik potrafi mieć zarówno fazę `type`, jak i fazę `ownership`. Test `reports_ownership_and_type_together` łączy dodawanie liczby do napisu z użyciem nazwy po przeniesieniu.

## Błędy własności

Komunikaty tej fazy powstają w dwóch miejscach: w analizie nazw oraz w analizie ucieczki, która nie startuje, gdy wcześniejsze błędy nie są puste. Przy błędzie typu możesz więc nie zobaczyć błędu ucieczki, który w poprawionym programie by się pojawił. Najpierw napraw typy i brakujące `move`, a dopiero potem czytaj komunikaty o tym, czy bufor z napisem jeszcze istnieje w chwili użycia.

Drzewo regionów powstaje także przy błędzie własności. Wypis przy użyciu nazwy po `move` pokazuje, że nazwa jest przeniesiona, i pokazuje deklarację, która tej nazwy użyła. Drzewo jest obrazem tego, co przejście zdążyło zbudować. Nie jest świadectwem, że program jest poprawny.

## Błędy generowania kodu

Pojawiają się tylko przy `bork build`. Samo sprawdzenie ich nie produkuje. Bramka `gate` w `src/codegen/gate.rs` nadal odrzuca m.in. funkcje dopisane na końcu wywołania i niedozwolone pola. `None`, `Some`, `!!`, `?:`, `?.length` oraz nullable `==`/`!=` nie są już ogólnie odrzucane; dla typów bez reprezentacji generator zwraca błąd fazy `codegen`.

Późniejsza emisja odrzuca m.in. brak `main`, niedozwolony wynik `main` i wywołania pośrednie. Float jako argument funkcji dostaje odmowę `codegen`. Osobny błąd dotyczy porównań floatów: frontend je przyjmuje, ale generator kieruje je do `value_as_int` i panikuje na `FloatValue`. To błąd kompilatora, nie ograniczenie semantyki porównań.

Błąd tej fazy ma kod wyjścia jeden i nie zostawia pliku wykonywalnego. Zestaw nullable buildów w `programs/build/conditionals/` sprawdza obsługiwane przypadki; `main` bez poprawnej sygnatury nadal jest odrzucany.

## Awaria kompilatora

Wynik historyczny z napisem jako argumentem kończył się kodem 101, gdy `value_as_int` bezwarunkowo wywoływał `into_int_value` na strukturze LLVM. Obecny `coerce_value_to_ty` przekazuje deskryptory. Nie przenoś starej paniki na bieżące zachowanie.

Float jako argument funkcji jest jawnie nieobsługiwany. Porównanie floatów ma inną usterkę: `combine_binary_values` sprawdza typ wyniku, a ten jest `bool`, więc operand trafia do `value_as_int`; odtworzenie kończy się paniką `FloatValue`.

> **OSTRZEŻENIE.** Awaria kompilatora na programie, który sprawdzenie uznało za poprawny, jest błędem generatora kodu. Nie czytaj jej jako zakazu języka. Sprawdzanie typów i analiza własności tego zapisu nie zabraniają. Dodatek C trzyma ten przypadek razem z innymi zaległościami.

Błąd narzędzia, a nie programu, kończy się kodem dwa. Należy tu złe polecenie, brak pliku, nakładające się ścieżki, brak `clang` albo brak opcji `codegen`. Tekst zaczyna się od `error:` i nie ma fazy.

## Gdy błędów jest kilka

Kompilator nie wybiera jednego błędu i nie milczy o reszcie. Dostajesz wszystkie, które dana faza zdążyła zebrać. Praktyczna kolejność naprawy jest taka. Najpierw faza `parse`, bo przy błędzie składni reszty nie ma. Potem faza `type`, zwłaszcza nieznane nazwy i niezgodne typy. Potem faza `ownership` dotycząca `move`. Faza `codegen` ma sens dopiero wtedy, gdy sprawdzenie jest czyste. Budowanie najpierw sprawdza program. Przy błędach sprawdzenia w ogóle nie wchodzi w LLVM. Wypisuje te błędy i kończy się kodem jeden.

Budowanie nullable `String` i typów prostych poza `unit` przechodzi dla obsługiwanych operacji, także dla `f32?`. Nullable `unit` i typy funkcji nie mają reprezentacji; zwrot napisu z regionu wewnętrznego nadal jest odrzucany przez analizę ucieczki przed LLVM.

## Podsumowanie

- Język nie ma wyjątków. Ma komunikaty kompilacji i trzy awarie w czasie działania.
- Linia błędu podaje fazę. Brak zakresu źródłowego usuwa numer linii.
- Faza `ownership` pochodzi albo z analizy nazw, albo z analizy ucieczki. Druga milczy, gdy wcześniej są inne błędy.
- Kontrola przed generowaniem kodu mówi wprost, że konstrukcja nie jest obsługiwana. Część późniejszej emisji mówi, że nie jest obsługiwana jeszcze.
- Deskryptory `String` są przekazywane do funkcji użytkownika. Float jako argument jest odrzucany, a porównanie floatów może wywołać panikę kompilatora.
- Kod jeden oznacza błąd programu lub odmowę codegenu. Kod dwa oznacza złe wywołanie albo brak narzędzia. Kod 101 w tabeli starych przykładów jest wynikiem historycznej awarii i nie powinien być przypisywany bieżącemu codegenowi bez ponownego testu.
