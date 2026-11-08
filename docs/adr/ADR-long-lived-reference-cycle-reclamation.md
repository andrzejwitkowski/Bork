# ADR: `Hold<T>` — długotrwałe silne relacje i odzyskiwanie nieosiągalnych cykli

**Status:** Proponowany. **Baza:** `46d7386`.

## Kontekst
`Ref<T>` wybiera strong, weak albo borrow przez `classify_internal`. Cykle strong między arenami są wykluczone kolejnością aren i automatycznymi weak back-edges. `docs/references.md:854` stwierdza brak cycle collectora. Nie udowodniono wycieku w obecnym `Ref<T>`, więc ta decyzja nie jest naprawą błędu. Dodaje możliwość: relacje strong o dowolnym kształcie grafu, które mają się zwalniać same.

## Ograniczenie twarde
`Ref<T>` nie zmienia się: semantyka, reprezentacja (`RefHandle`), `RefKind`, klasyfikator, weak back-edges, piny, dropy i ABI pozostają jak są. Każda wcześniejsza propozycja modyfikująca `Ref<T>` jest odrzucona.

## Decyzja
1. **Typ.** `Hold<T>` jest osobnym typem z własnym uchwytem `HoldHandle`, własnym rejestrem komórek i własnymi funkcjami `bork_hold_*`. W v1 `T` to wyłącznie klasa. Uchwyt może być pusty.
2. **Graf.** Węzły `Hold` żyją w osobnej domenie runtime (komórki ze stałym id), poza arenami `Ref`. Węzeł ma: payload, licznik korzeni, licznik krawędzi przychodzących i flagę koloru dla kolektora. Metadane krawędzi przechowują cel i liczność, nie adresy slotów, więc relokacja bufora `Vec` nie zmienia grafu.
3. **Korzenie.** Korzeniem jest każdy `Hold` trzymany poza payloadem węzła `Hold`: lokal, argument, pole struktury lub klasy zarządzanej przez `Ref`, element kontenera spoza grafu. Korzeń to lease z licznikiem. Nowy lease powstaje przed zwolnieniem starego. Pole w storage `Ref` zwalnia lease przez istniejące `register_drop`.
4. **Krawędzie.** Krawędzią jest `Hold` w payloadzie węzła `Hold`, także przez owned klasy, enumy i kontenery. Niezmiennik: `rc = korzenie + krawędzie przychodzące`. Kopie, przeniesienia, nadpisania, zmiany wariantu i operacje kontenerów go zachowują.
5. **Interakcja z `Ref<T>`, jednokierunkowa.** Storage zarządzany przez `Ref` może trzymać `Hold` (korzeń). Payload `Hold` nie może tranzytywnie zawierać `Ref<T>`, borrowów, `&T` ani ukrytych owning captures. Sprawdza to typeck przy deklaracji i przy instancjacji generyków. Nie ma konwersji `Hold` ↔ `Ref` ani aliasingu. Wymiana danych odbywa się przez odczyt i materializację kopii w domenie odbiorcy. Dzięki temu graf `Hold` nie może trzymać przy życiu obiektów `Ref`, a `Ref` nie widzi grafu `Hold`.
6. **Dostęp.** Odczyt i mutacja przez jawny guard (pin) węzła, o osobnej proweniencji grafowej. W v1 nie eksportujemy zwykłego `&T` z guarda `Hold` do ścieżek zakładających kontrolkę areny. Funkcje operujące na grafie przyjmują `Hold<T>`. Pin węzła nie chroni adresu relokowanego bufora `Vec` w payloadzie, a typeck traktuje takie odwołanie jako ograniczone do trwania guarda.
7. **Alokacja.** Jawna składnia konstrukcji `Hold::new(...)`, bez niejawnej promocji z `Ref`. Konstrukcja węzła z krawędziami do jeszcze nieistniejących węzłów odbywa się przez mutację po alokacji (guard), bo Bork nie ma konstrukcji cyklicznej w jednym wyrażeniu.
8. **Odzyskiwanie.**
   - Spadek `rc` do zera zwalnia węzeł natychmiast (RC).
   - Spadek `rc` do wartości dodatniej rejestruje kandydata.
   - Na jawnym punkcie zbierania (próg liczby kandydatów, `Hold::collect()`, wyjście z domeny) budujemy zbiór `S`: kandydaci i pełne domknięcie ich krawędzi wychodzących. Na kopiach liczników odejmujemy tylko krawędzie ze źródłem w `S`. Węzły z dodatnią resztą są korzeniami lub osiągalne z korzeni, i wraz z ich domknięciem pozostają żywe. Reszta `S` jest nieosiągalna i zwalniana jako grupa.
   - Zwalnianie grupy: najpierw wyzerowanie krawędzi, potem zwolnienie komórek, żeby drop jednego węzła nie obserwował zwolnionego innego.
   - Debug i testy: pełny mark z korzeni jako oracle.
9. **Bez finalizerów w v1.** Drop payloadu nie może wykonywać kodu użytkownika, który obserwuje inne węzły grupy. Zwalniane są tylko pola, które same są `Hold` (liczniki) albo owned dane bez efektów.
10. **Współbieżność.** v1 jest jednowątkowe: `Hold<T>` nie jest `Send`/`Sync`, a kolektor działa synchronicznie na wątku właściciela domeny. Kolektor nie przerywa się w trakcie pina: węzły przypięte są traktowane jako korzenie.
11. **FFI.** W v1 `Hold<T>` nie przekracza granicy FFI. Przekazanie wymaga jawnej kopii danych. Rozszerzenie wymaga osobnego ADR.

## Czy to GC?
Tak, w szerokim sensie. Samo zliczanie referencji nie odzyska cyklu, więc odzyskiwanie cykli wymaga próby usunięcia albo śledzenia. Kolektor:
- działa tylko na grafie `Hold`;
- nie skanuje stosu, `Ref`, aren ani dowolnych payloadów;
- opiera się na dokładnej ewidencji korzeni i krawędzi, a nie na skanowaniu konserwatywnym;
- nie jest współbieżny ani inkrementalny w v1.

Dokumentacja nie może twierdzić „bez GC". Zdanie z `docs/references.md:492` („bez garbage collectora") dotyczy `Ref<T>` i wymaga doprecyzowania, nie zmiany semantyki `Ref`.

## Kompromisy
- Wykrywanie nie jest natychmiastowe: cykl żyje do następnego punktu zbierania. Pamięć cyklu jest zajęta do tego czasu.
- Pauza jest proporcjonalna do `S`, a nie ograniczona. Limit kandydatów na przebieg i próg są konfigurowalne.
- Koszt pamięci małych węzłów (metadane na węzeł): nieznany, do zmierzenia (T9).
- Pominięty korzeń oznacza użycie po zwolnieniu. To największe ryzyko, stąd oracle i zakaz `&T` w v1.

## Konsekwencje
Zmiany w frontendzie, typeck, własności, planerze, backendzie LLVM, MIR (RFC 0006), runtime i narzędziu `bork-debug`. `Ref<T>` i jego testy pozostają nietknięte i służą jako regresja.
