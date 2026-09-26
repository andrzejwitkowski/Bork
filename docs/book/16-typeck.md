# Rozdział 15. Jak kompilator sprawdza typy

## Ten rozdział obejmuje

- po co obok drzewa składni powstaje drugie drzewo, już z typami
- jakie typy istnieją tylko po sprawdzeniu, a jakich nie ma w składni
- jak oczekiwany typ zmienia znaczenie literału, pustej wartości i warunku
- dlaczego funkcje wbudowane przyjmują więcej, niż mówi ich nominalna sygnatura
- dlaczego samo drzewo z typami nie wystarcza generatorowi kodu

## Drugie drzewo opisuje program po typach

Drzewo składni, opisane w rozdziale 13, ma kształt tekstu. Nie wie jeszcze, jaki typ ma literał `0` ani czy nazwa została skopiowana. Sprawdzanie typów buduje drugie drzewo. W kodzie nazywa się ono reprezentacją pośrednią. Skrót HIR, od angielskiego high-level intermediate representation, pojawia się w nazwach struktur `HirProgram` i `HirExpr`. W zdaniach zostaję przy reprezentacji pośredniej. Rozdział 12 wprowadził to pojęcie przy opisie kolejności pracy kompilatora.

Każde wyrażenie w nowym drzewie niesie typ, zakres źródłowy i rodzaj wyrażenia. Wyrażenia `move` i `promote` z drzewa składni stają się zwykłymi nazwami. Rodzaj użycia, w kodzie `UseKind`, mówi wtedy, że było to przeniesienie albo promocja. Pozostałe warianty tego rodzaju to użycie lokalne, współdzielenie i kopia. Sprawdzanie typów wpisuje rodzaj użycia w funkcji `check_ident`. Nie zgłasza przy tym błędów własności. Własność zgłasza analiza opisana w poprzednim rozdziale. Reprezentacja pośrednia tylko zapisuje, jak sprawdzanie typów widziało odczyt. Generator kodu później ufa temu zapisowi, gdy kopiuje deskryptor napisu. Na liczbach i na napisach używanych w `main` testy trzymają oba opisy razem. Na parametrach funkcji dopisanej na końcu wywołania analiza własności i tak widzi typ nieznany, o czym była mowa w rozdziale 14.

Wywołanie traci ciało funkcji dopisanej na końcu jako osobną wartość i zostawia flagę, że taka funkcja była. Ciało i tak jest sprawdzane tam, gdzie sprawdzanie typów schodzi w blok. Flaga służy później kontroli przed generowaniem kodu. Deklaracja zmiennej dostaje typ oraz pole `alloc_in_binding`. Na tym etapie pole jest puste. Wypełni je dopiero wyniesienie alokacji, opisane w następnym rozdziale. Parametry w reprezentacji pośredniej nie mają zakresów źródłowych. Zostają same napisy z nazwami.

## Jakie typy pojawiają się dopiero teraz

Typ w reprezentacji pośredniej, struktura `Ty` w pliku `src/hir/ty.rs`, ma rodzaj i znacznik, czy wartość może być pusta. Rodzaj jest typem prostym, nazwą, tablicą o długości, typem funkcji, zakresem albo typem nieznanym.

Zakres i typ nieznany nie mają odpowiednika w drzewie składni. Zakres istnieje tylko jako typ wyrażenia `lo..hi`. Typ nieznany tłumi dalsze komunikaty, gdy wcześniejszy błąd i tak zepsuł wyrażenie. Porównanie z typem nieznanym nie dokłada kolejnego oczekiwania. Nie tłumi natomiast analizy własności.

Funkcja `is_copy` jest prawdziwa dla typu prostego, który nie może być pusty. Funkcja `uses_arena_storage` jest prawdziwa dla napisu i dla tablicy, która nie może być pusta. Drugi predykat mówi analizie ucieczki i generatorowi kodu, że wartość ma bajty w buforze regionu, a nie samą liczbę w rejestrze.

Tłumaczenie typu ze składni, funkcja `lower_type`, odrzuca pustą tablicę, czyli zapis `[T]?`, oraz każdą nazwę poza `String`.

## Jak sprawdzanie typów schodzi po programie

Funkcja `typeck::check` zbiera sygnatury, odmawia ponownego zdefiniowania funkcji wbudowanych, a potem sprawdza ciała. Środowisko ma stos zakresów, mapę funkcji, listę komunikatów, wektor typów deklaracji i głębokość pętli. Wektor typów deklaracji, w kodzie `decl_tys`, rośnie przy każdej deklaracji, w kolejności źródła. Analiza własności zdejmuje z niego typy po kolei. Dlatego sprawdzanie typów musi skończyć się wcześniej. Rozdział 12 tłumaczy, czemu komunikaty i tak wypisują się w odwrotnej kolejności.

Instrukcje są w pliku `src/typeck/stmt.rs`. Wyrażenia są podzielone. Plik `expr/mod.rs` rozdziela rodzaje i obsługuje nazwy. Plik `expr/binary.rs` obsługuje arytmetykę, porównania, koniunkcję, alternatywę i zakres. Plik `expr/call.rs` obsługuje wywołania, funkcję dopisaną na końcu i funkcje wbudowane. Plik `expr/control.rs` obsługuje warunek i blok użyty jako wartość. Plik `expr/array.rs` obsługuje literał tablicy, indeks i wycinek. Plik `expr/field.rs` obsługuje odczyt pola i odczyt z operatorem `?.`.

Ostatnia instrukcja bloku, który ma dać wartość, musi być gołym wyrażeniem. W przeciwnym razie typ bloku to `unit`. Robi to funkcja `check_value_block_in_current_scope`. Dlatego `return` w gałęzi warunku nie jest wartością tej gałęzi. Jest instrukcją. Gałąź jako wyrażenie ma typ `unit`, jeśli ostatnią rzeczą w niej nie jest gołe wyrażenie.

## Oczekiwany typ steruje literałem i pustą wartością

Sprawdzanie typów przekazuje w dół oczekiwany typ, jeśli otoczenie go zna. Literał całkowity bez oczekiwania ma typ `i32`. Gdy otoczenie oczekuje typu całkowitego, literał przyjmuje ten typ. Literał zmiennoprzecinkowy bez oczekiwania ma typ `f64`. Słowo `None` bez oczekiwanego typu jest błędem. Nie staje się „jakimś” typem pustym.

Warunek bez `else`, gdy nikt nie oczekuje wartości, ma typ `unit`. Gdy otoczenie oczekuje typu, a drugiej gałęzi nie ma, komunikat mówi, że warunek produkujący wartość wymaga gałęzi `else`. Zakres tego komunikatu bywa pusty.

## Funkcje wbudowane są specjalnym przypadkiem

Plik `expr/call.rs` rozpoznaje `print`, `println` i `concat`, zanim sprawdzi liczbę argumentów zwykłej funkcji. Dlatego `print` przyjmuje napis, choć tabela sygnatur w `src/builtins.rs` mówi, że `print` bierze `i32` i zwraca `unit`. To jest przypadek szczególny, a nie ogólna zasada, że jeden typ można podstawić pod drugi. Inna funkcja o parametrze `i32` napisu nie przyjmie.

Funkcja dopisana na końcu wywołania dokleja się jako ostatni argument typu funkcyjnego. Jej ciało jest sprawdzane w nowym zakresie. Analiza własności robi osobne zejście po drzewie składni i nie widzi typów wyliczonych dla parametrów tej funkcji. Dwa zejścia mają dwa środowiska. Stąd wydruk drzewa regionów potrafi pokazać współdzielenie tam, gdzie sprawdzanie typów widzi kopię. Rozdział 14 opisał ten skutek od strony analizy własności.

## Dlaczego generator kodu nie może iść tylko po tym drzewie

Reprezentacja pośrednia nie ma numerów regionów. Komentarz w `src/hir/mod.rs` mówi, że regiony żyją w drzewie z analizy własności. Generator kodu ma iść wspólnym spacerem, opisanym w następnym rozdziale. Na każdym bloku, pętli, warunku i funkcji dopisanej na końcu bierze kolejne dziecko węzła regionu. Jeśli sprawdzanie typów wyrzuci albo wstawi blok inaczej niż analiza własności, spacer nie znajdzie dziecka.

Funkcja `peel_blocks` jest skopiowana w reprezentacji pośredniej i w analizie własności. Komentarz przy kopii każe trzymać obie wersje w zgodzie. Nie ma jednego wspólnego traitu. Jest konwencja. Zmiana tylko w jednym miejscu psuje uzgodnienie drzew.

Pole `alloc_in_binding` jest puste po sprawdzeniu typów. Wypełnia je wyniesienie alokacji, już na gotowym drzewie, w funkcji `frontend::check`. To jedyna adnotacja dokładana między sprawdzeniem typów a generowaniem kodu.

Testy w `src/typeck/tests/mod.rs` idą zwykle przez `frontend::check`, a nie przez samo sprawdzanie typów. Dzięki temu łapią także własność. Plik `nullable.rs` trzyma operator `?:`, wykrzykniki `!!` i porównania wartości pustych. Plik `closures.rs` trzyma liczbę argumentów funkcji dopisanej na końcu wywołania. Gdy dodajesz operator, dopisujesz tu przypadek z dokładnym tekstem komunikatu. Rozdział 21 wraca do tej kolejności pracy.

## Podsumowanie

- Reprezentacja pośrednia niesie typ i rodzaj użycia nazwy. Nie niesie numeru regionu.
- Słowo `move` w drzewie składni staje się rodzajem użycia na nazwie.
- Sprawdzanie typów nie zgłasza błędów własności. Analiza własności nie czyta rodzaju użycia z reprezentacji pośredniej.
- Oczekiwany typ steruje literałami, słowem `None` i tym, czy warunek bez `else` jest błędem, czy ma typ `unit`.
- Dwa zejścia, sprawdzanie typów i analiza własności, spotykają się dopiero we wspólnym spacerze. Zgodność `peel_blocks` jest warunkiem tego spotkania.
