# O tej książce

Ta książka uczy języka Bork i jednocześnie objaśnia kod kompilatora napisanego w Ruście. Nie jest listą życzeń. Jeśli jakiejś funkcji nie ma w parserze, w sprawdzaniu typów, w analizie własności albo w generatorze kodu, nie opisuję jej jako części języka. Jeśli występuje tylko w notatkach projektowych z katalogu `docs/superpowers` albo w pliku `TODO.md`, zaznaczam, że jest to plan albo zaległość, a nie gotowa możliwość.

## Dla kogo jest ta książka

Zakładam, że piszesz już w jakimś języku programowania. Najwygodniej będzie, jeśli znasz choć jeden z trzech punktów odniesienia, do których Bork sam się porównuje.

Kotlin albo inny język z blokami, z warunkiem użytym jako wyrażenie i z funkcją dopisywaną na końcu wywołania da ci znajomą składnię. Rust albo C++ da ci słownictwo własności i przeniesienia, nawet jeśli w Borku działa ono inaczej niż sprawdzanie pożyczek. Odrobina wiedzy o kompilatorach wystarczy w części drugiej. Wystarczy wiedzieć, że program najpierw jest czytany jako tekst, potem dostaje strukturę, potem jest sprawdzany, a na końcu zamieniany na kod maszynowy. Nie musisz znać biblioteki Inkwell ani protokołu serwera językowego.

Część pierwszą da się czytać bez zaglądania do plików Rusta. Część druga podaje nazwy funkcji i plików. Część trzecia zakłada, że masz repozytorium otwarte obok książki.

## Jak czytać

Są trzy sensowne przejścia.

Ścieżka języka prowadzi przez przedmowę, rozdziały od 1 do 11 oraz dodatki A i C. Przy każdym listingu, o którym piszę, że został uruchomiony, warto uruchomić go samodzielnie. Listing, o którym piszę, że przechodzi tylko sprawdzenie, kompilator akceptuje poleceniem `bork plik.bork`, ale polecenie `bork build` albo go odrzuca, albo kończy się awarią kompilatora.

Ścieżka kompilatora zaczyna się od rozdziału 2, bo bez powodu istnienia regionów dalsze fazy wyglądają jak zbiór niepowiązanych przejść. Potem idą rozdziały od 12 do 18 i rozdział 20. Rozdział 20 przeprowadza jeden krótki program przez kolejne funkcje kompilatora.

Ścieżka osoby, która chce zmieniać kompilator, to rozdziały od 19 do 22 i dodatek C. Rozdział 21 pokazuje, które testy łapią którą fazę, i jak nowe słowo kluczowe, na przykład `while`, musiało przejść przez wszystkie warstwy.

Komunikaty w książce pochodzą z uruchomienia programu `bork` złożonego w tym środowisku z LLVM 23.1.2. Linia błędu ma postać `plik:linia:kolumna: error: faza: treść`. Kolumna liczy znaki od początku wiersza. Gdy błąd nie ma pozycji w pliku, numer linii znika. Tak jest na przykład przy próbie zwrócenia wyniku funkcji `concat`.

## Umowy typograficzne

Listingi są numerowane w obrębie rozdziału. Pod listingiem zdania omawiają, co program robi i jaki był wynik uruchomienia, jeśli uruchomienie było możliwe. Numer w nawiasie, na przykład `(1)`, odsyła do komentarza w kodzie tylko wtedy, gdy kod naprawdę ma taki znacznik.

Ramki mają trzy role. Nota dopowiada fakt, który łatwo przeoczyć. Wskazówka mówi, co zrobić w praktyce. Ostrzeżenie dotyczy programu, który sprawdzenie akceptuje, a budowanie odrzuca albo na którym kompilator się wywraca. Osobno ostrzeżenie dotyczy zachowania, które przerywa już uruchomiony program.

Kod Borka jest w blokach oznaczonych `bork`. Kod Rusta, który jest częścią kompilatora, jest w blokach `rust` i ma w podpisie ścieżkę pliku. Diagramy są zapisane w Mermaid. W pliku PDF są obrazkami.

## Czego książka nie obiecuje

W tej wersji Bork nie ma modułów, polecenia `import`, pakietów ani przestrzeni nazw. Nie ma struktur, wyliczeń definiowanych przez programistę, cech ani typów ogólnych. Nie ma wyjątków. Nie ma jednoargumentowego minusa, więc nie zapiszesz literału ujemnego. Instrukcji nie rozdziela się średnikiem. Nie ma garbage collectora. Nie ma referencji w stylu `&` i `&mut` z Rusta. Nie ma interpretera, trybu `bork run` ani debuggera. Generator kodu nie tłumaczy funkcji dopisanej na końcu wywołania, wartości `Some` i `None`, operatora `?:` ani asercji `!!`.

Napis przekazany do funkcji napisanej przez programistę jest akceptowany przy sprawdzaniu. Przy budowaniu kompilator przerywa pracę awaryjnie. To błąd kompilatora, a nie reguła języka. Rozdział 17 pokazuje miejsce w kodzie.

## O źródłach tej książki

Rozdziały leżą w katalogu `docs/book`. Przykłady, które uruchamiałem, są w `docs/book/przyklady` razem z plikiem `WYNIKI.md`. Historia commitów projektu jest tłem rozdziału 19. Nie streszczam każdego pull requestu.

Język i kompilator są projektem Andrzeja Witkowskiego. Ta książka jest opisem tego kodu dla innych programistów i nie zmienia semantyki. Gdy dokument `docs/language.md` rozmija się z kompilatorem, opisuję to, co robi kompilator, i mówię, na czym polega różnica. Najważniejszy przykład dotyczy konkatenacji dwóch zmiennych napisowych wewnątrz bloku. Dokument pokazuje wywołanie bez `move`. Kompilator takiego programu nie przyjmuje.
