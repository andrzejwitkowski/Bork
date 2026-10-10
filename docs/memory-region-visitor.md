# `RegionVisitor` w analizie pamięci

Ten dokument pokazuje, jak `RegionVisitor` przechodzi przez program Borka i jak
`memory::Planner` buduje na tej podstawie `MemoryPlan` dla obiektów klasowych
oraz `Ref<T>`.

## Ważne ograniczenie składni

W tym branchu klasy zawierają pola, ale nie mają jeszcze metod klasowych.
Funkcje operujące na klasach są funkcjami top-level.

~~~bork
class Node {
    name: String
    next: Ref<Node>
}
~~~

Dlatego poniższe funkcje pełnią rolę operacji/metod na `Node`:

~~~bork
fun make_chain(): Ref<Node> { ... }
fun main() { ... }
~~~

## Przykładowy program

~~~bork
class Node {
    name: String
    next: Ref<Node>
}

fun make_chain(): Ref<Node> {
    val tail = Node("tail")
    val head = Node("head", tail)
    return head
}

fun main() {
    val node = Node("local")
    val reference: Ref<Node> = node

    if val live = reference {
        live.name
    }
}
~~~

## Gdzie zaczyna się visitor

`memory::plan` tworzy `Planner` i uruchamia wspólny, tylko-do-odczytu walker:

~~~rust
let mut planner = Planner::default();
region_walk::walk_program_readonly(program, &report.roots, &mut planner);
~~~

`walk_program_readonly` odwiedza każdą funkcję. Dla każdej funkcji walker
uruchamia jej ciało, przechodzi przez statementy i wyrażenia, a następnie
wywołuje callbacki zaimplementowane przez `Planner`.

Ogólny schemat przejścia wygląda tak:

~~~text
walk_program_readonly
└── funkcja
    ├── begin_function_body
    ├── walk_block
    │   ├── on_stmt
    │   │   └── walk_expr
    │   │       ├── dziecko/dzieci
    │   │       └── after_expr
    │   └── następny statement
    └── end_function_body
~~~

`RegionVisitor` jest więc interfejsem callbacków. Nie przechowuje sam programu
i nie tworzy runtime'owych aren. `Planner` używa callbacków, żeby zapamiętywać
pochodzenie obiektów, zakresy leksykalne i przepływ `Ref<T>`.

## 1. Wejście do `make_chain`

Na początku visitor wywołuje:

~~~rust
begin_function_body(...)
~~~

Planner zakłada scope funkcji:

~~~text
scope: make_chain
variables: {}
allocations: []
~~~

Wewnętrznie zapisuje identyfikator scope'u w `scope_ids` i tworzy mapę nazw w
`scopes`.

## 2. `val tail = Node("tail")`

Dla deklaracji zmiennej `on_stmt` najpierw odwiedza wartość, a dopiero potem
wiąże nazwę z originami tej wartości:

~~~rust
driver.walk_expr(self, value)?;
self.bind(name, self.origins(value));
~~~

Walker dochodzi do `Node("tail")`. Po odwiedzeniu argumentu konstruktora
wywołuje `after_expr` dla `ObjectConstruct`.

Planner wykonuje wtedy `add_allocation` i zapisuje lokalizację konstruktora
(`Span`) jako identyfikator alokacji:

~~~text
tail -> { Span_tail }

allocation:
    site: Span_tail
    class_name: Node
    class: Lexical
    lifetime_domain: LexicalScope(make_chain)
~~~

`Span_tail` oznacza miejsce `Node("tail")` w kodzie źródłowym. Nie jest to
wskaźnik do obiektu ani adres areny.

## 3. `val head = Node("head", tail)`

Najpierw visitor analizuje argumenty konstruktora:

~~~text
"head" -> {}
tail    -> { Span_tail }
~~~

Dla `tail` wyrażenie `Ident` wykonuje lookup w bieżących scope'ach i odnajduje
`{ Span_tail }`.

Potem `after_expr` tworzy alokację dla `head`:

~~~text
head -> { Span_head }
~~~

Pole `next` ma typ `Ref<Node>`, dlatego Planner rozpoznaje, że wartość `tail`
jest zapisywana jako managed reference w obiekcie `head`. Tworzy krawędź:

~~~text
Span_head ─── Ref ───> Span_tail
~~~

W tej chwili stan wygląda tak:

~~~text
head:
    class: Lexical
    lifetime: LexicalScope(make_chain)

tail:
    class: Dynamic
    lifetime: LexicalScope(make_chain)

edge:
    head -> tail
~~~

`tail` został oznaczony jako `Dynamic`, bo musi być możliwy do wskazywania
przez `Ref<Node>` przechowywany w `head`.

## 4. `return head`

Funkcja zwraca `Ref<Node>`, a `head` jest obiektem `Node`. Typechecker wstawia
w HIR konwersję w rodzaju:

~~~text
RefCreate(Ident("head"))
~~~

`after_expr` dla `RefCreate` oznacza origin jako dynamiczny. Następnie `on_stmt`
dla `return` widzi wartość typu managed `Ref` i promuje jej origin do domeny
programowej:

~~~text
head -> Dynamic + Program
~~~

Po zakończeniu przejścia Planner propaguje lifetime po zapisanych krawędziach:

~~~text
head [Program] ───> tail
~~~

W efekcie `tail` również zostaje promowany:

~~~text
tail -> Dynamic + Program
~~~

Wynik dla `make_chain`:

| obiekt | arena | lifetime |
| --- | --- | --- |
| `tail` | `Dynamic` | `Program` |
| `head` | `Dynamic` | `Program` |

## 5. Wejście do `main`

Po `end_function_body` dla `make_chain` visitor zaczyna `main` z nowym scope'em:

~~~text
scope: main
variables: {}
~~~

### `val node = Node("local")`

Powstaje zwykła lokalna alokacja:

~~~text
node -> { Span_node }

allocation:
    class_name: Node
    class: Lexical
    lifetime_domain: LexicalScope(main)
~~~

### `val reference: Ref<Node> = node`

Ponieważ oczekiwany typ to `Ref<Node>`, typechecker reprezentuje tę operację
jako `RefCreate(node)`.

Visitor odzyskuje origin `node` i oznacza alokację jako dynamiczną:

~~~text
node:
    class: Dynamic
    lifetime: LexicalScope(main)
~~~

Nie ma jeszcze promocji do `Program`, bo `reference` nie jest zwracane z
funkcji ani zapisywane w miejscu o programowym lifetime.

## 6. `if val live = reference`

`PresenceMatch` ma kanoniczną sekwencję w `region_walk` (`presence_match`). `Planner` nadpisuje
tylko hook `bind_presence_guard`, który wiąże nazwę do originów jej źródła:

~~~rust
driver.walk_expr(self, &bindings.head.value)?;
region_walk::region_enter(...);
self.bind_presence_guard(&bindings.head)?;
for binding in &bindings.tail {
    driver.walk_expr(self, &binding.value)?;
    self.bind_presence_guard(binding)?;
}
driver.walk_block(...);
region_walk::region_exit(...);
~~~

Codegen nadpisuje całe `presence_match`, bo po każdym źródle `tail` potrzebuje gałęzi do bloku
cleanup, zanim nazwa zostanie związana.

Kolejność jest taka:

~~~text
reference -> { Span_node }

enter PresenceSome region
└── live -> { Span_node }
    └── live.name
exit PresenceSome region
~~~

`live` nie tworzy nowej alokacji. Jest tymczasowym widokiem/borrowem na obiekt
`node`, ważnym tylko wewnątrz gałęzi `PresenceSome`.

W tej gałęzi visitor może użyć `live.name`, ale nie dodaje kolejnego obiektu do
`MemoryPlan`.

## Końcowy `MemoryPlan`

Po przejściu całego programu Planner ma w przybliżeniu:

~~~text
allocations:
  1. Node("tail")
     class = Dynamic
     lifetime = Program

  2. Node("head", tail)
     class = Dynamic
     lifetime = Program

  3. Node("local")
     class = Dynamic
     lifetime = LexicalScope(main)

edges:
  head -> tail
~~~

Codegen używa potem `allocation_site` (`Span`) bieżącego konstruktora, znajduje
odpowiedni `ArenaPlan` i wybiera:

- bieżącą arenę leksykalną dla `Lexical`; albo
- nową arenę dynamiczną dla `Dynamic`;
- domenę `Program` albo konkretny `LexicalScope` dla lifetime'u.

## Rola poszczególnych callbacków

| callback | rola w `Planner` |
| --- | --- |
| `begin_function_body` | zakłada scope funkcji i zapisuje jego arena id |
| `end_function_body` | zamyka scope funkcji |
| `enter_region` | zakłada scope dla bloku/gałęzi/pętli |
| `exit_region` | zamyka scope regionu |
| `on_stmt` | śledzi deklaracje, przypisania i `return` |
| `presence_match` | kanoniczna sekwencja `if val` / `when`; woła `bind_presence_guard` |
| `bind_presence_guard` | wiąże nazwę obecnego `Ref` z originami w gałęzi `Some` |
| `after_expr` | wylicza originy wyrażenia po poznaniu jego dzieci |
| `skip_closure` | pomija ciało closure w tym przejściu |
| `loop_latch` | w tym plannerze nie wykonuje dodatkowej pracy |

Najważniejsze jest to, że `RegionVisitor` nie zarządza bezpośrednio pamięcią.
Jest mechanizmem przejścia po HIR. Konkretna implementacja `Planner` zamienia
odwiedzone konstrukcje na:

~~~text
originy + scope'y + krawędzie Ref
                ↓
          MemoryPlan
                ↓
       LLVM codegen + runtime arenas
~~~

Runtime'owe operacje, takie jak `bork_arena_create_dynamic` albo
`bork_ref_drop`, są wykonywane dopiero później przez wygenerowany kod LLVM.
