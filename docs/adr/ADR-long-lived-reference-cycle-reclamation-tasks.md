# Implementation tasks — Hold<T> cycle reclamation (Codex proposal)

- [ ] **Zabezpieczyć kontrakt `Ref`.** Cel: zachować istniejące zachowanie. Akceptacja: niezmienione layout i ABI `RefHandle`, klasyfikacja, weak back-edges, piny i dropy; istniejące testy runtime oraz `programs/build/managed_refs/` zachowują oczekiwania.

- [ ] **Dodać osobny typ i konstrukcję.** Cel: obsłużyć `Hold<T>`, `None`, `hold_new`, kopiowanie i przenoszenie. Akceptacja: frontend, drukowanie typów, odwiedzanie typów, monomorfizacja i diagnostyka rozpoznają nowy typ; żadna operacja nie trafia omyłkowo do glue `Ref`.

- [ ] **Wymusić bezpieczny payload.** Cel: sprawdzać ograniczenia tranzytywnie. Akceptacja: odrzucane są zagnieżdżone `Ref`, borrowy, ukryte owning captures i niedopuszczalne zasoby, również przez enumy, kontenery i instancje generyczne.

- [ ] **Zaimplementować widok grafowy.** Cel: zapewnić dostęp bez eksportowania zwykłego `&T`. Akceptacja: pin przeżywa nadpisanie źródła; widok nie może uciec, trafić do zwykłej metody ani FFI; projekcje kontenerów nie przechowują adresów przez relokację.

- [ ] **Dodać niezależny runtime.** Cel: zapewnić identyfikatory, storage, liczniki korzeni i krawędzi oraz stany życia `Hold`. Akceptacja: rejestr nie utrzymuje obiektów jako korzeń; stare identyfikatory, obce domeny i obce wątki są odrzucane przed dostępem do payloadu.

- [ ] **Pokryć wszystkie mutacje i granice własności.** Cel: utrzymać zgodność payloadu z grafem. Akceptacja: konstrukcje, nadpisania, self-assignment, zmiany wariantu, operacje `Vec`, tablice i materializacje zachowują liczność lease’ów; relokacja bufora nie zmienia krawędzi. Zewnętrzne agregaty zawierające `Hold` mają kompletny cleanup korzeni.

- [ ] **Dodać odzyskiwanie i bezpieczne punkty.** Cel: wdrożyć kolejkę RC oraz znakowanie od korzeni. Akceptacja: odzyskiwane są self-cycle, cykle wielowęzłowe i ich ogony; osiągalne cykle pozostają żywe; zbieranie nie zachodzi podczas mutacji, FFI ani dropu `Ref`; grupowy cleanup zwalnia każdą krawędź i alokację raz.

- [ ] **Zintegrować aktywny backend i MIR.** Cel: nadać `Hold` osobny layout, operacje i harmonogram cleanupów. Akceptacja: działają programy przez obecny backend LLVM; MIR, printer, verifier i opis RFC 0006 uwzględniają ten sam kontrakt. Obsługa wyłącznie w MIR nie spełnia kryterium.

- [ ] **Zamknąć kontrakt FFI.** Cel: udostępnić tokeny z jawnym retain/release i zbieraniem w stanie spoczynku. Akceptacja: zatrzymany token chroni cykl, release umożliwia odzyskanie, callback blokuje przebieg, a domena nie jest zamykana z pozostawionymi uchwytami hosta.

- [ ] **Dodać obserwowalność i dokumentację.** Cel: umożliwić sprawdzenie osobnego grafu. Akceptacja: narzędzia pokazują korzenie, piny, krawędzie z licznością i stany `Hold`, zachowując odczyt obecnego debug ABI aren; dokumentacja opisuje ograniczenia v1 i nazywa kolektor GC.

- [ ] **Zweryfikować poprawność i koszt.** Cel: sprawdzić bezpieczeństwo oraz praktyczną cenę decyzji. Akceptacja: testy obejmują temporaries, argumenty i wyniki, wiele korzeni, duplikaty krawędzi, construction roots, piny, nested payloady, relokacje i cleanup regionów; pomiary raportują pamięć małych węzłów i pauzy dla dużych grafów. Testy codegen są uruchamiane wąsko i według `AGENTS.md`: `-j1`, jeden wątek testów, właściwe `LD_LIBRARY_PATH`, tło i log.

Pominięto implementację, uruchamianie testów i pomiary; plików nie zmieniono.
Główne ryzyko: niekompletna ewidencja korzeni lub mutacji może zwolnić żywy obiekt; pauzy pozostają nieograniczone.
