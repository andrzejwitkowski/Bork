# 21. Jeden program przez cały kompilator

## Przykład, który łączy fazy

Zamiast dopisywać kolejne pojęcia, prześledźmy jeden program od tekstu do wyniku. Ten przykład pochodzi dokładnie z `programs/build/classes/walkthrough_point.bork`:

```bork
// exit: 8
class Point {
    name: String
    x: i32
    y: i32
}

fun sum(p: &Point): i32 {
    return p.x + p.y
}

fun main(): i32 {
    val p = Point("a", 3, 4)
    return sum(&p) + p.name.length
}
```

Program nie wypisuje ósemki. Zwraca `8` jako kod zakończenia procesu. Ta różnica ma znaczenie podczas testowania: narzędzie sprawdzające tylko standardowe wyjście nie zobaczy wyniku. Dyrektywa `// exit: 8` jest komentarzem dla języka i oczekiwaniem dla infrastruktury korpusu.

Wartość obliczamy ręcznie: `sum` dodaje współrzędne `3 + 4`, a długość napisu `"a"` wynosi jeden bajt. Wynik całego wyrażenia to `7 + 1 = 8`. Prosty rachunek stanie się punktem odniesienia dla wszystkich reprezentacji pośrednich.

## Tekst, struktura i znaczenie nazw

Parser rozpoznaje klasę, dwie funkcje oraz instrukcje wewnątrz ich bloków. Na poziomie AST zapis `Point("a", 3, 4)` ma postać wywołania. Dopiero analiza semantyczna zna tablicę deklaracji i ustala, że nazwa `Point` oznacza konstruktor klasy. Wywołanie `sum(&p)` wygląda składniowo podobnie, ale zostaje związane z funkcją.

Użycie `p` w `sum` odnosi się do parametru, a użycie `p` w `main` do lokalnego wiązania. Tekst nazwy jest taki sam, lecz deklaracje są różne. To jedna z przyczyn wprowadzania identyfikatorów definicji i wiązań. Nie wolno globalnie potraktować wszystkich wystąpień napisu `p` jako tej samej zmiennej.

W tablicy pól `Point` kolejność pozostaje następująca: `name` ma indeks 0, `x` indeks 1, a `y` indeks 2. Dalsze fazy mogą używać indeksów zamiast szukać pola po napisie przy każdym odczycie. Jest to przejście od wygodnego zapisu źródłowego do formy odpowiadającej układowi danych.

[[diagram:root-03|Od dwóch nazw do dwóch operacji|Wywołanie konstruktora i wywołanie funkcji zaczynają od podobnej składni, ale kontrola typów nadaje im różne znaczenie w HIR.]]

## Typowanie krok po kroku

Konstruktor dostarcza oczekiwanych typów argumentom. Napis ma typ `String`, a literały `3` i `4` są sprawdzane wobec `i32` z deklaracji pól. Nie są tu potrzebne ogólne konwersje liczbowe. Lokalne `p` otrzymuje typ `Point`.

Sygnatura `sum` wymaga `&Point`. Wyrażenie `&p` wyraża pożyczenie obiektu, a nie utworzenie zarządzanego `Ref<Point>`. W ciele `sum` odczyty `p.x` i `p.y` mają typ `i32`, więc ich dodawanie również ma typ `i32`. Zgadza się on z zadeklarowanym typem zwrotu.

W `main` odczyt `p.name` prowadzi do pola typu `String`, a `.length` daje `i32`. Całe końcowe dodawanie łączy dwa `i32`. W typowanym HIR pola klasy mają już ustalone indeksy. Schemat najważniejszych informacji wygląda tak:

```text
Konstrukcja Point:
  pole 0: "a" : String
  pole 1: 3   : i32
  pole 2: 4   : i32

sum:
  p : &Point
  add(field(p, 1), field(p, 2)) : i32

main:
  p : Point
  add(call(sum, borrow(p)), length(field(p, 0))) : i32
```

To schemat dydaktyczny, a nie dosłowny format dumpu HIR. Zachowuje informację potrzebną do zrozumienia przejścia: znaczenie nazw, indeksy pól oraz typ każdego wyniku.

## Własność i plan pamięci

Pożyczka w wywołaniu `sum(&p)` pozwala funkcji odczytać pola, zachowując właściciela w `main`. Po powrocie wolno więc odczytać `p.name.length`. Gdyby wywołanie konsumowało właściciela przez odpowiedni `move`, późniejszy odczyt wymagałby innego uzasadnienia; sama zgodność typów nie wystarczyłaby.

Rzeczywisty wydruk polecenia `bork --dump-arenas` dla tego programu jest następujący:

```text
Arenas
├── fun sum
│   └── p [Borrow ← call site]
└── fun main
    └── p [Local]
```

Raport pokazuje perspektywę analizy własności: skąd pochodzi pożyczka i w jakim regionie zadeklarowano lokalne wiązanie. Nie jest jeszcze kompletnym diagramem fizycznych alokacji. `MemoryPlan` osobno rozstrzyga klasyfikację adresowalnych obiektów.

Planner aktualnej wersji jest konserwatywny wobec wywołań bez podsumowań ucieczki. Obiekt przekazany do funkcji może otrzymać dynamiczną arenę w domenie programu, również gdy argument jest pożyczony. Nie znaczy to, że ciało `sum` faktycznie zachowuje wskaźnik. Znaczy, że analiza nie dowodzi dokładniejszego czasu życia. Ten przykład pokazuje różnicę między rzeczywistym zachowaniem programu a bezpiecznym przybliżeniem kompilatora.

[[diagram:root-04|Trzy pytania o ten sam obiekt|Wiązanie ma region leksykalny, plan może wybrać dłużej żyjącą arenę, a uchwyt runtime wskazuje konkretne sterowanie i obiekt.]]

## Dane i instrukcje

Backend reprezentuje `String` przez wskaźnik na bajty i długość. Rekord `Point` zawiera ten deskryptor oraz dwa pola `i32`. Wartość obiektowa używana przez generator jest uchwytem łączącym arenę i wskaźnik do obiektu. Dzięki temu odczyt danych i operacje na czasie życia mają dostęp do obu informacji.

Odczyty pól można przedstawić w uproszczonym LLVM IR. Poniższy fragment jest schematem operacji wewnątrz `sum`, pomija pobieranie wskaźnika z uchwytu oraz szczegóły ABI:

```llvm
; Schemat: %obj wskazuje na strukturę Point.
%px = getelementptr %Point, ptr %obj, i32 0, i32 1
%py = getelementptr %Point, ptr %obj, i32 0, i32 2
%x = load i32, ptr %px
%y = load i32, ptr %py
%value = add i32 %x, %y
; Przed ret obowiązują właściwe operacje cleanup.
```

`getelementptr` wylicza adres pola według typu struktury; samo nie pobiera wartości. Odczyt wykonuje `load`. Rozdzielenie adresu i wartości jest istotne również przy przypisaniach: lewa strona potrzebuje miejsca zapisu, a prawa obliczonej wartości.

Funkcje Borka mogą mieć dodatkowe parametry techniczne, takie jak arena rodzica i sink dla wyniku posiadającego dane. Nie są one widoczne w programie źródłowym. W tym przykładzie `sum` zwraca liczbę, ale nadal uczestniczy w protokole aren. Publiczny punkt wejścia programu ma odrębną rolę względem wewnętrznych funkcji.

Po emisji modułu sprawdzana jest poprawność LLVM IR. Następnie powstaje plik obiektowy, a linker łączy go z runtime'em i bibliotekami systemowymi. Dopiero uruchomienie programu oraz porównanie kodu zakończenia z `8` zamyka całą ścieżkę weryfikacji.

## Jak debugować ten przebieg

Jeżeli wynik jest błędny, pytaj po kolei o zachowanie informacji. Czy AST ma właściwy priorytet końcowego dodawania? Czy `sum` oznacza właściwą definicję? Czy pole `name` ma indeks zero? Czy długość wynosi jeden bajt? Czy pożyczony uchwyt trafia do właściwego parametru ABI? Czy cleanup odbywa się po ostatnim użyciu?

Takie pytania lokalizują problem. Zmiana kodu alokatora w odpowiedzi na błąd indeksu pola byłaby działaniem w niewłaściwej warstwie. Reprezentacje pośrednie są nie tylko etapami tłumaczenia, lecz również punktami obserwacji.

Ćwiczenie: zmień nazwę napisu na `"ż"`, nie zmieniając współrzędnych. Jaki wynik powinien dać odczyt długości bajtowej? Odpowiedź: `ż` jest kodowane w UTF-8 przez dwa bajty, więc suma wynosi `9`. Liczba widocznych znaków i liczba bajtów to różne wielkości.

Źródła: [program walkthrough_point](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/programs/build/classes/walkthrough_point.bork), [planer](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/memory.rs), [układ wartości backendu](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/codegen/llvm/context.rs), [ABI](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/codegen/llvm/call_abi.rs).

# 22. Laboratorium: przewiduj, uruchamiaj, wyjaśniaj

## Metoda pracy z ćwiczeniem

Dobre ćwiczenie kompilatorowe wymaga przewidywania. Przed uruchomieniem zapisz: czy program powinien przejść parser, kontrolę typów, analizę własności oraz backend? Następnie określ oczekiwany wynik albo kategorię błędu. Dopiero potem porównaj przewidywanie z narzędziem. Jeśli wynik się różni, znajdź pierwszą fazę, której model był niepoprawny.

W tym rozdziale pliki pochodzą z korpusu. Pełne programy są dołączone w katalogu `przyklady`, z zachowaną strukturą ścieżek. Własne modyfikacje zapisuj pod inną nazwą. Pliki w katalogach `check_fail` i `build_fail` celowo demonstrują odrzucenie programu; błąd jest tam oczekiwanym wynikiem.

## Laboratorium A: inferencja jako przepływ informacji

Otwórz `programs/check/inference/literal_fixed_by_use.bork` i zestaw go z `programs/check_fail/inference/literal_conflicting_uses.bork`. Znajdź deklarację nieadnotowanego literału, a następnie wszystkie użycia wymuszające jego typ. Narysuj graf: węzłem centralnym jest nieznany typ, a krawędziami wymagania z wywołań.

Pytanie: dlaczego przypisanie `val x = 1` nie musi od razu ustalać `i32`? Odpowiedź: kompilator może odroczyć decyzję i zebrać ograniczenia z dalszej części ciała. Wartość domyślna jest wybierana dopiero wtedy, gdy nie ustaliło jej inne wymaganie.

Pytanie trudniejsze: czy wymuszenie jednocześnie `u8` i `f32` oznacza, że kompilator powinien wybrać wspólny większy typ? Odpowiedź: nie w tym systemie. Inferencja nie jest automatyczną polityką konwersji. Sprzeczne wymagania mają prowadzić do diagnozy.

[[diagram:root-05|Graf sprzecznych ograniczeń|Jedna zmienna inferencyjna nie może jednocześnie oznaczać dwóch różnych typów wymaganych przez użycia.]]

## Laboratorium B: własność po rozgałęzieniu

Porównaj `programs/check_fail/conditionals/if_one_path_move_then_use.bork` z `programs/check/conditionals/if_return_move_then_use.bork`. Zaznacz ścieżki, które docierają do instrukcji po `if`. Przy każdej dopisz stan właściciela: dostępny albo przeniesiony.

Odpowiedź na podstawowe pytanie brzmi: do analizy użycia po `if` trzeba uwzględnić każdą ścieżkę, która może do niego dojść. Jeżeli jedna z nich przeniosła właściciela, nie można bezwarunkowo odczytać go dalej. Ścieżka zakończona `return` nie dochodzi do późniejszej instrukcji; to właśnie dlatego informacja o przepływie sterowania należy do analizy własności.

Narysuj teraz wariant z dwiema gałęziami kończącymi funkcję. Pytanie „jaki stan właściciela po `if`?” traci praktyczny sens, ponieważ ten punkt nie jest osiągalny. Kompilator nadal może sprawdzać strukturę kodu, ale nie powinien mylić nieosiągalnego połączenia z normalnym przepływem.

## Laboratorium C: decyzja kompilatora a liczniki runtime'u

Uruchom `programs/build/managed_refs/weak_back_edge_usage.bork`. Program tworzy rodzica i dziecko oraz wiąże je w obie strony. Przed uruchomieniem zaznacz kierunki krawędzi. Następnie przeczytaj komentarz korpusu, który wskazuje silną krawędź rodzic–dziecko i słabą krawędź powrotną.

Pytanie: czy kopiowanie słabego uchwytu do parametru powinno automatycznie tworzyć silny uchwyt? Odpowiedź: nie. W tym przykładzie przekazanie zachowuje rodzaj uchwytu. Odczyt celu nadal musi sprawdzić obecność. Gdy cel przestaje żyć, słaba krawędź daje brak wartości zamiast prawa do odczytu zwolnionej pamięci.

Pytanie dodatkowe: dlaczego licznik odwołań do areny nie jest licznikiem odwołań do pojedynczego obiektu? Odpowiedź: arena grupuje obiekty, a odwołania podtrzymują jej domenę życia. Kilka odrębnych obiektów może uczestniczyć w tych samych licznikach. Ta różnica jest istotna podczas interpretowania `strongCount`.

## Laboratorium D: koszt wzrostu Vec

Program `programs/build/vec/push_grow_sum.bork` dodaje dziesięć liczb i odczytuje ich sumę. Oczekiwany kod zakończenia wynosi `45`. Rozpisz pojemności `4`, `8`, `16`, długość po każdym `push` oraz momenty kopiowania bufora.

Pytanie: czy każdy `push` ma koszt stały? Odpowiedź: operacja bez wzrostu ma stałą pracę związaną z dopisaniem elementu, ale wzrost wymaga przeniesienia lub skopiowania dotychczasowej zawartości. Przy geometrycznym wzroście liczbę takich prac można uśrednić w długiej sekwencji.

Pytanie istotne dla aren: czy stary bufor natychmiast wraca do ogólnego alokatora? Odpowiedź: nie należy tak zakładać. Arena zarządza większymi fragmentami pamięci i może utrzymywać miejsce po starym buforze do końca swojego czasu życia. Analiza złożoności operacji oraz analiza zużycia pamięci w konkretnym alokatorze to osobne zadania.

[[diagram:root-06|Kiedy Vec płaci za wzrost|Dopisywanie do wolnego miejsca jest krótką ścieżką. Przekroczenie pojemności prowadzi przez alokację większego bufora i obsługę istniejących elementów.]]

## Laboratorium E: statyczne wywołanie traitu

Otwórz `programs/build/traits/dispatch_bound.bork`. Trait `Show` ma dwie implementacje. Funkcja `say<T: Show>` jest wywoływana z `Circle` i `Square`. Wynik na standardowym wyjściu to kolejno `circle` i `square`, każde w osobnym wierszu.

Przewidź, jakie konkretne implementacje wywoła każda instancja `say`. Odpowiedź: dla `Circle` wybierana jest implementacja `Show for Circle`, a dla `Square` — `Show for Square`. Wybór zostaje rozstrzygnięty podczas instancjonowania kodu. W runtime nie potrzeba tablicy wirtualnej do tego wywołania.

Usuń bound w roboczej kopii. Dlaczego ciało generycznej funkcji nie powinno po prostu czekać, aż zobaczy jakiś konkretny typ? Ponieważ kontrakt ciała generycznego ma być sprawdzalny względem jego deklarowanych wymagań. Bound daje prawo użycia metody; późniejsza monomorfizacja wybiera jej implementację.

## Laboratorium F: check i build

Zestaw `programs/check/methods/trailing_closure.bork` z `programs/build_fail/methods/trailing_closure.bork`. To celowo pokazana granica zakresu implementacji. Frontend rozumie daną konstrukcję, a bramka backendu ją odrzuca.

Odpowiedź na pytanie „czy język to obsługuje?” wymaga więc doprecyzowania etapu. W pracy nad kompilatorem używaj określeń „parsuje”, „przechodzi frontend” oraz „buduje się i działa”. Pozwala to uniknąć zarówno przedwczesnej deklaracji gotowości, jak i niesłusznego uznania poprawnego frontendu za uszkodzony.

Źródła: [korpus i dyrektywy testowe](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/tests/programs.rs), [przykład Vec](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/programs/build/vec/push_grow_sum.bork), [przykład dispatchu](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/programs/build/traits/dispatch_bound.bork).

# 23. Kompendium: gdzie szukać odpowiedzi

## Trzy zapisy, trzy znaczenia

`&T` oznacza pożyczony widok wartości. Nie ma autonomicznego prawa przedłużania życia właściciela. W aktualnym języku występuje w dozwolonych pozycjach, takich jak parametry i lokalne `val`; nie jest ogólnym polem do przechowywania pożyczek w obiektach. Rozdziały 9 i 10 wyjaśniają związek tej reguły z regionami.

`Ref<T>` oznacza zarządzany, sprawdzany pod kątem obecności uchwyt. Wiąże się z protokołem aren i silnych lub słabych odwołań. Nie należy utożsamiać go z pożyczką ani zakładać, że zwykła kropka od razu daje prawo dostępu do celu. Guard jest częścią uzasadnienia odczytu.

`T?` jest zapisem `Option<T>`. To wartość sumy wariantów, a nie uniwersalny bit nullable dopięty do dowolnej reprezentacji. Zagnieżdżenie zapisuje się jako `Option<Option<T>>` albo `Option<T>?`. Sam skrót `i32??` jest odrzucany przez gramatykę tej wersji, mimo że wewnętrzny system typów dopuszcza dwie warstwy opcji. `None` ma znaczenie zależne od kontekstu i rozwiązywania nazw; prelude nie jest zwolnione z reguł widoczności.

[[diagram:root-07|Pożyczka, uchwyt i opcja|Podobieństwo powierzchniowej składni nie oznacza wspólnego protokołu życia. Każdy z trzech typów odpowiada na inne pytanie.]]

## Szybka mapa składni

| Zapis | Znaczenie i rozdział |
|---|---|
| `val x = ...` | Stałe wiązanie; pola obiektu mają własne reguły, rozdział 2 |
| `var x = ...` | Wiązanie dopuszczające przypisanie, rozdział 2 |
| `fun f(x: T): R` | Sygnatura funkcji, rozdziały 2 i 5 |
| `class C { x: T }` | Rekord z polami w kolejności deklaracji, rozdział 2 |
| `enum E { A, B(x: T) }` | Suma wariantów, rozdział 6 |
| `when value { ... }` | Dopasowanie wzorców i kontrola wyczerpywania, rozdział 6 |
| `trait X { ... }` | Kontrakt metod, rozdział 8 |
| `impl X for C { ... }` | Implementacja kontraktu, rozdział 8 |
| `T: X` i `where T: X` | Wymaganie traitu, rozdział 8 |
| `&value` | Pożyczenie, rozdział 9 |
| `move value` | Jawna operacja przeniesienia, rozdziały 9–10 |
| `promote value` | Operacja promocji danych, rozdziały 10 i 14 |
| `if val x = value` | Sprawdzenie obecności i ograniczone wiązanie, rozdział 12 |
| `[T; N]` | Tablica o długości należącej do typu, rozdział 13 |
| `Vec<T>()` | Rosnący kontener wbudowany, rozdział 14 |

Tabela jest drogowskazem, a nie kompletną gramatyką. Szczególnie przy `move`, `promote`, polach i receiverach znaczenie zależy od rodzaju wartości. Aktualny backend ma dodatkowe ograniczenia omawiane przy konkretnych konstrukcjach.

## Komendy, które rzeczywiście istnieją

Sprawdzenie źródła nie wymaga LLVM. Z katalogu repozytorium można uruchomić:

```bash
cargo run --bin bork -- programs/build/basics/return_constant.bork
cargo run --bin bork -- --dump-arenas plik.bork
```

Nie ma tu potrzeby dopisywania podkomendy `check`. Nazwa pliku bez `build` uruchamia frontend. Natywny backend wymaga kompilatora z cechą Cargo `codegen`, LLVM 23 oraz narzędzia linkującego dostępnego jako `clang`:

```bash
cargo run --features codegen -- build -o /tmp/program plik.bork
cargo run --features codegen -- build -g -o /tmp/program-g plik.bork
```

Nie pomyl profilu budowania kompilatora w Cargo z wyborem debugowania programu Borka. `cargo build --release` optymalizuje narzędzie napisane w Ruście. Flaga `bork build -g` dotyczy emitowanego programu, informacji debugowych i debugowego runtime'u. Rzeczywiste ustawienia optymalizacji backendu omawia rozdział 18.

Do uruchomienia wybranych testów służą między innymi:

```bash
cargo test programs_corpus_check
cargo test --workspace --features codegen
cargo test --features codegen --test programs
```

Pierwsza komenda obejmuje frontendową część korpusu. Ostatnia uruchamia zestawy korpusu zdefiniowane w `tests/programs.rs`, także budowanie zwykłe i debugowe, gdy włączona jest cecha `codegen`. Instalacja LLVM w niestandardowym prefiksie może wymagać zmiennej `LLVM_SYS_231_PREFIX`; szczegóły środowiska są niezależne od poprawności programu źródłowego.

## Diagnoza według pierwszego niespełnionego kontraktu

| Objaw | Pierwsze miejsce do sprawdzenia |
|---|---|
| Nieoczekiwany token lub nowa linia | `parser.lalrpop`, `layout.rs`, rozdział 3 |
| Nazwa wskazuje złą deklarację | `defs.rs`, resolver, środowisko, rozdział 4 |
| Sprzeczne typy literału | `infer/`, `typeck/solve.rs`, rozdział 5 |
| Brak wariantu w `when` | `typeck/exhaustive.rs`, rozdział 6 |
| Brak implementacji lub niejednoznaczna metoda | `typeck/traits.rs`, rozdział 8 |
| Odczyt po przeniesieniu | `sema/walk/branches.rs`, rozdział 9 |
| Dane przeżywają lokalny region | `escape.rs`, `hoist.rs`, rozdział 10 |
| Wygasły cel `Ref` | `bork_runtime/src/refs.rs`, rozdział 12 |
| Zły bajt lub długość napisu | `llvm/text.rs`, `runtime/text.rs`, rozdział 13 |
| Check działa, build odrzuca konstrukcję | `codegen_gate.rs`, rozdział 15 |
| Błąd weryfikatora LLVM | Emiter danego wyrażenia i CFG, rozdział 16 |
| Licznik pozostaje po return | Cleanup ścieżek wyjścia, rozdział 17 |
| Brak pliku runtime albo linkera | `codegen/link.rs`, rozdział 18 |
| Dobra diagnoza, złe podkreślenie w edytorze | Span i konwersja pozycji LSP, rozdziały 3 i 20 |

Źródła: [definicja CLI](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/main.rs), [kanoniczne typy](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/ty/mod.rs), [wspólna analiza](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/src/frontend.rs), [korpus](https://github.com/andrzejwitkowski/Bork/blob/56f4910a24b689087fe2dee30247651b7b432268/tests/programs.rs).

# 24. Słownik pojęć i mapa powiązań

## Reprezentacje i identyfikacja

**ABI — Application Binary Interface.** Umowa opisująca przekazywanie argumentów, wyniku i informacji technicznych na poziomie kodu maszynowego lub IR. Sygnatura widoczna w źródle może różnić się od sygnatury emitowanej funkcji, ponieważ backend dodaje parametry zarządzania pamięcią.

**AST — Abstract Syntax Tree.** Drzewo opisujące strukturę źródła po parsowaniu. Zachowuje pojęcia bliskie składni, np. wywołanie po nazwie. Nie musi jeszcze zawierać rozstrzygnięcia, czy zapis oznacza konstruktor czy funkcję.

**AstId, NodeId.** Identyfikatory węzłów w określonych reprezentacjach programu. Pozwalają przypinać metadane bez używania samego tekstu nazwy albo pozycji źródłowej jako uniwersalnego klucza. Nie należy zakładać wymienności różnych przestrzeni identyfikatorów.

**DefId.** Identyfikator definicji. Oddziela tożsamość deklaracji od jej pisowni. Jest przydatny dla typów nominalnych, sygnatur, rozwiązywania nazw i instancjonowania generyków.

**HIR — High-level Intermediate Representation.** Reprezentacja pośrednia bliższa semantyce niż AST. W Borku zawiera typy i rozstrzygnięte operacje, takie jak konstrukcja obiektu albo pole o znanym indeksie. Nadal zachowuje strukturę wysokiego poziomu potrzebną analizom.

**Interner i TyId.** Interner przechowuje jedną kanoniczną reprezentację danego kształtu typu w kontekście kompilacji. `TyId` identyfikuje zapis w tej strukturze. Porównywanie identyfikatorów ma sens w kontekście, który je utworzył; obcy identyfikator nie jest przenośnym opisem typu.

**Span.** Przedział w źródle pozwalający wskazać miejsce diagnozy i wrócić od reprezentacji pośredniej do tekstu. Offset bajtowy nie jest tym samym co numer widocznego znaku ani pozycja UTF-16 używana w protokole edytora.

## Typy i wybór operacji

**Bound.** Wymaganie wobec parametru typu. W deklaracji `T: Show` daje ciału generycznej funkcji prawo do korzystania z metod `Show`. Nie nadaje dowolnie wszystkich operatorów ani pól, jakie mogłyby istnieć w konkretnej przyszłej instancji.

**Coherence.** Spójność zbioru implementacji traitów: kompilator ma zapobiegać konfliktującym implementacjom, których wybór nie byłby jednoznaczny. Bork porównuje nagłówki implementacji, a ograniczenia bounds nie służą mu do rozłączania nakładających się przypadków.

**Exhaustiveness — wyczerpywanie.** Własność dopasowania, według której każdy dozwolony przypadek jest pokryty. Nie jest równoważna policzeniu nazw wariantów: wzorce zagnieżdżone mogą pokrywać tylko część wartości wariantu.

**Inferencja.** Wyprowadzanie brakujących typów z ograniczeń otoczenia. Obejmuje zbieranie wymagań, rozwiązywanie zmiennych oraz wykrywanie sprzeczności. Wybór domyślnego typu literału jest jednym z elementów, a nie całą inferencją.

**Monomorfizacja.** Wytwarzanie konkretnych instancji kodu generycznego dla wybranych argumentów typowych. W Borku jest także miejscem rozstrzygania wywołań traitowych do konkretnych implementacji. Nie oznacza dynamicznego wyboru metody przy każdym wywołaniu.

**Typ nominalny.** Typ, którego tożsamość wynika z deklaracji. Dwie klasy z takimi samymi polami nie stają się z tego powodu jednym typem. Ich `DefId` pozostają różne.

**Unifikacja.** Próba doprowadzenia dwóch opisów typu do zgodności przez rozwiązanie niewiadomych. Jeśli po jednej stronie jest `Vec<α>`, a po drugiej `Vec<i32>`, może ustalić `α = i32`. Nie tworzy zgody między dowolnymi różnymi konstruktorami typów.

## Pamięć i czas życia

**Arena.** Obszar zarządzania pamięcią umożliwiający grupowanie alokacji i wspólny protokół zwalniania. Arena stronicowana dodaje kolejne strony bez przesuwania dotychczasowych obiektów. Jej sterowanie i same strony danych pełnią różne role.

**Borrow — pożyczka.** Ograniczone prawo dostępu do wartości właściciela. Analiza musi wykazać, że właściciel żyje wystarczająco długo. Pożyczka nie jest samodzielnym mechanizmem odzyskiwania pamięci.

**Cleanup.** Operacje przy opuszczaniu zakresu lub ścieżki wykonania: odpinanie odwołań, kończenie guardów i zwalnianie właściwych regionów. Muszą wystąpić także przy `return`, `break` i `continue`, zgodnie z tym, jakie zakresy rzeczywiście zostają opuszczone.

**Domena czasu życia.** Opis tego, jak długo obiekt lub arena mają pozostać dostępne. Może być związana z zakresem leksykalnym lub z dłuższym życiem programu. Jest informacją analizy, której realizację dobiera backend i runtime.

**Escape.** Ucieczka wartości poza zakres, w którym zapewniono jej pamięć. Sprawdzenie ucieczki odpowiada na pytanie, czy użycie wartości w dłuższym kontekście pozostaje bezpieczne. Nie jest tym samym co test, czy zmienna pojawia się w `return`.

**Hoist.** Przeniesienie miejsca wykonania alokacji do odpowiednio długowiecznej domeny. Może pozwolić od razu utworzyć dane w docelowej arenie, zamiast później kopiować już zbudowany wynik. Wymaga znajomości odbiorcy i kształtu wyrażenia.

**Move i promote.** Operacje języka wpływające na własność i docelowy region danych. Ich realizacja zależy od rodzaju wartości; nie wolno utożsamiać zapisu `move` z fizycznym skopiowaniem jednego wskaźnika. Backend nie obsługuje wszystkich rodzajów wartości, które rozumie frontend.

**Outlives.** Relacja „żyje co najmniej tak długo jak”. Jeżeli region odbiorcy może przetrwać region danych, zapis do odbiorcy wymaga dodatkowego uzasadnienia, relokacji lub jest odrzucany. Porównywanie samych numerów linii nie oddaje tej relacji.

**Sink.** Miejsce lub domena przeznaczona na wynik. Przekazanie docelowej areny podczas budowy wartości może uniknąć tymczasowych danych, które po powrocie funkcji utraciłyby ważność.

**Silna i słaba krawędź.** Silna uczestniczy w podtrzymywaniu życia celu zgodnie z protokołem runtime'u; słaba umożliwia sprawdzany dostęp, ale nie utrzymuje celu samodzielnie. W Borku klasyfikacja wiąże się z arenami, a nie ze zwykłą adnotacją `weak` na każdym użyciu.

## Kod natywny i narzędzia

**CFG — Control-flow Graph.** Graf bloków podstawowych i możliwych przejść sterowania. Ułatwia zrozumienie warunków, pętli, short circuit oraz wszystkich wyjść wymagających cleanupu.

**DWARF.** Format informacji debugowej łączącej kod natywny z funkcjami, liniami, zakresami i typami źródłowymi. Obecność prawidłowego kodu maszynowego nie gwarantuje jeszcze użytecznych informacji debugowych.

**GEP — getelementptr.** Instrukcja LLVM obliczająca adres elementu wewnątrz struktury danych. Nie wykonuje odczytu wartości; do tego potrzebne jest np. `load`.

**Phi.** Instrukcja SSA wybierająca wartość na podstawie bloku, z którego przyszło sterowanie. Nie jest funkcją obliczającą obie gałęzie warunku. Powiązanie argumentu phi z rzeczywistym poprzednikiem ma znaczenie dla poprawności IR.

**SSA — Static Single Assignment.** Forma, w której konkretna nazwa wartości IR jest definiowana raz. Zmienna mutowalna z języka źródłowego może początkowo być realizowana przez komórkę pamięci oraz `load` i `store`; nie przeczy to istnieniu SSA dla nazw instrukcji.

**LSP — Language Server Protocol.** Protokół łączący edytor z analizą języka. Wspólne wywołanie frontendu pomaga utrzymać zgodność diagnoz z CLI, ale wymaga także prawidłowej obsługi wersji dokumentu i pozycji tekstu.

[[diagram:root-08|Dlaczego kompilator potrzebuje kilku modeli|Składnia odpowiada na pytanie o strukturę, typy o sens operacji, model pamięci o ważność danych, a CFG o kolejność wykonania.]]

# Dodatek A. Źródła, wersja i weryfikacja

## Dokładny punkt odniesienia

Repozytorium: [andrzejwitkowski/Bork](https://github.com/andrzejwitkowski/Bork). Gałąź odczytana przez `git fetch origin main`: `main`. Commit: `56f4910a24b689087fe2dee30247651b7b432268`. Tytuł commita: `feat: trait method resolution and static dispatch (TS-P5-04) (#188)`. Data commita: 30 października 2026, 13:52:58, strefa `+01:00`. Data opracowania: 2 listopada 2026.

Podręcznik nie opisuje niezatwierdzonych plików roboczych ani rozwoju z innych gałęzi. W szczególności nie przypisuje tej wersji typów skojarzonych traitów. Plany i historyczne RFC służą jako kontekst; za dowód bieżącego zachowania uznano implementację i testy przypiętego commita.

Źródła projektu zawierają licencję Apache License 2.0; jej pełna kopia jest dołączona do materiałów jako `LICENSE-Bork.txt`. Cytowane programy i fragmenty implementacji pochodzą z Borka. Niniejszy tekst jest opracowaniem objaśniającym tę wersję, a nie normatywną specyfikacją wszystkich przyszłych wydań.

## Co sprawdzono podczas przygotowania

Zbudowano kompilator ze snapshotu wskazanego commita, z LLVM 23 oraz cechą Cargo `codegen`, bez domyślnej cechy `lsp`. Uruchomiono `tests/programs.rs`. Wszystkie trzy testy zbiorcze zakończyły się powodzeniem: korpus `check` i `check_fail`, korpus `build` i `build_fail` oraz wariant debugowy `build` i `build_fail`.

W lokalnym środowisku nie było programu `clang`. Etap końcowego linkowania wykonywał systemowy GCC, dostępny dla wywołania linkera przez tymczasowy alias `clang`. Emisja LLVM i runtime pochodziły z właściwego snapshotu. Wynik potwierdza działanie przykładów w tym środowisku; nie jest dodatkowym testem instalacji Clanga ani przenośności na inne systemy.

Osobno uruchomiono program `walkthrough_point.bork`: wynik to kod zakończenia `8`. Wydruk aren w rozdziale 21 pochodzi z tego samego kompilatora. Programy dołączone w materiałach odpowiadają plikom ze snapshotu. Fragmenty objaśniające, pseudokod oraz schematy IR nie są deklarowane jako osobne uruchomione programy.

Sprawdzenie składu PDF obejmuje obecność wszystkich rozdziałów i rysunków, ekstrakcję tekstu, polskie znaki, odsyłacze oraz próbki wyrenderowanych stron. Te kontrole służą jakości książki; nie zastępują pełnej weryfikacji semantycznej całego kompilatora. Dane techniczne i wyniki kontroli znajdują się również w pliku `weryfikacja.json`.

## Dokumentacja narzędzi do dalszej nauki

Podręcznik opiera opis Borka na źródłach projektu. Poniższe dokumenty są przydatne do pogłębienia pojęć użytych w jego implementacji; nie stanowią specyfikacji samego języka Bork:

- [LALRPOP — dokumentacja projektu](https://lalrpop.github.io/lalrpop/): gramatyki i generowanie parsera.
- [LLVM Language Reference Manual](https://llvm.org/docs/LangRef.html): znaczenie instrukcji i reguły poprawności IR. Dokumentacja internetowa rozwija się wraz z LLVM, dlatego szczegóły wersyjne trzeba zestawiać z LLVM 23 użytym przez Borka.
- [LLVM Kaleidoscope Tutorial](https://llvm.org/docs/tutorial/): samodzielne ćwiczenie implementacji małego języka z backendem LLVM.
- [Language Server Protocol 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/): protokół komunikacji z edytorem.

## Jak odświeżyć książkę po zmianie main

Najpierw ustal nowy commit i różnicę względem wersji wskazanej na okładce. Dla zmienionej cechy prześledź parser, typowanie, własność, HIR, planowanie i backend. Zaktualizuj przykłady pozytywne i negatywne, a następnie uruchom odpowiednie testy. Dopiero potem zmień twierdzenia w tekście, diagramy oraz identyfikator wersji.

Sama zamiana krótkiego SHA na okładce nie aktualizuje podręcznika. Szczególnie wrażliwe miejsca to reprezentacja typów, semantyka `Option`, protokół uchwytów `Ref`, ABI oraz lista konstrukcji odrzucanych przez backend. Indeks i spis treści należy ponownie przeliczyć po składzie.

# Dodatek B. Indeks tematyczny

Numery prowadzą do początku rozdziałów omawiających dane pojęcie. Indeks działa także jako zestaw odsyłaczy w czytniku PDF. Szczegółowe podrozdziały są dostępne w zakładkach dokumentu.

[[indeks]]
