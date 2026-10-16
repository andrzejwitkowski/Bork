# Ref<T>: wariant A — rzeczywiste drzewo lifetime aren

**Status:** użytkownik wybrał wariant A problemu przeżywalności ancestorów. Ten dokument rozwija plan; nie implementuje rozwiązania ani nie zmienia zatwierdzenia pozostałych decyzji.

**Baza:** remote main `56edc60afc3b945ef47f7da6fbe917f1ccef3c4b`, zgodnie z [pełnym planem](../plans/2026-10-06-managed-ref-arenas.md). Bez ponownego przepisywania AST/parser/runtime analizy.

## 1. Gwarancja, którą wybieramy

Dla każdej rzeczywistej relacji parent P → child C:

```text
koniec payload lifetime C <= koniec payload lifetime P
parent.id < child.id
```

„Koniec” oznacza zakończenie drops i destrukcji oraz niedostępność payloadu. Dopiero potem wolno recyklować bytes parenta. Pozostawienie samego control blocka P nie spełnia tej gwarancji.

W fazie destrukcji P może być Destroying, gdy dropuje Strong do C. Payload P nadal istnieje podczas kaskady. Nie wymagamy, aby stan P pozostał Alive przez wszystkie callbacks; wymagamy poprawnej kolejności i zakazujemy nowych normalnych obserwacji po wejściu w Destroying.

Konsekwencja: `isAncestor(T,S)` jest wystarczającym uzasadnieniem BorrowRef. Dowód outlives następuje przy planowaniu i tworzeniu aren, nie przy każdej dereferencji.

Nie utrzymujemy parenta przez ukryty child→parent Strong. Nie dopuszczamy ancestor→Weak fallback. Nie naprawiamy błędnego drzewa runtime reparentingiem ani wymuszonym zniszczeniem child przy dodatnim strong_count.

## 2. Minimalna polityka tworzenia aren

| Arena | Parent w pierwszej wersji | Lifetime payloadu | Escape |
|---|---|---|---|
| R0 | Brak: sentinel | Cały program | Nie jest zwykłym RC payloadem |
| Lexical | Aktualna fizyczna lexical arena albo R0 | Structured scope/call/iteration | Nie może uciec jako surowy payload |
| Dynamic swobodna | R0 | Dopóki strong_count > 0 | Dozwolony |
| Dynamic scoped | Konkretny żyjący lexical parent P | RC, ale najwyżej do końca P | Wszyscy owners/pins/publikacje ograniczeni do P |

Scoped nie oznacza nowego publicznego typu ani osobnego modelu RC. To wewnętrzna decyzja parenta dynamicznej construction group, z wymaganym dowodem confinement.

```text
R0 — trwała domena; nie posiada strong do wszystkich children
├── L — lexical region
│   ├── C — dynamic scoped; musi umrzeć przed recycle L
│   └── L2 — nested lexical region
├── A — dynamic swobodna
└── B — dynamic swobodna
```

Pod dynamic parentem nie tworzymy w MVP niezależnych children. Owned pola, String bytes, array buffers i nested owned records używają tej samej dynamicznej areny. Niezależna uciekająca grupa powstaje pod R0. To celowe ograniczenie drzewa, a nie publicznej możliwości budowania grafu obiektów.

Nie tworzymy domyślnie dynamic arena dla każdego obiektu. Zwykłe constructors nadal trafiają do lexical/specified sinka. Dynamic construction group pojawia się dla escaping, adresowalnego targetu lub niezależnego managed lifetime; wszystkie jej owned payloads współdzielą sink.

R0 nie inkrementuje strong_count dzieci. Swobodna dynamic arena ma początkowo `strong_count=1` dla hidden construction root i może umrzeć długo przed zakończeniem programu.

## 3. Jak compiler wybiera parenta

Planner działa na typed HIR i istniejących region facts, zanim emitowana jest konstrukcja i zanim powstaje stabilna object identity.

Minimalne wewnętrzne facts:

```text
LifetimeDomain = Program | LexicalScope(ScopeId)
CreationParent = DomainRoot | LexicalScope(ScopeId)

ArenaPlan {
    allocation_site
    destination
    lifetime_domain
    creation_parent
}
```

ScopeId i allocation_site są statycznymi identyfikatorami kompilatora. Nie zastępują runtime arena_id. Codegen mapuje lexical scope na aktualną rzeczywistą generation; jeśli arena została elided, na właściwego fizycznego ancestor ownera.

Reguły w kolejności:

1. Zwykły, nieuciekający owned obiekt pozostaje w istniejącym lexical/consumer sinku.
2. Owned części dynamic construction group pozostają w jej sinku; nie tworzą kolejnych RC aren.
3. Niezależna dynamic grupa dostaje lexical parent P tylko z pozytywnym dowodem confinement wszystkich jej owners i publikacji do P.
4. Grupa escaping lub z nieznanym lifetime dostaje parent R0.
5. Przechwycone adresowalne targets Ref również podlegają analizie: escaping storage nie może otrzymać Arena/Borrow do krócej żyjącego lexical payloadu.

Facts są propagowane do stabilnego planu przed codegen. To analiza przepływu wartości i wymagań alokacyjnych w compilerze, nie runtime analiza grafu referencji. Nie dodajemy runtime DFS/SCC/collectora ani wymagania, aby użytkownik analizował ownership graph.

### Co obejmuje dowód confinement

Nie wystarczy śledzić nazwę `val child`. Trzeba uwzględnić:

- Hidden owning roots construction group, kopie owning T i transfery move/return.
- Ref create/copy/rehome do lokali, pól, array elements i całych aggregates.
- Aliasy receivers: destination jest areną obiektu/bufferu, nie scope lokalnej nazwy.
- Temporary pins z `if val`, `when`, `?.`, Elvis oraz wszystkie exits ich scope.
- Argumenty i wyniki wywołań, branch merges i loop iterations.

W pierwszej wersji dopuszczamy publikacje confined child tylko do storage, którego lifetime jest dowiedzione jako ograniczone do P: samego P, jego lexical descendants i scoped groups również ograniczonych do P. Każdy external root/pin musi mieć cleanup wewnątrz P.

Zapis do swobodnej dynamic areny lub nieznanego receivera oznacza brak confinement, nawet gdy chwilowo taki obiekt ma tylko lokalny binding w P. Lokalna nazwa nie dowodzi lifetime areny.

Konserwatywnie nie publikujemy scoped handles poza P nawet jako Weak. Pozwolenie na takie handles jest możliwą późniejszą precyzją analizy, nie warunkiem poprawności MVP.

### Transitive escape jest obowiązkowy

Jeśli C ma pole Ref do lexical obiektu p, a C ucieka, nie wystarczy zaplanować tylko C pod R0. Target p również musi otrzymać odpowiedni przedalokacyjny plan albo zachować dowiedzione dozwolone lexical użycie bez ucieczki.

W szczególności zwykła lexical arena nie jest RC-retainable. Nie wolno tworzyć Weak do niej i następnie przez weak_upgrade udawać, że strong_count przedłuża jej payload. Dla escaping Ref target planner wybiera dynamic construction group przed utworzeniem targetu; jej owned payloads również trafiają do tej grupy.

Jeżeli wymaganie Program pojawia się przez captured Ref, propagacja może zmienić plan kilku constructors. Żaden z tych obiektów nie jest jeszcze opublikowany, więc nie ma runtime migracji ani deep-copy cyklicznego grafu. Nie przenosimy już istniejącego object body po fakcie.

## 4. Funkcje: rzeczywiste ancestry zamiast fikcyjnego scope

Obecny report daje każdej funkcji osobny statyczny root. Dla A runtime root callee musi być child aktualnej lexical generation callera, aby przekazany Ref do lexical caller targetu mógł pozostać Borrow.

Proponowany internal ABI:

```text
bork.f(lexical_parent, [result_sink jeśli potrzebny], user_args...)
```

- Main zachowuje zewnętrzne C ABI i pobiera R0 wewnętrznie.
- Callee tworzy lexical root pod lexical_parent; caller nadal wykonuje swój scope.
- Runtime C ABI oraz source signatures/arity nie dostają publicznego parametru.
- Lexical parent to aktualna fizyczna arena, nie dynamic allocation sink.
- Optional result_sink wybiera miejsce owned wyniku; nie określa parenta lexical scope.
- Hidden params obsłużyć jawnie: obecne zip source params i LLVM params/args nie może ich przypadkowo skonsumować.

Confinement call summaries rozróżniają znane nonescaping użycie, przepływ do wyniku oraz persistent/unknown escape. Nieznane lub nieudowodnione przypadki wybierają Program/R0. MVP może być konserwatywne dla recursion; nie może zakładać nonescape dlatego, że nie zobaczyło zapisu w samym call site.

Powrót Ref do lexical obiektu callee nie jest legalnym dangling return. Target musi być wcześniej zaplanowany w dynamic group albo wyniku caller sinka z poprawnym lifetime. Powrót Ref do obiektu callera może zachować jego identity zgodnie z provenance i rebase rules.

## 5. Klasyfikacja pozostaje dokładnie zadana

| Warunek | RefKind | RC na utworzeniu |
|---|---|---|
| S == T, ta sama generation | Arena | 0 |
| T jest ancestor S | Borrow | 0 |
| S != T i S.id < T.id | Strong | target strong++ |
| Pozostały | Weak | target weak++ |

Klasyfikujemy według faktycznych source i target controls. Source pola to arena receiver object body, nie lokalny binding. Nie używamy statycznych site IDs.

Strong/Weak mogą zachowywać payload/control tylko zgodnie z trybem docelowej areny. Nie publikujemy ich do zwykłego lexical targetu; allocation facts i assertions chronią tę granicę. Borrow do R0/lexical ancestor nie potrzebuje RC ani dead check.

Dowody są oddzielne:

1. `S.id<T.id` dla każdego persistent Strong wyklucza strong cycles.
2. Lifetime tree plus owner confinement wyklucza dangling Borrow do ancestor.
3. Ta sama generation i poprawny scope/ownership wykluczają dangling ArenaRef.
4. Successful weak_upgrade i guard do końca użycia wykluczają dangling Weak observation.

Roots/pins nie są persistent payload edges. Nie można zapisać guarda do pola z pominięciem klasyfikacji. W A pin scoped child również musi zakończyć się przed parent cleanup.

## 6. Root ownership, destruction i cleanup

Dynamic group zaczyna z hidden root strong ownerem. Przekazanie owning wyniku przekazuje ten root; Ref publication tworzy sklasyfikowany edge. Drop lokalnego aliasu obiektu nie niszczy osobno arena-owned object body: jego drop glue jest wykonane przez arena destruction registry, dokładnie raz.

Adresowalne, opublikowane obiekty nie relokują się i nie podlegają niezależnemu wcześniejszemu payload free. Move owning handle zmienia owner ledger, nie przesuwa object body. Existing borrow/move rules nadal zabraniają niepoprawnej mutacji/reseating adresowalnego miejsca.

Przy końcu lexical P:

```text
cleanup nested scopes i guards
drop managed locals/roots w reverse construction order
state(P) = Destroying
drop arena-owned managed values/fields P, w tym Strong do scoped children
    release child -> zero -> Destroying -> drops -> payload free -> Dead
assert live_scoped_children(P) == 0
recycle/reset payload P
state(P) = Dead
```

Podział obowiązków na local cleanup i registry musi wykluczać double-drop. Registry P nie jest listą wszystkich referencji w systemie: zawiera wyłącznie jego własne wartości wymagające destruction. Kolejność pomiędzy entries i locals wynika z construction ownership, nie z iteracji HashMap.

Normal exit, return, break, continue i loop reset wywołują ten sam mechanizm cleanup. Return najpierw zabezpiecza/przekazuje wynik; nie wynosi scoped root poza P. Continue niszczy porzucone nested scopes i bieżące iteration locals; reset daje nową generation. Break nie pomija nested drops.

### Mały runtime assertion zamiast grafu

Lexical parent przechowuje `live_scoped_children: usize`:

- Increment przy utworzeniu bezpośredniej scoped dynamic child.
- Decrement po zakończeniu destruction jej payloadu, przed zgłoszeniem child jako zakończonej.
- Zero wymagane przed recycle lexical parent payloadu.

To jeden licznik per lexical control, bez list children, reverse edges ani cycle detection. Nie decyduje o lifetime, nie zastępuje dowodu compilerowego i nie utrzymuje parenta przy życiu. Naruszenie jest błędem invariantów, zakończonym fail-fast, a nie pretekstem do siłowego free żyjącej child.

Metadata parenta może przeżyć jego payload dzięki istniejącemu w planie ancestry weak lease. Ten lease służy wyłącznie bezpiecznemu parent-chain traversal/control lifetime; nie jest root ownership payloadu.

## 7. Przykłady rozstrzygające problem

### Prawdziwy scoped parent–child

```text
R0
└── P lexical id=10
    └── C scoped dynamic id=20

obiekt p w P -> obiekt c w C: Strong
obiekt c w C -> obiekt p w P: Borrow
```

Wszyscy owners C mieszczą się w P. Drop lokalnego root C może nie zniszczyć C, jeżeli `p.child` nadal trzyma Strong. Cleanup P dropuje również to pole; C kończy destruction, zanim bytes P są recyklowane. Backref ma zero RC.

### Starsza zewnętrzna arena X zatrzymuje C

Jeśli source zamierza zapisać c do starszej swobodnej X i używać po zakończeniu P, confinement nie jest spełnione. Planner tworzy C pod R0 od początku. Gdy C zachowuje Ref do p, również p musi otrzymać odpowiedni escaping plan, jeśli dotąd był lexical targetem.

Po takim planie areny p i c mogą być niezależnymi siblings R0. Przy `p.id<c.id` forward jest Strong, backref Weak. To nie ancestor→Weak: ancestor relacji w tym drzewie po prostu nie ma. Publiczne typy obu pól pozostają Ref; użytkownik obsługuje None identycznie.

### Dwa dynamiczne obiekty logicznie parent/child

`node.parent` określa relację obiektów, nie runtime aren. Swobodnie escaping obiekty mogą należeć do jednej dynamic areny, albo do dwóch siblings R0. W pierwszym przypadku oba edges są Arena, w drugim Strong/Weak według ID.

Nie obiecujemy Borrow tylko dlatego, że pole nazywa się parent. Nie obiecujemy, że dowolne nested dynamic arena może przeżyć swój rzeczywisty parent: A wyklucza taką sytuację z definicji.

## 8. Konkretne kroki i pliki — rozszerzenie etapów 5/8/11/13

| Krok | Zmiany w repo | Wynik / testy / zależności |
|---|---|---|
| A1. Facts lifetime i provenance | `src/memory.rs` (nowy), `src/hir/mod.rs`, `src/frontend.rs`, `src/escape.rs` | ArenaPlan, LifetimeDomain, CreationParent i origins Ref; test actual receiver vs local binding. Po typowaniu i records |
| A2. Confinement i allocation plan | `src/memory.rs`, `src/hoist.rs`, `src/escape.rs`, `src/sema/{walk,report}.rs` | Propagacja escape przez calls/fields/arrays/returns; R0 dla unknown; zakaz późnej migracji. Test transitive captured target. Po A1 |
| A3. Runtime tree policy | `crates/bork_runtime/src/lib.rs`, `src/codegen/llvm/arena.rs`, `src/codegen/regions.rs` | R0, parent/depth/id, generation; tylko dopuszczone kombinacje parent/mode, live_scoped_children. Bez grafu. Po A2 i runtime controls |
| A4. Hidden caller context | `src/codegen/llvm/{mod,context,emit_fn,expr}.rs`, `src/region_walk.rs`, `src/codegen/llvm/region_emit.rs` | Internal lexical_parent, jawny hidden param binding, aktualny loop phi; source/main ABI bez zmian. Po A3 |
| A5. Zintegrowany cleanup | `src/codegen/llvm/{emit_fn,drop,managed_ref,region_emit}.rs`, runtime lib.rs | Cleanup boundaries dla każdego exit, child Dead przed parent recycle, exactly-once drops. Po needs_drop, RC, A3/A4 |
| A6. Release gate A | `src/memory.rs` unit tests, runtime tests, `tests/build.rs`, `programs/{build,check,check_fail}/managed_ref` | Wszystkie scenariusze poniżej oraz dotychczasowy baseline. Po A1–A5; bez nowego frameworka |

`drop.rs` i `managed_ref.rs` są nowymi modułami już przewidzianymi w głównym planie. Nie dodajemy osobnego lifetime runtime, ownership engine ani nowego backendu. AST/parser i public type API nie potrzebują dodatkowych nodes tylko dla A; stosują zmiany Ref/records/presence z głównego planu.

Potential regressions: nadmierna dynamic alokacja, utrata ancestry na call, mylenie alloc_sink z lexical_parent, hidden param/source arg przesunięcie, stale loop generation, zbyt optymistyczny unknown-call summary, pominięte Ref w aggregate, double-drop registry/local.

## 9. Testy akceptacyjne wariantu A

1. Confined P lexical/C dynamic: Strong forward, Borrow back; zero weak bookkeeping backref.
2. P payload nie jest recyklowany przed destruction C; registry drops nadal mogą czytać wymagane payload bytes.
3. X dłużej żyjąca zatrzymuje C: plan C pod R0 przed allocation, nigdy scoped child P.
4. Escaping C ma Ref do p: transitive plan p również bez lexical dangling i bez przenoszenia opublikowanych pointerów.
5. Nieznany receiver/call: brak fałszywego confinement; konserwatywny Program/R0.
6. Zapis scoped targetu do confined sibling jest poprawny; wszystkie jego Strong edges znikają przed parent recycle.
7. Hidden root move, field replacement i zwracany aggregate nie omijają confinement.
8. Weak pin scoped targetu kończy się na normal exit, return, break, continue; target żyje do końca borrow use.
9. Funkcja dostaje Ref do lexical caller targetu: actual ancestor Borrow, bez Strong/Weak do lexical payloadu.
10. Recursive calls i loop generation mają różne IDs i poprawnego actual parenta; empty region elision nie tworzy fikcyjnych arenas.
11. Runtime test świadomie łamiący confinement kończy się assertion/fail-fast przed recycle P; nie force-free C.
12. Dead controls i pooling nie „ożywiają” starych Weak ani nie zrywają parent metadata lease.
13. Seeded property tests generują tylko drzewa dozwolone przez A, losowe edges i kolejność cleanup zgodną z owner domains. Sprawdzają ordered Strong i child payload-before-parent. Analiza cykli tylko w testach.

Harness i komendy pozostają jak w pełnym planie. Przed release wymagane native build/run przypadki cleanup; sam module verify nie wykrywa błędów lifetime.

## 10. Granica MVP i późniejsze rozszerzenia

MVP A obejmuje pełny classifier, same-arena grafy, niezależne dynamic siblings R0, confined dynamic child pod lexical parentem, interprocedural actual ancestry, transitive allocation planning oraz wszystkie cleanup paths. Zachowuje publiczne Ref/None i guard-based presence sugar z pełnego planu.

Nie obejmuje dowolnych nested RC arenas pod mortal dynamic parentem. Później można dopuścić wybrane takie children wyłącznie z dowodem, że dominujący owner/pin parenta istnieje do zakończenia wszystkich child owners. Nie dodawać tej analizy bez rzeczywistej potrzeby; zwykłe owned podstruktury już współdzielą sink parenta.

W kolejnych optymalizacjach poprawiać precyzję nonescape summaries i ograniczać liczbę dynamic groups, nie osłabiać invariantów. Destruction queue musi zachować ukończenie child destruction przed parent payload recycle — samo odłożenie dropu child na później byłoby niepoprawne dla Borrow backrefs.

**Najważniejsza konsekwencja A:** compiler automatycznie wybiera bezpieczne drzewo przed alokacją. Użytkownik nadal buduje dowolny graf obiektów i widzi wyłącznie Ref, ale runtime nie dopuszcza child payload żyjącego dłużej niż jego rzeczywisty parent.
