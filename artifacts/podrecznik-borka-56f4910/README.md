# Bork. Budowa kompilatora

Podręcznik po polsku na podstawie main Borka:
`56f4910a24b689087fe2dee30247651b7b432268` (30.10.2026).

- `Bork-budowa-kompilatora.pdf` — książka z aktywnym spisem, zakładkami i indeksem.
- `podrecznik.md` — wersja Markdown z ilustracjami SVG.
- `wstep.md`, `rozdzialy/`, `zakonczenie.md` — pliki do redagowania.
- `rysunki/` — 46 rysunków SVG oraz ich definicje DOT.
- `przyklady/programs/` — programy przywołane w książce, bez zmian względem commita.
- `przyklady/walkthrough_point.ll` — rzeczywisty LLVM IR przed optymalizacją dla rozdziału 21.
- `weryfikacja.json` — zakres i wyniki kontroli.
- `LICENSE-Bork.txt` — licencja źródłowego projektu (Apache-2.0).

Przykłady Borka buduj kompilatorem ze wskazanego commita. Dyrektywy `exit`,
`stdout` i `diag` opisują oczekiwania korpusu. Katalogi `check_fail` i
`build_fail` zawierają przykłady celowo niepoprawne lub nieobsługiwane.
Fragmenty i schematy z tekstu nie zawsze są kompletnymi programami.

Weryfikacja wykonawcza: 3 testy zbiorcze `tests/programs.rs` przeszły
na kompilatorze z tej wersji i LLVM 23. Na tej maszynie końcowy etap
linkowania wykonał GCC przez tymczasowy alias `clang`; nie jest to test
instalacji Clanga. Szczegóły są w dodatku A i w `weryfikacja.json`.

## Ponowny skład

Potrzebny jest Python 3.12+, Graphviz (`dot`) i fonty IBM Plex oraz DejaVu
w ścieżkach wskazanych na początku `sklad.py`. Na innej maszynie można
zmienić te ścieżki na lokalne pliki TTF. Biblioteki składu instaluje się
w osobnym środowisku Python z `requirements.txt`.

```bash
python przygotuj_rysunki.py
python polacz.py
python sklad.py
python sprawdz.py
```

`sprawdz.py /sciezka/do/snapshotu` dodatkowo sprawdza wszystkie ścieżki
źródeł i kopiuje programy z podanego snapshotu do `przyklady/`.
Nie uruchamia ponownie kompilatora; wynik testów w metadanych odnosi się
do przygotowania tego wydania. Po zmianie kompilatora wymaga odświeżenia.

PDF ma format 190 × 250 mm. Tekst jest przeszukiwalny, a schematy są
wektorowe. Rysunki i proza są opracowaniem objaśniającym wskazaną wersję,
a cytowane programy pochodzą z repozytorium Borka.
