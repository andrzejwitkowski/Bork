# Rozdział 16. Hoist, escape i `region_walk`

## Ten rozdział obejmuje

- Kiedy te trzy passy w ogóle biegną
- Wzorzec hoista i jego granice
- Model głębokości w `escape::place`
- Po co jest jeden spacer regionów
- Flaga `codegen_push`

## Tylko na czystym programie

W `frontend::check`, gdy wektor diagnostyk po semie i typecku jest pusty:

1. `region_walk::stamp_codegen_push(&hir, &mut report)` ustawia
   `ArenaNode.codegen_push`.
2. `hoist::annotate` dopisuje `alloc_in_binding`.
3. `escape::check_function` na każdej funkcji może dopisać diagnostyki i
   wtedy HIR wraca do `None`.

Przy błędzie typu hoist się nie wykona. Nie zobaczysz „drugiego” błędu
escape obok błędu typu. To upraszcza passy (zakładają drzewo, które typeck
uważa za spójne) i utrudnia diagnostykę łączoną.

## Hoist

`src/hoist.rs`. Cel: inicjalizator, który i tak natychmiast ucieka do
zewnętrznego `var`, budować od razu w jego arenie.

`try_hoist` patrzy na instrukcję `i` oraz `i+1`:

- `i` to `VarDecl` o nazwie `inner`, jeszcze bez `alloc_in_binding`,
- `i+1` to `Assign` na nazwę (nie na indeks),
- wartość deklaracji to literał napisowy albo wywołanie,
- prawa strona przypisania to identyfikator `inner` z `UseKind::Move`,
- nazwa celu jest w zbiorze `outer` (parametry i nazwy z zewnątrz) albo
  w nazwach już zadeklarowanych w tym bloku.

Wtedy `alloc_in_binding = Some(cel)`.

Zbiór `outer` przy zejściu w blok, `for` i `while` jest sumą dotychczasowych
nazw. Hoist nie przeskakuje między gałęziami `if`. Nie patrzy na dominację.
Dwie linie albo nic.

Codegen czyta pole przy emisji deklaracji i ustawia sink alokacji na dom
celu. Jeśli pole jest puste, inicjalizator idzie do bieżącej płyty, a
`move` przy przypisaniu kopiuje bajty (`copy_into_arena`).

## Escape

`src/escape.rs`. Pytanie: na jakiej głębokości leżą bajty, i czy konsument
jest co najmniej tak płytki, żeby je jeszcze widzieć.

`Local { decl_depth, value_depth }` na wiązaniu. `value_depth` potrafi
być płytsze niż deklaracja, gdy hoist albo sink położył bajty wyżej. Sam
pass nie zapisuje tego z powrotem do semy. Odrzuca albo przepuszcza.

`place(expr, sink)`:

| Wyrażenie | Głębokość bajtów |
|---|---|
| literał liczbowy, bool, napis, `None` | 0 |
| literał tablicy | maksimum z głębokości elementów i z głębokości bufora; bufor to sink, a gdy sink jest `None` albo 0, bieżąca głębokość |
| indeks typu z areną | głębokość wartości tablicy, nie bieżącego bloku |
| wycinek | głębokość odbiorcy |
| identyfikator Move/Promote przy sinku `Some(d)` | `d` |
| identyfikator Move/Promote przy sinku `Some(0)` (return) | `value_depth` źródła |
| `concat` | sink albo bieżąca głębokość |
| inne wołanie typu z areną | maksimum głębokości argumentów |
| `if` | maksimum gałęzi |

`return` wartości z areną przy głębokości > 0 jest błędem. Potem
`check_return_string_form` osobno zabrania `concat` i `move`/`promote` w
wyniku, nawet gdy głębokość wyszła 0. Stąd `return concat` pada także w
funkcji bez zagnieżdżenia.

Przypisanie liczy `place` ze sinkiem równym `decl_depth` celu. Jeśli
`value_depth` celu zostałoby głębsze niż deklaracja, błąd „assigning a
value whose bytes live in an inner region”.

Gałąź `if` otwarta jako region produkujący wartość (`yields: true`), bez
sinku, z bajtami na głębokości tej gałęzi, daje błąd o `String` w gałęzi.
Tekst jest na sztywno o `String`. Warunek jest `uses_arena_storage`.

Diagnostyki escape mają `Phase::Ownership`. Część z nich ma `span: None`
(`concat` na `return`). CLI pomija wtedy numer linii.

## Jeden spacer

`src/region_walk.rs` istnieje po to, żeby stempel, harmonogram codegenu i
emisja LLVM nie miały trzech lekko różnych pętli po HIR. Komentarz na
górze pliku: jeden spacer zsynchronizowany z dziećmi `ArenaNode`.

`RegionSite` nazywa miejsca, które otwierają dziecko raportu: blok, pętla
`for`, `while`, gałęzie `if`, domknięcie. Etykiety muszą pasować do tych,
które sema wpisała. Inaczej `take_child` nie znajdzie węzła.

`RegionVisitor` ma haki: wejście w funkcję, `enter_region`, `loop_latch`,
`exit_region`, `skip_closure`, `after_expr`, oraz nadpisywalne
`for_loop`, `while_loop`, `if_expr`. Domyślna ścieżka instrukcji nie
obsługuje bloku i pętli — te idą przez region. Jeśli ktoś wywoła je jak
zwykłą instrukcję, dostanie `unreachable!`. To jest assert na programiście
kompilatora, nie diagnostyka użytkownika.

`short_circuit_logical_operands` pozwala emisji `&&` / `||` nie schodzić
w prawy operand zwykłą ścieżką, bo prawa strona jest w osobnym bloku
podstawowym. Stempel i tak musi wiedzieć, czy prawa strona alokuje. Stąd
poprawka, która dla stempla ogląda prawą stronę, a dla emisji nie.

## Kto dostaje `bork_arena_push`

`codegen_push_for_region`:

- etykieta domknięcia → nigdy,
- w przeciwnym razie `block_may_allocate_sink` na obranym ciele.

Predykat jest prawdziwy, gdy w bloku jest literał napisu albo tablicy,
`move`/`promote` typu z areną, `concat`, przypisanie lub `return` czegoś,
co może alokować, zagnieżdżony blok albo gałąź, która może alokować.
Fałszywy dla samych liczb, `break`, `continue`, `None`.

`stamp_codegen_push` ustawia korzeniowi funkcji `codegen_push = true`
zawsze. Dziecku ustawia wynik predykatu. Domknięciu trailing ustawia
`false` jawnie.

Skutek, opisany w `docs/memory-model.md` i zrealizowany w
`src/codegen/regions.rs`: dump nadal ma węzeł na każdy region, ale LLVM
woła `bork_arena_push` tylko gdy flaga jest prawdziwa. Blok, w którym są
same `i32`, ma węzeł w dumpu i nie ma płyty w runtime. Funkcja zawsze
pcha jedną płytę na wejściu, nawet gdy nic nie alokuje. To jest
uproszczenie, nie konieczność modelu. Płyta funkcji i tak wraca na listę
przy `pop`.

Pętla: jeden enter (jeśli flaga), na zatrzasku `reset`, na końcu jeden
exit. `TODO.md` prosi, żeby ten kontrakt nie rozjechał się między
visitorami. Dziś emisja i stempel dzielą `region_walk`, więc rozjazd
jest trudniejszy niż przy dwóch ręcznych pętlach. Nadal da się go zrobić,
nadpisując hak i zapominając o `latch`.

## Podsumowanie

- Hoist, stempel i escape biegną tylko po czystym typecku i semie.
- Hoist rozpoznaje dokładnie dwie sąsiednie instrukcje.
- Escape liczy głębokość bajtów i zabrania `return` świeżego napisu, nawet z głębokości 0, gdy formą jest `concat` albo `move`.
- `region_walk` jest wspólnym spacerem stempla i LLVM. HIR i raport muszą mieć tyle samo dzieci.
- `codegen_push` odcina puste regiony od `bork_arena_push`. Domknięcia nie pchają nigdy, i tak nie dochodzą do emisji.
