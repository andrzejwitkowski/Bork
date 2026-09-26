# Rozdział 13. Jak kompilator czyta tekst programu

## Ten rozdział obejmuje

- czym jest lekser i czym jest parser w tym projekcie
- w jakiej kolejności wiążą się operatory
- jakie węzły ma drzewo składni
- po co kompilator przerabia nowe linie wewnątrz nawiasów
- jak błąd czytania staje się komunikatem

## Jedna gramatyka czyta i składa

Lekser zamienia tekst na tokeny, czyli słowa, liczby, operatory i końce wierszy. Parser układa tokeny w drzewo. W Borku obie te rzeczy są zapisane w jednym pliku, `src/parser.lalrpop`. Narzędzie LALRPOP generuje z tej gramatyki kod Rusta. Plik `build.rs` uruchamia to generowanie przy każdej kompilacji kompilatora. Wejście dla reszty programu to funkcja `parse` w `src/lib.rs`. Najpierw normalizuje nowe linie w nawiasach, potem woła parser programu. Błąd ma typ `ParseError` z biblioteki LALRPOP.

Nie ma osobnego pliku z listą tokenów. Lekser jest blokiem dopasowań na początku gramatyki.

## Co lekser pomija, a co zamienia na token

Pomijane są spacje, tabulatory i komentarz `//` do końca wiersza. Znak powrotu karetki też jest odstępem. Dzięki temu zamiana nowych linii wewnątrz nawiasów na odstęp naprawdę ukrywa je przed lekserem. Przejście do nowego wiersza, które nie stoi tuż przed `else`, staje się tokenem `NL`. Słowo `else` wchłania poprzedzające puste wiersze i komentarze, żeby warunek mógł złamać linię przed drugą gałęzią.

Słowa kluczowe w lekserze to `if`, `fun`, `val`, `var`, `for`, `in`, `return`, `Some`, `None`, `move` i `promote`. Reszta, w tym `while` i `true`, idzie zwykłą ścieżką i jest porównywana jako konkretny napis w regule gramatyki.

Liczba całkowita musi zmieścić się w `i64`. W przeciwnym razie gramatyka zgłasza `integer literal out of range for i64`. Liczba z kropką, która nie da się odczytać, daje `invalid float literal`. Napis przechodzi przez funkcję usuwającą sekwencje ucieczki, zapisaną w `src/ast.rs`.

## Kolejność operatorów

Najmocniej wiążą się wywołanie, indeks, wycinek, odczyt pola i przyrostkowe `!!`. Potem jest negacja `!`. Potem mnożenie i dzielenie. Potem dodawanie i odejmowanie. Potem porównania. Potem zakres `..`. Potem koniunkcja `&&`. Potem alternatywa `||`. Najsłabiej wiąże się `?:`.

Operator `?:` jest prawostronnie łączny, bo prawa strona reguły woła z powrotem tę samą regułę. Pozostałe operatory dwuargumentowe są lewostronnie łączne.

Dokument `docs/language.md` w jednym zdaniu o kolejności wymienia wywołania, mnożenie, dodawanie, porównania, zakres i `?:`. Pomija `&&`, `||` i `!`, które w gramatyce są. Ta książka trzyma się gramatyki. Koniunkcja wiąże mocniej niż alternatywa. Obie wiążą słabiej niż porównanie i mocniej niż `?:`.

Indeks i granice wycinka używają reguły porównania, nie pełnego wyrażenia. Bez dodatkowych nawiasów nie włożysz `?:`, `||`, `&&` ani `..` do środka nawiasów kwadratowych.

## Drzewo składni

Plik `src/ast.rs` definiuje drzewo. Program ma listę funkcji. Funkcja ma nazwę, parametry, typ wyniku i ciało. Parametr ma rodzaj wiązania, stały albo zmienny, nazwę z zakresem źródłowym i typ.

Typ w drzewie składni jest albo typem prostym, albo nazwą, albo tablicą o długości, albo typem funkcji. Każdy z nich może mieć znacznik pustej wartości, choć później sprawdzanie typów odrzuca pustą tablicę.

Instrukcja jest blokiem, deklaracją, przypisaniem, pętlą `for`, pętlą `while`, `break`, `continue`, blokiem `move`, powrotem albo wyrażeniem użytym jako instrukcja. Cel przypisania jest albo nazwą, albo indeksem. Inny cel ginie już w gramatyce.

Wyrażenie jest literałem, indeksem, wycinkiem, nazwą, przeniesieniem, promocją, `None`, `Some`, działaniem, negacją, odczytem pola, wywołaniem albo warunkiem. Wywołanie może nieść funkcję dopisaną na końcu. Ta funkcja ma parametry, ciało, znacznik `move` i opcjonalną listę przenoszonych nazw.

Lista przenoszonych nazw jest wartością opcjonalną. Brak listy oznacza zgadywanie. Lista pusta oznacza jawne „nic nie przenoś”. Tej różnicy nie wolno spłaszczyć do jednego pustego wektora.

Zakres źródłowy jest parą `start` i `end` w bajtach. Notatka projektowa pierwszej wersji parsera mówiła, że zakresów nie będzie. To jest nieaktualne. Zakresy są w całym drzewie.

## Po co przerabiać nowe linie

Funkcja w `src/layout.rs` zamienia `\n` na `\r` wewnątrz nawiasów okrągłych, na głębokości nawiasów klamrowych z chwili otwarcia nawiasu. Pomija napisy i komentarze. Długość pliku w bajtach się nie zmienia, więc pozycje komunikatów się nie przesuwają. Testy w `tests/parser.rs` sprawdzają pusty program, próbkę z funkcją na końcu wywołania, przypisanie do indeksu, parametry stałe i zmienne, bloki `move`, komentarze, kolejność operatorów, pętlę `while` i miejsca błędów. To jest pierwszy plik testowy, który warto czytać, gdy ruszasz gramatykę.

## Od błędu parsera do komunikatu

Funkcja `from_parse` w `src/diag.rs` zamienia warianty błędu LALRPOP na fazę `parse`. Błąd zgłoszony przez regułę gramatyki dostaje zakres o początku i końcu równym długości pliku, czyli pozycję na końcu. Dlatego zdanie `assignment target must be a name or name[index]` wskazuje koniec pliku. Poprawka byłaby lokalna albo w tłumaczeniu błędu, albo w akcji gramatyki, która dziś nie niesie pozycji. Nikt jej jeszcze nie zrobił.

Funkcja `frontend::check` przy błędzie składni wraca natychmiast. Sprawdzanie typów i analiza własności się nie wykonują.

## Podsumowanie

- Gramatyka LALRPOP jest jedynym lekserem i jedynym parserem.
- Nowa linia jest tokenem, chyba że wewnątrz nawiasów została zamieniona na odstęp albo stoi przed `else`.
- Brak listy przy `move` oznacza zgadywanie. Pusta lista oznacza świadomą rezygnację ze zgadywania.
- Kolejność operatorów obejmuje negację, koniunkcję i alternatywę, nawet jeśli skrót w dokumentacji języka je pomija.
- Błąd zgłoszony przez regułę gramatyki ma pozycję na końcu pliku.
- Drzewo składni nie niesie typów wywnioskowanych. To robota sprawdzania typów.
