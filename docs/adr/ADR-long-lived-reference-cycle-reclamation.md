# ADR: `Hold<T>` dla obiektów i cykli, które mają żyć długo

- Status: propozycja; kodu jeszcze nie ma; backend pamięci Hold pozostaje nierozstrzygnięty.
- Baza projektu: `origin/main`, commit `46d7386f62dbf6bd38c737ac509f1974a5813b7e`.
- Decyzja: dodać osobny typ `Hold<T>`. Istniejący `Ref<T>` pozostaje bez zmian.

## W skrócie

- `Ref<T>` nadal używa obecnych aren i ich reguł strong/weak.
- `Hold<T>` dostaje własny graf, identyfikatory i kolektor. Sposób przechowywania bajtów pozostaje otwarty: (A) `hold_new` tworzy obiekt w osobnym magazynie Hold; (B) uchwyt przejmuje istniejący obiekt z regionu, a strony z żywymi zakresami trafiają do alokatora zdolnego ponownie używać wolnych dziur. Zakres tego alokatora pozostaje do ustalenia.
- Uchwyt `Hold` trzymany poza grafem jest rootem. Root utrzymuje wskazany węzeł i wszystko, do czego prowadzą jego pola `Hold`.
- Pole `Hold` wewnątrz węzła `Hold` jest krawędzią grafu. Pole `Hold` w obiekcie `Ref` jest osobnym rootem.
- Przy zbieraniu mechanizm przechodzi po grafie `Hold`: zachowuje węzły osiągalne z rootów, a zwalnia resztę, także nieosiągalne cykle.
- To nie skanuje całej pamięci Borka ani stosu. Jest jednak kolekcją cykli, czyli GC w szerszym sensie.
- Odpinanie stron nie jest odrzucone z powodu „zmarnowanych 4 KiB”: wolne zakresy strony mogą być ponownie użyte. Otwarta pozostaje decyzja, czy adres starszego obiektu i ponowne użycie dziur uzasadniają dodatkowy ledger alokacji i osobny właściciel stron.

## Co już robi `Ref<T>`

`Ref<T>` jest powiązany z arenami. Mechanizm wykonawczy wybiera `Strong` albo `Weak` według wieku aren. Krawędź wsteczna jest słaba, więc silne cykle między różnymi arenami nie powstają. To obecna zasada języka, nie wykazany wyciek `Ref`.

`Hold<T>` jest nowym typem, a nie nowym wariantem `RefKind`. Nie zmieniamy layoutu ani ABI `RefHandle`, klasyfikatora, pinów, back-edges ani dropów `Ref`. Nie ma konwersji aliasującej `Ref<T>` ↔ `Hold<T>`.

W pierwszej wersji `Hold<T>` może wskazywać nie-nullowalną klasę, a samo `Hold<T>` może być puste (`None`). Nie jest to `Option<Hold<T>>`. Każdy niepusty uchwyt jest strong. Nie ma `WeakHold<T>`.

## Gdzie leżą obiekty `Hold`

Każdy `Hold` wskazuje węzeł o stabilnej tożsamości, stanie życia i danych. Uchwyt nie wskazuje slotu w `Vec`; może wskazywać albo do osobnego magazynu Hold, albo do zakresu przejętego z areny. Graf, rooty i kolektor są takie same w obu wariantach. Żaden wariant nie zmienia ABI ani semantyki `Ref<T>`.

Dwa sposoby umieszczenia danych pozostają otwarte:

- **Osobny magazyn:** `hold_new(Node(...))` tworzy węzeł w magazynie Hold od początku. Adres jest stabilny do zebrania, ale nie zachowuje adresu wcześniejszego obiektu regionowego.
- **Przejęcie z regionu:** przy końcu regionu uchwyt Hold przejmuje istniejący węzeł pod tym samym adresem. Strony/bloki z żywymi zakresami przechodzą pod nowego właściciela; martwe zakresy w nich wracają do alokatora. Pozostałe strony areny mogą być resetowane i użyte ponownie. Ten wariant wymaga ewidencji wszystkich alokacji i przekazania ich dropów/allocatorów.

W obu wariantach węzeł Hold zachowuje adres do czasu zwolnienia. Nie ustalono jeszcze składni jawnego przejęcia z regionu ani tego, czy wolne zakresy będą dostępne dla wszystkich aren, czy tylko dla alokatora Hold.

Dane węzła mogą zawierać klasy, enumy i kolekcje. Nie mogą zawierać `Ref`, borrowów ani surowych wskaźników, których życia graf Hold nie kontroluje. Kompilator ma sprawdzać to także przez zagnieżdżone pola i typy generyczne.

## Korzenie, krawędzie i czas życia

„Root” to uchwyt, który znajduje się poza danymi węzłów Hold. Rootami są m.in. uchwyty w lokalach, parametrach, wynikach funkcji, zwykłych kolekcjach i polach obiektów zarządzanych przez `Ref`. Krawędź to uchwyt `Hold` zapisany wewnątrz danych innego węzła Hold.

```text
root_count(v)     = liczba aktywnych uchwytów spoza grafu Hold
incoming_count(v) = liczba krawędzi Hold wskazujących na v
strong_count(v)   = root_count(v) + incoming_count(v)
```

Uchwyt w lokalu żyje do cleanupu regionu, o ile program nie wyczyści lub nie nadpisze go wcześniej. Bork nie usuwa uchwytów na podstawie ostatniego użycia zmiennej. Węzeł może przeżyć region, w którym go utworzono, jeśli trzyma go root lub ścieżka krawędzi od roota. Gdy zostaje tylko zamknięty cykl bez rootów, cykl czeka na najbliższe zbieranie.

| Gdzie zapisano `Hold<T>` | Co oznacza | Co zwalnia uchwyt |
|---|---|---|
| lokal, parametr, wynik lub temporary | root | `None`, nadpisanie, `move` albo cleanup regionu |
| pole w danych węzła Hold | krawędź do innego węzła | nadpisanie pola albo cleanup właściciela |
| element `Vec<Hold<T>>` w węźle Hold | krawędź | `set`, `pop`, `clear` lub cleanup; `push` dodaje krawędź |
| element zewnętrznego Vec/mapy lub pole obiektu `Ref` | root | usunięcie/nadpisanie pola albo cleanup właściciela; `push`/`insert` dodaje root |
| aktywna obserwacja | tymczasowy root-pin | wyjście z guarda |

### Wiele rootów z różnych aren `Ref`

```text
arena Ref A: pole Hold ──root──► H1 ──edge──► H2
arena Ref B: pole Hold ──root────────────────► H2
```

Każdy slot `Hold` ma własny root. Jeśli arena A zginie, jej drop usuwa tylko root do H1. H2 nadal żyje przez B; H1 może zostać zwolniony, jeśli nikt inny do niego nie prowadzi. Gdy znikną oba rooty, następne zbieranie może zwolnić cały nieosiągalny fragment grafu.

Hold nie utrzymuje aren `Ref` przy życiu. Działa to w drugą stronę: dopóki obiekt `Ref`-właściciel żyje, jego pole `Hold` jest rootem i trzyma osiągalny graf. Wiele pól w różnych arenach daje wiele niezależnych rootów. Usunięcie jednego zwalnia tylko to, co nie jest potrzebne pozostałym. To może wyglądać jak wyciek, ale obiekt jest nadal osiągalny.

Aby zwolnić root wcześniej, po dodaniu obsługi `Hold` będzie można wyczyścić pole (np. `if val owner = owner_ref { owner.graph = None }`) albo usunąć element z Vec/mapy. Main dopuszcza przypisanie do pola klasy przez presence guard dla `Ref` (`src/sema/tests/regional.rs:53–70`); zadanie T6 musi potwierdzić i przetestować ten sam store dla pola `Hold`. Bork nie czyści uchwytów przy ostatnim użyciu zmiennej.

`dynamic Program` pozwala obiektowi przeżyć wyjście z bloku, ale nie oznacza automatycznie, że jest wieczny. Każda dynamic arena ma ukryty strong root; compiler rejestruje jego drop w `root_owner` (`src/codegen/llvm/managed_ref.rs:100–139`, `crates/bork_runtime/src/arena.rs:731–734`). Jeśli `root_owner` jest leksykalny, root znika przy cleanupie. Jeśli jest niezwalnianym `DomainRoot`, root może trwać do shutdown. Parent link do `DomainRoot` sam jest weak metadata i nie utrzymuje payloadu dziecka.

## Pola klas, listy i mapy

Składnia `Hold`, `hold_new` i `hold_mut` jest propozycją; nie jest jeszcze wykonywalnym kodem. `hold_mut` to nowy guard Hold: przypina węzeł i daje wyłączny widok do zapisu tylko na czas bloku. W obecnym Borku `if val` na `Ref` pozwala zapisać pole klasy (test `src/sema/tests/regional.rs:53–70`); nie zakładamy, że ten wyjątek obejmie nowy widok Hold. Przykład używa kandydackiego `hold_new`, więc nie rozstrzyga backendu pamięci.

```bork
class Node {
    id: i32
    next: Hold<Node>
    children: Vec<Hold<Node>>
}

fun make_cycle() {
    val a: Hold<Node> = hold_new(Node(1, None, Vec<Hold<Node>>()))
    val b: Hold<Node> = hold_new(Node(2, None, Vec<Hold<Node>>()))

    hold_mut(a) { an ->
        an.next = b
        an.children.push(b)
    }
    hold_mut(b) { bn ->
        bn.next = a
    }
} // cleanup regionu zwalnia lokalne rooty a i b

fun main() {
    make_cycle()
    hold_collect()
}
```

Pole klasy w węźle Hold jest krawędzią. `Vec<Hold<T>>` wewnątrz węzła dodaje jedną krawędź na element. Jeśli Vec leży poza grafem Hold, jej elementy są rootami.

Nagłówek Vec-a należącego do Hold musi używać alokatora, który przeżywa region. W wariancie osobnego magazynu jest to alokator Hold; przy przejęciu trzeba przekazać lub przepiąć token w nagłówku i wszystkie bufory. Wzrost bufora może zmienić jego adres, ale nie może korzystać z martwej areny.

Bork nie ma jeszcze mapy językowej. Poniższy zapis pokazuje planowaną semantykę, nie istniejące API:

```bork
class Directory {
    by_id: Map<i32, Hold<Node>>
}

// Mapa w węźle Hold: jej wartości są krawędziami.
// Mapa poza grafem Hold: jej wartości są rootami.
directory.by_id.insert(42, node)
directory.by_id.remove(42)
```

W pierwszej wersji `Hold` nie może być kluczem mapy. `insert` dodaje root/krawędź, a replace/remove/clear zwalniają zastąpioną wartość. Rehash nie może zmienić tożsamości krawędzi. `Map` wymaga osobnego zadania językowego.

## Jak zbieramy nieosiągalne cykle

W pierwszej wersji używamy znakowania od wszystkich rootów. Zbieramy tylko osobny graf Hold; nie skanujemy stosu, aren Ref ani dowolnych bajtów procesu. Trial deletion można rozważyć później po pomiarach.

W punkcie, w którym żadna operacja nie modyfikuje grafu:

```text
marked = {}
queue = wszystkie węzły Hold z root_count > 0 lub aktywnym pinem

while queue niepusta:
    v = queue.pop()
    jeśli v nie jest marked:
        marked.add(v)
        queue.extend(krawędzie Hold wychodzące z v)

garbage = wszystkie żywe węzły Hold - marked
ustaw wszystkie węzły garbage jako Destroying
odłącz krawędzie garbage dokładnie raz
zwolnij dane i pamięć garbage
```

Przykład:

```text
A -> B -> C
     ^    |
     |____|
```

- Root A istnieje: A, B i C są osiągalne; nic nie zwalniamy.
- Root A znika i nie ma innych rootów: A, cykl B↔C i jego ogon są nieosiągalne; zwalniamy całą grupę.
- Jeśli istnieje niezależny root C: B i C zostają, a A można zwolnić.

Samo wyszukanie cyklu nie wystarcza, bo trzeba też zwolnić nieosiągalne obiekty prowadzące do niego. Znakowanie od rootów obejmuje cykle i ogony.

Collector nie uruchamia się podczas mutacji ani dropu Ref. Cała grupa najpierw przechodzi do Destroying, a dopiero potem traci krawędzie. W v1 drop nie uruchamia kodu użytkownika, nie ma resurrection, wątków ani uchwytów FFI.

To jest GC w szerokim sensie: automatyczne odzyskiwanie nieosiągalnych cykli. Nie jest to GC całego Borka. Dotyczy tylko `Hold`, bez skanowania stosu i aren Ref.

## Odpinanie regionu z zachowaniem adresu

To jest wykonalny wariant nowego `Hold<T>`: uchwyt Hold może wskazywać obiekt utworzony wcześniej w regionie; nie musi być `Ref` ani zmieniać istniejącego `Ref<T>`. Żeby adres się nie zmienił, blok pamięci zawierający obiekt musi pozostać zaalokowany. Można przenieść własność `Box<Page>` do innego alokatora bez przesuwania jego bajtów. Nie można jednak zwolnić strony jako całości, póki żyje choć jeden obiekt pod adresem na tej stronie.

### Przypięta pamięć to nie to samo co zmarnowana

1. **Dwa obiekty na wspólnej stronie.** A (32 B) ma przetrwać, B (32 B) nie. Strona ma 4 KiB danych. A utrzymuje całą stronę zaalokowaną, ale nie musi blokować użycia pozostałych zakresów: po poznaniu adresu i rozmiaru B można jego blok oddać na listę wolnych bloków i użyć ponownie. To nie zwalnia pamięci strony do systemu; pozwala alokatorowi wykorzystać jej wolne miejsce.

2. **Arena z 10 stronami, Hold-y na 2.** Jeśli ewidencja pokaże, że żywe obiekty i ich dane są tylko na dwóch stronach, te dwie można odpiąć; pozostałe osiem zresetować i normalnie użyć ponownie. Jeżeli 10 000 różnych stron ma po jednym obiekcie Hold 32 B, ich 39,06 MiB pozostaje zaalokowane, z czego 0,305 MiB to same obiekty, a do 38,76 MiB może być wolnymi zakresami do ponownego użycia. Ta wolna pojemność nie jest „zmarnowana”, jeśli nowy alokator ją wykorzysta. Może jednak pozostać bezczynna, być niedostępna dla zwykłych aren, jeśli udostępnimy ją tylko Hold-om, albo fragmentować się przez rozmiary i wyrównanie. `hold_new` także zajmuje pamięć na swoje strony; bez pomiaru nie wolno twierdzić, że zawsze zużywa mniej.

3. **Obiekt z `Vec<String>`.** Strona węzła nie musi zawierać nagłówka Vec, jego bufora ani bajtów String. Zachowanie adresu węzła wymaga zachowania lub przekazania własności wszystkich nadal żywych alokacji. Nagłówek Vec przechowuje token areny używany przy wzroście; po końcu regionu trzeba skierować alokacje i zwalnianie do właściciela, który nadal żyje. Późniejszy wzrost może przesunąć bufor Vec, choć sam węzeł Hold pozostanie pod starym adresem.

### Co już umie alokator, a czego brakuje

Arena ma `free(ptr, size)` i listy wolnych bloków rozmiarów 16–2048 B; znany, zwolniony blok może być ponownie użyty. To jest użyteczny mechanizm do odzyskiwania dziur. Produkcyjny runtime nie zachowuje jednak pełnej ewidencji zakresów potrzebnej do zwolnienia wszystkich nie-Hold alokacji przy końcu regionu. Tryb `debug-runtime` prowadzi diagnostyczny rejestr alokacji (`crates/bork_runtime/src/arena/debug/diag.rs:327–363`), ale nie zarządza własnością stron ani produkcyjnymi free-listami. `DropEntry` przechowuje tylko adres i funkcję dropu (`crates/bork_runtime/src/arena.rs:96–99`), więc sam nie wystarczy do ustalenia rozmiarów obiektów ani do przekazania ich bloków na free-listę. Istniejące size-class listy też nie łączą dowolnych sąsiednich dziur, a przy użyciu większego bloku część jego nadmiaru nie jest śledzona.

Po końcu regionu trzeba więc wiedzieć dla każdej alokacji: jej zakres, właściciela i czy należy do przeżywającego obiektu Hold lub jego danych. Wykonać zwykłe dropy raz, zwolnić nie-Hold bloki, a żywe zakresy oraz stronę przekazać alokatorowi, który może ponownie używać wolnych dziur. Można zatrzymać wyłącznie strony z żywymi zakresami, nie całą 10-stronicową arenę. Gdy ostatni obiekt z danej strony zniknie, stronę można zresetować i oddać do puli. Jeśli nie ma przyszłego zapotrzebowania na wolne zakresy, strona pozostaje zaalokowana do tego czasu.

Nagłówki Vec i ich bufory wymagają dodatkowej obsługi: trzeba zachować albo przepiąć token alokatora, uwzględnić `capacity`, zagnieżdżone bufory i późniejsze `grow`/`free`. Samo zachowanie nagłówka lub strony obiektu nie wystarcza. Duże alokacje powyżej 2048 B są osobnymi blokami z `Layout` zaokrąglanym do wielokrotności 4 KiB (`arena.rs:251–263`); nie trzeba przypinać dla nich sąsiednich stron, ale trzeba przenieść właściciela i dropy.

Obecny globalny `ArenaPool` nie ma limitu liczby aren (`arena.rs:402–415`). Limit `MAX_SPARE_PAGES = 8` dotyczy zapasowych stron wewnątrz jednej areny; reset czyści free-listy, zachowuje stronę 0, odkłada do ośmiu dalszych stron i zwalnia nadmiarowe oraz duże bloki (`arena.rs:349–363`). Jeśli przyszła pula będzie ograniczona, metadane areny można zwolnić po przekazaniu żywych stron/bloków i stanu alokatora. Nie wolno zwolnić bajtów wskazywanych przez Hold. „Obiekt wiszący” musi mieć właściciela swojej pamięci.

### Dwie opcje pamięci — decyzja odłożona

Obie opcje zachowują ten sam typ `Hold`, graf, korzenie i kolektor:

- **A. Osobny magazyn przez `hold_new`.** Najprostsza własność: obiekt od razu powstaje w pamięci Hold. Nie zachowuje adresu obiektu, który już powstał w regionie.
- **B. Przejęcie obiektu z regionu.** Zachowuje adres, odpina tylko strony/bloki z żywymi zakresami, a znane martwe zakresy udostępnia ponownie alokatorowi. Pozostałe strony areny resetuje się. Wymaga pełnej ewidencji alokacji, rozdzielenia dropów i przekazania właściciela/allocator tokenów.

Codex i Claude potwierdzili po ponownej ocenie, że wolne zakresy na przypiętej stronie mogą być ponownie użyte; sama liczba przypiętych stron nie jest miarą zmarnowanej pamięci. Ich wstępna preferencja A wynika z mniejszego zakresu zmian i braku pomiarów, nie z dowodu, że B zużyje więcej pamięci. B ma dodatkowy koszt ewidencji i poprawnego przekazania pamięci, ale może oszczędzić alokacje oraz spełnia wymaganie zachowania starego adresu.

Nie wybieramy tu jeszcze backendu. Trzeba porównać oba przy tych samych regułach cache i zapotrzebowaniu, w tym przypadek 10-stronicowej areny z Hold-ami na dwóch stronach. Jeśli zachowanie adresu wcześniej utworzonego obiektu jest wymaganiem, A nie spełnia wymagania i wybór musi paść na B po zaliczeniu testów bezpieczeństwa. Jeśli nie jest wymaganiem, wynik pamięci i koszt implementacji mogą rozstrzygnąć między A i B.

## Skutki i ryzyka

`Hold` wymaga zmian w parserze, typechecku, regułach własności, kodgenie, drop glue, runtime i debuggerze. To osobny system, a nie mała zmiana obecnego `Ref`.

Największe ryzyko to pominięty root lub krawędź: wtedy collector może zwolnić żywy obiekt. Markowanie przechodzi cały graf Hold, więc pauza rośnie z jego rozmiarem. Ustalenie maksymalnego bezpiecznego rozmiaru grafu wymaga pomiarów.
