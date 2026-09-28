# Dodatek C. Czego kompilator jeszcze nie potrafi

Ten dodatek wymienia ograniczenia aktualnego kodu i rozbieżności między kodem a dokumentacją. Awarie przywołane niżej są wynikami historycznymi, chyba że opis wyraźnie mówi, że zostały ponownie sprawdzone. Nic z tej listy nie jest propozycją naprawy.

## Rzeczy świadomie niezaimplementowane

Aktualny kompilator ma ograniczone pożyczki `&T`: dla niekopiowanych typów, w parametrach i stałych widokach, z jawnym `&nazwa` przy wywołaniu. Nie ma pełnego borrow checkera, `&mut`, zwrotu referencji ani przechowywania referencji w polach. Działają reborrow przy bezpośrednim wywołaniu wewnątrz `if`/pętli oraz odczyt i zapis elementów przez `&[T; N]`.

Pozostałe ograniczenia z `src/escape.rs`, `docs/language.md` i `docs/memory-model.md`: nie ma areny wyniku po stronie wywołującego, więc zwrot wyniku `concat` i napisu z regionu wewnętrznego jest odrzucany; `promote` nie działa przy `return`. Wyniesienie alokacji rozpoznaje tylko określone, sąsiednie instrukcje. Nullable `String?` i typy proste poza `unit` mają codegen dla `Some`, `None`, `!!`, `?:` oraz `==`/`!=`. Nullable tablice odrzuca sprawdzanie typów, a nullable `unit` i typy funkcji nie mają reprezentacji. Generowanie kodu nadal nie tłumaczy funkcji dopisanej na końcu wywołania ani wywołań pośrednich. Porównania floatów przechodzą sprawdzanie typów, ale obecnie panikują w codegenie; float jako argument funkcji jest odrzucany. `main` nie przyjmuje parametrów i nie zwraca typu innego niż `i32` lub `unit`. Nie ma modułów, struktur, typów ogólnych ani wyjątków.

Plik `TODO.md` wymienia prace nad samym kompilatorem, a nie nad semantyką języka. Są to rozcięcie gościa generowania kodu, kursor regionów zamiast makra, test zgodności kontroli przed generowaniem kodu ze sprawdzeniem, więcej testów par wejście-wyjście oraz uporządkowanie planów w `docs/superpowers/plans/`.

## Gdzie dokument mówi co innego niż kod

Przykład `concat(left, right)` przy dwóch zmiennych napisowych w `docs/language.md` jest pokazany jako wzorzec. Sprawdzenie odrzuca go komunikatem, że `left` nie jest kopiowalne i trzeba je przenieść do bloku. Zdanie o kolejności operatorów w tym samym dokumencie pomija `&&`, `||` i `!`. Gramatyka te piętra ma.

Notatka projektowa z 23 września 2026 o reprezentacji pośredniej i LLVM mówi o LLVM 18 i o kolejności „najpierw własność, potem typy”. Kod używa LLVM 23, a sprawdzanie typów wykonuje się przed analizą własności. Specyfikacja serwera edytora mówi o komunikatach parsera. Kod woła pełne `frontend::check`. Specyfikacja pierwszej wersji regionów mówi o braku pełnego sprawdzania typów. Sprawdzanie typów jest. Nagłówek w `README` mówi o diagnostyce składni, a akapit niżej i kod robią pełne sprawdzenie. Specyfikacja parsera mówi o wyniku `Unit` i o braku zakresów źródłowych. W kodzie typ nazywa się `unit` i zakresy są. Dokument języka mówi, że dzielenie minimalnej liczby całkowitej przez minus jeden przerywa program. Strażnik w `guard_int_div` ten przypadek obsługuje, ale literału ujemnego nie da się zapisać, bo nie ma jednoargumentowego minusa.

## Awarie i ostre krawędzie, które zostały uruchomione

Kompilator był złożony z opcją `codegen`, na LLVM 23.1.2 i rustc 1.98.1.

W historycznym przebiegu przekazanie napisu do funkcji użytkownika kończyło się awarią `into_int_value` (kod 101). Aktualny listing uruchomiłem ponownie z binarką `codegen`: build zakończył się kodem 0, program wypisał `hello` i zwrócił 0. Stary wiersz wyników pozostaje historyczny.

Arytmetyka floatów ma ścieżkę emisji. Porównanie `f64`, np. `x > 1.0`, przechodzi sprawdzanie typów, ale wybór ścieżki codegenu odbywa się według typu wyniku `bool`, więc porównanie trafia do emisji całkowitoliczbowej. Ponowne uruchomienie kończy się paniką `FloatValue`/`IntValue` (kod 101). Float jako argument funkcji jest osobno odrzucany przez codegen.

Indeks poza zakresem i dzielenie przez zero kompilują się. Proces użytkownika kończy się sygnałem przerwania, w powłoce kodem 134, bez tekstu z Borka.

Przepełnienia bufora nie uruchamiano dużym literałem w tej książce, bo krótki przykład trudno przepchnąć ponad 4096 bajtów bez pętli, która alokuje, a pętla czyści bufor przy obiegu. Kod w bibliotece wykonawczej przerywa się wtedy tekstem `arena overflow`, z podaną liczbą bajtów i pojemnością 4096. Test jednostkowy biblioteki to zamyka i nie jest to komunikat kompilacji.

Zwrot wyniku `concat` i część błędów analizy ucieczki nie mają zakresu źródłowego. Wiersz poleceń pomija wtedy linię i kolumnę.

Błąd zgłoszony przez regułę gramatyki, na przykład zły cel przypisania albo za duża liczba, wskazuje koniec pliku.

Parametry funkcji dopisanej na końcu wywołania mają w analizie własności typ nieznany, więc wydruk drzewa pokazuje współdzielenie dla parametrów, które sprawdzanie typów uważa za kopiowalne liczby. Sprawdzenie przechodzi. To myli przy czytaniu `--dump-arenas`, ale nie jest błędem użytkownika.

Komunikat analizy ucieczki o gałęzi warunku mówi o napisie. Warunek w kodzie obejmuje każdy typ trzymany w buforze regionu, także tablicę.

To, że `u32` albo alias `Byte` nie może być wynikiem `main` zwracającego `i32`, nie jest ograniczeniem szerokich liczb całkowitych w ogóle. Funkcja pomocnicza na `i64` i porównanie w `main` działają. Na sprawdzonym programie z odejmowaniem kod wyjścia wyniósł 7. Ograniczenie `main` dotyczy typu wyniku `main`, a nie lokalnej wartości `i64`.

Są dwie stałe 4096: w `src/arena.rs` i w bibliotece wykonawczej. Analiza własności nie woła żadnej. Łatwo poprawić zły plik.

Opakowanie o wewnętrznej niezgodności harmonogramu potrafi ukryć zwykły komunikat o braku wsparcia w tekście, który brzmi jak błąd wewnętrzny kompilatora. Czasem niezgodność naprawdę dotyczy dzieci drzewa regionów. Czasem jest tylko przedrostkiem.

Specyfikacje w `docs/superpowers` opisują świat sprzed tablic, sprzed pętli `while` i sprzed LLVM 23. Plik `TODO.md` każe je przenieść, gdy wchłoną się w dokument albo w zamknięte poprawki. Nie są mapą bieżącego kodu.

## Co jest pokryte testami

Aktualny korpus w `programs/` zawiera pozytywne i negatywne przypadki pożyczek, reborrow w pętlach oraz programy do sprawdzania i budowania; CI uruchamia go przez `tests/programs.rs`. W tej aktualizacji zbudowałem i uruchomiłem sześć przykładów nullable z `programs/build/conditionals/`, w tym `f32?`; krótki dodatkowy test objął `Some`, `?:`, `!!` oraz `==`/`!=` dla `f32?` i `f64?`. Ponownie sprawdziłem też przekazanie napisu do funkcji: budowanie i wykonanie przeszły. Porównanie floatów nadal panikuje, a float jako argument funkcji jest odrzucany.

## Kolejność zaufania

Gdy komentarz, specyfikacja i kod się różnią, sprawdzaj najpierw test wykonawczy w `tests/build.rs` lub korpus `programs/`, potem `frontend::check`, kod fazy, `docs/language.md`, a na końcu `docs/superpowers`. O braku pełnego borrow checkera można mówić tylko z zastrzeżeniem, że ograniczone `&T` już istnieje. Wiersz z `concat(left, right)` nadal wymaga uzgodnienia z aktualną diagnostyką.
