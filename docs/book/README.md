# Bork in Action

Książka techniczna o języku Bork i jego kompilatorze. Opisuje drzewo
`main` w rewizji `cd8fee4` (stan repozytorium w chwili pisania). Kod
kompilatora nie został zmieniony na potrzeby tej książki.

Wszystkie listingi oznaczone jako **uruchomione** zostały sprawdzone
komendą `bork` zbudowaną z flagą `codegen` (LLVM 23.1.2, rustc 1.98.1)
albo samą ścieżką `frontend::check`, gdy codegen danego programu nie
obejmuje. Komunikaty błędów są cytowane dosłownie.

## Spis treści

### Materiały wstępne

1. [Przedmowa](00-przedmowa.md)
2. [O tej książce](01-o-tej-ksiazce.md)

### Część I. Język

3. [Rozdział 1. Pierwszy program i narzędzia](02-wprowadzenie.md)
4. [Rozdział 2. Filozofia: regiony zamiast GC i borrow checkera](03-filozofia.md)
5. [Rozdział 3. Składnia, wiązania i układ pliku](04-skladnia.md)
6. [Rozdział 4. Typy](05-typy.md)
7. [Rozdział 5. Funkcje, wywołania i domknięcia](06-funkcje.md)
8. [Rozdział 6. Kontrola przepływu](07-przeplyw.md)
9. [Rozdział 7. Tablice](08-tablice.md)
10. [Rozdział 8. Regiony, Copy, Shared i Move](09-regiony.md)
11. [Rozdział 9. Gdzie żyją bajty `String`](10-stringi.md)
12. [Rozdział 10. Gdy kompilator borks: diagnostyka i aborcja](11-bledy.md)
13. [Rozdział 11. Biblioteka, brak modułów i idiomy](12-biblioteka-idiomy.md)

### Część II. Architektura kompilatora

14. [Rozdział 12. Pipeline od źródła do binarki](13-pipeline.md)
15. [Rozdział 13. Lexer, parser i AST](14-parser.md)
16. [Rozdział 14. Analiza regionów (`sema`)](15-sema.md)
17. [Rozdział 15. Typeck i typowany HIR](16-typeck.md)
18. [Rozdział 16. Hoist, escape i `region_walk`](17-pamiec-srodkowa.md)
19. [Rozdział 17. Bramka, LLVM i runtime](18-codegen.md)
20. [Rozdział 18. Diagnostyki, CLI i serwer LSP](19-lsp.md)

### Część III. Jak czytać repozytorium

21. [Rozdział 19. Mapa repozytorium](20-mapa.md)
22. [Rozdział 20. Jeden program przez cały kompilator](21-slad.md)
23. [Rozdział 21. Testy, debugowanie i nowa funkcja języka](22-praca-z-repo.md)
24. [Rozdział 22. Ćwiczenia](23-cwiczenia.md)

### Dodatki

25. [Dodatek A. Ściągawka składni](A-sciagawka.md)
26. [Dodatek B. Słowniczek](B-slowniczek.md)
27. [Dodatek C. Niedokończone, rozjazdy i paniki kompilatora](C-niedokonczone.md)

Programy użyte w książce leżą w [`przyklady/`](przyklady/). PDF tej książki:
[`bork-in-action.pdf`](bork-in-action.pdf). Skład: [`build-pdf.sh`](build-pdf.sh).
