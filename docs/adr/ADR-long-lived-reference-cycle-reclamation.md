# ADR: Długotrwałe silne relacje i odzyskiwanie cykli przez `Hold<T>`

- **Status:** Proposed — finalna propozycja rady, iteracja 7/7.
- **Baza:** `46d7386f62dbf6bd38c737ac509f1974a5813b7e`.
- **Zakres:** decyzja architektoniczna; rozwiązanie nie zostało zaimplementowane ani przetestowane.

## Kontekst i dowody

Sprawdzony checkout odpowiada wskazanej bazie i nie zawiera zmian roboczych.

Istniejący `Ref<T>` realizuje inny kontrakt niż potrzebne tutaj trwałe, dowolnie skierowane relacje silne:

- [`RefHandle`](../../crates/bork_runtime/src/refs.rs#L12) zawiera `control`, `object` i `kind`. `drop_ref_value` zwalnia lease zależnie od `RefKind`.
- `classify_internal` w `crates/bork_runtime/src/arena.rs:961` wybiera `Arena`, `Borrow`, `Strong` albo `Weak`. [Dokumentacja](../../docs/references.md#L264) uzasadnia brak cyklu złożonego wyłącznie z silnych krawędzi między różnymi arenami ich rosnącymi identyfikatorami. Nowa funkcja nie jest naprawą wykazanego wycieku obecnego `Ref`.
- [`observe`](../../crates/bork_runtime/src/refs.rs#L290) zapewnia obserwacji silnego uchwytu osobny pin, niezależny od późniejszego nadpisania źródłowego slotu.
- Obecny backend tworzy areny dynamiczne i rejestruje ich construction roots w `src/codegen/llvm/managed_ref.rs:87`. `emit_ref_create`, w tym samym pliku, odczytuje kontrolkę z arenowej reprezentacji obiektu. Nie jest to gotowa ścieżka alokacji `Hold`.
- [`check_aliased_borrows`](../../src/typeck/expr/call.rs#L573) jawnie nie wykrywa aliasowania przez inną nazwę. RFC 0004 ma status `Proposed`; nie można zakładać, że zamknął tę lukę.
- [`src/mir/mod.rs`](../../src/mir/mod.rs#L1) stwierdza, że MIR nie ma jeszcze konsumenta. Implementacja ograniczona do MIR nie wystarczy aktywnemu backendowi.
- `managed_ref_target_ok` w `src/ty/query.rs:251` dopuszcza klasy i enumy. Poniższe ograniczenie celu `Hold` do klasy jest decyzją v1, nie istniejącą regułą `Ref`.

Runtime aren używa TLS (`arena.rs:463`). Samo TLS nie dowodzi bezpieczeństwa przenoszenia uchwytów między wątkami.

Repozytorium używa numerowanych RFC i dokumentów `docs/superpowers/`; katalog `docs/adr/` obecnie nie istnieje. Wskazana przez użytkownika ścieżka oznacza utworzenie nowego dokumentu przez koordynatora.

## Rozstrzygnięcie poprzedniej iteracji

Wspólna decyzja obejmuje osobny typ, graf i korzenie, jednokierunkową interakcję z `Ref`, brak finalizerów użytkownika w v1 oraz jawne nazwanie odzyskiwania cykli formą GC.

Rewiduję rekomendację trial deletion: **v1 wybiera znakowanie całej domeny `Hold` od jawnych korzeni**, zgodnie z propozycją drugiego agenta. Jest prostsze do sprawdzenia i nie potrzebuje kompletnego bufora kandydatów.

Przyjmuję również, że osobna arena 4 KiB na każdy węzeł nie jest wymogiem architektury. Obiekcja dotycząca zwykłego `&T` zostaje zamknięta przez ograniczony widok grafowy opisany poniżej.

## Decyzja

### Niezależność od `Ref<T>`

Wprowadzamy `Hold<T>`, początkowo dla nie-nullowalnych klas.

`Hold` ma własny `HoldHandle`, identyfikatory, rejestr węzłów, liczniki, stany życia i rodzinę operacji `bork_hold_*`. Uchwyt jest nieprzezroczystym tokenem osobnej domeny, a nie `RefHandle` ani kontrolką areny.

Każdy niepusty `Hold` jest silny. Nie istnieją automatyczne weak back-edges. V1 nie udostępnia słabego odpowiednika `Hold`.

Kopiowanie uchwytu zachowuje tożsamość celu i dodaje lease; przeniesienie pozostawia źródło puste. `None` nie utrzymuje węzła.

**Nie zmieniamy semantyki, reprezentacji, ABI, klasyfikacji, pinów ani zachowania dropów istniejącego `Ref<T>`.** Nie dodajemy wariantu do `RefKind` i nie kierujemy operacji `Hold` do funkcji `bork_ref_*`. Wszystkie wcześniejsze rekomendacje wymagające takich zmian zostają odrzucone.

### Alokacja i własność payloadu

Proponowana jawna forma konstrukcji to:

```text
hold_new(Node(...)) → Hold<Node>
```

Jest to intrinsic tworzący nowy węzeł z konstrukcji klasy. Nie przejmuje ani nie opakowuje istniejącego targetu `Ref`.

Jednostką życia jest węzeł wraz z jego tranzytywnie owned payloadem. Każda należąca do niego alokacja ma jednego właściciela i jest zwalniana dokładnie raz. Współdzielenie między węzłami odbywa się przez `Hold`.

Payload v1 może zawierać wspierane owned klasy, enumy i kontenery. Tranzytywnie wyklucza:

- `Ref<T>` i zewnętrzne borrowy;
- ukryte owning captures i nieopisane uchwyty natywne;
- zasoby wymagające finalizerów użytkownika.

Dopuszczalność jest sprawdzana strukturalnie również przez pola i instancje generyczne. Nie zakładamy istnienia gotowego predykatu gwarantującego te własności.

Pamięć węzłów musi być niezależnie zwalniana i nierelokowana. Alokator nie jest częścią kontraktu `Ref`; ewentualne współdzielenie niskiego poziomu alokacji nie może współdzielić jego kontrolek ani reguł życia. ADR nie narzuca strony ani areny na węzeł.

### Jawna interakcja z `Ref<T>`

Dozwolony kierunek retencji:

```text
storage poza grafem, także zarządzany przez Ref
    → korzeń Hold
    → graf Hold
```

Pole `Hold` w zwykłym obiekcie jest zewnętrznym korzeniem. Kompilator generuje cleanup nowego typu przez istniejący mechanizm rejestrowania dropów (`bork_arena_register_drop`, `arena.rs:709`). Nie zmienia to implementacji dropu uchwytu `Ref`.

Kierunek `Hold → Ref` jest w v1 zabroniony tranzytywnie. Dzięki temu nie powstają cykle utrzymujące się wzajemnie przez dwa różne systemy pamięci.

Nie ma konwersji aliasujących `Hold ↔ Ref`. Przekazanie danych wymaga jawnej materializacji niezależnej wartości w domenie odbiorcy. Proponowane `hold_copy(view.path)` kopiuje owned dane; napotkane uchwyty `Hold` klonuje, tworząc w wyniku korzenie zewnętrzne. Nie jest to kopia całego grafu ani konwersja jego tożsamości.

### Dostęp, obserwacje i mutacje

Obserwacja, np. przez `if val view = h`, tworzy osobny pin `Hold`, zaliczany do korzeni. Nadpisanie `h` nie unieważnia obserwacji.

Widok ma odrębną proweniencję grafową. **Nie jest zwykłym `T`, `&T` ani arenowym deskryptorem obiektu.** V1 zabrania przechowywania, zwracania, przechwytywania i przekazywania takiego widoku do zwykłych metod klasy lub FFI. Funkcje operujące na grafie przyjmują `Hold<T>`.

Odczyt pola zwraca skopiowaną wartość prostą, nowy ukorzeniony `Hold` albo wymaga jawnego skopiowania owned danych. Projekcje wnętrza agregatu istnieją tylko podczas pojedynczej operacji; nie tworzą trwałego aliasu do bufora. Indeksy i warianty są sprawdzane w tej operacji.

Wszystkie zapisy oraz operacje kontenerów przechodzą przez lowering zachowujący właściciela grafowego. Nie udostępniamy surowego `&T` jako skrótu wdrożeniowego. Pin węzła sam w sobie nie chroniłby adresu relokowanego bufora `Vec`.

## Korzenie i krawędzie

Korzeniami są wszystkie aktywne lease’y spoza payloadów węzłów:

- lokale, argumenty, wyniki i temporaries;
- uchwyty w zewnętrznych agregatach i kontenerach, także w storage zarządzanym przez `Ref`;
- construction roots;
- piny obserwacji;
- jawnie zatrzymane uchwyty FFI.

Korzeń istnieje do jego jawnego zwolnienia lub przewidzianego cleanupu. Nie zakładamy usuwania korzeni przy ostatnim użyciu zmiennej.

Rejestr węzłów i kolejki runtime **nie są korzeniami**. Nie skanujemy stosu ani pamięci `Ref`: wygenerowane operacje aktualizują liczniki korzeni.

Krawędziami są uchwyty w payloadzie węzła, również zagnieżdżone w jego owned agregatach i kontenerach. Źródłem jest właściciel całego payloadu. Metadata przechowują identyfikatory celów i liczność, bez adresów relokowalnych slotów.

Obowiązuje:

```text
strong_count(v) = root_count(v) + liczba przychodzących krawędzi do v
```

Przeniesienie uchwytu między właścicielami zmienia jego rolę. Nowy lease powstaje przed usunięciem starego. Relokacja bufora w obrębie tego samego właściciela nie zmienia grafu.

## Odzyskiwanie

Zwolnienia aktualizują liczniki i oznaczają domenę jako wymagającą obsługi. Właściwe odzyskiwanie jest odroczone do bezpiecznego punktu.

1. Kolejka zerowych liczników iteracyjnie usuwa część acykliczną.
2. Kolektor rozpoczyna znakowanie od wszystkich węzłów z `root_count > 0`.
3. Przechodzi po pełnym domknięciu ich wychodzących krawędzi.
4. Wszystkie nieoznaczone węzły tworzą zbiór odzyskiwany.

**Osiągalny cykl pozostaje żywy w całości.** Po zwolnieniu ostatniej utrzymującej go ścieżki następny przebieg odzyskuje cykl i jego nieosiągalne ogony. Same SCC nie definiowałyby właściwego zbioru.

Przed zwolnieniem pamięci cały odzyskiwany zbiór przechodzi do `Destroying`. Następnie odłączamy każdą krawędź dokładnie raz, wykonujemy strukturalny cleanup i zwalniamy storage. Cleanup payloadu nie może ponownie zwolnić lease’ów odłączonych w pierwszej fazie. Nie wykonujemy kodu użytkownika ani resurrection.

## Punkty zbierania i bezpieczeństwo mutacji

V1 wymusza jeden wątek właściciela domeny. Uchwytów nie wolno przesyłać między wątkami ani łączyć między domenami. ABI sprawdza domenę przed dostępem do węzła.

Przebieg jest synchroniczny i niereentrantny. Nie może działać podczas niezatwierdzonej konstrukcji, mutacji, cleanupu ani aktywnego wywołania lub callbacku FFI.

Kompilator emituje sprawdzenie oczekującej pracy po pełnych wyrażeniach oraz zakończonych cleanupach regionów. Wewnątrz obecnego dropu `Ref` wolno jedynie zwolnić korzeń `Hold` i zgłosić oczekującą pracę; nie uruchamiamy tam kolektora. Dostępne jest również jawne `hold_collect()`.

Żaden surowy adres payloadu nie może przetrwać poza chronioną operacją. Przebieg widzi dopiero spójne liczniki, krawędzie i zawartość slotów. Niepełna mutacja lub błąd alokacji nie może pozostawić częściowo zatwierdzonego grafu.

Warunek bezpieczeństwa jest następujący: każdy dozwolony zewnętrzny dostęp ma policzony korzeń, znakowanie obejmuje wszystkie krawędzie, a graf nie zmienia się podczas analizy i zwalniania. Wówczas żaden dostępny obiekt nie należy do zbioru odzyskiwanego.

## Areny, cleanup i FFI

Regiony nadal wyznaczają cleanup zewnętrznych slotów `Hold`, ale nie czas życia ich targetów. Węzeł może przeżyć region, jeśli utrzymuje go inny korzeń lub ścieżka z korzenia.

V1 obsługuje wyłącznie wygenerowany cleanup pamięci i lease’ów. Finalizery użytkownika, kolejność destrukcji zasobów zewnętrznych i resurrection wymagają osobnej decyzji.

FFI otrzymuje wyłącznie token `Hold`, zatrzymany jako jawny korzeń przez osobne retain/release. Nie otrzymuje zwykłego `T*` ani tokenu `Ref`. Kopiowanie tokenu bez retain narusza kontrakt ABI.

Host wykonuje operacje na wątku właściciela i zapewnia punkt zbierania po zakończeniu wywołań oraz zwolnieniach. Zamknięcie domeny wymaga wcześniejszego oddania uchwytów FFI.

## Czy jest to GC?

**Tak.** RC uzupełnione wykrywaniem i odzyskiwaniem nieosiągalnych cykli jest formą garbage collection.

Wybrany wariant wykonuje pełny tracing **grafu `Hold`**. Nie jest pełnym tracingiem całej pamięci Borka: nie skanuje stosu, aren `Ref` ani arbitralnych bajtów payloadu.

Można uniknąć pełnego przejścia domeny przez trial deletion lub podobną analizę kandydatów. Nadal byłoby to cykliczne GC, z najgorszym przypadkiem obejmującym cały graf. Samo RC nie odzyska dowolnego silnego cyklu.

## Alternatywy

| Alternatywa | Rozstrzygnięcie |
|---|---|
| Zmiana `Ref`, jego weak back-edges, pinów albo dropów | Odrzucona przez twarde ograniczenie użytkownika. |
| Samo RC | Odrzucone: nie odzyskuje silnych cykli. |
| RC i trial deletion | Możliwa późniejsza optymalizacja; v1 wybiera prostszy dowód kompletności znakowania od korzeni. |
| Ogólny collector obejmujący wszystkie areny i `Ref` | Poza zakresem i niepotrzebny do realizacji celu. |
| Zwykłe `&T` do payloadu `Hold` | Odrzucone w v1: wymaga dodatkowego dowodu proweniencji i bezpieczeństwa aliasowania. |
| Dwukierunkowe utrzymywanie `Hold ↔ Ref` | Odłożone: wymaga wspólnej analizy osiągalności obu systemów. |

## Konsekwencje i ryzyka

Znakowanie kosztuje `O(V+E)` na przebieg. Metadata grafu kosztują `O(V+E)`, a pamięć robocza `O(V)`. Osobno należy doliczyć payloady, ewidencję owned alokacji oraz ich cleanup. Zmiana całego agregatu kosztuje co najmniej obsłużenie zawartych w nim krawędzi.

Przebieg może zatrzymać wątek na czas przejścia całej domeny. Sprawdzanie oczekującej pracy po wyrażeniach nie zapewnia ograniczenia czasu pauzy. Polityka rzadszych przebiegów może powstać po pomiarach.

Kosztem bezpieczeństwa v1 są ograniczenia widoków, FFI i payloadów. Najpoważniejsze ryzyko stanowi pominięty korzeń lub niespójna mutacja: może to spowodować zwolnienie żywego obiektu. Nadmiarowy korzeń powoduje retencję pamięci.

Dokumentacja musi zawęzić obecne stwierdzenie „no garbage collector” z `docs/memory-model.md:3`, opisując osobno model arenowy i opcjonalną domenę `Hold`.
