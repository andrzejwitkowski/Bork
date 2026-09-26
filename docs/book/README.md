<p align="center">
  <img src="../../assets/bork-logo.png" alt="Bork" width="280">
</p>

# Bork in Action

Książka techniczna o języku Bork i jego kompilatorze. Opisuje drzewo `main` w rewizji `cd8fee4`, czyli stan repozytorium w chwili sprawdzania przykładów. Kod kompilatora nie został zmieniony na potrzeby tej książki.

Listing oznaczony jako uruchomiony został sprawdzony poleceniem `bork`, zbudowanym z opcją `codegen`, na LLVM 23.1.2 i rustc 1.98.1, albo samym sprawdzeniem `frontend::check`, gdy generowanie kodu tego programu nie obejmuje. Komunikaty błędów są cytowane tak, jak wypisuje je kompilator.

Książka ma trzy części. Pierwsza uczy języka. Druga tłumaczy, jak kompilator czyta program, sprawdza go i tłumaczy na kod maszynowy. Trzecia mówi, w jakiej kolejności czytać repozytorium i jak dopisać konstrukcję. Dodatki zbierają składnię, słownik i listę rzeczy, których ta wersja kompilatora jeszcze nie robi.

## Spis treści

### Materiały wstępne

1. [Przedmowa](00-przedmowa.md)
2. [O tej książce](01-o-tej-ksiazce.md)

### Część I. Język

3. [Rozdział 1. Pierwszy program i sposób jego uruchomienia](02-wprowadzenie.md)
4. [Rozdział 2. Po co Borkowi regiony pamięci](03-filozofia.md)
5. [Rozdział 3. Z czego składa się plik źródłowy](04-skladnia.md)
6. [Rozdział 4. Typy danych i ich znaczenie](05-typy.md)
7. [Rozdział 5. Funkcje, parametry i wywołania](06-funkcje.md)
8. [Rozdział 6. Warunki, pętle i powrót z funkcji](07-przeplyw.md)
9. [Rozdział 7. Tablice o znanej z góry długości](08-tablice.md)
10. [Rozdział 8. Kopiowanie, współdzielenie i przeniesienie wartości](09-regiony.md)
11. [Rozdział 9. Jak Bork przechowuje napisy w pamięci](10-stringi.md)
12. [Rozdział 10. Komunikaty kompilatora i błędy podczas działania](11-bledy.md)
13. [Rozdział 11. Funkcje wbudowane i sposób pisania programów](12-biblioteka-idiomy.md)

### Część II. Architektura kompilatora

14. [Rozdział 12. Od pliku źródłowego do gotowego programu](13-pipeline.md)
15. [Rozdział 13. Jak kompilator czyta tekst programu](14-parser.md)
16. [Rozdział 14. Jak kompilator sprawdza własność nazw](15-sema.md)
17. [Rozdział 15. Jak kompilator sprawdza typy](16-typeck.md)
18. [Rozdział 16. Jak kompilator wybiera miejsce na napis](17-pamiec-srodkowa.md)
19. [Rozdział 17. Jak powstaje kod maszynowy](18-codegen.md)
20. [Rozdział 18. Komunikaty błędów, polecenia i edytor](19-lsp.md)

### Część III. Jak czytać repozytorium

21. [Rozdział 19. Jak czytać repozytorium](20-mapa.md)
22. [Rozdział 20. Jeden program od źródła do uruchomienia](21-slad.md)
23. [Rozdział 21. Testy, szukanie przyczyny błędu i nowa konstrukcja języka](22-praca-z-repo.md)
24. [Rozdział 22. Ćwiczenia](23-cwiczenia.md)

### Dodatki

25. [Dodatek A. Zestawienie składni](A-sciagawka.md)
26. [Dodatek B. Słownik pojęć](B-slowniczek.md)
27. [Dodatek C. Czego kompilator jeszcze nie potrafi](C-niedokonczone.md)

Programy użyte w książce leżą w katalogu [`przyklady/`](przyklady/). Raport z redakcji językowej jest w [`REDAKCJA.md`](REDAKCJA.md). PDF tej książki to [`bork-in-action.pdf`](bork-in-action.pdf). Skład robi [`build-pdf.sh`](build-pdf.sh).
