# Rozdział 14. Jak kompilator sprawdza własność nazw

## Ten rozdział obejmuje

- czym jest analiza własności i czym różni się od bufora pamięci
- jak powstaje drzewo regionów dla każdej funkcji
- według jakiej reguły nazwa jest kopiowana, współdzielona albo musi być przeniesiona
- jak pętla i warunek zmieniają to, co wolno przenieść
- czego ta analiza świadomie nie sprawdza

## Analiza własności opisuje nazwy, a nie bajty

Analiza własności odpowiada na pytanie, czy daną nazwę wolno w tym miejscu odczytać. W kodzie katalog tej analizy nazywa się `sema`, od angielskiego semantic analysis, czyli analizy znaczenia nazw. Dalej piszę o niej jako o analizie własności. Nie przydziela ona pamięci. Plik `src/arena.rs`, który opisuje bufor o pojemności 4096 bajtów, zaczyna się komentarzem, że ta analiza go nie używa. Zamiast bufora powstaje drzewo. Każdy węzeł drzewa opisuje jeden region: które nazwy w nim powstały i które nazwy z zewnątrz zostały w nim użyte.

Węzeł nazywa się w kodzie `ArenaNode`. Ma numer, etykietę tekstową, listę wiązań, listę obserwacji i listę dzieci. Numer pochodzi z licznika w strukturze `Analyzer` i jest zwykłą liczbą całkowitą bez znaku. Nie ma osobnego typu identyfikatora regionu. Nie ma też wyliczenia rodzaju regionu, choć starsze notatki projektowe o takim wyliczeniu wspominały. Region jest albo zwykły, albo jest blokiem `move`. Różnicę widać po funkcji, która go otwiera. Zwykły region otwiera `open_ordinary`. Blok `move` otwiera `open_move`. Etykieta, na przykład `fun main`, `Block` albo `ForLoop (i)`, jest napisem do wydruku i do komunikatów.

Korzeń drzewa dla całego programu trzyma struktura `ArenaReport`. Ma ona po jednym korzeniu na funkcję.

## Co pamięta jedno wiązanie

Wiązanie w środowisku analizy, w kodzie typ `EnvBinding`, pamięta numer regionu, etykietę tego regionu, typ, rodzaj własności, flagę przeniesienia i pochodzenie nazwy. Typ jest albo znany, albo nieznany. Znany typ pochodzi z drzewa składni. Typ nieznany, w kodzie `Unknown`, dostają parametry funkcji dopisanej na końcu wywołania. Analiza własności nie czyta typów, które sprawdzanie typów wyliczyło dla tych parametrów. Pochodzenie nazwy jest albo deklaracją, albo przechwyceniem w bloku `move`.

Rodzaj własności, w kodzie typ `Ownership`, ma cztery warianty. `Local` oznacza nazwę powstałą w bieżącym regionie. `Copy` oznacza odczyt wartości, którą wolno skopiować. `Shared` oznacza odczyt nazwy stałej, której nie wolno skopiować, i pamięta etykietę regionu, z którego nazwa pochodzi. `Moved` oznacza, że własność została przeniesiona, i też pamięta etykietę źródła.

Wydruk drzewa pomija obserwacje lokalne, żeby nie powtarzać deklaracji. Obserwacje kopiowania, współdzielenia i przeniesienia zostają na wydruku.

## Jak otwiera się region

Wejście do analizy jednej funkcji nazywa się `analyze_with_decl_tys`. Najpierw zapisuje, czy parametry każdej funkcji są stałe, czy zmienne. Potem woła `open_ordinary` z etykietą złożoną ze słowa `fun` i nazwy funkcji, z ciałem funkcji i z listą parametrów.

Otwarcie regionu, czy zwykłego, czy blokiem `move`, zaczyna się od zdjęcia opakowań. Funkcja `peel_blocks` usuwa bloki, które zawierają tylko jeden wewnętrzny blok, i liczy, ile takich nawiasów zdjęła. Licznik ląduje w polu `compacted_braces`. Dzięki temu zapis `{ { instrukcje } }` nie tworzy dwóch regionów. Potem analiza bierze nowy numer, buduje ramkę regionu i wiąże parametry. Dla bloku `move` dodatkowo przechwytuje nazwy z listy. Przechwycenie przenosi źródło, cieniuje nazwę jako przechwyconą, dopisuje przeniesienie do dziecka, a po zamknięciu regionu oznacza źródło u rodzica jako przeniesione. Następnie analiza schodzi w instrukcje. Na końcu zdejmuje cienie, czyli przywraca nazwy, które były widoczne przed wejściem.

Lista nazw przy `move` jest rozstrzygana w funkcji `resolve_move_captures`. Jeśli programista podał listę, wygrywa ona ze zgadywaniem, także gdy jest pusta. Jeśli listy nie ma, kompilator zbiera nazwy wolne w bloku, odrzuca parametry i zostawia nazwy żywe, których typ nie jest kopiowalny. Wyrażenie `move` i wyrażenie `promote` nie liczą się jako zmienne wolne. Ta różnica jest zapisana w drzewie składni. Brak listy to wartość pusta. Pusta lista to lista podana jawnie. Nie wolno ich spłaszczyć do jednego pustego wektora, bo znaczą co innego.

## Reguła jednego odczytu

Decyzja o kopiowaniu, współdzieleniu i przeniesieniu mieści się w funkcji `classify_use` w pliku `src/sema/policy.rs`. Kolejność sprawdzeń jest stała.

Jeśli wiązanie jest już przeniesione, odczyt jest błędem. Komunikat mówi, że nazwy użyto po przeniesieniu, i podaje etykietę regionu, z którego ją przeniesiono.

Jeśli numer regionu wiązania jest taki sam jak numer bieżącego regionu, odczyt jest lokalny. Nazwa żyje w tym regionie, więc nie przekracza jego granicy.

Jeśli typ jest kopiowalny, odczyt jest kopią. Źródło zostaje żywe. Kopiowalne są tylko typy proste, które nie mogą być puste. Mowa o tym była w rozdziale 4.

Jeśli wiązanie jest stałe, odczyt jest współdzieleniem. Region wewnętrzny widzi bajty regionu zewnętrznego, ale ich nie kopiuje i nie unieważnia nazwy.

W pozostałych przypadkach odczyt jest błędem. Chodzi o zmienną z regionu zewnętrznego, której nie wolno skopiować. Komunikat każe przenieść nazwę słowem `move` do regionu o bieżącej etykiecie.

Przypisanie do zmiennej z regionu zewnętrznego omija tę regułę po lewej stronie. Jeśli cel jest zmienną, nie został przeniesiony, a jego region ma mniejszy numer niż region bieżący, analiza nie traktuje lewej strony jako odczytu. Prawa strona jest sprawdzana osobno. Dzięki temu zapis `outer = wyrażenie` zmienia istniejącą zmienną, a nie próbuje jej przenieść.

Krótsze podpowiedzi, gdy programista napisał gołą nazwę tam, gdzie potrzebne jest `move`, buduje funkcja `bare_ident_move_message`. Dla argumentu wywołania tekst każe użyć `move`, żeby przekazać własność. Dla przypisania w tej samej okolicy każe użyć `move`, żeby własność przenieść. Gdy regiony się różnią, zostaje dłuższy tekst o braku kopiowania.

## Pętla zabrania przenosić to, co było widać na wejściu

Przy pętli `for` i przy pętli `while` analiza odkłada na stos zbiór wszystkich nazw widocznych w tej chwili. Zbiór nazywa się w kodzie `loop_move_ban`. Po ciele pętli analiza go zdejmuje. Próba przeniesienia nazwy z tego zbioru kończy się błędem. Komunikat tłumaczy, że kolejna iteracja zobaczyłaby nazwę już przeniesioną. Nazwa utworzona wewnątrz pętli do zbioru nie należy, więc wolno ją przenieść w tej samej iteracji, w której powstała.

## Warunek scala przeniesienia z obu gałęzi

Przy `if` analiza zapamiętuje, które nazwy są przeniesione, schodzi w gałąź prawdziwą, przywraca flagi, schodzi w gałąź `else`, jeśli druga gałąź istnieje, przywraca flagi jeszcze raz i scala wynik funkcją `apply_moved_merge`. Nazwa jest przeniesiona po całym warunku, gdy była przeniesiona już przed nim albo gdy została przeniesiona w obu gałęziach. Sama gałąź prawdziwa, bez `else`, nie zostawia przeniesienia na zewnątrz. Nie ma tu śledzenia, że nazwa jest martwa tylko przy prawdziwym warunku. Albo obie gałęzie ją przenoszą, albo po warunku nazwa jest nadal żywa, o ile żyła wcześniej.

## Czego ta analiza nie sprawdza

Nie sprawdza, czy bajty wyniku `concat` przeżyją powrót z funkcji. To robi analiza ucieczki, opisana w rozdziale 16. Nie nadaje typów parametrom funkcji dopisanej na końcu wywołania i zostawia je jako nieznane. Typ nieznany nie jest kopiowalny. Test `unknown_type_is_not_treated_as_copy` pilnuje tego wyboru. Lepiej dostać zbędny komunikat o braku kopiowania przy programie, który i tak ma błąd typu, niż puścić wartość, która mogłaby uciec z regionu. Skutek uboczny widać w wydruku drzewa. Parametr takiej funkcji, który sprawdzanie typów uznało za liczbę całkowitą, w wydruku potrafi wyglądać jak współdzielenie.

Indeks pętli `for` jest w tej analizie zwykłą liczbą całkowitą. Nie ma ścieżek wyjątków, bo język ich nie ma. Pole `compacted_braces` liczy zdjęte nawiasy, a nie bajty w buforze.

Słowo `promote` jest obsługiwane przy zejściu w wyrażenie, w funkcji `apply_expr_promote`. W katalogu testów jednostkowych analizy nie ma osobnego pliku o promocji. Zachowanie jest w zejściu i w testach wyższego poziomu, które budują program. Milczenie testów jednostkowych nie znaczy, że promocja jest niezaimplementowana. Program z rozdziału 8, który przypisuje `promote`, daje się zbudować.

Wydruk drzewa nie powstaje w tej analizie. Powstaje w pliku `src/dump.rs`. Serwer edytora używa tej samej funkcji.

## Podsumowanie

- Analiza własności buduje drzewo regionów i flagi przeniesienia. Nie przydziela buforów o pojemności 4096 bajtów.
- Funkcja `classify_use` jest definicją kopiowania, współdzielenia i przeniesienia.
- Brak listy przy `move` oznacza zgadywanie. Pusta lista oznacza rezygnację ze zgadywania.
- Pętla zabrania przenosić nazwy widoczne na wejściu. Warunek zostawia przeniesienie tylko wtedy, gdy zrobiły je obie gałęzie.
- Analiza ucieczki i wyniesienie alokacji są później i nie leżą w tym katalogu.
- Parametry funkcji dopisanej na końcu wywołania mają tu typ nieznany, więc wydruk drzewa potrafi pokazać współdzielenie tam, gdzie sprawdzanie typów widzi kopię.
