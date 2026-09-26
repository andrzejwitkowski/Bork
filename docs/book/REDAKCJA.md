# Raport redakcyjny

## Ostatni przebieg

Każdy rozdział został przeczytany jeszcze raz pod kątem trzech usterek: poszatkowanych zdań, kalek z angielskiego i zbyt częstej pierwszej osoby. Werdykt dla wszystkich rozdziałów, przedmowy i dodatków: do druku po tym przebiegu. Skład ramek zostawia sam nagłówek, a długie identyfikatory w tekście mogą się łamać.

Przed: „Mają dwóch autorów. Pierwszym jest analiza nazw. Drugim jest analiza ucieczki.”

Po: „Komunikaty tej fazy powstają w dwóch miejscach: w analizie nazw oraz w analizie ucieczki, która nie startuje, gdy wcześniejsze błędy nie są puste.”

Przed: „Sprawdzanie typów nie zatrzymuje się na pierwszym błędzie. Zbiera listę.”

Po: „Sprawdzanie typów nie zatrzymuje się na pierwszym błędzie, tylko zbiera całą listę, a potem i tak uruchamia się analiza własności.”

Przed: „Ten dopisek powstaje, gdy błąd spaceru po regionach jest opakowywany jako niezgodność harmonogramu. Brzmi to surowo.”

Po: „Ten dopisek powstaje, gdy błąd przejścia po drzewie regionów, czyli funkcji `region_walk`, jest opakowywany jako niezgodność harmonogramu. Komunikat jest napisany tak, jakby kompilator zepsuł się wewnętrznie.”

Przed: „Dwie instrukcje w jednym wierszu nie są programem.”

Po: „Instrukcje rozdziela się nową linią, a nie średnikiem, więc dwie instrukcje zapisane w jednym wierszu parser odrzuca.”

Przed: „Wydruk ma węzeł, a w czasie działania pętla nie dostaje własnego bufora.”

Po: „W wydruku drzewa regionów ten blok nadal jest osobnym węzłem, natomiast w czasie działania pętla nie dostaje własnego bufora.”

Przed: „Region jest gruby.”

Po: „Region jest gruboziarnisty, bo napis i duża tablica utworzone w tym samym bloku dzielą jeden bufor 4096 bajtów i giną razem.”

Przed: „Literału ujemnego nie zapiszesz, więc tej ścieżki nie uruchamiałem z pliku `.bork`.”

Po: „Literału ujemnego nie da się zapisać, więc tej ścieżki nie uruchamiano z pliku `.bork`.”

Ten sam sposób łączenia zdań i te same zamiany słów (`region_walk` jako przejście po drzewie, bufor zamiast „czasu życia bajtów”, forma bezosobowa zamiast „sprawdziłem”) są w pozostałych rozdziałach, nie tylko w cytatach powyżej.

## Przebieg poprzedni

Każdy rozdział został przeczytany dwa razy. Pierwsze czytanie sprawdzało, czy programista, który nie zna Borka, dowie się z tekstu tego, co obiecuje lista „Ten rozdział obejmuje”. Drugie czytanie sprawdzało, czy zdanie jest pełne, czy termin jest objaśniony przy pierwszym użyciu i czy nagłówek da się zrozumieć bez znajomości wewnętrznych skrótów kompilatora. Zdanie, które brzmiało jak notatka, etykieta albo neologizm, zostało przepisane. Poniżej jest werdykt i dwa albo trzy przykłady poprawki. Wersja „przed” to zdanie odrzucone w redakcji. Wersja „po” jest w pliku, który wchodzi do PDF.

## Przedmowa

Werdykt: do druku. Czytelnik dowiaduje się, skąd wzięła się zasada regionu, czym ta książka różni się od opisu języka na papierze i że część programów przechodzi sprawdzenie, a nie daje się zbudować.

Przed: „Bork wybiera regiony zamiast GC i borrow checkera.”

Po: „Bork wybiera inną zasadę. Nawiasy klamrowe nie są tylko sposobem grupowania instrukcji. Oznaczają region pamięci.”

Przed: „Nie ma GC i nie ma referencji do schowania w polu struktury.”

Po: „Nie ma odśmiecania w tle i nie ma ogólnych referencji, które można by schować w polu struktury. Struktur zresztą w obecnej wersji języka nie ma.”

## O tej książce

Werdykt: do druku. Widać, dla kogo jest książka, jak ją czytać i czego nie obiecuje.

Przed: „Przy każdym listingu warto odpalić go samemu.”

Po: „Przy każdym listingu, o którym piszę, że został uruchomiony, warto uruchomić go samodzielnie.”

Przed: „Gdy language.md się rozjeżdża, wygrywa kompilator.”

Po: „Gdy dokument `docs/language.md` rozmija się z kompilatorem, opisuję to, co robi kompilator, i mówię, na czym polega różnica.”

## Rozdział 1. Pierwszy program i sposób jego uruchomienia

Werdykt: do druku. Czytelnik uruchamia pierwszy program, odróżnia sprawdzenie od budowania i widzi dwa pierwsze błędy.

Przed: „Samą poprawność programu sprawdzisz bez biblioteki LLVM.”

Po: „Samą poprawność programu sprawdzisz bez biblioteki LLVM. LLVM to biblioteka, która później tłumaczy sprawdzony program na kod maszynowy. Na tym etapie nie jest potrzebna.”

Przed: „Dump aren pokazuje Local.”

Po: „Znacznik `Local` przy nazwie oznacza, że nazwa powstała w tym regionie. W rozdziale 8 pojawią się pozostałe znaczniki: kopiowanie, współdzielenie i przeniesienie.”

## Rozdział 2. Po co Borkowi regiony pamięci

Werdykt: do druku. Region, arena i trzy sposoby przejścia granicy są opisane zanim pojawią się szczegóły składni.

Przed: „Słowo region będę dalej używał w tym właśnie znaczeniu.”

Po: „Dalej używam słowa region w tym właśnie znaczeniu. Region jest jednocześnie fragmentem programu i czasem życia pamięci z nim związanej.”

Przed: „Dziecko nie kopiuje znaków i nie odbiera własności.”

Po: „Region wewnętrzny nie kopiuje znaków i nie odbiera własności.”

## Rozdział 3. Z czego składa się plik źródłowy

Werdykt: do druku. Widać, że plik jest listą funkcji, że nowa linia kończy instrukcję i że blok otwiera region.

Przed: „Cel przypisania stoi po lewej stronie dwukropka przypisania.”

Po: „Cel przypisania stoi po lewej stronie znaku równości.”

Przed: „NL w nawiasach znika.”

Po: „Dzięki temu zamiana nowych linii wewnątrz nawiasów na odstęp naprawdę ukrywa je przed lekserem.” To zdanie jest w rozdziale 13. W rozdziale 3 ta sama zasada jest opisana jako łamanie wiersza wewnątrz nawiasów, bez skrótu „NL” w nagłówku.

## Rozdział 4. Typy danych i ich znaczenie

Werdykt: do druku. Czytelnik odróżnia typy proste, napis, tablicę i wartość pustą oraz wie, co jest kopiowalne.

Przed: „Słowo obniżyć znaczy tu tłumaczenie na LLVM. Dalej będę mówił o tłumaczeniu, a słowo obniżyć zostawiam.”

Po: „Generator kodu nie umie przetłumaczyć tej konstrukcji na instrukcje. Odrzuca ją kontrola przed generowaniem kodu.”

Przed: „Copy = prymityw nienullowalny.”

Po: „Kopiowalne są tylko typy proste, które nie mogą być puste.”

## Rozdział 5. Funkcje, parametry i wywołania

Werdykt: do druku. Widać kształt funkcji, własność na granicy wywołania i to, że napis jako argument funkcji użytkownika wywraca kompilator.

Przed: „String do user function: bramka milczy, ICE w expr.rs.”

Po: „Wywołanie z napisem przechodzi sprawdzenie. Przy budowaniu kompilator kończy się awarią w `src/codegen/llvm/expr.rs`, w funkcji, która zakłada liczbę całkowitą.”

Przed: „Trailing closure jest w AST, codegen nie.”

Po: „Funkcja dopisana na końcu wywołania jest w drzewie składni. Generator kodu jej nie tłumaczy i zgłasza zwykły komunikat.”

## Rozdział 6. Warunki, pętle i powrót z funkcji

Werdykt: do druku. Warunek jako wyrażenie, zakres otwarty z prawej, `while`, `break`, `continue` i zwieranie są opisane razem ze skutkiem w regionie.

Przed: „Analiza nie rozjeżdża się z tym, co naprawdę zostanie wyemitowane.”

Po: „Analiza pozostaje zgodna z tym, co naprawdę zostanie wyemitowane.”

Przed: „for 0..5 to suma 10, half-open.”

Po: „Zakres `0..5` jest otwarty z prawej strony. Ciało wykonuje się dla 0, 1, 2, 3 i 4. Suma wynosi 10.”

## Rozdział 7. Tablice o znanej z góry długości

Werdykt: do druku. Długość jako część typu, indeks, wycinek i napis w tablicy są wyjaśnione, a nie tylko nazwane.

Przed: „Długość jest typem.”

Po: nagłówek „Dlaczego długość należy do typu tablicy” i zdanie: „Długość nie jest wyrażeniem liczonym w czasie działania. Jest częścią typu, tak jak szerokość liczby `i32` jest częścią typu, a nie wartością trzymaną obok liczby.”

Przed: „Slice to widok, bounds literalne, brak runtime check.”

Po: „Wycinek wymaga granic będących literałami typu `i32`. Typ wyniku ma długość równą różnicy tych granic. To widok na te same elementy, a nie kopia.”

## Rozdział 8. Kopiowanie, współdzielenie i przeniesienie wartości

Werdykt: do druku. Trzy sposoby użycia nazwy, `move`, `promote`, przypisanie do zmiennej zewnętrznej i zakaz w pętli są opisane pełnymi zdaniami.

Przed: „Dwa sprawdzania, jedna faza.”

Po: „Własność nazw i czas życia bajtów mają jedną fazę w komunikacie.”

Przed: „Jawna lista wygrywa z zgadywaniem.”

Po: „Jeśli programista podał listę, wygrywa ona ze zgadywaniem, także gdy jest pusta.”

Przed: „Czas życia bajtów jest drugim sitem.”

Po: „Czas życia bajtów sprawdza dopiero analiza ucieczki.”

## Rozdział 9. Jak Bork przechowuje napisy w pamięci

Werdykt: do druku. Czytelnik widzi deskryptor, miejsce alokacji, dwie sąsiednie instrukcje i przykład z dokumentacji, który się nie kompiluje.

Przed: „Bajty String.”

Po: „Adres i długość, a osobno znaki.” Dalej: napis w programie jest parą adresu i długości, a znaki leżą w buforze regionu.

Przed: „Zwrot przeniesionego albo podniesionego napisu.”

Po: „Zwrot napisu przeniesionego albo użytego ze słowem `promote` radzi użyć zwykłej nazwy albo literału.”

## Rozdział 10. Komunikaty kompilatora i błędy podczas działania

Werdykt: do druku. Fazy, kształt linii, błędy składni, typów, własności i generowania kodu oraz awaria kompilatora są rozdzielone.

Przed: „Inkwell panikuje na into_int_value.”

Po: „Inkwell, czyli biblioteka Rusta, przez którą kompilator woła LLVM, w takiej sytuacji przerywa proces, zamiast zwrócić błąd.”

Przed: „User error span = EOF.”

Po: „Błąd zgłoszony przez regułę gramatyki wskazuje koniec pliku, bo akcja gramatyki nie niesie pozycji.”

## Rozdział 11. Funkcje wbudowane i sposób pisania programów

Werdykt: do druku. Widać, że biblioteka języka to trzy funkcje, i które zapisy dają się zbudować.

Przed: „typeck nie przyjmie wycinka o zmiennych granicach.”

Po: „Wycinka o granicach trzymanych w zmiennych sprawdzanie typów nie przyjmie, bo typ wyniku nie miałby znanej długości.”

Przed: „Konsolidacja z runtime'em.”

Po: „Program użytkownika jest konsolidowany z biblioteką wykonawczą.”

Przed: „Programy, które warto przepisywać.”

Po: „Jak pisać programy, które kompilator potrafi zbudować.”

## Rozdział 12. Od pliku źródłowego do gotowego programu

Werdykt: do druku po przepisaniu od czystej kartki. Rozdział zaczyna od pytania, co kompilator musi wiedzieć, zanim wolno mu emitować kod, a nazwy plików stoją na końcu jako mapa. Prześledzony jest `18-trace.bork` (wydruk regionów, kod 0, wypis `sum`, kod procesu 3) oraz plik z błędem własności i typu naraz.

Zdania do 5 słów: 0 z 128, czyli 0,00%. Słów w prozie, bez listingów: 2279.

Przed: „Analiza własności nie dostaje całej reprezentacji pośredniej. Dostaje drzewo składni i cienki wektor typów deklaracji.”

Po: „Zanim kompilator wyemituje choć jedną instrukcję, musi odpowiedzieć na kilka pytań, które nie mają ze sobą nic wspólnego poza tym, że dotyczą tego samego pliku.”

## Rozdział 13. Jak kompilator czyta tekst programu

Werdykt: do druku po przepisaniu od czystej kartki. Widać, po co drzewo składni powstaje wcześniej niż typy, czemu nowa linia w nawiasie nie kończy instrukcji i czemu błąd lewej strony przypisania wskazuje koniec pliku. Wydruki pochodzą z uruchomienia kompilatora.

Zdania do 5 słów: 0 z 96, czyli 0,00%. Słów w prozie, bez listingów: 1683.

Przed: „Słowa kluczowe w lekserze to `if`, `fun`, `val`, `var`, `for`, `in`, `return`, `Some`, `None`, `move` i `promote`.”

Po: „Zamiana jest bajt za bajt, więc pozycja błędu w komunikacie nadal wskazuje oryginalny plik, a nie przerobioną kopię.”

## Rozdział 14. Jak kompilator sprawdza własność nazw

Werdykt: do druku po przepisaniu od czystej kartki. Cztery odpowiedzi o odczycie są narysowane, zanim padną nazwy funkcji, a pętla i scalanie gałęzi `if` mają własne przebiegi z komunikatem.

Zdania do 5 słów: 0 z 81, czyli 0,00%. Słów w prozie, bez listingów: 1353.

Przed: „Korzeń drzewa dla całego programu trzyma struktura `ArenaReport`. Ma ona po jednym korzeniu na funkcję.”

Po: „Liczbę całkowitą wolno przeczytać wiele razy, bo odczyt nic nie zabiera. Napis albo tablica tak się nie zachowują.”

## Rozdział 15. Jak kompilator sprawdza typy

Werdykt: do druku po przepisaniu od czystej kartki. Oczekiwany typ jest pokazany na parze programów, z których jeden przechodzi (`f(1)` przy parametrze `i64`), a drugi dostaje komunikat o argumencie `i32`. Pusta tablica i zapis do `val` są odmowami, których kontekst nie uzupełnia.

Zdania do 5 słów: 0 z 81, czyli 0,00%. Słów w prozie, bez listingów: 1480.

Przed: „Typ w reprezentacji pośredniej, struktura `Ty` w pliku `src/hir/ty.rs`, ma rodzaj i znacznik, czy wartość może być pusta.”

Po: „Oczekiwany typ spływa w dół, do literału, i literał przyjmuje właśnie ten typ, o ile jest liczbowy.”

## Rozdział 16. Jak kompilator wybiera miejsce na napis

Werdykt: do druku po przepisaniu od czystej kartki. Trzy pytania o miejsce padają dopiero po czystych typach i własności. Listing `15-assign-up.bork` wypisuje `b` po zamknięciu bloku, a zwrot `concat` daje komunikat fazy `ownership` bez numeru wiersza.

Zdania do 5 słów: 0 z 74, czyli 0,00%. Słów w prozie, bez listingów: 1280.

Przed: „Plik `src/escape.rs` pyta, na jakiej głębokości leżą bajty i czy odbiorca leży nie głębiej niż one.”

Po: „Literał jest kładziony w regionie nazwy, która go przyjmuje, a nie w bloku, w którym akurat stoi znak równości.”

## Rozdział 17. Jak powstaje kod maszynowy

Werdykt: do druku po przepisaniu od czystej kartki. Kontrola przed emisją, deskryptor `{ ptr, i64 }` i awaria przy napisie przekazanym do funkcji użytkownika są rozdzielone. `None` daje fazę `codegen`, a `26-pass-string.bork` kończy się kodem 101 w `src/codegen/llvm/expr.rs`.

Zdania do 5 słów: 0 z 77, czyli 0,00%. Słów w prozie, bez listingów: 1253.

Przed: „Symbol `main` jest funkcją `main` w konwencji C. W LLVM ma typ `i32` bez parametrów.”

Po: „Przekazanie napisu do funkcji napisanej przez programistę przechodzi sprawdzenie, przechodzi kontrolę kształtów i wywraca emisję.”

## Rozdział 18. Komunikaty błędów, polecenia i edytor

Werdykt: do druku po przepisaniu od czystej kartki. Linia terminala jest rozłożona na pola, a podpowiedź w edytorze ma pokazany prawdziwy tekst dla parametru `x` (`Local` w regionie `fun add`). Różnica bajtów i pozycji protokołu jest opisana bez skrótu z nazwy funkcji w roli podmiotu zdania.

Zdania do 5 słów: 0 z 85, czyli 0,00%. Słów w prozie, bez listingów: 1424.

Przed: „Plik `src/main.rs` przyjmuje dwie formy. Pierwsza to `bork`, opcjonalnie z `--dump-arenas`, i ścieżka pliku.”

Po: „Odpowiedź nie jest typem w sensie reprezentacji pośredniej. Jest zdaniem o regionie i o tym, jak nazwa została sklasyfikowana.”

## Rozdział 19. Jak czytać repozytorium

Werdykt: do druku. Jest mapa katalogów, kolejność czytania i ostrzeżenie, że plik modelu bufora nie jest drzewem regionów.

Przed: „Drzewo, które warto znać. Bramka, LLVM i runtime. Escape odrzuca bajty, które przeżyłyby swoją płytę.”

Po: „Katalogi, które mają znaczenie. Plik `escape.rs` odrzuca bajty, które byłyby użyte po zwolnieniu ich bufora.”

Przed: „Spacer jest używany przez stempel.”

Po: „Spacer jest wtedy używany przez oznaczenie regionów, a część metod tylko przez emisję LLVM.”

## Rozdział 20. Jeden program od źródła do uruchomienia

Werdykt: do druku. Jeden program przechodzi od tekstu do kodu wyjścia 3, a czytelnik widzi, co na każdym kroku robi kompilator.

Przed: „Węzeł w dumpu, brak płyty. Zatrzask nie resetuje cudzej płyty.”

Po: „Węzeł pętli zostaje w raporcie, ale nie prosi o `bork_arena_push`. Wydruk ma węzeł, a w czasie działania pętla nie dostaje własnego bufora.”

Przed: „Stempel, hoist i escape milczą.”

Po: „Oznaczenie buforów, wyniesienie i ucieczka nie mają tu pracy poza korzeniem funkcji.”

## Rozdział 21. Testy, szukanie przyczyny błędu i nowa konstrukcja języka

Werdykt: do druku. Widać dwa polecenia testów, jak czytać fazę komunikatu i w jakiej kolejności doszła pętla `while`.

Przed: „Bez haka while LLVM i raport się rozjadą.”

Po: „Bez tego wygenerowany kod i raport przestaną pasować do siebie w chwili, gdy ciało alokuje napis.”

Przed: „Jeśli emisji nie ma, bramka jest obowiązkowa.”

Po: „Jeśli emisji nie ma, odmowa w kontroli przed generowaniem kodu jest obowiązkowa.”

## Rozdział 22. Ćwiczenia

Werdykt: do druku. Zadania da się rozwiązać kompilatorem, a odpowiedzi cytują komunikat albo plik.

Przed: „Nazwij funkcję, która panikuje, i powiedz, czemu bramka tego nie łapie.”

Po: „Nazwij funkcję, która się wywraca, i powiedz, czemu kontrola przed generowaniem kodu tego nie łapie.”

Przed: „Bramka ogląda kształt HIR, więc Call milczy.”

Po: „Kontrola przed generowaniem kodu ogląda kształt reprezentacji pośredniej. Wywołanie z argumentem napisowym wygląda jak zwykłe wywołanie, więc kontrola milczy.”

## Dodatek A. Zestawienie składni

Werdykt: do druku. Zestawienie zostało, ale każde zestawienie ma zdanie, które mówi, jak je czytać.

Przed: „Gwiazdka: checker przyjmuje, bork build nie obniża albo panikuje.”

Po: „Gwiazdka przy wierszu znaczy, że sprawdzenie program przyjmuje, a `bork build` albo odrzuca go komunikatem, albo kończy się awarią kompilatora.”

Przed: „Copy: nienullowalny prymityw. Reszta nie.”

Po: „Kopiowalne są tylko typy proste, które nie mogą być puste. Napis, tablica, typ funkcji i wartość pusta kopiowalne nie są.”

## Dodatek B. Słownik pojęć

Werdykt: do druku. Hasła są zdaniami. Arena w dwóch znaczeniach jest rozdzielona. Nie ma haseł „płyta” i „bramka”.

Przed: „Arena. W runtime: płyta 4096 bajtów. Bramka: przejście po HIR.”

Po: „Arena. Bufor jednego regionu, o pojemności 4096 bajtów. Kontrola przed generowaniem kodu. Zejście po reprezentacji pośredniej, zanim powstanie moduł LLVM.”

Przed: „Escape. Pilnuje, żeby bajty nie były użyte po śmierci płyty.”

Po: „Analiza ucieczki. Przejście, które liczy głębokość bajtów i odrzuca użycie, które przeżyłoby zwolnienie bufora regionu.”

## Dodatek C. Czego kompilator jeszcze nie potrafi

Werdykt: do druku. Lista braków jest opisana zdaniami. Różnice dokumentu i kodu oraz awarie są oddzielone od rzeczy pokrytych testami.

Przed: „Niedokończone, rozjazdy i paniki kompilatora. Pętla resetuje płytę.”

Po: „Czego kompilator jeszcze nie potrafi. Krótki przykład trudno przepchnąć ponad 4096 bajtów bez pętli, która alokuje, a pętla czyści bufor przy obiegu.”

Przed: „Bramka milczy. Kod procesu 101.”

Po: „Przekazanie napisu do funkcji użytkownika przechodzi sprawdzenie i kontrolę przed generowaniem kodu, a potem kompilator kończy się awarią. Kod procesu kompilatora to 101.”
