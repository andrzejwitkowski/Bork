# ADR: `Hold<T>` — długotrwałe silne referencje i odzyskiwanie cykli

- **Status:** Proposed; projekt, niezaimplementowany.
- **Baza:** `origin/main` `46d7386f62dbf6bd38c737ac509f1974a5813b7e`.
- **Zakres:** nowy typ referencji i jego graf runtime; bez zmian zachowania istniejącego `Ref<T>`.

## 1. Kontekst i decyzja

Bork ma już `Ref<T>`. Jego runtime klasyfikuje uchwyty jako `Arena`, `Borrow`, `Strong` lub `Weak` według relacji aren. Strong back-edges są automatycznie słabe, więc cykle strong między różnymi arenami nie powstają. To świadomy kontrakt, nie wykazany wyciek.

Celem jest dodać opcjonalny typ do długotrwałych, dowolnie skierowanych relacji strong, w tym cykli, które mają być odzyskiwane, gdy staną się nieosiągalne. Wprowadzamy osobny typ `Hold<T>` oraz odrębny graf runtime. To nie jest alias, podtyp ani nowy `RefKind`.

Twarda granica:

- `Ref<T>` zachowuje semantykę, layout `RefHandle`, ABI, klasyfikator, weak back-edges, piny i dropy.
- Operacje `Hold` mają własne typy/ABI/runtime; nie są kierowane do `bork_ref_*`.
- Nie ma niejawnej konwersji ani aliasu `Ref<T> ↔ Hold<T>`.

`Hold<T>` zaczyna się od jawnej alokacji nowego obiektu w osobnej domenie. Nie owija istniejącego obiektu `Ref`; przeniesienie danych między domenami wymaga jawnej materializacji kopii.

## 2. Typ, węzły i payload

V1 obsługuje `Hold<T>` dla nie-nullowalnych klas. Sam typ `Hold<T>` jest nullable i ma pustą wartość `None` — nie jest to `Option<Hold<T>>`. Każdy niepusty uchwyt jest silny. Publicznego `WeakHold<T>` w v1 nie ma.

Każdy węzeł grafu ma stabilny identyfikator/generację, stan życia, `root_count`, krawędzie do innych węzłów `Hold` oraz własny payload. Graf i jego liczniki są oddzielone od `ArenaControl`, `RefHandle` i arenowej klasyfikacji. Węzeł może wewnętrznie używać istniejącego niskopoziomowego alokatora, ale nie dzieli z `Ref<T>` kontrolki ani reguł życia.

Payload v1 może zawierać owned dane, klasy, enumy i kontenery. Tranzytywnie zabronione są:

- `Ref<U>` oraz zewnętrzne borrowy (`&T`, `&rw T`);
- ukryte owning captures, surowe wskaźniki zachowywane poza operacją i nieopisane zasoby natywne;
- finalizery użytkownika lub inne destruktory mogące uruchomić kod Borka, obserwować/wskrzeszać inne węzły.

Typecheck ma egzekwować ograniczenia przez pola, enum payloads, kontenery i instancje generyczne. To nowa reguła dla `Hold`, nie zmiana dozwolonych pól `Ref`.

## 3. Korzenie, krawędzie i okres życia

Niezmiennik grafu:

```text
root_count(v)     = liczba aktywnych uchwytów spoza payloadów Hold
incoming_count(v) = liczba aktywnych krawędzi Hold wskazujących na v
strong_count(v)   = root_count(v) + incoming_count(v)
outgoing(v)       = uchwyty Hold zapisane w payloadzie v
```

Każda kopia, mutacja i cleanup aktualizuje te wartości dokładnie raz. Rooty nie są wyliczane przez skanowanie stosu; kodgen/cleanup utrzymuje je jawnie.

| Miejsce przechowywania `Hold<T>` | Rola w grafie | Kiedy lease znika |
|---|---|---|
| lokal, parametr, temporary lub wynik funkcji | root | przy jawnej wymianie/przeniesieniu albo cleanupie regionu; Bork nie zakłada NLL/ostatniego użycia |
| pole w payloadzie węzła Hold | silna krawędź z tego węzła do celu | przy nadpisaniu/usunięciu pola lub cleanupie właściciela |
| element `Vec<Hold<T>>` / mapy będącej częścią payloadu Hold | silna krawędź z właściciela kolekcji | przy `push`/`insert`, zastąpieniu, `remove`, `clear` lub cleanupie |
| element zewnętrznego `Vec`/mapy albo pole zwykłego obiektu, w tym zarządzanego przez `Ref` | root | przy usunięciu elementu/pola albo cleanupie obiektu/regionu |
| aktywna obserwacja/guard | tymczasowy root-pin | na końcu guarda |
| FFI | poza v1; ewentualne przyszłe API musi wystawiać retain/release jako jawny root | według osobnego kontraktu FFI |

Obiekt może przeżyć region, w którym powstał, jeśli istnieje root albo ścieżka krawędzi strong z roota. Jeżeli po cleanupie regionu zostaje wyłącznie cykl wewnętrznych krawędzi, cykl jest nieosiągalny i kwalifikuje się do odzyskania przy następnym safe poincie kolektora. Acykliczne węzły z zerowym root i zerowymi krawędziami przychodzącymi można zwolnić natychmiast przez RC.

### Interakcja z `Ref<T>`

Dozwolony jest jeden kierunek retencji:

```text
storage zarządzany przez Ref ── Hold jako root ──► graf Hold
```

Pole `Hold<T>` w obiekcie istniejącego systemu `Ref` jest zewnętrznym rootem. Nowe drop glue zwalnia go przez `hold_release`; nie zmienia to layoutu, klasyfikacji ani ABI `Ref`. Implementacja musi wykonać release tego pola podczas cleanupu payloadu, przed przejściem zewnętrznej areny do `Dead` i zwrotem payloadu do puli.

W drugą stronę, payload Hold nie może zawierać `Ref` ani borrowów. Nie istnieje aliasujący most między grafami. Dzięki temu nie powstają cykle, które przechodzą przez dwa niezależne systemy pamięci. Jawne kopiowanie owned danych może utworzyć niezależny obiekt w drugiej domenie.

## 4. Pola klas, listy i mapy

Przykład pól i listy. Składnia `Hold`, `hold_new` i guarda dla nowego typu jest propozycją; nie jest jeszcze wykonywalnym kodem:

```bork
class Node {
    id: i32
    next: Hold<Node>
    children: Vec<Hold<Node>>
}

fun make_cycle() {
    val a: Hold<Node> = hold_new(Node(1, None, Vec<Hold<Node>>()))
    val b: Hold<Node> = hold_new(Node(2, None, Vec<Hold<Node>>()))

    if val an = a {
        an.next = b              // krawędź a -> b
        an.children.push(b)      // druga, niezależna krawędź a -> b
    }
    if val bn = b {
        bn.next = a              // cykl b -> a
    }
} // a i b przestają być rootami przy cleanupie regionu

fun main() {
    make_cycle()
    hold_collect()               // przykład jawnego safe pointu
}
```

`Hold` w polu klasy/enumu jest rootem, jeżeli właściciel leży poza grafem Hold; jest krawędzią, jeżeli właściciel należy do payloadu Hold. Store musi utworzyć/retainować nowy lease przed zwolnieniem starego. Klonowanie uchwytu zwiększa właściwy root/edge count; `move` przenosi lease.

`Vec<Hold<T>>` jest obsługiwanym przykładem kontenera: każda pozycja jest osobnym lease’em. Reallokacja bufora nie zmienia krawędzi, ponieważ graf przechowuje stabilne identyfikatory węzłów, nie adresy slotów. Operacje push/replace/remove/clear aktualizują graf atomowo z perspektywy safe pointu.

W repozytorium z `origin/main` nie ma jeszcze mapy językowej. Jej przyszły kontrakt jest jednak określony przez tę decyzję; poniższa składnia jest wyłącznie ilustracyjna:

```bork
class Directory {
    by_id: Map<i32, Hold<Node>>
}

// Jeśli Directory leży w grafie Hold, wartości mapy są krawędziami.
// Jeśli mapa leży poza grafem, każda wartość Hold jest rootem.
directory.by_id.insert(42, node)
directory.by_id.remove(42)
```

W v1 klucze `Map` nie są typu `Hold`. Wartość `Hold` jest śledzona przy insert, replace, remove i clear; rehash/relokacja nie zmienia tożsamości krawędzi. Implementacja mapy i jej API są osobnym zadaniem językowym, nie istniejącą funkcją Borka.

## 5. Wykrywanie i zwalnianie cykli

V1 wybiera pełne znakowanie od jawnych rootów wyłącznie w domenie Hold. To prostsze do sprawdzenia niż trial deletion. Trial deletion może zostać zaproponowany później po pomiarach, ale musi zachować mark-collector jako oracle testowy/debugowy.

W safe poincie, gdy nie ma mutacji, callbacków ani aktywnych niechronionych widoków:

```text
marked = {}
queue = wszystkie węzły Hold z root_count > 0 lub aktywnym pinem

while queue niepusta:
    v = queue.pop()
    jeśli v nie jest marked:
        marked.add(v)
        queue.extend(wszystkie Hold krawędzie z v)

garbage = wszystkie żywe węzły Hold - marked

ustaw wszystkie węzły garbage jako Destroying
odłącz krawędzie garbage dokładnie raz
zwolnij owned payloady i storage garbage
```

Cały zbiór `garbage` przechodzi do `Destroying` przed odłączaniem krawędzi. W v1 cleanup nie wywołuje kodu użytkownika i nie pozwala na resurrection. Usuwanie krawędzi z garbage do oznakowanego węzła zwalnia tę krawędź, ale nie narusza jego osiągalności.

Przykład grafu:

```text
A -> B -> C
     ^    |
     |____|
```

- Root `A` istnieje: znakowanie przechodzi `A, B, C`; nic nie jest zwalniane.
- Root `A` znika i nie ma innych rootów: żaden węzeł nie jest oznaczony; zwalniane są `A`, cykl `B ↔ C` i wszystkie ich owned payloady.
- Zamiast tego istnieje niezależny root `C`: `B` i `C` zostają oznaczone przez `C -> B -> C`; `A` jest nieoznaczone i może zostać zwolnione.

Samo wykrycie SCC nie wystarcza: trzeba zwolnić także nieosiągalne ogony wychodzące z cyklu. Markowanie od rootów rozstrzyga oba przypadki.

### Czy to jest garbage collection?

Tak, w szerokim sensie: automatyczne odzyskiwanie nieosiągalnych cykli jest kolekcją. To nie jest tracing GC całej Borka: nie skanuje stosu, aren `Ref` ani arbitralnych bajtów procesu; przegląda wyłącznie jawny graf `Hold`. Nie można obiecywać „braku GC” dla programu, który używa `Hold`.

## 6. Safe points i bezpieczeństwo

- W v1 runtime domeny Hold jest jednowątkowy. Mutacja grafu i kolekcja nie przeplatają się; kolejka zbierania jest obsługiwana po zakończeniu pełnych wyrażeń/cleanupów regionu lub przez jawne `hold_collect()`.
- `Hold<T>` nie jest `Send`/`Sync`. FFI nie przechowuje surowych wskaźników ani uchwytów Hold w v1.
- Dostęp do payloadu jest ograniczony guardem z osobnym pinem. Widok nie może opuścić guarda, zostać przechowany w zwykłym `Ref`, przekazany do zwykłej metody ani użyty jako zwykłe `&T`.
- Nieznany destructor, opaque native resource albo ukryty owning capture jest odrzucany w payloadzie v1.
- Uchwyty do węzła w stanie `Destroying` nie mogą być obserwowane, kopiowane ani zapisywane do nowych slotów; taki dostęp kończy się błędem runtime w debug i jest niedopuszczalny w kodzie zweryfikowanym.
- Jeżeli ewidencja rootów/krawędzi jest niekompletna, typ lub operacja jest odrzucana, a nie zbierana konserwatywnie w ciszy.
- Spadek niecyklicznego węzła do zera może być obsłużony natychmiast; cykle czekają do safe pointu. Pauza markowania jest proporcjonalna do żywego/osiągalnego grafu i może objąć cały graf Hold.

## 7. Alternatywy i konsekwencje

| Alternatywa | Rozstrzygnięcie |
|---|---|
| Zmiana istniejącego `Ref<T>` / `RefKind` | Odrzucona: narusza wymaganie użytkownika i obecny kontrakt. |
| Samo RC | Odrzucone: nie odzyskuje nieosiągalnych cykli strong. |
| Trial deletion produkcyjnie w v1 | Odłożone: optymalizacja dopiero po pomiarach; pełny mark pozostaje oracle. |
| Pełny GC całej pamięci Borka | Odrzucony: nie jest potrzebny dla nowej domeny i wymagałby skanowania/stosowych map. |
| Dwukierunkowe `Ref` ↔ `Hold` | Odrzucone w v1: tworzyłoby cykle między niezależnymi rejestrami. |
| `Map<K, Hold<T>>` | Semantyka krawędzi opisana; sama mapa nie istnieje jeszcze na bazowym `main`. |

Konsekwencje: zmiany w parserze/typeck, własności, plannerze, glue/cleanup, aktywnym LLVM backendzie, runtime oraz debug narzędziu. Wartość `Ref<T>` i jego testy zostają niezmienione. Jest to nowy system i wymaga osobnych testów bezpieczeństwa; samo dodanie nowego uchwytu nie jest małą zmianą runtime.

## 8. Ryzyka i niewiadome

1. Brak choć jednego root albo krawędzi może spowodować użycie po zwolnieniu; pełny mark nie naprawi błędnej ewidencji.
2. Ograniczenia payloadu/guardów mogą być kosztowne i wymagają przejrzystych diagnostyk.
3. Pełny mark może powodować długie pauzy; trial deletion rozważyć wyłącznie po benchmarkach i z oracle.
4. Kontenery zagnieżdżone, generyki, mapy i active enum variants muszą zgadzać się z visitorami.
5. Kolejność cleanupu, FFI i zamknięcie domeny wymagają precyzyjnej specyfikacji w implementacyjnym RFC.
6. Nowa funkcja jest formą GC w szerokim sensie; należy odpowiednio opisać gwarancje języka.
