# Rozdział 18. Diagnostyki, CLI i serwer LSP

## Ten rozdział obejmuje

- Jedną strukturę `Diagnostic` dla CLI i LSP
- Zachowanie binarki `bork`
- Protokół, który `bork-lsp` naprawdę ogłasza
- Rozszerzenie Cursora / VS Code
- Czego hover nie pokazuje

## `Diagnostic`

`src/diag.rs`. Cztery fazy, dwa poziomy (`Error`, `Warning`), wiadomość,
opcjonalny `Span`. Nie ma kodów numerycznych, podpowiedzi `help:` ani
notatek `note:`. Jedna linia, jedna myśl. Gdy kompilator chce powiedzieć
dwie rzeczy, dostajesz dwie diagnostyki. Listing 2.4 (pętla i move) jest
takim przypadkiem.

Mapowania:

| Funkcja | Faza |
|---|---|
| `from_parse` | `parse` |
| `from_sema` | `ownership` |
| błędy typeck (budowane w `Env::error`) | `type` |
| `escape` | `ownership` |
| `gate` i `codegen_error` | `codegen` |

LSP nie uruchamia `bork build`, więc fazy `codegen` w edytorze nie
zobaczysz, dopóki ktoś nie wpieje bramki w `frontend::check`. Dziś bramka
jest tylko w `codegen::build`. Program z `None` w edytorze jest „czysty”,
a `bork build` go odrzuca. To jest ważne dla czytelnika, który ufa
zielonej lampce.

## CLI

`src/main.rs`. Dwie formy:

```text
bork [--dump-arenas] <plik>
bork build [-o ścieżka] <plik>
```

`--dump-arenas` drukuje na stdout tylko gdy `report` jest `Some`. Przy
błędzie parse jest cicho (poza stderr). Przy błędzie typu drzewo jest.
`--help` kończy się kodem 0. Nieznany znacznik kończy się kodem 2.

`build` bez feature `codegen` nie próbuje LLVM. Mówi, że feature jest
wymagany. Z feature: check, potem `build`, potem albo sukces (kod 0, nawet
gdy programem jest `println` i nic nie zwraca), albo diagnostyki (1), albo
toolchain (2).

Nakładanie ścieżek jest sprawdzane najpierw porównaniem ścieżek, potem
`canonicalize`, gdy oba pliki istnieją. Nie jest to pełne rozwiązanie
linków symbolicznych w każdą stronę. Wystarcza, żeby nie nadpisać źródła
domyślnym wyjściem `plik` z `plik.bork` tylko wtedy, gdy to naprawdę ten
sam plik. Domyślne wyjście ma obcięte rozszerzenie, więc `a.bork` → `a`.
Konflikt trzeba wymusić ręcznie (`-o a.bork` przy wejściu `a.bork`).

## Serwer

`src/bin/bork_lsp.rs` i `src/lsp.rs`. Feature `lsp`. Transport stdio,
biblioteka `tower-lsp`.

Ogłaszane możliwości:

- synchronizacja pełnym tekstem dokumentu (`FULL`),
- hover,
- komenda `bork.dumpArenas`.

Nie ma uzupełniania, definicji, referencji, formatowania, symboli ani
code action. `didChange` bierze ostatnią zmianę z powiadomienia i liczy
analizę od zera. Nie ma inkrementalnego parsera.

Analiza to `lsp::analyze_source` → `frontend::check`. Ten sam potok co
CLI. Specyfikacja rozszerzenia z 21 września 2026 opisywała diagnostyki
samego parsera. To jest nieaktualne. README w jednym nagłówku nadal mówi
„Parse diagnostics”, a trzy zdania niżej opisuje pełny check. Wierz
`analyze_source`, nie nagłówkowi.

Diagnostyka LSP ma `source: "bork"` i treść `"{faza}: {wiadomość}"`.
Pozycje są UTF-16, zgodnie z protokołem. Pusty span (start == koniec) jest
rozszerzany o jedną jednostkę, żeby podkreślenie było widoczne.

Epoka na URI odrzuca spóźnione `publishDiagnostics`, gdy użytkownik zdążył
wpisać kolejną wersję. To jest jedyna współbieżność, o którą serwer dba.
Nie ma puli procesów roboczych.

## Hover i dump

Hover szuka słowa pod kursorem (ASCII alfanumeryczne i `_`) i trafienia w
span wiązania w drzewie aren. Tekst markdown:

```text
`nazwa` in arena `etykieta`
Ownership: …
```

Nie ma typu HIR. Specyfikacja zostawiła typ na hoverze jako follow-up.
Commit, który podpinał LSP do pełnego checku, też traktował typ jako
opcjonalny. Nadal go nie ma.

Komenda `bork.dumpArenas` bierze opcjonalne URI albo pierwszy otwarty
dokument. Wynik to JSON `{ "dump": "..." }` plus `window/logMessage`.
Przy błędzie parse JSON ma `{ "error": "..." }`. Dump przy błędach
semantycznych dopisuje linie `# semantic errors`. To ten sam ASCII co
`--dump-arenas`, plus ten dopisek.

## Rozszerzenie

`tools/bork-lsp-extension`. Aktywacja na języku `bork`, rozszerzenie pliku
`.bork`. Gramatyka TextMate w `syntaxes/bork.tmLanguage.json`. Konfiguracja
nawiasów i komentarza `//` w `language-configuration.json`.

`server.js` uruchamia `target/debug/bork-lsp` z katalogu workspace, a gdy
binarki nie ma, `cargo run --quiet --bin bork-lsp`. Bez otwartego folderu
rozszerzenie nie startuje serwera. Komenda UI nazywa się
`bork.dumpArenasView` i woła komendę serwera `bork.dumpArenas`. Test
`test/server.test.js` pilnuje tej różnicy nazw oraz wyboru binarki.

Nie ma ustawień w `package.json`. Nie ma adaptera debuggera. Podświetlanie
nie wie o błędach typów; od tego są diagnostyki LSP.

## Podsumowanie

- Jedna struktura `Diagnostic` zasila CLI i LSP. Codegen do LSP nie dochodzi.
- Zielony plik w edytorze może nadal paść na `bork build` (bramka albo panic).
- Hover pokazuje arenę i własność, nie typ.
- Dump w edytorze i `--dump-arenas` dzielą `dump::dump_arenas`.
- Serwer liczy check od zera przy każdej zmianie i wyrzuca spóźnione odpowiedzi epoką URI.
