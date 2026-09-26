# Rozdział 16. Jak kompilator wybiera miejsce na napis

## Ten rozdział obejmuje

- kiedy kompilator w ogóle zajmuje się miejscem bajtów
- jak dwie sąsiednie instrukcje przenoszą alokację do zmiennej docelowej
- jak głębokość regionu decyduje, czy napis wolno zwrócić albo przypisać
- po co oznaczenie regionów, analiza ucieczki i generator kodu idą jednym przejściem po drzewie
- które regiony naprawdę pobierają bufor w czasie działania

## Tylko program bez wcześniejszych błędów

Gdy lista komunikatów po sprawdzeniu typów i po analizie własności jest pusta, funkcja `frontend::check` robi jeszcze trzy rzeczy. Najpierw `region_walk::stamp_codegen_push` oznacza, które regiony mają pobrać bufor. Potem `hoist::annotate` dopisuje do deklaracji informację, w której zmiennej zewnętrznej alokować napis. Na końcu `escape::check_function` sprawdza każdą funkcję. Jeśli analiza ucieczki dopisze komunikat, reprezentacja pośrednia znika z wyniku, a drzewo regionów zostaje.

Przy błędzie typu wyniesienie alokacji się nie wykona. Nie zobaczysz drugiego komunikatu o ucieczce obok błędu typu. Przejścia zakładają drzewo, które sprawdzanie typów uważa za spójne. Łączona diagnostyka, w której jeden plik pokazuje naraz błąd typu i błąd ucieczki, w tej wersji nie powstaje.

## Wyniesienie alokacji rozpoznaje dokładnie dwie sąsiednie instrukcje

Plik `src/hoist.rs` rozwiązuje następujący układ. Programista deklaruje zmienną wewnętrzną, a w następnym wierszu przenosi ją do zmiennej, która żyje dalej. Bajty i tak mają wylądować w arenie tej dalszej zmiennej. Kompilator może więc zbudować je od razu tam, zamiast budować je w bieżącym buforze i zaraz kopiować.

Funkcja `try_hoist` patrzy na instrukcję o numerze `i` oraz na następną. Pierwsza musi być deklaracją zmiennej o nazwie wewnętrznej, jeszcze bez wskazania miejsca alokacji. Druga musi być przypisaniem do nazwy, a nie do indeksu tablicy. Wartość deklaracji musi być literałem napisowym albo wywołaniem. Prawa strona przypisania musi być nazwą wewnętrzną użytą jako przeniesienie. Nazwa celu musi być już w zakresie: wśród parametrów, wśród nazw z zewnątrz albo wśród nazw zadeklarowanych wcześniej w tym bloku.

Gdy te warunki są spełnione, pole `alloc_in_binding` dostaje nazwę celu. Generator kodu czyta to pole przy emisji deklaracji i każe alokować wynik w arenie tej nazwy. Jeśli pole jest puste, inicjalizator idzie do bieżącej areny, a późniejsze przeniesienie kopiuje bajty funkcją `copy_into_arena`.

Zbiór nazw zewnętrznych, przy zejściu w blok, w pętlę `for` i w pętlę `while`, jest sumą nazw widzianych do tej pory. Wyniesienie nie przeskakuje między gałęziami warunku. Nie patrzy, która instrukcja dominuje nad inną w grafie sterowania. Albo dwie sąsiednie linie pasują do wzorca, albo alokacja zostaje w bieżącym regionie.

## Analiza ucieczki liczy głębokość bajtów

Plik `src/escape.rs` pyta, na jakiej głębokości leżą bajty i czy odbiorca leży nie głębiej niż one. Tylko wtedy bajty jeszcze istnieją w chwili użycia. Głębokość zero to region funkcji. Każdy region wewnętrzny jest o jeden głębszy.

Wiązanie pamięta dwie liczby. `decl_depth` to głębokość deklaracji nazwy. `value_depth` to głębokość bajtów. Druga liczba potrafi być mniejsza od pierwszej, gdy wyniesienie alokacji albo miejsce przeznaczenia położyło bajty wyżej, czyli bliżej funkcji. Samo przejście nie zapisuje tej poprawki z powrotem do analizy własności. Albo odrzuca program, albo go przepuszcza.

Funkcja `place` wylicza głębokość wyrażenia. Literał liczbowy, logiczny, napisowy i słowo `None` mają głębokość zero, bo nie zależą od bufora wewnętrznego regionu. Literał tablicy bierze maksimum z głębokości elementów i z głębokości bufora tablicy. Bufor idzie do miejsca przeznaczenia, a gdy go nie ma, do bieżącej głębokości. Indeks wartości trzymanej w arenie ma głębokość tablicy, a nie głębokość bloku, w którym stoi indeks. Wycinek ma głębokość odbiorcy. Przeniesienie i promocja przy znanym miejscu przeznaczenia mają głębokość tego miejsca. Przy powrocie z funkcji, gdzie miejsce przeznaczenia ma głębokość zero, zostaje głębokość źródła. Wynik `concat` ma głębokość miejsca przeznaczenia albo głębokość bieżącą. Inne wywołanie typu trzymanego w arenie bierze maksimum głębokości argumentów. Warunek bierze maksimum obu gałęzi.

Powrót wartości trzymanej w arenie, gdy jej głębokość jest większa od zera, jest błędem. Potem funkcja `check_return_string_form` osobno zabrania zwrócić wynik `concat` oraz wartość przeniesioną albo promowaną, nawet gdy wyliczona głębokość wyszła zero. Dlatego `return concat(...)` pada także w funkcji bez zagnieżdżonego bloku. Nie ma jeszcze areny wyniku po stronie wywołującego, więc świeży napis nie ma gdzie przeżyć powrotu.

Przypisanie liczy głębokość z miejscem przeznaczenia równym głębokości deklaracji celu. Jeśli bajty byłyby głębsze niż deklaracja celu, komunikat mówi, że przypisujesz wartość, której bajty żyją w regionie wewnętrznym.

Gdy gałąź warunku jest regionem, który produkuje wartość, nie ma miejsca przeznaczenia, a bajty leżą na głębokości tej gałęzi, kompilator zgłasza błąd. Tekst komunikatu mówi o napisie w gałęzi. Warunek w kodzie jest szerszy: obejmuje każdy typ, dla którego `uses_arena_storage` jest prawdziwe, a więc także tablicę. Słowo w komunikacie jest węższe niż sprawdzany warunek.

Komunikaty analizy ucieczki mają fazę `ownership`, tę samą co analiza własności. Część z nich nie ma zakresu źródłowego. Dotyczy to między innymi zwrotu wyniku `concat`. Wiersz poleceń pomija wtedy numer linii i kolumny.

## Jedno przejście trzyma oznaczenie regionów i generator kodu w zgodzie

Plik `src/region_walk.rs` istnieje po to, żeby oznaczanie regionów, układ wywołań w generatorze kodu i emisja instrukcji LLVM nie miały trzech lekko różnych pętli po reprezentacji pośredniej. Komentarz na górze pliku mówi, że jest jedno przejście po drzewie, zsynchronizowane z dziećmi węzła regionu.

Miejsca, które otwierają dziecko raportu, nazywa typ `RegionSite`. Są to blok, pętla `for`, pętla `while`, gałęzie warunku i funkcja dopisana na końcu wywołania. Etykiety muszą pasować do tych, które wpisała analiza własności. Inaczej funkcja `take_child` nie znajdzie węzła i kompilator zgłosi wewnętrzną niezgodność harmonogramu regionów.

Obiekt, który przechodzi po węzłach, w kodzie typ `RegionVisitor`, w wybranych miejscach wykonuje własną czynność. Ma osobne punkty na wejście w funkcję, na wejście w region, na powrót na początek obiegu pętli, na wyjście z regionu, na pominięcie funkcji dopisanej na końcu i na chwilę po wyrażeniu. Domyślna ścieżka instrukcji nie obsługuje bloku i pętli, bo te idą przez region. Jeśli ktoś wywoła je jak zwykłą instrukcję, dostanie `unreachable!`. To jest kontrola programisty kompilatora, a nie komunikat dla autora programu w Borku.

Koniunkcja i alternatywa zwierają się, więc prawa strona jest w osobnym bloku podstawowym i emisja nie schodzi w nią zwykłą ścieżką. Oznaczenie regionów i tak musi wiedzieć, czy prawa strona alokuje. Dlatego ogląda prawą stronę, a emisja jej nie emituje drugi raz.

## Który region pobiera bufor

Funkcja `codegen_push_for_region` decyduje o wywołaniu `bork_arena_push` w wygenerowanym kodzie. Etykieta funkcji dopisanej na końcu wywołania nigdy nie pobiera bufora. Dla pozostałych regionów decyduje predykat `block_may_allocate_sink` na ciele po zdjęciu opakowań.

Predykat jest prawdziwy, gdy w bloku jest literał napisu albo tablicy, przeniesienie lub promocja typu trzymanego w arenie, wywołanie `concat`, przypisanie albo powrót czegoś, co może alokować, zagnieżdżony blok albo gałąź, która może alokować. Jest fałszywy dla samych liczb, dla `break`, dla `continue` i dla słowa `None`.

Oznaczanie ustawia korzeniowi funkcji flagę pobrania bufora zawsze. Dziecku ustawia wynik predykatu. Funkcji dopisanej na końcu wywołania ustawia tę flagę na fałsz wprost.

Skutek, opisany w `docs/memory-model.md` i zrealizowany w `src/codegen/regions.rs`, jest taki, że w wydruku drzewa nadal jest węzeł na każdy region, ale wygenerowany kod woła `bork_arena_push` tylko wtedy, gdy flaga jest prawdziwa. Blok, w którym są same liczby `i32`, jest w tym wydruku osobnym węzłem, choć w czasie działania nie ma własnego bufora. Funkcja zawsze pobiera jeden bufor na wejściu, nawet gdy nic nie alokuje. To jest uproszczenie tej wersji kompilatora. Bufor funkcji i tak wraca na listę wolnych przy wyjściu.

Pętla, jeśli flaga jest prawdziwa, wchodzi raz, przy powrocie na początek obiegu czyści wskaźnik bufora bez oddawania go do puli i wychodzi raz. Plik `TODO.md` prosi, żeby ten kontrakt nie rozszedł się między kodem, który przechodzi po drzewie. Dziś emisja i oznaczanie dzielą `region_walk`, więc utrzymanie zgodności jest prostsze niż przy dwóch ręcznych pętlach. Nadal da się ją zepsuć, nadpisując punkt pętli i zapominając o czyszczeniu przy powrocie na początek obiegu.

## Podsumowanie

- Wyniesienie alokacji, oznaczenie buforów i analiza ucieczki biegną tylko po czystym sprawdzeniu typów i własności.
- Wyniesienie rozpoznaje dokładnie dwie sąsiednie instrukcje i każe zbudować napis od razu w arenie celu.
- Analiza ucieczki liczy głębokość bajtów. Zabrania zwrócić świeży napis, nawet z głębokości zero, gdy formą wyniku jest `concat`, `move` albo `promote`.
- Wspólne przejście `region_walk` trzyma oznaczenie regionów i generator kodu przy tych samych dzieciach drzewa regionów.
- Flaga pobrania bufora odcina puste regiony od `bork_arena_push`. Funkcje dopisane na końcu wywołania nie pobierają bufora, bo i tak nie dochodzą do emisji.
