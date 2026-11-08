# ADR: `Hold<T>` dla obiektów i cykli, które mają żyć długo

- Status: propozycja; kodu jeszcze nie ma.
- Baza projektu: `origin/main`, commit `46d7386f62dbf6bd38c737ac509f1974a5813b7e`.
- Decyzja: dodać osobny typ `Hold<T>`. Istniejący `Ref<T>` pozostaje bez zmian.

## W skrócie

- `Ref<T>` nadal używa obecnych aren i ich reguł strong/weak.
- `Hold<T>` dostaje własny magazyn węzłów i własne liczniki. `hold_new(...)` tworzy obiekt od razu w tym magazynie. Nie przejmuje obiektu wcześniej utworzonego w zwykłej arenie.
- Uchwyt `Hold` trzymany poza grafem jest rootem. Root utrzymuje wskazany węzeł i wszystko, do czego prowadzą jego pola `Hold`.
- Pole `Hold` wewnątrz węzła `Hold` jest krawędzią grafu. Pole `Hold` w obiekcie `Ref` jest osobnym rootem.
- Przy zbieraniu mechanizm przechodzi po grafie `Hold`: zachowuje węzły osiągalne z rootów, a zwalnia resztę, także nieosiągalne cykle.
- To nie skanuje całej pamięci Borka ani stosu. Jest jednak kolekcją cykli, czyli GC w szerszym sensie.
- Pomysł zachowania adresu zwykłego obiektu przez odpinanie stron areny został oceniony niżej. Nie wchodzi do pierwszej wersji, bo może przypiąć całe strony wraz z niepowiązanymi danymi.

## Co już robi `Ref<T>`

`Ref<T>` jest powiązany z arenami. Mechanizm wykonawczy wybiera `Strong` albo `Weak` według wieku aren. Krawędź wsteczna jest słaba, więc silne cykle między różnymi arenami nie powstają. To obecna zasada języka, nie wykazany wyciek `Ref`.

`Hold<T>` jest nowym typem, a nie nowym wariantem `RefKind`. Nie zmieniamy layoutu ani ABI `RefHandle`, klasyfikatora, pinów, back-edges ani dropów `Ref`. Nie ma konwersji aliasującej `Ref<T>` ↔ `Hold<T>`.

W pierwszej wersji `Hold<T>` może wskazywać nie-nullowalną klasę, a samo `Hold<T>` może być puste (`None`). Nie jest to `Option<Hold<T>>`. Każdy niepusty uchwyt jest strong. Nie ma `WeakHold<T>`.

## Gdzie leżą obiekty `Hold`

Każdy `Hold` wskazuje węzeł w osobnym magazynie Hold, poza zwykłymi arenami `Ref`. Węzeł ma stabilny identyfikator, stan życia i dane obiektu. Uchwyt przechowuje identyfikator, a nie adres slotu w Vec.

`hold_new(Node(...))` tworzy obiekt od razu w magazynie Hold. Jego adres nie zmienia się do czasu zwolnienia. Nie przejmujemy węzła wcześniej zaalokowanego w zwykłej arenie `Ref`.

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

Składnia `Hold`, `hold_new` i `hold_mut` jest propozycją; nie jest jeszcze wykonywalnym kodem. `hold_mut` to nowy guard Hold: przypina węzeł i daje wyłączny widok do zapisu tylko na czas bloku. W obecnym Borku `if val` na `Ref` pozwala zapisać pole klasy (test `src/sema/tests/regional.rs:53–70`); nie zakładamy, że ten wyjątek obejmie nowy widok Hold.

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

Nagłówek Vec-a Hold musi używać alokatora domeny Hold. Nie może zachować uchwytu do areny regionu, która zostanie zresetowana, bo wtedy późniejszy `push` użyłby martwej areny. Wzrost bufora może zmienić adres bufora, ale nie zmienia krawędzi grafu: graf śledzi identyfikatory węzłów, nie adresy slotów.

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

To jest wykonalny wariant nowego `Hold<T>`: uchwyt Hold może wskazywać obiekt utworzony wcześniej w regionie; nie musi być `Ref` ani zmieniać istniejącego `Ref<T>`. Żeby adres się nie zmienił, pamięć zawierająca obiekt musi pozostać zaalokowana. Można przenieść własność `Box<Page>` do rejestru Hold bez przesuwania jego bajtów. Nie można jednak oddać tej strony do ponownego użycia ani zwolnić jej, póki żyje choć jeden obiekt pod tym adresem.

Najprostszy wariant „zatrutej areny” zatrzymuje całą arenę do zniknięcia ostatniego Hold. To bezpieczne, ale zatrzymuje też wszystkie jej strony, duże bloki i metadane. Precyzyjniejszy wariant odpina tylko strony i bloki, w których znajdują się żywe obiekty, a metadane areny zwalnia osobno. On nadal musi zachować pamięć sąsiadów na przypiętej stronie oraz przenieść alokator i obowiązki sprzątania.

### Przykłady kosztu przy rzadkich obiektach Hold

1. **Dwa obiekty na wspólnej stronie.** A (32 B) ma przetrwać, B (32 B) nie. Strona ma 4 KiB danych. Zachowanie adresu A przypina całą stronę — 128 razy więcej bajtów niż sam A — choć drop B musi wykonać się dokładnie raz. Bajty B zostają fizycznie na stronie, ale nie mogą być ponownie użyte. `hold_new` też potrzebuje pamięci dla A; różnica polega na tym, że A może trafić do strony wspólnego magazynu Hold, a strona regionu z B może wrócić do puli.

2. **Mały obiekt z wielu krótkich regionów.** Jeśli z 10 000 różnych stron zostaje po jednym obiekcie 32 B, odpinanie zatrzymuje co najmniej 39,06 MiB danych stron dla 0,305 MiB żywych obiektów (128×, bez metadanych i buforów). Wspólny magazyn Hold może upakować takie obiekty razem, więc nie musi przypinać strony źródłowej z każdej areny. To porównanie opisuje rzadkie przeżycie; gdy większość obiektów na danej stronie ma przetrwać, odpinanie może wypaść korzystniej i trzeba to zmierzyć.

3. **Obiekt z `Vec<String>`.** Strona węzła nie musi zawierać nagłówka Vec, jego bufora ani bajtów String. Zachowanie adresu węzła wymaga zachowania całego wciąż posiadanego grafu pamięci. Nagłówek Vec przechowuje token areny używany przy wzroście; po końcu regionu trzeba przekierować go do alokatora Hold. Późniejszy wzrost może przesunąć bufor Vec, choć sam węzeł Hold pozostanie pod starym adresem. Przy `hold_new` węzeł, nagłówek i bufory od początku należą do magazynu Hold.

### Co trzeba przenieść i czego nie wolno zwolnić

`DropEntry` zawiera adres i funkcję dropu, nie granice wszystkich alokacji ani informację, który obiekt jest ich właścicielem (`crates/bork_runtime/src/arena.rs:96–99`). Przy końcu regionu trzeba odróżnić dropy zwykłych obiektów od odroczonych dropów Hold i wykonać każdy dokładnie raz. Trzeba też śledzić strony z obiektami, duże bloki z ich `Layout` oraz posiadane bufory Vec/String. Duże alokacje powyżej 2048 B są osobnymi blokami zaokrąglanymi do wielokrotności 4 KiB (`arena.rs:251–263`); można je odpinać osobno, ale nadal trzeba przekazać ich właściciela i późniejsze dropy.

Obecny globalny `ArenaPool` nie ma limitu liczby aren (`arena.rs:402–415`). Limit `MAX_SPARE_PAGES = 8` dotyczy zapasowych stron wewnątrz pojedynczej areny; reset zwalnia nadmiarowe strony i duże bloki (`arena.rs:349–363`). Jeśli przyszła, ograniczona pula będzie pełna, można odrzucić tylko pamięć bez żywych obiektów. Metadane `Arena` można zwolnić dopiero po przekazaniu żywych stron, bloków, stanu alokatora i obowiązków dropu pod innego właściciela. „Obiekt wiszący” nie może oznaczać zwolnionych bajtów: pamięć wskazywana przez Hold nadal musi być zaalokowana, aż Hold ją zbierze.

### Decyzja dla v1

W v1 wybieramy `hold_new`: obiekt powstaje od razu w osobnym magazynie Hold, niezależnym od aren Ref. Nie robimy adopcji już utworzonego obiektu z zachowaniem adresu. Powodem nie jest zależność adresu od `Ref` — nowy Hold może przechowywać adres sam — lecz koszt przypinania stron źródłowych przy rzadkim przeżyciu oraz złożoność przenoszenia pamięci, alokatora i dropów.

Odpinanie pozostaje możliwym eksperymentem, jeśli wymaganie zachowania pierwotnego adresu okaże się ważne. T12 ma porównać je z `hold_new` przy różnym odsetku przeżywających obiektów, zmierzyć przypięte bajty (także sąsiadów i bufory), koszt nowych stron po odpięciu oraz poprawność dropów i wzrostu Vec. Nie włączać wariantu bez jawnego limitu retencji i testów przejęcia/zwolnienia pamięci.

## Skutki i ryzyka

`Hold` wymaga zmian w parserze, typechecku, regułach własności, kodgenie, drop glue, runtime i debuggerze. To osobny system, a nie mała zmiana obecnego `Ref`.

Największe ryzyko to pominięty root lub krawędź: wtedy collector może zwolnić żywy obiekt. Markowanie przechodzi cały graf Hold, więc pauza rośnie z jego rozmiarem. Ustalenie maksymalnego bezpiecznego rozmiaru grafu wymaga pomiarów.
