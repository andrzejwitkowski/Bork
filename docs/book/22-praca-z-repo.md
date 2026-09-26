# Rozdział 21. Testy, szukanie przyczyny błędu i nowa konstrukcja języka

## Ten rozdział obejmuje

- które polecenie uruchamia który zestaw testów
- gdzie szukać, gdy binarka się wywraca, a sprawdzenie milczy
- w jakiej kolejności warstw doszła do języka pętla `while`
- które braki są już zapisane w `TODO.md`
- czego nie robić w pierwszej poprawce

## Dwa polecenia testów, bo generowanie kodu jest osobną opcją

Z pliku `README` i z `.github/workflows/ci.yml` wynikają dwa polecenia. Pierwsze to `cargo test --workspace`. Drugie to `cargo test --workspace --features codegen`.

Pierwsze polecenie nie składa LLVM. Łapie parser, analizę własności, sprawdzanie typów, analizę ucieczki przez `frontend::check`, serwer edytora oraz testy jednostkowe puli w bibliotece wykonawczej. Kontrolę przed generowaniem kodu łapie tylko o tyle, o ile test tej kontroli jest za warunkiem opcji `codegen`. Zadanie ciągłej integracji o nazwie `codegen` jest osobne. Instaluje LLVM 23 i `clang`, puszcza przestrzeń roboczą z tą opcją i jeszcze sprawdza spakowaną bibliotekę LLVM.

Testy w `tests/parser.rs` utwierdzają tokeny, nowe linie, słowo `move`, kolejność operatorów i zakresy błędów. Testy w `src/sema/tests` utwierdzają własność nazw. Testy w `src/typeck/tests` utwierdzają typy, często przez pełne sprawdzenie. Plik `tests/arrays.rs` sprawdza tablice bez uruchamiania binarki. Plik `tests/build.rs` buduje program i sprawdza kod wyjścia albo standardowe wyjście. Plik `src/codegen/gate.rs` trzyma teksty o braku wsparcia. Plik `src/lsp.rs` sprawdza pozycje, podpowiedź i wydruk. Skrzynka `bork_runtime` sprawdza przesunięcie wskaźnika, wyrównanie i przepełnienie. Test `tools/bork-lsp-extension/test/server.test.js` pilnuje ścieżki binarki wobec `cargo run`.

Plik `tests/build.rs` jest kompilowany tylko z opcją `codegen`. Bez niej znika z zestawu. Nie dziw się, że samo `cargo test` nie widzi testu sumy pętli `for`.

Testy budowania piszą źródło do katalogu tymczasowego cargo, wołają binarkę `bork` z podpoleceniem `build` i ścieżką `-o`, a potem uruchamiają powstały program. Gdy dodajesz zachowanie widoczne w czasie działania, dopisuj tu oczekiwany kod albo tekst wyjścia. Test jednostkowy samej emisji tego nie zastępuje. Listingi w tej książce opierają się na tym samym kontrakcie.

## Gdzie szukać, gdy wynik nie zgadza się z oczekiwaniem

Gdy drzewo regionów nie ma węzła, którego oczekujesz, błąd jest w zejściu analizy własności, w `src/sema/walk.rs`, a nie w LLVM. Gdy drzewo jest, a `bork build` kończy się awarią kompilatora, błąd jest w emisji albo w funkcji `coerce_value_to_ty`. Wydruk drzewa dostaniesz poleceniem `cargo run --bin bork -- --dump-arenas plik.bork`.

Prefiks fazy mówi, którego katalogu nie czytać w pierwszej kolejności. Faza `parse` prowadzi do gramatyki. Faza `type` prowadzi do `src/typeck`. Faza `ownership` bez słów o użyciu po przeniesieniu i bez słów o braku kopiowania często pochodzi z `src/escape.rs`, a nie z `src/sema/policy.rs`. Słowa o regionie wewnętrznym, o `concat` i o wartości przeniesionej albo promowanej są z analizy ucieczki. Faza `codegen` prowadzi do kontroli albo do emisji.

Komunikat o wewnętrznej niezgodności harmonogramu regionów znaczy, że wspólny spacer nie dostał dziecka węzła albo dostał je w złym miejscu. Porównaj etykietę w analizie własności z miejscem w typie `RegionSite`. Najczęstsza przyczyna przy nowej konstrukcji jest taka, że analiza własności otwiera region, a gość emisji o nim nie wie, albo odwrotnie.

Awaria w `into_int_value` znaczy, że wartość LLVM nie jest liczbą całkowitą. Patrz, jaki typ ma wyrażenie. Napis i liczba zmiennoprzecinkowa na ścieżce `emit_call_with_values` są znanymi ofiarami. Poprawka należy do `coerce_value_to_ty`: dopasowanie wariantu wartości, a dla struktury przekazanie deskryptora bez rzutowania na `i64`. Nie zakrywaj tego łapaniem paniki.

Przerwanie procesu użytkownika bez komunikatu kompilatora to zwykle indeks albo dzielenie. Uruchom binarkę pod debuggerem i zobacz, czy stanęła w `abort`. Przepełnienie bufora regionu daje panikę Rusta z tekstem `arena overflow`, bo biblioteka wykonawcza jest pisana w Ruście i ta panika nie jest łapana przez program w Borku.

Weryfikacja modułu LLVM po emisji wywraca budowanie, gdy reprezentacja pośrednia LLVM jest zepsuta, na przykład przy złym typie scalenia gałęzi albo przy braku powrotu. To lepsze niż cichy zły kod. Gdy weryfikacja pada, błędu nie szukaj w `clang`.

> **WSKAZÓWKA.** Binarka `bork` bez opcji `codegen` jest szybka, gdy poprawiasz komunikaty sprawdzenia. Włączaj tę opcję dopiero wtedy, gdy sprawdzenie jest czyste i chcesz zobaczyć kontrolę przed generowaniem kodu albo sam proces.

## Jak do języka doszła pętla while

Poniżej jest kolejność, którą widać po fakcie na `while`, `break` i `continue`. Nie jest to przepis skopiowany z planu. Jest to kolejność warstw, które musiały ruszyć, bo każda z nich w kodzie o pętli `while` wie.

Najpierw gramatyka. Dochodzi terminal i produkcja instrukcji. Dla `while` warunek jest w nawiasach, a ciało jest blokiem. `break` i `continue` są instrukcjami z zakresem źródłowym. Test w `tests/parser.rs` sprawdza, że nowa linia działa i że słowo nie jest identyfikatorem w złym miejscu.

Potem drzewo składni dostaje warianty instrukcji `while`, `break` i `continue`. Na tym etapie nie ma typów.

Potem sprawdzanie typów. Warunek oczekuje `bool`. Głębokość pętli rośnie tak samo jak przy `for`. `break` poza pętlą jest błędem typu. Dochodzi nowy wariant instrukcji w reprezentacji pośredniej. Test trzyma dokładny tekst komunikatu.

Potem analiza własności. Region `WhileLoop` otwiera się zwykłą ścieżką. Obowiązuje ten sam zakaz przeniesienia co przy `for`. Jeśli zapomnisz zakazu, istniejący test przeniesienia nazwy zewnętrznej w pętli nie pokryje `while`, dopóki go nie skopiujesz. Warto go skopiować.

Jeśli ciało jest blokiem, nie dokładaj drugiego regionu w sprawdzaniu typów. Analiza własności i reprezentacja pośrednia muszą mieć po jednym dziecku. Inaczej spacer się nie zepnie.

Nowa instrukcja musi być odwiedzona w analizie ucieczki i w wyniesieniu alokacji. Inaczej deklaracja wewnątrz `while` nie będzie widziana. Dla tej pętli wystarczy zejść w ciało tak, jak przy `for`.

We wspólnym spacerze dochodzi miejsce regionu i hak pętli `while`. Oznaczenie pobrania bufora zaczyna widzieć alokacje w ciele. Bez tego wygenerowany kod i raport przestaną pasować do siebie w chwili, gdy ciało alokuje napis.

Pętla `while` nie potrzebuje odmowy w kontroli przed generowaniem kodu, bo emisja ją umie. Nowa konstrukcja, której emisja nie umie, musi dostać odmowę w `src/codegen/gate.rs`, żeby użytkownik dostał komunikat zamiast awarii kompilatora. Lekcją jest napis jako argument funkcji użytkownika. Kontrola go nie zna, a emisja kończy się awarią.

Emisja dokłada bloki podstawowe: nagłówek, ciało, zatrzask, czyszczenie bufora, jeśli region go pobrał, oraz `break` jako skok do bloku wyjścia. Potem test w `tests/build.rs` ma konkretny kod wyjścia. Dla `while` z `break` przy wartości 3 jest to 3.

Na końcu dokument. `docs/language.md`, a jeśli ruszasz pamięć, także `docs/memory-model.md`. Plik `TODO.md` prosi, żeby dokument nadążał za kontrolą przed generowaniem kodu. Ta książka opisuje jedną rewizję i nie zaktualizuje się sama. Przy zmianie semantyki aktualizuj co najmniej `docs/language.md`.

Konstrukcja, która jest tylko skrótem składniowym, może skończyć się w parserze zamianą na istniejące drzewo. W tym repozytorium prawie nic się tak nie dzieje. Wyrażeniowe `move` i regionalne `move` są osobnymi węzłami. Nowy skrót lepiej zostawić osobnym węzłem, jeśli ma własny komunikat.

## Braki, które są już zapisane

Plik `TODO.md` wymienia między innymi rozcięcie gościa generowania kodu, żeby emiter funkcji nie był jednocześnie gościem spaceru regionów. Wymienia kursor regionów zamiast makra, gdy makro dalej urośnie. Wymienia test, że harmonogram pętli to jedno wejście, wiele czyszczeń i jedno wyjście. Wymienia synchronizację kontroli przed generowaniem kodu z frontendem. Wymienia błędy spaceru zawsze jako błąd z komunikatem, bez gubienia tekstu użytkownika w opakowaniu o niezgodności harmonogramu. Wymienia więcej testów par wejście-wyjście na zagnieżdżonym warunku i na miejscach przeznaczenia napisów. Prosi też, żeby nie commitować luźnych plików `smoke.bork` w korzeniu repozytorium.

Komunikat o niezgodności harmonogramu wokół liczb zmiennoprzecinkowych jest przykładem dwóch z tych punktów. Tekst użytkownika, że dodawanie zmiennoprzecinkowe nie jest obsługiwane, utonął w opakowaniu. Kontrola przed generowaniem kodu w ogóle nie powiedziała, że liczba zmiennoprzecinkowa nie jest tłumaczona.

## Czego nie robić w pierwszej poprawce

Nie zaczynaj od LLVM. Dopisz test parsera albo sprawdzania typów, który na czerwono nazywa zachowanie, i doprowadź `frontend::check` do tego tekstu. Odmowę w kontroli przed generowaniem kodu dodaj w tym samym patchu, jeśli emisji nie będzie. Brak takiej odmowy przy nowej składni zamienia przyszłą awarię kompilatora w coś, co użytkownik zobaczy dopiero jako panikę.

Nie zmieniaj `peel_blocks` tylko w jednym z dwóch miejsc.

Nie naprawiaj awarii `into_int_value` przez odrzucenie wszystkich napisów w kontroli, jeśli w `main` napisy działają. To obcięłoby listingi, które są legalne. Wąskie miejsce to rzutowanie argumentu wywołania.

## Podsumowanie

- Polecenie `cargo test --workspace` nie uruchamia LLVM. Opcja `codegen` uruchamia testy w `tests/build.rs`.
- Wydruk regionów debuguje analizę własności. Awaria `into_int_value` debuguje emisję. Niezgodność harmonogramu debuguje wspólny spacer.
- Nowa konstrukcja idzie warstwami: gramatyka, drzewo składni, sprawdzanie typów, analiza własności, ucieczka i wyniesienie, spacer, kontrola, emisja, test binarki.
- Jeśli emisji nie ma, odmowa w kontroli przed generowaniem kodu jest obowiązkowa.
- Plik `TODO.md` jest listą znanych braków. Sprawdź ją, zanim opiszesz różnicę dokumentu i kodu jako własne odkrycie.
