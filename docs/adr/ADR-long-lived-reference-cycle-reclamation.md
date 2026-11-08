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

## 2. Gdzie leżą obiekty `Hold`

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

Hold nie utrzymuje aren `Ref` przy życiu. Działa to w drugą stronę: żywy obiekt w arenie `Ref` może utrzymywać graf Hold przez pole `Hold`. Jeśli wiele takich obiektów żyje w różnych arenach, każdy z nich jest rootem. Dopóki którykolwiek root istnieje, collector musi zachować węzły osiągalne z niego. To może wyglądać jak wyciek, ale jest żywą ścieżką, a nie martwym cyklem. Aby zwolnić root wcześniej, wyczyść pole po bezpiecznej obserwacji Ref, np. `if val owner = owner_ref { owner.graph = None }`, albo usuń uchwyt z Vec/mapy. Bork nie robi tego przy ostatnim użyciu zmiennej.

Warto rozróżnić dwie rzeczy w modelu Borka:

- Parent link dynamicznej areny do `DomainRoot` jest weak metadata; sam nie utrzymuje payloadu dziecka.
- Każda dynamiczna arena ma też ukryty strong root. Compiler rejestruje jego drop w `root_owner` (`src/codegen/llvm/managed_ref.rs:100–139`, `crates/bork_runtime/src/arena.rs:731–734`). Jeśli owner jest leksykalny, root znika przy jego cleanupie. Jeśli ownerem jest niezwalniany `DomainRoot`, ten strong root może utrzymać arenę i jej pola Hold do zamknięcia procesu.

Zatem: Hold nie przypina areny Ref, ale pole Hold wewnątrz długo żyjącej areny Ref może długo utrzymywać graf Hold. To ograniczenie trzeba pokazać użytkownikowi w dokumentacji, a test T6 ma pokryć oba przypadki.

## Pola klas, listy i mapy

Składnia poniżej jest propozycją. Nie jest jeszcze wykonywalnym programem.

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
        an.next = b
        an.children.push(b)
    }
    if val bn = b {
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

## Pomysł odpinania stron areny

Pomysł użytkownika: utworzyć obiekt normalnie w regionie, a przy końcu regionu zostawić go pod tym samym adresem i przenieść stronę pamięci do magazynu długowiecznych obiektów.

Da się zachować adres, ale nie da się bezpiecznie zresetować strony, na której nadal leży żywy obiekt. W Borku strona ma 4 KiB. Jeśli na niej leży jeden obiekt Hold, cała strona pozostaje zajęta; mogą na niej zostać też bajty innych obiektów. Przy wielu małych obiektach w różnych arenach może to przypiąć jedną lub więcej stron z każdej areny. To jest rzeczywista retencja pamięci. Zwykłe obiekty na przypiętej stronie nadal trzeba logicznie zniszczyć i wykonać ich dropy dokładnie raz, ale ich bajtów nie można oddać do ponownego użycia.

Obecny allocator ma strony `Box<Page>` po 4 KiB; reset strony oddaje ją do puli albo zwalnia. DropEntry zna adres i DropFn, nie mapę wszystkich obiektów ani ich danych. `Vec`/`String` mogą mieć bufory na innych stronach, a nagłówek Vec zachowuje uchwyt do areny używany przy wzroście. Odpinanie wymagałoby śledzić wszystkie te alokacje, przenieść ich właściciela, odpalić dropy innych obiektów dokładnie raz i nie użyć strony ponownie, gdy choć jeden obiekt w niej żyje.

W pierwszej wersji wybieramy `hold_new`: obiekt trafia od początku do osobnego storage Hold i zachowuje tam stabilny adres. Nie zachowujemy adresu obiektu, który wcześniej był zwykłą alokacją regionu. Odpinanie stron zostaje alternatywą do prototypu; trzeba zmierzyć liczbę przypiętych bajtów, dane sąsiadów i koszt przenoszenia własności Vec/String.

## Skutki i ryzyka

`Hold` wymaga zmian w parserze, typechecku, regułach własności, kodgenie, drop glue, runtime i debuggerze. To osobny system, a nie mała zmiana obecnego `Ref`.

Największe ryzyko to pominięty root lub krawędź: wtedy collector może zwolnić żywy obiekt. Markowanie przechodzi cały graf Hold, więc pauza rośnie z jego rozmiarem. Ustalenie maksymalnego bezpiecznego rozmiaru grafu wymaga pomiarów.
