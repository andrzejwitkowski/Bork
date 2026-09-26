# Rozdział 15. Typeck i typowany HIR

## Ten rozdział obejmuje

- `Ty`, `Prim`, `UseKind`
- Jak checker schodzi po instrukcjach i wyrażeniach
- Różnicę AST i HIR na `move` / `promote`
- Wnioskowanie literałów i truciznę `Unknown`
- Dlaczego HIR nie wystarcza backendowi sam

## Po co osobne drzewo

AST jest kształtem źródła. HIR jest tym samym kształtem po typach, z
kilkoma rzeczami wyciętymi albo doklejonymi:

- każde wyrażenie to `HirExpr { ty, span, kind }`,
- `Expr::Move` i `Expr::Promote` stają się `Ident { use_kind: Move | Promote }`,
- `Call` traci ciało domknięcia jako `Option<Closure>` i zostawia flagę
  `has_trailing_closure` (ciało i tak jest osobno w miejscach, gdzie
  typeck schodził w blok; flaga służy bramce),
- `VarDecl` dostaje `ty` oraz `alloc_in_binding`, na starcie `None`,
- nie ma spanów na parametrach, są gołe `String`.

`UseKind` to `Local`, `Shared`, `Copy`, `Move`, `Promote`. Typeck
wpisuje go w `check_ident`. **Nie** emituje błędów własności. Komentarz i
podział odpowiedzialności są świadome: własność krzyczy sema, HIR tylko
opisuje, jak typeck widział użycie. Codegen później ufa `UseKind` przy
kopii deskryptora. Gdyby typeck i sema się rozjechały, backend kopiowałby
co innego, niż sema pozwoliła. Na prymitywach i na napisach w `main` testy
trzymają je razem. Na parametrach domknięcia sema i tak ma `Unknown`.

## `Ty`

`src/hir/ty.rs`. `Ty { kind, nullable }`.

`TyKind`: `Prim(Prim)`, `Named(String)`, `Array { elem, len }`,
`Func { params, ret }`, `Range(Box<Ty>)`, `Unknown`.

`Range` i `Unknown` nie mają odpowiednika w `ast::Type`. Zakres istnieje
tylko po typecku, jako typ wyrażenia `lo..hi`.

`is_copy`: nienullowalny prymityw. `uses_arena_storage`: `String` albo
nienullowalna tablica. To drugie jest predykatem „czy escape i codegen
muszą myśleć o płycie”.

`lower_type` odrzuca `[T]?` i każdą nazwę poza `String`.

## Zejście

`typeck::check` zbiera sygnatury, odmawia redefinicji wbudowanych, potem
sprawdza ciała. `Env` ma stos zakresów, mapę funkcji, diagnostyki, wektor
`decl_tys` (dopisywany przy każdej deklaracji, w kolejności źródła) i
`loop_depth`.

Instrukcje są w `typeck/stmt.rs`. Wyrażenia są podzielone:

| Plik | Co |
|---|---|
| `expr/mod.rs` | dyspozytor, identyfikatory, `UseKind` |
| `expr/binary.rs` | arytmetyka, porównania, `&&` `\|\|`, zakres |
| `expr/call.rs` | wołania, trailing closure, wbudowane |
| `expr/control.rs` | `if`, bloki jako wartość |
| `expr/array.rs` | literał, indeks, wycinek |
| `expr/field.rs` | `.` i `?.` |

Ostatnia instrukcja bloku, który ma produkować wartość, musi być
`Stmt::Expr`. W przeciwnym razie typ bloku to `unit`
(`check_value_block_in_current_scope`). Dlatego `return` w gałęzi `if`
nie jest „wartością gałęzi”. Jest instrukcją, a gałąź jako wyrażenie ma
typ `unit`, jeśli ostatnią rzeczą nie jest gołe wyrażenie.

## Literały i oczekiwany typ

Checker przekazuje `expected: Option<&Ty>` w dół. Literał całkowity bez
oczekiwania to `i32`. Z oczekiwaniem całkowitym przyjmuje ten typ. Float
bez oczekiwania to `f64`. `None` bez oczekiwania to błąd, nie „jakiś
nullable”.

`Unknown` tłumi kaskadę: porównanie z `Unknown` nie dokłada kolejnego
`expected`. Nie tłumi semy.

## Wbudowane a zwykłe wołanie

`call.rs` rozpoznaje `print` / `println` / `concat` zanim sprawdzi
arność użytkownika. Dlatego `print` przyjmuje `String`, choć tabela
`builtins::signatures` mówi `(i32) -> unit`. To jest specjalny przypadek,
nie ogólna podtypowość. Inna funkcja o parametrze `i32` nie przyjmie
`String`.

Trailing closure dokleja się jako ostatni argument typu funkcyjnego.
Ciało jest sprawdzane w nowym zakresie. Sema robi osobny spacer po AST i
**nie** widzi typów wyliczonych dla parametrów domknięcia. Dwa spacery,
dwa środowiska. To jest powód rozjazdu `Shared` kontra `Copy` z rozdziału 5.

## HIR a backend

Backend nie powinien odtwarzać zasięgu z HIR. Ma iść `region_walk`, który
na każdym `For`, bloku, `if` i domknięciu bierze kolejne dziecko
`ArenaNode`. Jeśli typeck wyrzuci albo wstawi blok inaczej niż sema,
spacer padnie. `peel_blocks` musi być wspólny. Jest skopiowany w `hir` i
w `sema` z komentarzem, żeby się nie rozjechał. Nie ma jednego wspólnego
traitu. Jest konwencja.

`alloc_in_binding` jest puste po typecku. Wypełnia je hoist, już na HIR,
mutowalnie, w `frontend::check`. To jedyna adnotacja middle-endu.

## Testy

`src/typeck/tests/mod.rs` jest duży i idzie przez `frontend::check`, nie
przez sam typeck. Dzięki temu łapie też własność. `nullable.rs` trzyma
`?:`, `!!` i porównania na nullable. `closures.rs` trzyma arność trailing
closure. Gdy dodajesz operator, dopisujesz tu przypadek z dokładnym
tekstem diagnostyki. Rozdział 21.

## Podsumowanie

- HIR niesie typ i `UseKind`. Nie niesie identyfikatora regionu.
- `move` w AST staje się `UseKind::Move` na identyfikatorze.
- Typeck nie zgłasza błędów własności. Sema nie czyta `UseKind`.
- Oczekiwany typ steruje literałami, `None` i tym, czy `if` bez `else` jest błędem czy `unit`.
- Dwa spacery, typeck i sema, spotykają się dopiero w `region_walk`. Zgodność `peel_blocks` jest warunkiem tej zgody.
