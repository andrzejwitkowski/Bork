# Implementation tasks — Hold<T> cycle reclamation (Claude proposal)

- **T0 Ścieżka i dokumentacja.** Potwierdzić z użytkownikiem lokalizację (`docs/rfcs/0007-…` zamiast `docs/adr/`). Poprawić `docs/references.md:492` i `:854`, aby rozróżniały `Ref` i `Hold`. Zależności: brak.
- **T1 Testy regresji `Ref`.** Zamrozić obecne zachowanie: klasyfikacja strong/weak/borrow, piny, dropy, ABI `RefHandle`. Uruchamiać przed i po każdym kolejnym zadaniu. Zależności: brak.
- **T2 Frontend i typeck.** Typ `Hold<T>` (tylko klasy), `Hold::new`, guard, reguła „brak `Ref`/borrowów/`&T` w payloadzie" tranzytywnie, także dla generyków. Zależności: T1.
- **T3 Własność i borrow.** Lease korzenia przy kopii, przeniesieniu, nadpisaniu i dropie. Proweniencja guarda; zakaz ucieczki `&T` poza guard. Zależności: T2.
- **T4 Planer i punkty zbierania.** Wstawianie `collect` na progu i przy wyjściu z domeny. Pokrycie pinami jako korzeniami. Zależności: T3.
- **T5 Runtime.** `HoldHandle`, rejestr komórek, `bork_hold_*`, liczniki, kandydaci, trial deletion, kolejność zwalniania grupy. Zależności: T2.
- **T6 Kontenery.** `Vec`, `Option`, enumy i owned klasy w payloadzie: krawędzie przez id celu, brak adresów slotów, bezpieczna relokacja bufora. Zależności: T5.
- **T7 Backend LLVM i MIR.** Operacje `hold_*` w tabeli operacji RFC 0006 (otwarte pytanie z iteracji 6) oraz lowering w aktywnym backendzie. Zależności: T4, T5.
- **T8 Oracle i `bork-debug`.** Pełny mark w debug i testach; widok grafu `Hold` w `DebugRegistry`. Zależności: T5.
- **T9 Testy i pomiary.** Konstrukcja, self-cycle, duplikaty krawędzi, ogony, wiele korzeni, piny, relokacja bufora, cykl przez `Vec`, mutacja podczas pina, regresja `Ref`. Pomiary: pamięć małych węzłów, pauza kolektora, koszt lease. Testy uruchamiać zgodnie z AGENTS.md (`-j1`, `--test-threads=1`, `nice`, `LD_LIBRARY_PATH`). Zależności: T5–T8.
- **T10 Poza zakresem v1 (osobne ADR).** FFI, wielowątkowość, finalizery, `&T` z guarda, kolektor inkrementalny.
