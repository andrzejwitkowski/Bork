# Rozdział 10. Komunikaty kompilatora i błędy podczas działania

## Ten rozdział obejmuje

- jak czytać linię błędu i co znaczą cztery fazy
- czego język nie oferuje w miejsce wyjątków
- które błędy przerywają kompilację, a które przerywają gotowy program
- dlaczego czasem kompilator kończy się awarią zamiast komunikatem
- w jakiej kolejności warto naprawiać kilka błędów naraz

## Język nie ma instrukcji obsługi błędu

Nie ma `throw`, `try`, `catch` ani typu `Result`. Pytajnik przy typie nie jest modelem błędu. Jest wartością, którą sprawdzenie rozumie, a generowanie kodu odrzuca. Program albo daje się skompilować, albo nie. W czasie działania są trzy awarie, które kod naprawdę wywołuje.

Dzielenie przez zero woła `abort` z biblioteki języka C. Proces kończy się sygnałem przerwania. W powłoce widać kod 134, a na wyjściu błędów nie ma tekstu Borka. Ten sam mechanizm przerywa program, gdy indeks tablicy nie mieści się w długości zapisanej w typie. Trzecia awaria dotyczy alokacji większej niż 4096 bajtów w jednym buforze areny. Biblioteka wykonawcza, napisana w Ruście, przerywa się wtedy komunikatem `arena overflow`, z liczbą potrzebnych bajtów i pojemnością 4096. Program w Borku nie może tego złapać.

> **NOTA.** W kodzie generatora jest także przerwanie przy dzieleniu najmniejszej liczby ze znakiem przez minus jeden. Literału ujemnego nie zapiszesz, więc tej ścieżki nie uruchamiałem z pliku `.bork`. Strażnik w funkcji `guard_int_div` porównuje dzielnik z zerem, a dla typów ze znakiem także parę złożoną z minimum typu i z minus jeden.

## Jak wygląda linia błędu

Funkcja `print_diagnostics` w `src/main.rs` drukuje ścieżkę, numer linii, numer kolumny, słowo `error`, nazwę fazy i treść. Gdy błąd nie ma zakresu w pliku źródłowym, numer linii i kolumny znikają. Zostaje `ścieżka: error: faza: treść`.

Zakres źródłowy, po angielsku span, jest parą pozycji bajtowych w pliku. Kolumna na wydruku liczy znaki od ostatniego przejścia do nowego wiersza, plus jeden. Przy tekście ASCII, a taki jest cały dzisiejszy lekser, znak i bajt wypadają w tym samym miejscu.

Fazy nazywają się `parse`, `ownership`, `type` i `codegen`. W strukturze błędu jest też poziom ostrzeżenia, ale programy, które uruchamiałem, produkują wyłącznie błędy. Wiersz poleceń i tak drukuje słowo `error`.

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

Komunikaty powstają w katalogu `src/typeck`. Rozdziały 4–7 cytowały te, które uruchomiłem. Kilka dotyczy całej funkcji, nie jednego wyrażenia. Brak powrotu na wszystkich ścieżkach, ponowna deklaracja funkcji wbudowanej, nieznana nazwa, nieznana funkcja oraz zła liczba argumentów należą do tej grupy.

Sprawdzanie typów nie zatrzymuje się na pierwszym błędzie. Zbiera listę. Potem i tak uruchamia się analiza własności. Dlatego jeden plik potrafi mieć zarówno fazę `type`, jak i fazę `ownership`. Test `reports_ownership_and_type_together` łączy dodawanie liczby do napisu z użyciem nazwy po przeniesieniu.

## Błędy własności

Mają dwóch autorów. Pierwszym jest analiza nazw. Drugim jest analiza ucieczki. Druga nie startuje, gdy wcześniejsze błędy nie są puste. Przy błędzie typu możesz nie zobaczyć błędu ucieczki, który w poprawionym programie by się pojawił. Najpierw napraw typy i brakujące `move`, potem czytaj komunikaty o czasie życia bajtów.

Drzewo regionów powstaje także przy błędzie własności. Wypis przy użyciu nazwy po `move` pokazuje, że nazwa jest przeniesiona, i pokazuje deklarację, która tej nazwy użyła. Drzewo jest obrazem tego, co przejście zdążyło zbudować. Nie jest świadectwem, że program jest poprawny.

## Błędy generowania kodu

Pojawiają się tylko przy `bork build`. Samo sprawdzenie ich nie produkuje. Zanim powstanie moduł LLVM, funkcja `gate` w `src/codegen/gate.rs` odrzuca konstrukcje, których generator nie umie przetłumaczyć. Należą do nich funkcja dopisana na końcu wywołania, `None`, `Some`, `!!`, operator `?:`, pole inne niż `length` na napisie albo tablicy oraz operator, który nie jest na liście obsługiwanych.

Późniejsza emisja dokłada między innymi brak funkcji `main`, wynik `main` o typie innym niż `i32` i `unit`, wywołanie pośrednie oraz działanie na liczbach zmiennoprzecinkowych. Ostatni tekst potrafi przyjść z dopiskiem `internal arena schedule mismatch`. Ten dopisek powstaje, gdy błąd spaceru po regionach jest opakowywany jako niezgodność harmonogramu. Treść po dwukropku mówi, czego naprawdę brakuje. Brzmi to surowo. Czasem jest wewnętrzną niezgodnością drzew, a czasem tylko opakowaniem zwykłego „jeszcze nie obsługujemy”.

Błąd tej fazy ma kod wyjścia jeden i nie zostawia pliku wykonywalnego. Pilnują tego testy odrzucenia `None` oraz braku `main`.

## Awaria kompilatora

To nie jest komunikat dla programisty Borka. Proces `bork` kończy się kodem 101 i śladem stosu Rusta. Sprawdziłem dwa przypadki. Argument napisowy funkcji użytkownika oraz porównanie wartości `f64` wchodzą w `value_as_int`. Tam wywołanie `into_int_value` zakłada, że wartość LLVM jest liczbą całkowitą. Dla napisu jest strukturą adresu i długości. Dla porównania `f64` jest liczbą zmiennoprzecinkową. Inkwell, czyli biblioteka Rusta, przez którą kompilator woła LLVM, w takiej sytuacji przerywa proces, zamiast zwrócić błąd.

Przyczyna leży w `coerce_value_to_ty`. Dla `bool` wartość jest zwężana. Dla liczby zmiennoprzecinkowej część ścieżek zwraca komunikat, że argument nie jest obsługiwany. Dla reszty kod woła konwersję na liczbę całkowitą bez sprawdzenia, czym wartość naprawdę jest.

> **OSTRZEŻENIE.** Awaria kompilatora na programie, który sprawdzenie uznało za poprawny, jest błędem generatora kodu. Nie czytaj jej jako zakazu języka. Sprawdzanie typów i analiza własności tego zapisu nie zabraniają. Dodatek C trzyma ten przypadek razem z innymi zaległościami.

Błąd narzędzia, a nie programu, kończy się kodem dwa. Należy tu złe polecenie, brak pliku, nakładające się ścieżki, brak `clang` albo brak opcji `codegen`. Tekst zaczyna się od `error:` i nie ma fazy.

## Gdy błędów jest kilka

Kompilator nie wybiera jednego błędu i nie milczy o reszcie. Dostajesz wszystkie, które dana faza zdążyła zebrać. Praktyczna kolejność naprawy jest taka. Najpierw faza `parse`, bo przy błędzie składni reszty nie ma. Potem faza `type`, zwłaszcza nieznane nazwy i niezgodne typy. Potem faza `ownership` dotycząca `move`. Faza `codegen` ma sens dopiero wtedy, gdy sprawdzenie jest czyste. Budowanie najpierw sprawdza program. Przy błędach sprawdzenia w ogóle nie wchodzi w LLVM. Wypisuje te błędy i kończy się kodem jeden.

Sprawdziłem budowanie programu z `None`. Faza to `codegen`, kod wyjścia jeden, pliku wynikowego nie ma. Program, który zwraca napis z regionu wewnętrznego, też kończy budowanie kodem jeden, z tekstem `inner region`, bez pliku wynikowego. Analiza ucieczki jest częścią sprawdzenia, więc budowanie nie udaje, że to problem LLVM.

## Podsumowanie

- Język nie ma wyjątków. Ma komunikaty kompilacji i trzy awarie w czasie działania.
- Linia błędu podaje fazę. Brak zakresu źródłowego usuwa numer linii.
- Faza `ownership` pochodzi albo z analizy nazw, albo z analizy ucieczki. Druga milczy, gdy wcześniej są inne błędy.
- Kontrola przed generowaniem kodu mówi wprost, że konstrukcja nie jest obsługiwana. Część późniejszej emisji mówi, że nie jest obsługiwana jeszcze.
- Przekazanie napisu do funkcji użytkownika nie daje komunikatu. Przerywa proces kompilatora.
- Kod jeden oznacza zły program. Kod dwa oznacza złe wywołanie albo brak narzędzia. Kod 101 oznacza awarię procesu kompilatora.
