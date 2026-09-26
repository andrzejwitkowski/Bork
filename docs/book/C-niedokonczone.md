# Dodatek C. Czego kompilator jeszcze nie potrafi

Ten dodatek wymienia miejsca, w których kod jest węższy niż to, co rozumie sprawdzenie programu, dokumentacja mówi co innego niż kod, albo kompilator kończy się awarią zamiast komunikatem. Nic z tej listy nie jest propozycją naprawy. Książka nie zmienia kompilatora.

## Rzeczy świadomie niezaimplementowane

Z pliku `src/escape.rs`, z `docs/language.md` i z `docs/memory-model.md` wynika następująca lista. Nie ma areny wyniku po stronie wywołującego, więc zwrot przeniesienia, zwrot wyniku `concat` i zwrot bajtów z regionu wewnętrznego są odrzucane. Promocja na powrocie z funkcji też jest odrzucana. Wyniesienie alokacji działa tylko dla dwóch sąsiednich instrukcji. Nie ma usuwania zbędnego kopiowania pamięci, gdy bajty już leżą w buforze celu. Nie ma osobnej areny tymczasowej na czas zwykłego wywołania. Generowanie kodu nie tłumaczy funkcji dopisanej na końcu wywołania, słów `Some` i `None`, wykrzykników `!!`, operatora `?:` ani pól innych niż `length`. Funkcja `main` nie ma parametrów i nie zwraca typu innego niż `i32` i `unit`. Wywołanie pośrednie, w którym rzeczą wywoływaną nie jest nazwa, nie jest tłumaczone. Nie ma modułów, struktur, typów ogólnych, wyjątków, sprawdzania pożyczek ani odśmiecania.

Plik `TODO.md` wymienia prace nad samym kompilatorem, a nie nad semantyką języka. Są to rozcięcie gościa generowania kodu, kursor regionów zamiast makra, test zgodności kontroli przed generowaniem kodu ze sprawdzeniem, więcej testów par wejście-wyjście oraz uporządkowanie planów w `docs/superpowers/plans/`.

## Gdzie dokument mówi co innego niż kod

Przykład `concat(left, right)` przy dwóch zmiennych napisowych w `docs/language.md` jest pokazany jako wzorzec. Sprawdzenie odrzuca go komunikatem, że `left` nie jest kopiowalne i trzeba je przenieść do bloku. Zdanie o kolejności operatorów w tym samym dokumencie pomija `&&`, `||` i `!`. Gramatyka te piętra ma.

Notatka projektowa z 23 września 2026 o reprezentacji pośredniej i LLVM mówi o LLVM 18 i o kolejności „najpierw własność, potem typy”. Kod używa LLVM 23, a sprawdzanie typów wykonuje się przed analizą własności. Specyfikacja serwera edytora mówi o komunikatach parsera. Kod woła pełne `frontend::check`. Specyfikacja pierwszej wersji regionów mówi o braku pełnego sprawdzania typów. Sprawdzanie typów jest. Nagłówek w `README` mówi o diagnostyce składni, a akapit niżej i kod robią pełne sprawdzenie. Specyfikacja parsera mówi o wyniku `Unit` i o braku zakresów źródłowych. W kodzie typ nazywa się `unit` i zakresy są. Dokument języka mówi, że dzielenie minimalnej liczby całkowitej przez minus jeden przerywa program. Strażnik w `guard_int_div` ten przypadek obsługuje, ale literału ujemnego nie da się zapisać, bo nie ma jednoargumentowego minusa.

## Awarie i ostre krawędzie, które zostały uruchomione

Kompilator był złożony z opcją `codegen`, na LLVM 23.1.2 i rustc 1.98.1.

Przekazanie napisu do funkcji użytkownika, czy literałem, czy przez `move`, czy przez stałą, przechodzi sprawdzenie i kontrolę przed generowaniem kodu, a potem kompilator kończy się awarią w `src/codegen/llvm/expr.rs`, w funkcji `value_as_int`, na wywołaniu `into_int_value`. Kod procesu kompilatora to 101.

Porównanie `f64`, na przykład warunek `x > 1.0` przy `x` typu `f64`, kończy się awarią w tym samym miejscu, bo wartość jest liczbą zmiennoprzecinkową LLVM. Dodawanie `f32` daje komunikat, a nie awarię. Treść mówi o wewnętrznej niezgodności harmonogramu i o tym, że dodawanie zmiennoprzecinkowe po spacerze nie jest jeszcze obsługiwane. Dwa operatory na liczbach zmiennoprzecinkowych kończą się na dwa różne sposoby.

Indeks poza zakresem i dzielenie przez zero kompilują się. Proces użytkownika kończy się sygnałem przerwania, w powłoce kodem 134, bez tekstu z Borka.

Przepełnienia bufora nie uruchamiałem dużym literałem w książce. Krótki przykład trudno przepchnąć ponad 4096 bajtów bez pętli, która alokuje, a pętla czyści bufor przy obiegu. Kod w bibliotece wykonawczej przerywa się tekstem `arena overflow`, z podaną liczbą bajtów i pojemnością 4096. Test jednostkowy biblioteki to zamyka. To nie jest komunikat kompilacji.

Zwrot wyniku `concat` i część błędów analizy ucieczki nie mają zakresu źródłowego. Wiersz poleceń pomija wtedy linię i kolumnę.

Błąd zgłoszony przez regułę gramatyki, na przykład zły cel przypisania albo za duża liczba, wskazuje koniec pliku.

Parametry funkcji dopisanej na końcu wywołania mają w analizie własności typ nieznany. Wydruk drzewa pokazuje współdzielenie dla parametrów, które sprawdzanie typów uważa za kopiowalne liczby. Sprawdzenie przechodzi. To myli przy czytaniu `--dump-arenas`. Nie jest błędem użytkownika.

Komunikat analizy ucieczki o gałęzi warunku mówi o napisie. Warunek w kodzie obejmuje każdy typ trzymany w buforze regionu, także tablicę.

To, że `u32` albo alias `Byte` nie może być wynikiem `main` zwracającego `i32`, nie jest ograniczeniem szerokich liczb całkowitych w ogóle. Funkcja pomocnicza na `i64` i porównanie w `main` działają. Na sprawdzonym programie z odejmowaniem kod wyjścia wyniósł 7. Ograniczenie `main` dotyczy typu wyniku `main`, a nie lokalnej wartości `i64`.

Są dwie stałe 4096: w `src/arena.rs` i w bibliotece wykonawczej. Analiza własności nie woła żadnej. Łatwo poprawić zły plik.

Opakowanie o wewnętrznej niezgodności harmonogramu potrafi ukryć zwykły komunikat o braku wsparcia w tekście, który brzmi jak błąd wewnętrzny kompilatora. Czasem niezgodność naprawdę dotyczy dzieci drzewa regionów. Czasem jest tylko przedrostkiem.

Specyfikacje w `docs/superpowers` opisują świat sprzed tablic, sprzed pętli `while` i sprzed LLVM 23. Plik `TODO.md` każe je przenieść, gdy wchłoną się w dokument albo w zamknięte poprawki. Nie są mapą bieżącego kodu.

## Co jest pokryte testami

Żeby lista braków nie przesłoniła reszty: parser, sprawdzanie typów liczb i tablic, reguła kopiowania, współdzielenia i przeniesienia, wydruk drzewa regionów, pętle `for` i `while` z `break` i `continue`, wypisywanie liczb i napisów w `main`, `concat` i promocja w `main`, wycinki o granicach będących literałami oraz konsolidacja z biblioteką wykonawczą są pokryte testami w `tests/build.rs` albo pełnym sprawdzeniem. Zostały powtórzone przy pisaniu tej książki. Braki są na brzegach: wartości puste w generowaniu kodu, liczby zmiennoprzecinkowe, napis jako argument wywołania i powrót świeżego napisu.

## Kolejność zaufania

Gdy komentarz, specyfikacja i kod się różnią, kolejność przyjęta w książce jest taka. Najpierw test wykonawczy w `tests/build.rs`. Potem `frontend::check` na przykładzie. Potem kod fazy. Potem `docs/language.md`. Na końcu `docs/superpowers`. Zdanie w `README` o braku sprawdzania pożyczek i braku odśmiecania zgadza się z modelem. Przykład `concat` na dwóch zmiennych napisowych się nie zgadza.
