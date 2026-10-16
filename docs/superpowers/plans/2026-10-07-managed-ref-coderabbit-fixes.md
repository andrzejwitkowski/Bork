# Naprawa sześciu uwag CodeRabbit w PR #21 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Delegation requires choosing that execution method explicitly.

**Goal:** Usunąć sześć potwierdzonych błędów lifetime, cleanupu i typowania z PR #21 oraz dowieść poprawności przez testy frontendu, runtime, LLVM IR i native build/run.

**Architecture:** Zachować istniejący planner aren i runtime RC. Poprawić kolejność destrukcji, zwalniać tymczasowe Ref po utworzeniu niezależnej obserwacji, odrzucać nieobsługiwane typy na etapie typowania i konserwatywnie promować targets przekazywane do wywołań. Rekord posiada payload swoich pól String i tablic: materializacja do areny właściciela korzysta z istniejących copierów, a tablice Ref z istniejącego glue elementów.

**Tech Stack:** Rust 2021, istniejące testy Cargo, LLVM 23, przypięty Inkwell; bez nowych zależności.

**Spec:** [Zatwierdzony model lifetime A](../specs/2026-10-06-managed-ref-lifetime-a-design.md), `docs/language.md`, `docs/memory-model.md` oraz sześć [uwag inline PR #21](https://github.com/andrzejwitkowski/Bork/pull/21/files).

## Global Constraints

- Wykonanie w `/home/enrju/projects/Bork/.worktrees/managed-ref-arenas`, branch `feat/managed-ref-arenas`; sprawdzony head `b4d18529b3a4d4d4f40968e8d05703c56b0ebb39`. Główny checkout jest na innym branchu i zawiera pliki użytkownika.
- Przed wykonaniem odczytać ten plan i spec z głównego checkoutu; istniejący worktree nie zawiera nieśledzonego pliku spec. Sprawdzić aktualny HEAD/status przed edycją, uwzględnić późniejsze zmiany użytkownika.
- „koniec payload lifetime C <= koniec payload lifetime P”; „parent.id < child.id”. Drops dziecka kończą się przed recyklingiem payloadu rodzica.
- „Nie utrzymujemy parenta przez ukryty child→parent Strong. Nie dopuszczamy ancestor→Weak fallback.” Zachować asercje invariantów; zmienić ich kolejność, nie usuwać.
- „Grupa escaping lub z nieznanym lifetime dostaje parent R0.” Promocja następuje przed alokacją; bez migracji opublikowanego obiektu.
- „Owned pola, String bytes, array buffers i nested owned records używają tej samej dynamicznej areny.” Ten plan poprawia pola String/tablic; nie zmienia tożsamości obiektów klas ani klasyfikatora Ref.
- Kopia payloadu dotyczy granicy zapisu do owned pola rekordu. Zwykłe odczyty Shared i dotychczasowe reguły move/promote pozostają zgodne z dokumentacją.
- `Ref<T>` obsługuje nie-nullowalną klasę; `?.` dla pól klasy wymaga managed `Ref<T>`. Dotychczasowe `?.length` i nullable struct fields zachować. Nullable class lowering nie należy do tej naprawy.
- Brak nowego frameworka testowego, collectora, publicznego ABI i ogólnego systemu interprocedural summaries.
- Zgodnie z wcześniejszym wymaganiem użytkownika publikowany branch ma mieć jeden commit zawierający wszystkie zmiany. Kroki mają lokalne checkpointy, nie osobne opublikowane commity. Commit/amend i push dopiero przy zleceniu publikacji po weryfikacji.

## Review Focus

1. Normal exit, return, break, continue i reset iteracji kończą drops/pins przed recyklingiem rodzica — zadania 1 i 4.
2. Strong, Weak, wygasły Weak i None: zwolnienie tymczasowego uchwytu nie zwalnia niezależnego pina i nie powoduje double-drop — zadanie 4.
3. `String? = None` pozostaje None, `Some("")` pozostaje obecne, tablica długości zero nie dereferencjonuje elementów — zadanie 6.
4. Istniejący String, tablica liczb, tablica String, slice i tablica Ref zapisane przez konstruktor lub alias receivera mają poprawny payload i cleanup — zadanie 6.
5. Klasa przekazana do funkcji, która zwraca ją jako Ref, także przez zagnieżdżone/rekurencyjne wywołanie, ma plan Dynamic/Program; String i Copy struct nie są przypadkiem traktowane jako klasy — zadanie 5.

## Mapa plików

Wszystkie ścieżki poniżej są względne wobec worktree PR #21.

| Pliki | Odpowiedzialność |
| --- | --- |
| `crates/bork_runtime/src/arena.rs`, `crates/bork_runtime/src/lib.rs` | Kolejność destruction i testy kontrolnych bloków/RC |
| `src/typeck/lower.rs`, `src/typeck/expr/field.rs`, `src/typeck/tests/managed_ref.rs` | Ograniczenia targetu Ref i odbiorcy `?.` |
| `src/codegen/llvm/region_emit.rs`, `src/codegen/llvm/presence.rs`, `src/codegen/llvm/mod.rs` | Owned temporaries, observation guards, regresje IR |
| `src/memory.rs` | Konserwatywne lifetime dla argumentów klas i targetów w tablicach Ref |
| `src/codegen/llvm/managed_ref.rs`, `src/codegen/llvm/array/emit.rs` | Wspólna materializacja payloadu pól i kopiowanie tablic Ref |
| `tests/build.rs` | Native build/run przez istniejące `build_and_run` i `stdout_of` |
| `docs/language.md`, `docs/memory-model.md` | Dokładna granica obsługiwanych typów oraz owned pola |

Kolejność: **#1 → #6 → #5 → #2 → #4 → #3** z oryginalnego review. Zadania poniżej są ponumerowane według wykonania.

## Task 1: Drops przed kontrolą scoped children — uwaga #1

**Files:** Modify `crates/bork_runtime/src/arena.rs::destroy_control`; test `crates/bork_runtime/src/lib.rs`, `tests/build.rs`.

**Interfaces:** Zachować `unsafe fn destroy_control(arena: *mut ArenaControl)` i publiczne `bork_arena_pop`/`bork_arena_reset`. Późniejsze zadania polegają na tym, że zarejestrowane drops mogą zakończyć lifetime scoped child podczas destruction rodzica.

- [ ] Dodać runtime test `scoped_child_registered_root_drops_before_parent_recycle`: utworzyć lexical parent i scoped dynamic child, zatrzymać child przez `bork_arena_retain_weak`, zarejestrować jego początkowy strong root na parent i wykonać pop parent. Asercje: child ma stan Dead, dropy zakończyły się przed recyklingiem; na końcu zwolnić weak lease. Nie odczytywać zwolnionego control blocka bez tego lease.

```rust
assert_eq!(unsafe { bork_arena_state(child) }, ArenaState::Dead);
assert_eq!(unsafe { bork_arena_state(parent) }, ArenaState::Dead);
```

- [ ] Uruchomić `cargo test -p bork_runtime scoped_child_registered_root_drops_before_parent_recycle -- --test-threads=1`; przed naprawą spodziewany abort procesu testowego na asercji scoped-child.
- [ ] W `destroy_control` zachować sprawdzenie Alive, ustawić Destroying, wyjąć i wykonać drops LIFO, następnie wymagać `live_scoped_children == 0`, dopiero potem oddać payload do poola. Nie force-free żywego dziecka.
- [ ] Dodać odpowiednik dla `bork_arena_reset` i native `builds_confined_ref_cleanup`: program `Box(7)` z lokalnym `Ref<Box>` ma exit 7 zamiast abortu; wersja z dwiema iteracjami sumuje wartości do exit 14.
- [ ] Uruchomić `cargo test -p bork_runtime -- --test-threads=1` oraz `cargo test --features codegen --test build builds_confined_ref_cleanup -- --test-threads=1`. Zapisać lokalny checkpoint i wyniki.

## Task 2: Wyłącznie obsługiwane targety Ref — uwaga #6

**Files:** Modify `src/typeck/lower.rs::lower_type`; test `src/typeck/tests/managed_ref.rs`, `tests/build.rs`; docs `docs/language.md`.

**Interfaces:** Zachować sygnaturę `lower_type`. Walidacja gałęzi `Type::ManagedRef` przyjmuje tylko `TyKind::Named(name)` obecny w `ClassTable`, z `nullable == false`; dotychczasowe Unknown i diagnostyka nested Ref pozostają.

- [ ] Dodać tabelaryczny test `managed_ref_target_must_be_nonnullable_class`. Osobne źródła z adnotacjami `Ref<String>`, `Ref<[i32; 2]>`, `Ref<Box?>`, `Ref<i32>` i `Ref<CopyStruct>` zgłaszają błąd typu; `Ref<Box>` z `Box(7)` i None przechodzą. `Ref<Ref<Box>>` nadal zgłasza błąd. Użyć istniejących `diags_of`/`hir_of`; diagnoza zawiera `managed Ref target must be a non-nullable class`.

```rust
assert!(diagnostics.iter().any(|d| d.message.contains("managed Ref target must be a non-nullable class")));
```

- [ ] Uruchomić `cargo test --lib managed_ref_target_must_be_nonnullable_class`; błędne obecnie akceptowane typy mają spowodować FAIL.
- [ ] Zmienić predykat w gałęzi `Type::ManagedRef` na sprawdzenie `classes.contains_key(name)` i braku nullable. Samo `Named` jest niewystarczające: `Ty::string` również daje Named("String").
- [ ] Dodać `build_rejects_invalid_managed_ref_targets`: istniejące `bork_build` zwraca kod 1, stderr zawiera diagnozę typu i nie powstaje binary dla String/tablicy/nullable klasy. Nie sprawdzać hipotetycznego runtime UB; testuje się granicę frontendu.
- [ ] Uruchomić oba testy filtrowane; zaktualizować dokumentację ograniczenia `Ref<T>`. Zapisać lokalny checkpoint.

## Task 3: Walidacja odbiorcy bezpiecznego dostępu — uwaga #5

**Files:** Modify `src/typeck/expr/field.rs::check_field`; test `src/typeck/tests/managed_ref.rs`, `tests/build.rs`; docs `docs/language.md`.

**Interfaces:** Zachować `check_field` i `HirExprKind::ObjectField`. Dla pól klasy `safe == true` jest poprawne wyłącznie przy `through_managed_ref == true`; diagnostyka pomija Unknown. Wcześniejsze gałęzie struct fields i buffer `.length` nadal używają własnej walidacji nullable.

- [ ] Dodać `safe_class_field_requires_managed_ref`: `box?.value` dla Box, `live?.value` dla `&Box` wewnątrz `if val` i pole nullable `Box?` zgłaszają diagnozę `?. on a class field requires Ref<T>`. Zwykłe `box.value` i `live.value` przechodzą.

```rust
assert!(diagnostics.iter().any(|d| d.message.contains("?. on a class field requires Ref<T>")));
```

- [ ] Uruchomić `cargo test --lib safe_class_field_requires_managed_ref`; przed zmianą oczekiwany FAIL.
- [ ] Dodać walidację przy `through_managed_ref` przed utworzeniem ObjectField. Nie używać komunikatu obiecującego nullable-class lowering, którego backend nie ma.
- [ ] W tym samym teście potwierdzić, że `ref?.value`, nullable `String?.length` i nullable struct field nadal przechodzą. Native `build_rejects_invalid_safe_class_field` ma kod 1 i brak binary; istniejący native/IR przypadek poprawnego Ref ma nadal poprawny wynik.
- [ ] Uruchomić `cargo test --lib safe_class_field_requires_managed_ref` oraz filtrowany test native z `--features codegen --test build`; zapisać lokalny checkpoint i doprecyzować docs.

## Task 4: Bilans owned temporary Ref i pina — uwaga #2

**Files:** Modify `src/codegen/llvm/region_emit.rs::emit_presence_with_driver`, `src/codegen/llvm/presence.rs::emit_safe_ref_field`; test `src/codegen/llvm/mod.rs`, `crates/bork_runtime/src/lib.rs`, `tests/build.rs`.

**Interfaces:** Zachować `observe_ref(reference) -> Result<(StructValue, PointerValue, IntValue), Diagnostic>`. Runtime `bork_ref_observe` zachowuje własny pin dla Strong/Weak. Zwalnianie tymczasowego handle wykorzystuje istniejące `cx.ref_drop_value_fn()`; nie zmieniać runtime ABI.

- [ ] Rozszerzyć testy IR presence/navigation: w odpowiedniej funkcji sekwencja jest `ref_observe` → `ref_drop_value(tymczasowy handle)` → rozgałęzienie presence. Testować if val, when oraz `?.`, także źródło będące polem Ref i wynikiem wywołania. Nie bazować na całkowitej liczbie dropów modułu, bo inne miejsca też poprawnie dropują Ref.
- [ ] Uruchomić `cargo test --features codegen --lib presence` i `cargo test --features codegen --lib safe_navigation`; nowe asercje mają FAIL przed zmianą.
- [ ] Po udanym `observe_ref` zwolnić owned temporary na wspólnej ścieżce przed branchem w obu emiterach. W presence najpierw wydobyć `control` potrzebny do bound `&T`; obserwacja utrzymuje target przy życiu po zwolnieniu handle. None i zero-cost Arena/Borrow przechodzą przez ten sam no-op drop.
- [ ] Dodać native `builds_presence_temporaries_cleanup` obejmujący if val, when, `?.`, early return, break i continue w pętli ze scoped targets `Box(7)`. Warianty pojedynczej obserwacji i return mają exit 7; pętla obserwująca trzy targets, zawierająca continue i break, ma exit 21. Runtime `observation_pin_survives_temporary_ref_drop`: po clone/observe/drop-clone licznik Strong odpowiada rootowi + oryginalnemu handle + pinowi; po release observation wraca do baseline. Dodać wariant Weak oraz wygasły Weak/None, zachowując metadata lease przed odczytami control blocka.

```rust
assert_eq!(unsafe { bork_arena_strong_count(target) }, baseline + 1); // pin po drop-clone
// Po bork_ref_release_observation(observation):
assert_eq!(unsafe { bork_arena_strong_count(target) }, baseline);
```

- [ ] Uruchomić filtrowane testy runtime, IR i native, potem `cargo test -p bork_runtime -- --test-threads=1`. Zapisać lokalny checkpoint.

## Task 5: Konserwatywny escape argumentów klas — uwaga #4

**Files:** Modify/test `src/memory.rs`; test `tests/build.rs`.

**Interfaces:** Zachować `memory::plan(program, report) -> (MemoryPlan, Vec<Diagnostic>)` i `mark_program(&Origins)`. W gałęzi Call obok managed Ref uwzględnić argumenty z `argument.ty.record_name().is_some()`: istniejący helper rozpoznaje klasę/borrow klasy, wyklucza String i Copy struct. Wynik call nadal nie ma precyzyjnego param-origin summary.

- [ ] Dodać `record_argument_passed_to_a_call_is_conservatively_program_lived`: `keep(n: Node): Ref<Node> { return n }`, caller tworzy x i przekazuje do keep. Asercje dla x: `ArenaClass::Dynamic`, `LifetimeDomain::Program`, `CreationParent::DomainRoot`. Drugi wariant wywołuje wrapper/rekurencyjną funkcję; przypadek String/Copy struct nie promuje unrelated allocation Node.

```rust
assert_eq!(allocation.class, ArenaClass::Dynamic);
assert_eq!(allocation.lifetime_domain, LifetimeDomain::Program);
assert_eq!(allocation.creation_parent, CreationParent::DomainRoot);
```

- [ ] Uruchomić `cargo test --lib record_argument_passed_to_a_call_is_conservatively_program_lived`; przed zmianą oczekiwany FAIL.
- [ ] Rozszerzyć filtr argumentów Call, używając istniejącego `record_name`, i zachować `mark_program` dla managed Ref. Dodać komentarz `ponytail:` wyjaśniający konserwatywną promocję przy braku escape summaries oraz możliwość przyszłego doprecyzowania. Nie dodawać analizy call graph do tej naprawy.
- [ ] Dodać native `builds_returned_ref_to_record_argument`: wcześniej utworzony x, potem dynamic owner z polem `Ref<Node>` wypełnionym `keep(x)` — build/run kończy się exit 0 bez próby Weak do lexical targetu. Osobny wariant owner utworzony przed x zachowuje Strong target `Node(7)` po zakończeniu scope; odczyt pola daje exit 7. Test nie wymaga, by back-edge Weak zatrzymywał payload.
- [ ] Uruchomić filtrowany test planera i native; istniejące `memory::tests` nadal przechodzą. Zapisać lokalny checkpoint.

## Task 6: Payload pól w arenie rekordu — uwaga #3

**Files:** Modify `src/codegen/llvm/managed_ref.rs`, `src/codegen/llvm/array/emit.rs`, `src/memory.rs`; test `src/codegen/llvm/mod.rs`, `src/memory.rs`, `tests/build.rs`; docs `docs/memory-model.md`, `docs/language.md`.

**Interfaces:** W `FnEmitter` dodać jeden wspólny helper `materialize_record_field(value: BasicValueEnum<'ctx>, ty: &Ty, owner: PointerValue<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic>`, używany przez konstruktor i zapis pola. Korzysta z `copy_into_arena`, `copy_array_into_arena` i istniejących nullable/presence helpers. Managed Ref nadal używa istniejących store/rehome/drop; Copy pola zachowują wartość, a zwykłe klasy zachowują identity.

- [ ] Dodać testy IR `record_fields_materialize_buffers_into_owner`: konstruktor i zapis istniejącego String/tablicy mają alokację/kopię do ownera rekordu, również jeśli źródło jest polem innego rekordu lub slice. Native `builds_record_owned_field_payloads` zwraca `Ref<Node>` zawierający lokalny `concat("a", "b")` i `[11, 22]`, po czym wykonuje nowe alokacje recyklujące stare lexical slabs; odczyt pól wypisuje `ab\n22\n`. Drugi wariant zapisuje `concat("x", "y")` do outer record z inner scope i wypisuje `xy\n`. Tablica String zawierająca lokalne concat daje po powrocie `cd\n` dla drugiego elementu. Kopia tablicy liczb pozostaje `[11, 22]`, gdy źródło jest później zmienione na `[99, 22]`.

```rust
assert_eq!(run.status.code(), Some(0));
assert_eq!(stdout_of(&run), "ab\n22\n"); // podstawowy wariant return + pool reuse
```

- [ ] Uruchomić filtrowane testy IR/native i potwierdzić FAIL przed zmianą. Sam poprawny stdout bez wymuszenia reuse poola nie wystarcza jako dowód braku dangling deskryptora.
- [ ] W obu miejscach zapisu materializować String i tablicę do `owner`, przywracając poprzedni `alloc_sink` także przy błędzie emitera. String? obsłużyć przez `emit_nullable_is_null` i istniejący presence split: None pozostaje oryginalnym null deskryptorem, kopiowana jest tylko obecna gałąź. Nie zamieniać None na pusty String wskutek zero-byte allocation. Tablica liczb korzysta z memcpy; tablica String z istniejącego kopiowania elementów wraz z bytes.
- [ ] Rozszerzyć `copy_array_into_arena` dla elementów Ref: nowy bufor w ownerze; każdy slot jest wyzerowany, source handle jest klonowany, a kopia trafia przez `store_fresh_ref` (rehome/consume + rejestracja drop). Nie wykonywać memcpy uchwytów Ref ani consume bez uprzedniego clone source. Wykorzystać istniejącą pętlę kopiowania elementów. Publiczna reguła zakazu `move` tablic Ref w codegen gate pozostaje dotychczasowa; rozszerzenie służy internal owned-field materialization.
- [ ] W plannerze przy `ArrayLit` o elementach Ref oraz index-assignment wartości Ref konserwatywnie `mark_program` ich origins przed alokacją targetów. To zabezpiecza również tablice istniejące przed publikacją do uciekającego rekordu; same kopie buforów nie przedłużają życia lexical targetu. Zachować aktualizację origins nazw tablic przez union przy index-assignment. Dodać `ponytail:` z ograniczeniem precyzji i przyszłym destination-aware escape. Test `ref_array_targets_are_program_lived_before_field_publication` obejmuje literal i późniejszy index-store do wcześniej pustej tablicy, z targetami Dynamic/Program/DomainRoot.
- [ ] Dodać native `builds_record_ref_array_owned_payload`: owner utworzony przed targetem `Box(7)`, przypisanie tablicy Ref, zakończenie scope źródłowego bufora, obserwacja targetu po powrocie; exit 7 bez scoped-child abortu. Przetestować zastąpienie pola tablicowego i source array czytane po kopiowaniu: stare sloty dropowane raz przy końcu ich areny, nowa kopia niezależnie.
- [ ] Dodać `builds_record_optional_and_empty_payloads`: wydruk `None ?: "none"`, następnie `Some("") ?: "none"`, długości pustego zwykłego String oraz tablic liczb i Ref długości zero daje dokładnie `none\n\n0\n0\n0\n`, exit 0. Dotychczasowe `[String; N]` i `[Ref<T>; N]` testy pozostają zielone.
- [ ] Uruchomić filtrowane testy IR/planera/native; opisać w docs, że rekord posiada kopię bytes/buforów swoich pól, a Ref elementy zachowują reguły klasyfikacji source→target. Zapisać lokalny checkpoint.

## Końcowa weryfikacja i publikacja

- [ ] `cargo fmt --all -- --check` i `git diff --check` kończą się kodem 0.
- [ ] `cargo test --workspace -- --test-threads=1` przechodzi (frontend, runtime i corpus check).
- [ ] `cargo test --workspace --no-default-features -- --test-threads=1` przechodzi (brak zależności nowych walidacji od LSP).
- [ ] Z LLVM 23 dostępnym tak jak w istniejącym CI: `cargo test --workspace --features codegen -- --test-threads=1` przechodzi, obejmując tests/build, weryfikację IR i native corpus. Nie zastępować tego samym module.verify.
- [ ] Przejrzeć diff wobec `origin/main`: wszystkie sześć uwag ma odpowiadającą zmianę/test, brak osłabionych asercji i ref memcpy; stdlib/existing helpers wykorzystane bez nowych dependencies.
- [ ] Potwierdzić czystość zmian względem użytkownika: edycje tylko w worktree PR #21, bez plików z głównego checkoutu poza tym planem.
- [ ] Przy zleceniu publikacji: uzyskać świeży remote head, sprawdzić brak cudzych commitów, amend jeden commit brancha, sprawdzić `git rev-list --count origin/main..HEAD == 1`, wypchnąć z jawnym `--force-with-lease=refs/heads/feat/managed-ref-arenas:<sprawdzony-remote-SHA>` i sprawdzić PR head/commit count. Nowe wersje main wymagają ponownej oceny bazy, nie ślepego resetu.

Nie dodawać automatycznych odpowiedzi ani resolve wątków CodeRabbit bez zlecenia. Ostrzeżenie docstring coverage nie jest jednym z sześciu błędów i nie należy do tego planu.
