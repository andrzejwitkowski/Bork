# Dodatek B. Słownik pojęć

Każde hasło jest zdaniem albo krótkim akapitem. Angielski termin z kodu podaję raz, obok polskiego, i dalej trzymam polski, chyba że w zdaniu chodzi o nazwę w źródle.

**Analiza ucieczki.** Przejście w pliku `src/escape.rs`, które liczy głębokość bajtów i odrzuca użycie, które przeżyłoby zwolnienie bufora regionu. Komunikat ma fazę `ownership`. W kodzie funkcja licząca głębokość nazywa się `place`.

**Analiza własności.** Przejście w katalogu `src/sema`, które buduje drzewo regionów i decyduje, czy nazwę wolno skopiować, współdzielić albo trzeba przenieść. Po angielsku semantic analysis. Nie przydziela buforów.

**Arena.** Bufor jednego regionu, o pojemności 4096 bajtów, z przesuwającym się wskaźnikiem. W źródłach ta sama nazwa oznacza jeszcze węzeł drzewa regionów, `ArenaNode`. To nie jest ten sam obiekt. Rozdział 2 i rozdział 14 rozdzielają te znaczenia.

**Drzewo składni.** Nietypowany wynik parsera, w kodzie AST, od angielskiego abstract syntax tree. Definicja jest w `src/ast.rs`.

**Głębokość.** Liczba regionów między funkcją a miejscem, w którym leżą bajty. Zero to region funkcji. Analiza ucieczki porównuje głębokość bajtów z głębokością odbiorcy.

**Kontrola przed generowaniem kodu.** Zejście po reprezentacji pośredniej w `src/codegen/gate.rs`, zanim powstanie moduł LLVM. Odrzuca konstrukcje, których emisja nie tłumaczy, komunikatem fazy `codegen`. Nie łapie każdego miejsca, w którym emisja kończy się awarią.

**Kopiowanie.** Przekroczenie granicy regionu przez wartość typu prostego, która nie może być pusta. Źródło zostaje żywe. W wydruku drzewa znacznik to `Copy`.

**Miejsce przeznaczenia.** Odbiorca bajtów wyniku: cel przypisania, powrót albo wynik `concat`. Steruje tym, w której arenie powstanie bufor wyniku. W kodzie bywa nazywane sink.

**Promocja.** Zapis `promote nazwa` po prawej stronie przypisania do zmiennej z regionu zewnętrznego. Kopiuje bajty do areny tej zmiennej i unieważnia źródło.

**Przeniesienie.** Zapis `move`, po którym źródłowa nazwa jest martwa. Bajty w starej arenie zostają niedostępne aż do wyczyszczenia bufora. W wydruku drzewa znacznik to `Moved`.

**Pula buforów.** Lista wolnych aren w bibliotece wykonawczej. Zwrot bufora oddaje go na listę. Wyczyszczenie wskaźnika bufora nie oddaje go na listę. Pobranie bierze bufor z listy albo alokuje nowy.

**Reprezentacja pośrednia.** Drzewo programu po sprawdzeniu typów. Po angielsku high-level intermediate representation, w kodzie HIR. Niesie typ i rodzaj użycia nazwy. Nie niesie numeru regionu.

**Region.** Fragment programu i czas życia pamięci z nim związany: ciało funkcji, blok, gałąź warunku, ciało pętli albo funkcja dopisana na końcu wywołania, o ile zdjęcie opakowań nie skleiło go z otoczeniem.

**Rodzaj użycia.** Adnotacja na nazwie w reprezentacji pośredniej: użycie lokalne, współdzielenie, kopia, przeniesienie albo promocja. W kodzie `UseKind`. Nie zastępuje analizy własności.

**Sprawdzenie.** Funkcja `frontend::check`: czytanie składni, sprawdzanie typów, analiza własności, a przy braku błędów także oznaczenie buforów, wyniesienie alokacji i analiza ucieczki.

**Typ nieznany.** Typ w reprezentacji pośredniej, który nie ma odpowiednika w składni. Tłumi część dalszych komunikatów o typie. Nie jest kopiowalny. W kodzie `Unknown`.

**Wartość pusta.** Typ z dopiskiem `?`. Sprawdzanie typów ją rozumie. Generowanie kodu jej nie tłumaczy.

**Współdzielenie.** Odczyt stałej, której nie wolno skopiować, z regionu zewnętrznego. Region wewnętrzny nie kopiuje bajtów i nie unieważnia nazwy. W wydruku drzewa znacznik to `Shared`.

**Wyniesienie alokacji.** Rozpoznanie dwóch sąsiednich instrukcji i ustawienie pola `alloc_in_binding`, żeby bajty powstały od razu w arenie zmiennej docelowej. Plik `src/hoist.rs`.

**Zakres źródłowy.** Para przesunięć bajtowych w pliku. W kodzie `Span`. Część komunikatów go nie ma. Wiersz poleceń pomija wtedy numer linii.

**Zdjęcie opakowań.** Usunięcie bloków, które zawierają tylko jeden wewnętrzny blok, żeby nie tworzyć regionu na same nawiasy. W kodzie `peel_blocks`. Licznik zdjętych nawiasów ląduje w `compacted_braces`.

**`codegen_push`.** Flaga na węźle regionu. Wygenerowany kod woła `bork_arena_push` tylko wtedy, gdy flaga jest ustawiona. Korzeń funkcji ma ją zawsze.

**`decl_tys`.** Wektor typów deklaracji w kolejności źródła, przekazany ze sprawdzania typów do analizy własności.

**`loop_move_ban`.** Zbiór nazw, których nie wolno przenieść w ciele pętli, bo były widoczne na wejściu do pętli.

**`unit`.** Typ funkcji bez zadeklarowanego wyniku. W składni pisze się małymi literami. Nie nazywa się `Unit`.
