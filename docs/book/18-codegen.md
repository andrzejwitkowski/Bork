# Rozdział 17. Jak powstaje kod maszynowy

## Ten rozdział obejmuje

- co kontrola przed generowaniem kodu przepuszcza, a co odrzuca komunikatem
- jak wygląda moduł LLVM i jak nazywają się funkcje
- jak emisja traktuje liczby, napisy, tablice i regiony
- jakie funkcje ma biblioteka wykonawcza i jak program jest z nią konsolidowany
- które programy przechodzą kontrolę, a potem wywracają kompilator

## Kontrola przed generowaniem kodu odrzuca konstrukcje, których emisja nie zna

Zanim powstanie moduł LLVM, funkcja `gate` w pliku `src/codegen/gate.rs` schodzi po reprezentacji pośredniej i zbiera komunikaty fazy `codegen`. Nie próbuje przetłumaczyć programu częściowo. Pierwsze trafienie zostaje komunikatem. Reszta konstrukcji i tak jest odwiedzana, więc jeden plik może dostać kilka komunikatów o braku wsparcia.

Bez własnego komunikatu tej kontroli przechodzą literały, nazwy, tablice, indeks, wycinek, arytmetyka całkowita, porównania liczb całkowitych, koniunkcja, alternatywa, zakres, negacja, wywołanie bez funkcji dopisanej na końcu, warunek oraz pole `length` na typie trzymanym w arenie.

Odrzucane są między innymi funkcja dopisana na końcu wywołania, słowa `None` i `Some`, operator `?:`, wykrzykniki `!!` oraz pole inne niż `length`. Dokładne teksty są w rozdziale 10. Ta kontrola nie wie o awarii, która powstaje później, gdy argumentem wywołania jest napis. Taki program przechodzi kontrolę i wywraca się w emisji. Plik `TODO.md` prosi, żeby kontrola była zsynchronizowana z tym, co emisja umie, albo żeby istniał test mówiący, które programy poprawne dla sprawdzania są świadomie odrzucane. Listy dla napisu przekazanego do funkcji użytkownika nie ma.

## Moduł wymaga funkcji main i deklaruje pozostałe funkcje wcześniej

Funkcja `emit_module` w `src/codegen/llvm/mod.rs` wymaga funkcji o nazwie `main`. Potem tworzy moduł LLVM o nazwie `bork` i budowniczego instrukcji. Deklaruje każdą funkcję z reprezentacji pośredniej, zanim wyemituje którekolwiek ciało. Dzięki temu wywołanie funkcji zdefiniowanej niżej w pliku ma już symbol. Następnie emiter regionów dostaje drzewo regionów i deklaracje wywołań biblioteki wykonawczej. Potem emitowane są ciała. Na końcu emiter regionów kończy pracę, a moduł jest weryfikowany.

Symbol `main` jest funkcją `main` w konwencji C. W LLVM ma typ `i32` bez parametrów. Wynik `unit` i tak zwraca stałe zero. Inna funkcja nazywa się `bork.` z dopisaną nazwą z programu i ma powiązanie wewnętrzne. Nie jest eksportowana.

Funkcja `main`, która zwraca coś poza `i32` i `unit`, odpada komunikatem, że taki wynik nie jest jeszcze obsługiwany przez generowanie kodu. Sprawdziłem to dla `i64`, `f64` i `bool`. Parametry funkcji `main` są odrzucane już przy deklaracji.

## Liczby, napisy i miejsce przeznaczenia

Liczba całkowita jest wartością LLVM o odpowiadającej szerokości. Wartość `bool` jest liczbą całkowitą. Liczba zmiennoprzecinkowa, tam gdzie emisja w ogóle dojdzie do instrukcji arytmetycznej, używa operacji zmiennoprzecinkowych LLVM. Dodawanie `f32` po przejściu `region_walk` zwraca błąd: wewnętrzna niezgodność harmonogramu, a w treści informacja, że dodawanie zmiennoprzecinkowe po tym przejściu nie jest jeszcze obsługiwane. Porównanie `f64` wpada w funkcję, która oczekuje liczby całkowitej, i kompilator kończy się awarią. Arytmetyka `i64` w funkcji pomocniczej, porównana potem w `main`, działa. Program z odejmowaniem dwóch wartości `i64` i progiem w warunku kończy się kodem 7. Ograniczenie liczb zmiennoprzecinkowych nie jest ograniczeniem szerokich liczb całkowitych.

Napis i tablica są strukturą z wskaźnika i długości typu `i64`. Literał napisu jest prywatną stałą globalną, a w wartości ląduje stały deskryptor. Pole `length` w języku ma typ `i32`. W deskryptorze długość jest typu `i64`.

Lokalny slot to alokacja na stosie funkcji, instrukcja `alloca`, w bloku wejścia. Dla typu trzymanego w arenie slot pamięta identyfikator regionu, w którym bajty powinny żyć. Na czas emisji wyniku przypisania, powrotu i wywołania `concat` emiter ustawia miejsce przeznaczenia alokacji. Na czas argumentów to miejsce czyści. Dzięki temu bajty wyniku idą do areny odbiorcy, a podwyrażenia nie alokują się wszystkie w tym samym miejscu. Rozdział 9 opisał tę regułę od strony języka.

Przeniesienie i promocja deskryptora wołają kopiowanie do areny celu. Dla tablicy jest osobna funkcja kopiująca. Wyniesienie alokacji sprawia, że kopii czasem nie trzeba, bo bajty już powstały we właściwej arenie. Osobnego przejścia, które usuwałoby zbędne kopiowanie pamięci, nie ma. Model pamięci wymienia je jako rzecz do zrobienia.

## Regiony w wygenerowanym kodzie są stosem uchwytów

Emiter regionów tłumaczy wejście, czyszczenie i wyjście na wywołania biblioteki wykonawczej. Wejście woła `bork_arena_push` i kładzie uchwyt na stos. Czyszczenie woła `bork_arena_reset`: wskaźnik w buforze wraca do zera, a bufor zostaje. Wyjście woła `bork_arena_pop` i zwraca bufor do puli. Na martwym punkcie wstawienia, na przykład po instrukcji powrotu, czyszczenie i zdjęcie są pomijane.

Powrót z funkcji zdejmuje uchwyty, które funkcja jeszcze trzyma. Historia poprawek w repozytorium dotyczyła podwójnego zdjęcia albo zostawienia uchwytu. Testy generowania kodu liczą pary wejść i wyjść. Gdy ruszasz instrukcję `return`, uruchom te testy, nie tylko gotową binarkę.

Alokacja użytkowa to `bork_arena_alloc`. Bierze uchwyt, rozmiar i wyrównanie. Wyrównanie musi być potęgą dwójki. Biblioteka wykonawcza przerywa program, gdy koniec alokacji przekroczy 4096 bajtów.

## Co emisja naprawdę tłumaczy

Tłumaczenie wyrażeń jest w plikach `src/codegen/llvm/expr.rs` oraz `src/codegen/llvm/array/emit.rs`. Następujące zachowania zostały sprawdzone zbudowanym programem.

Arytmetyka `i32` i `i64` oraz porównania liczb całkowitych działają. Dzielenie ma strażnika, który woła `abort`, gdy dzielnik jest zerem, a także przy skrajnym przypadku minimalnej liczby całkowitej dzielonej przez minus jeden. Warunek jest albo wartością scaloną z dwóch gałęzi, albo sterowaniem ze skokiem. Pętle `for` i `while` obsługują `break` i `continue`. Koniunkcja i alternatywa zwierają się. `println` i `print` wypisują liczby, literały napisowe i napisy lokalne w `main`. `concat` dwóch napisów, które da się załadować jako struktury w `main`, buduje jeden bufor w miejscu przeznaczenia i kopiuje do niego oba argumenty. Nie ma osobnej areny tymczasowej na czas zwykłego wywołania. Literał tablicy, indeks ze strażnikiem, wycinek jako przesunięcie wskaźnika plus nowy deskryptor oraz pole `length` działają. Przypisanie elementu tablicy, w tym elementu napisowego przez `move` w `main`, też działa.

Wywołanie funkcji użytkownika idzie albo przez emisję, która najpierw liczy argumenty, albo przez ścieżkę po przejściu `region_walk`, która dostaje gotowe wartości. Ta druga ścieżka woła `coerce_value_to_ty`. Dla `bool` zwęża wartość do liczby całkowitej, dla liczby zmiennoprzecinkowej zwraca błąd, że argument wywołania po tym przejściu nie jest obsługiwany, a dla reszty woła `value_as_int`. Ta funkcja zakłada, że wartość LLVM jest liczbą całkowitą, i woła `into_int_value` bez sprawdzenia wariantu, więc struktura napisu powoduje awarię kompilatora. W tej rewizji jest to linia 901 w `src/codegen/llvm/expr.rs`. Ślad biblioteki Inkwell nie jest kontraktem języka, tylko objawem tej luki.

Wywołanie, którego wywoływaną rzeczą nie jest nazwa, daje komunikat, że wywołania pośrednie nie są jeszcze obsługiwane, o ile ścieżka w ogóle tam wejdzie. Funkcja dopisana na końcu wywołania zwykle odpada wcześniej, w kontroli przed generowaniem kodu.

## Biblioteka wykonawcza trzyma pulę buforów

Plik `crates/bork_runtime/src/lib.rs` trzyma pulę w muteksie, czyli w blokadzie, którą inicjuje się raz. Pobranie zdejmuje bufor z wektora albo alokuje nowy. Zwrot czyści wskaźnik i odkłada bufor. Regiony żywe w tym samym czasie dostają różne bufory. Regiony sąsiednie w czasie mogą dostać ten sam.

Funkcje widoczne z C są następujące. `bork_arena_push` zwraca wskaźnik na bufor z puli. `bork_arena_reset` zeruje przesunięcie i zostawia bufor. `bork_arena_pop` zwraca bufor do puli. `bork_arena_alloc` przesuwa wskaźnik i zwraca adres. `bork_print_i64` i `bork_println_i64` piszą liczbę na standardowe wyjście i opróżniają bufor. `bork_print_str` i `bork_println_str` piszą napis podany wskaźnikiem i długością.

Nie ma symbolu `bork_abort`. Generator kodu deklaruje `abort` z biblioteki C.

Testy biblioteki wykonawczej sprawdzają wyrównanie i przepełnienie. Są w tym samym pliku, pod warunkiem kompilacji testów, i wchodzą w `cargo test --workspace` nawet bez opcji `codegen` kompilatora. Archiwum do konsolidacji powstaje dopiero przy tej opcji, w pliku `build.rs`.

## Konsolidacja używa clang i statycznego archiwum

Plik `src/codegen/link.rs` woła `clang`. W uproszczeniu polecenie łączy plik obiektowy z archiwum `libbork_runtime.a` oraz z bibliotekami `gcc_s`, `util`, `rt`, `pthread`, `m` i `dl`. Ścieżka archiwum pochodzi ze zmiennej środowiskowej `BORK_RUNTIME_LIB` albo z wartości wkompilowanej pod tą samą nazwą. Plik `build.rs` ustawia ścieżkę wyszukiwania bibliotek na katalog, w którym leży sama binarka `bork`, żeby `libLLVM` mogło leżeć obok niej. Skrypt `scripts/bundle-llvm.sh` kopiuje używany plik `libLLVM`.

Maszyna, która tylko uruchamia skompilowany program w Borku, nie potrzebuje LLVM. Maszyna, która uruchamia `bork build`, potrzebuje `clang`. Jeśli biblioteka LLVM nie została spakowana obok binarki, potrzebuje też instalacji LLVM 23.

## Co zostało uruchomione przy pisaniu tej książki

Kompilator złożony z opcją `codegen`, na LLVM 23.1.2, zbudował i uruchomił między innymi zwrot 42 z dodawania, sumę pętli równą 10, pętlę `while` z `break` o wyniku 3, pętlę z `continue` o sumie 8, wypisanie `hi`, współdzielenie napisu, przeniesienie napisu dające `ab`, konkatenację dającą `LR`, promocję dającą `temp`, przypisanie napisu, sekwencje ucieczki, dzielenie `8/2` z kodem 4, tablicę z wycinkiem i kodem 12, wycinek drukujący `20` i `2`, indeks poza zakresem oraz dzielenie przez zero jako przerwanie procesu, a także brak funkcji `main` jako komunikat. Wyniki są powtórzone w `docs/book/przyklady/WYNIKI.md`.

## Podsumowanie

- Kontrola przed generowaniem kodu odcina wartości puste, operator `?:`, wykrzykniki `!!` i funkcję dopisaną na końcu wywołania zwykłym komunikatem.
- Nie odcina napisu jako argumentu funkcji użytkownika. Emisja wtedy kończy się awarią kompilatora.
- `main` to funkcja `main` w konwencji C, zwracająca `i32`. Pozostałe funkcje nazywają się `bork` z kropką i nazwą.
- Regiony w LLVM to stos uchwytów do biblioteki wykonawczej. Pętla czyści bufor, a nie oddaje go do puli przy każdym obrocie.
- Napis i tablica to struktura ze wskaźnika i długości. Literał napisu leży w stałej globalnej.
- Program jest konsolidowany przez `clang` ze statyczną biblioteką wykonawczą. Sam kompilator ładuje `libLLVM` w wersji 23.
