# Rozdział 14. Analiza regionów (`sema`)

## Ten rozdział obejmuje

- `ArenaNode`, `Ownership`, `Analyzer`
- Wejście w region: `open_ordinary` i `open_move`
- `classify_use` i politykę transferu
- Scalanie `moved` po `if` i zakaz w pętli
- Czego sema świadomie nie wie

## Sema nie alokuje płyt

`src/arena.rs` zaczyna się komentarzem, że sema go nie używa. Sema buduje
drzewo opisujące, które nazwy w którym bloku żyją i jak przekroczyły
granicę. Identyfikator to `ArenaNode.id: usize` z licznika
`Analyzer::alloc_id`. Nie ma typu `ArenaId`. Nie ma też enumu `RegionKind`,
choć plany o nim wspominały. Region jest albo zwykły (`open_ordinary`),
albo move (`open_move`). Reszta to etykieta napisowa.

## Struktury

`ArenaReport { roots }` — jeden korzeń na funkcję.

`ArenaNode` ma `id`, `label`, `compacted_braces`, `codegen_push`,
`bindings`, `observations`, `children`. `bindings` to deklaracje i
przechwycenia. `observations` to użycia spoza deklaracji (Copy, Shared,
Move). Dump pomija obserwacje `Local`, żeby nie dublować deklaracji, z
wyjątkiem miejsc, gdzie Local i tak wpada w listę bindings.

`Ownership` to `Local`, `Copy`, `Shared { from }`, `Moved { from }`.

`EnvBinding` pamięta `arena_id`, `arena_label`, `ty` (`Known(ast::Type)`
albo `Unknown`), `kind`, `moved`, `origin` (`Declared` albo `Captured`).

`Analyzer` trzyma środowisko `HashMap<String, EnvBinding>`, sygnatury
`fun_sigs`, kolejkę `decl_tys`, stos `loop_move_ban` i wektor błędów
`SemaError { message, name, span }`.

## Wejście

`analyze_with_decl_tys` rejestruje kind parametrów każdej funkcji, potem
`analyze_function` woła `open_ordinary(az, "fun {name}", body, params)`.

`open_ordinary` / `open_move` (`src/sema/walk.rs`):

1. `peel_blocks` zdejmuje opakowania z samych bloków i liczy je.
2. `open_frame` bierze nowe `id`, buduje `RegionFrame`, wiąże parametry.
3. Dla move: `bind_capture` każdego imienia z listy (jawnej albo
   wywnioskowanej). Przechwycenie woła `move_source`, cieniuje nazwę jako
   `Captured`, dopisuje `Ownership::Moved` do dziecka i po `finish`
   ustawia `moved` u rodzica.
4. `walk_block` schodzi w instrukcje.
5. `finish` zdejmuje cienie.

`resolve_move_captures`: jawna lista wygrywa. Przy `None` zbierane są
`free_vars_in_block`, odfiltrowane o parametry, zostają nazwy żywe i
nie-Copy. `Expr::Move` i `Expr::Promote` nie są zmiennymi wolnymi.

## Klasyfikacja użycia

`classify_use` w `src/sema/policy.rs` jest całym modelem Copy/Shared/Move
w kilkunastu liniach:

1. `binding.moved` → błąd `use of nazwa after move from etykieta`.
2. Ta sama `arena_id` → `Local`.
3. `ty.is_copy()` → `Copy`.
4. `Val` → `Shared { from: etykieta rodzica }`.
5. Inaczej → `` nazwa is not Copy; move it into etykieta with move ``.

Assign-up obchodzi krok 1–5 dla **lewej** strony: jeśli cel jest `Var`,
nie moved, i `arena_id < node.id`, sema nie woła `note_use`.

`bare_ident_move_message` daje krótsze podpowiedzi przy transferze:
`use move nazwa to pass ownership` dla argumentu, `use move nazwa to
transfer ownership` dla przypisania w tej samej okolicy, oraz dłuższy
tekst „not Copy” gdy areny się różnią.

## Pętla i `if`

Na `For` i `While` sema pcha na `loop_move_ban` zbiór wszystkich kluczy
środowiska i zdejmuje go po ciele. `move_source` zwraca `InLoop`, a
`error_move_in_loop` buduje zdanie o kolejnych iteracjach.

Na `If` sema zapamiętuje zbiór `moved`, schodzi w `then`, przywraca flagi,
schodzi w `else` jeśli jest, przywraca znowu i woła `apply_moved_merge`:
nazwa jest moved po `if`, gdy była moved wcześniej **albo** (moved w then
**i** moved w else, gdy else istnieje). Then-only nie zostaje.

To jest analiza na flagach boolowskich, nie na siatce ścieżek. Nie ma
„moved jeśli warunek jest prawdziwy”. Albo obie gałęzie, albo nic.

## Czego sema nie robi

- Nie sprawdza, czy bajty `concat` przeżyją `return`. To escape.
- Nie typuje parametrów trailing closure (`Ty::Unknown` w `walk.rs`).
- Nie rozumie indeksu `for` inaczej niż jako `Int`.
- Nie ma ścieżek wyjątków, bo język ich nie ma.
- Nie kompaktuje aren bajtowo. `compacted_braces` dotyczy tylko gołych
  nawiasów w AST.

`Ty::Unknown` nie jest Copy. Test `unknown_type_is_not_treated_as_copy`
trzyma ten konserwatyzm. Lepiej dostać zbędne „not Copy” przy już błędnym
typie niż puścić ucieczkę.

`promote` jest w `apply_expr_promote`. W katalogu `src/sema/tests/` nie ma
osobnego pliku o promote. Zachowanie jest w `walk.rs` i w testach
wyższego poziomu (check + codegen). Gdy czytasz semę, nie wnioskuj z
milczenia testów jednostkowych, że promote jest niezaimplementowany.
Listing 8.3 przechodzi `bork build`.

## Testy, które definiują kontrakt

| Plik | Co zamyka |
|---|---|
| `sema/tests/smoke.rs` | `peel_blocks`, próbka MVP |
| `sema/tests/infer.rs` | brak fałszywych move przy Copy |
| `sema/tests/expr_move.rs` | wyrażeniowy `move`, ponowny move, assign-up |
| `sema/tests/regional.rs` | bloki move, pętle, scalanie `if`, kompaktowanie |
| `sema/tests/call.rs` | `val`/`var` na granicy wywołania |
| `sema/policy.rs` testy | `classify_use` i treść podpowiedzi |

Dump ASCII jest w `src/dump.rs`, nie w semie. Sema produkuje drzewo, dump
je drukuje. LSP używa tego samego `dump_arenas`.

## Podsumowanie

- Sema buduje drzewo `ArenaNode` i flagi `moved`. Nie dotyka płyt 4 KiB.
- `classify_use` jest definicją Copy, Shared i Move.
- `None` kontra `Some([])` przy przechwyceniu jest różnicą językową i różnicą w AST.
- Pętla zabrania move nazw widocznych na wejściu. `if` wymaga zgodności obu gałęzi.
- Escape i hoist są później i nie żyją w tym katalogu.
- Parametry domknięcia mają w semie typ `Unknown`, więc dump potrafi pokazać Shared tam, gdzie typeck widzi Copy.
