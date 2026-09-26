# Rozdział 18. Komunikaty błędów, polecenia i edytor

## Ten rozdział obejmuje

- jedną strukturę komunikatu dla wiersza poleceń i dla edytora
- co robią polecenia `bork` i `bork build`
- co naprawdę ogłasza serwer językowy
- co pokazuje rozszerzenie Cursora i Visual Studio Code
- czego podpowiedź pod kursorem nie zawiera

## Jeden komunikat ma fazę, treść i opcjonalny zakres

Plik `src/diag.rs` definiuje strukturę `Diagnostic`. Są cztery fazy: czytanie składni, własność, typ i generowanie kodu. Są dwa poziomy: błąd i ostrzeżenie. Jest treść i opcjonalny zakres źródłowy. Nie ma kodów numerycznych, nie ma linii zaczynających się od `help:` i nie ma linii zaczynających się od `note:`. Jeden komunikat niesie jedną myśl. Gdy kompilator chce powiedzieć dwie rzeczy, dostajesz dwa komunikaty. Program z pętlą i przeniesieniem z rozdziału 2 jest takim przypadkiem.

Czytanie składni zamienia błąd parsera funkcją `from_parse` i dostaje fazę `parse`. Błędy analizy własności dostają fazę `ownership` przez `from_sema`. Błędy sprawdzania typów powstają w środowisku sprawdzania i mają fazę `type`. Analiza ucieczki też dostaje fazę `ownership`. Kontrola przed generowaniem kodu i błędy emisji mają fazę `codegen`.

Serwer edytora nie uruchamia `bork build`. Fazy generowania kodu w edytorze nie zobaczysz, dopóki ktoś nie wstawi tej kontroli do `frontend::check`. Dziś kontrola jest tylko w `codegen::build`. Program ze słowem `None` w edytorze wygląda na poprawny, a `bork build` go odrzuca. Zielone podkreślenie w edytorze nie znaczy, że powstanie plik wykonywalny.

## Dwa polecenia i kody zakończenia

Plik `src/main.rs` przyjmuje dwie formy. Pierwsza to `bork`, opcjonalnie z `--dump-arenas`, i ścieżka pliku. Druga to `bork build`, opcjonalnie z `-o` i ścieżką wyjścia, oraz ścieżka pliku.

Wydruk drzewa regionów pojawia się na standardowym wyjściu tylko wtedy, gdy pole raportu w wyniku sprawdzenia jest obecne. Przy błędzie składni jest cicho, poza komunikatami na standardowym wyjściu błędów. Przy błędzie typu drzewo zostaje. Prośba o pomoc kończy się kodem zero. Nieznany znacznik kończy się kodem dwa.

Polecenie `build` bez opcji kompilacji `codegen` nie próbuje wołać LLVM. Mówi, że ta opcja jest wymagana. Z opcją najpierw sprawdza program, potem buduje. Sukces kończy się kodem zero, także gdy program tylko wypisuje tekst i funkcja `main` nie ma zadeklarowanego wyniku. Komunikaty programu kończą się kodem jeden. Problem z narzędziami, na przykład brak `clang`, kończy się kodem dwa. Awaria samego kompilatora, czyli panika Rusta, kończy się kodem 101. Przerwanie procesu użytkownika przez `abort` daje w powłoce kod 134.

Nakładanie ścieżki wejścia i wyjścia jest sprawdzane najpierw porównaniem ścieżek, a potem kanonizacją, gdy oba pliki istnieją. Nie jest to pełne rozwiązanie dowiązań symbolicznych w każdą stronę. Wystarcza, żeby nie nadpisać źródła domyślnym wyjściem wtedy, gdy to naprawdę ten sam plik. Domyślne wyjście ma obcięte rozszerzenie, więc `a.bork` staje się `a`. Konflikt trzeba wymusić ręcznie, podając `-o a.bork` przy wejściu `a.bork`.

## Serwer liczy sprawdzenie od zera przy każdej zmianie

Pliki `src/bin/bork_lsp.rs` i `src/lsp.rs` składają serwer, gdy włączona jest opcja `lsp`. Transport idzie przez standardowe wejście i wyjście. Biblioteka to `tower-lsp`.

Ogłaszane możliwości to synchronizacja pełnym tekstem dokumentu, podpowiedź pod kursorem oraz polecenie `bork.dumpArenas`. Nie ma uzupełniania, przechodzenia do definicji, wyszukiwania referencji, formatowania, listy symboli ani szybkich poprawek. Powiadomienie o zmianie bierze ostatnią wersję tekstu i liczy analizę od zera. Nie ma parsera przyrostowego.

Analiza woła `lsp::analyze_source`, a ta woła `frontend::check`. To ta sama kontrola co w wierszu poleceń. Specyfikacja rozszerzenia z 21 września 2026 opisywała komunikaty samego parsera. To jest nieaktualne. Plik `README` w jednym nagłówku nadal mówi o diagnostyce składni, a trzy zdania niżej opisuje pełne sprawdzenie. Wierz funkcji `analyze_source`, a nie nagłówkowi.

Komunikat w edytorze ma źródło `bork` i treść złożoną z fazy oraz wiadomości. Pozycje są w jednostkach UTF-16, zgodnie z protokołem. Pusty zakres, w którym początek równa się końcowi, jest rozszerzany o jedną jednostkę, żeby podkreślenie było widoczne.

Licznik epoki na adresie dokumentu odrzuca spóźnione publikowanie komunikatów, gdy użytkownik zdążył wpisać kolejną wersję. To jest jedyna współbieżność, o którą serwer dba. Nie ma puli procesów roboczych.

## Podpowiedź pokazuje region i własność, a nie typ

Podpowiedź pod kursorem szuka słowa złożonego ze znaków ASCII, cyfr i podkreślenia. Trafienie porównuje z zakresem wiązania w drzewie regionów. Tekst w Markdownie mówi, przy jakiej etykiecie regionu leży nazwa, i jaki jest rodzaj własności. Nie ma typu z reprezentacji pośredniej. Specyfikacja zostawiła typ na podpowiedzi jako rzecz do zrobienia. Commit, który podpiął serwer do pełnego sprawdzenia, też traktował typ jako opcjonalny. Nadal go nie ma.

Słowo arena w tym zdaniu podpowiedzi znaczy etykietę regionu z drzewa analizy własności. Nie znaczy bufora 4096 bajtów. Rozdział 2 rozdziela te dwa znaczenia. W tekście podpowiedzi angielskie słowo `arena` zostaje, bo tak formatuje je serwer.

Polecenie `bork.dumpArenas` bierze opcjonalny adres dokumentu albo pierwszy otwarty dokument. Wynik to JSON z polem `dump` oraz komunikat w oknie dziennika. Przy błędzie składni JSON ma pole `error`. Wydruk przy błędach znaczenia dopisuje linie zaczynające się od `# semantic errors`. To ten sam tekst co `--dump-arenas`, plus ten dopisek.

## Rozszerzenie uruchamia serwer z katalogu projektu

Katalog `tools/bork-lsp-extension` aktywuje się dla języka `bork` i dla plików o rozszerzeniu `.bork`. Gramatyka kolorowania jest w `syntaxes/bork.tmLanguage.json`. Konfiguracja nawiasów i komentarza `//` jest w `language-configuration.json`.

Plik `server.js` uruchamia `target/debug/bork-lsp` z katalogu otwartego projektu. Gdy tej binarki nie ma, woła `cargo run --quiet --bin bork-lsp`. Bez otwartego folderu rozszerzenie nie startuje serwera. Polecenie w interfejsie nazywa się `bork.dumpArenasView` i woła polecenie serwera `bork.dumpArenas`. Test `test/server.test.js` pilnuje tej różnicy nazw oraz wyboru binarki.

W pliku `package.json` nie ma ustawień ani adaptera debuggera, a kolorowanie składni nie wie o błędach typów, bo od tego są komunikaty serwera.

## Podsumowanie

- Jedna struktura komunikatu zasila wiersz poleceń i edytor. Generowanie kodu do edytora nie dochodzi.
- Plik bez podkreśleń w edytorze może nadal paść przy `bork build`, albo komunikatem kontroli, albo awarią kompilatora.
- Podpowiedź pod kursorem pokazuje etykietę regionu i własność, a nie typ.
- Wydruk w edytorze i znacznik `--dump-arenas` dzielą funkcję `dump_arenas`.
- Serwer liczy sprawdzenie od zera przy każdej zmianie i wyrzuca spóźnione odpowiedzi licznikiem epoki dokumentu.
