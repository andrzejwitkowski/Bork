# Bork: zadania do zamknięcia enumów przed traitami

Audyt z 26.10.2026, gałąź `fix/option-if-val-regions`. Początkowy punkt odniesienia: `c95d813b98ab0f0bc2d6d433dee3e8269d1abb8e`; końcowa weryfikacja przykładów: **`b9afb9400af9b918d82e363e7049c270e465dcc7`**. Raport opisuje problemy i proponowane naprawy; nie wprowadza napraw do kompilatora.

**Rekomendacja: najpierw T01–T07.** T01 pozwala skompilować program czytający zwolnioną pamięć. T02–T07 dotyczą ownership, typowania i inferencji, na których będą opierały się traity. T08–T12 domykają prelude, backend i dokumentację etapu enumów.

W trakcie audytu katalog roboczy zmieniał się niezależnie od tego audytu. Poprawka `src/sema/walk/branches.rs`, rejestrująca bindingi zwykłego `when`, została w tym czasie zapisana jako `b9afb94`. Zbudowano ją ponownie w oddzielnym katalogu i powtórzono reprodukcje. T02 uwzględnia już tę poprawkę: zwykły match działa, ale ścieżka `Option` nadal ma błąd.

## Zakres i źródła wymagań

Podane adresy `/pull/49`–`/pull/59` zwracają 404. Istnieją natomiast **issues** o tych numerach. Poniższe opisy zostały odczytane przez GitHub API, a powiązanie z implementacją potwierdzone historią lokalnego repozytorium.

| Zgłoszenie | Zakres | Implementacja obecna w badanym kodzie |
| --- | --- | --- |
| [#49](https://github.com/andrzejwitkowski/Bork/issues/49) | Deklaracje enumów, konstruktory, generyki | PR #149, commit `f3b0d93` |
| [#50](https://github.com/andrzejwitkowski/Bork/issues/50) | Wzorce i tryb bindingów | PR #151, commit `8716c2f` |
| [#51](https://github.com/andrzejwitkowski/Bork/issues/51) | Exhaustiveness, czyli pokrycie wszystkich przypadków, i nieosiągalne ramiona | PR #153, commit `112c5fa` |
| [#52](https://github.com/andrzejwitkowski/Bork/issues/52) | Layout, konstrukcja i cleanup enumów | PR #155, commit `9be625a` |
| [#53](https://github.com/andrzejwitkowski/Bork/issues/53) | Natywne wykonanie `when` | PR #158, commit `17a4bfc` |
| [#54](https://github.com/andrzejwitkowski/Bork/issues/54) | `Option`, `Result`, nullable i interop z `Ref` | PR-y #160–#162 i dalsze poprawki na badanej gałęzi |

W chwili odczytu #49–#53 były zamknięte, #54 otwarte. Traity to osobne, otwarte zadania: [#55 — deklaracje i impl](https://github.com/andrzejwitkowski/Bork/issues/55), [#56 — bounds i solver](https://github.com/andrzejwitkowski/Bork/issues/56), [#57 — coherence](https://github.com/andrzejwitkowski/Bork/issues/57), [#58 — rozwiązywanie metod i monomorfizacja](https://github.com/andrzejwitkowski/Bork/issues/58), [#59 — associated types](https://github.com/andrzejwitkowski/Bork/issues/59).

Ważna zmiana projektu: oryginalne #54 zakładało rozdzielenie `T?` i `Option<T>`. Aktualny kod i nowsze fragmenty RFC definiują **`T?` jako `Option<T>`**. W raporcie przyjmuję obecną semantykę. Nie proponuję przywracania starego nullable.

## Jak zweryfikowano problemy

- Zbudowano kompilator z `--features codegen`, używający LLVM 23.
- Uruchomiono `cargo test --workspace --features codegen`: **714 testów zaliczonych, 0 błędów** na bazowym stanie. Następnie powtórzono cały zestaw na odizolowanym checkoutcie `b9afb94`: ponownie **714/714, 0 błędów**.
- Dla opisanych niżej przypadków uruchamiano oddzielnie frontend, `bork build`, a po udanym buildzie także wynikowy program.
- Na odizolowanym buildzie źródeł zgodnych z `b9afb94` ponownie potwierdzono T01–T09 i T11 oraz dwa dodatkowe problemy operatorów. Zwykłe bindingi wzorców po poprawce działają: przykłady kontrolne drukują `abc` i `inner`; problem `Option` z T02 pozostaje.
- Wyodrębniono z tego dokumentu wszystkie **17 bloków `bork`** i automatycznie sprawdzono podane wyniki frontend/build/wykonania. Przykład debugowy z T10 zbudowano z `-g` i potwierdzono opis `present`/`value` w jednostce DWARF Borka.
- Linkowanie odbywało się przez dostępny GCC wystawiony jako `clang`; emisja obiektów pozostała w LLVM 23. Nie sprawdzano rzeczywistego linkera Clang.
- Błędne odczyty z T01 powtórzono trzykrotnie. Sprawdzono też kontrolne warianty bez ponownego użycia areny oraz z payloadem budowanym bezpośrednio w konstruktorze docelowym.

Przykłady są kompletnymi plikami. Standardowe odtworzenie, po zapisaniu przykładu jako `/tmp/repro.bork`:

```bash
cargo run --features codegen --bin bork -- /tmp/repro.bork
cargo run --features codegen --bin bork -- build -o /tmp/repro /tmp/repro.bork
/tmp/repro
```

Jeżeli LLVM jest zainstalowane w lokalnym prefiksie, trzeba ustawić jego `LLVM_SYS_231_PREFIX` i `LD_LIBRARY_PATH`. Pierwsza próba testów bez ścieżki do `libz3.so.4` nie wystartowała; wynik 714/714 pochodzi z ponownego uruchomienia z poprawionym środowiskiem.

## Kolejność realizacji

| ID | Priorytet | Zadanie | Zależność od traitów |
| --- | --- | --- | --- |
| T01 | P0 | Utrzymać przy życiu zagnieżdżone payloady | Warunek bezpieczeństwa całej dalszej pracy |
| T02 | P1 | Domknąć bindingi wzorców w ownership, również po rewrite `Option` | Podstawa ownership w ciałach metod i funkcji generycznych |
| T03 | P1 | Inferować wynik `when` | Potrzebne również dla typów wynikających z bounds i projekcji |
| T04 | P1 | Uwzględnić `when` w analizie przepływu sterowania | Wspólna reguła dla funkcji i metod traitów |
| T05 | P1 | Walidować zakres literałów wzorców | Warunek poprawności natywnego matchingu |
| T06 | P1 | Walidować instancje enumów już we frontendzie | Konieczne przed solverem i normalizacją typów |
| T07 | P1 | Unifikować typy przy inferencji pożyczonych argumentów | Wcześniejszy błąd generyków, blokuje naturalne użycie #56/#58 |
| T08 | P2 | Rozwiązywać `Some` i `None` zgodnie ze scope | Domknięcie obietnicy zwykłego prelude w #54 |
| T09 | P2 | Oddzielić tożsamość prelude `Option` od samej nazwy | Domknięcie #54; istotne przed rozbudową tabel definicji |
| T10 | P2 | Poprawić debug ABI nullable po migracji na enum | Regresja narzędzi po #54 |
| T11 | P2 | Obsłużyć `unit` w payloadzie | Brakujący przypadek zwykłych enumów generycznych |
| T12 | P2 | Uzgodnić dokumentację i kryteria #54 z implementacją | Zapobiega implementowaniu sprzecznych wymagań |

T01 i T02 mogą zmieniać oczekiwane plany pamięci i raporty ownership. Dopiero po ustabilizowaniu zachowania należy aktualizować goldeny. T03 i T04 warto realizować obok siebie, ponieważ oba używają informacji o wyniku ramienia.

## T01. P0 — zwrócony enum może wskazywać na zwolniony payload

**Co jest źle.** Życie zewnętrznego slotu enuma nie zapewnia życia obiektów wskazywanych przez jego payload. Konstruktor potrafi zachować handle do obiektu z krótszej areny. Po wyjściu z funkcji inna alokacja nadpisuje obiekt nadal osiągalny przez wynik.

```bork
class N { n: i32 }
enum E { A(n: N) }

fun make(): E {
    val n = N(42)
    return E.A(n)
}

fun churn() {
    val n = N(12345)
}

fun main(): i32 {
    val e = make()
    churn()
    when e {
        A(n) => { println(n.n) }
    }
    return 0
}
```

**Obecnie:** frontend i build kończą się sukcesem, program drukuje `12345`. Wynik powtórzył się 3/3 razy. Bez `churn()` drukuje `42`.

**Powinno być:** `42`, również po dowolnych późniejszych alokacjach. Dopuszczalna semantyka przekazania obiektu nie może zależeć od zawartości zwolnionej strony pamięci.

Drugi wariant dotyczy bezpośrednio nowego `Option`:

```bork
class Box { x: String? }

fun make(): Box {
    val x: String? = Some(concat("ab", "cd"))
    return Box(x)
}

fun churn() {
    val x: String? = Some("01234567890123456789")
}

fun main(): i32 {
    val b = make()
    churn()
    when b.x {
        Some(s) => { println(s.length) }
        None => { println(0) }
    }
    return 0
}
```

**Obecnie:** `0`, zamiast `4`, 3/3 razy. Zamiana ciała `make` na `return Box(Some(concat("ab", "cd")))` daje prawidłowe `4`. Problemem jest przeniesienie wcześniej utworzonego slotu do dłużej żyjącego właściciela, a nie sam konstruktor `Some`.

Odtworzono też `Outer.Value(Inner.Value(String))`, z `Inner` wcześniej zapisanym w lokalnym bindingu: odczyt długości po `churn()` zwracał `825243960` zamiast `4`. Ta konkretna liczba jest skutkiem błędnego odczytu i nie jest stabilnym wymaganiem testu.

**Przyczyna w kodzie:**

- `src/codegen/llvm/managed_ref.rs::materialize_owned_value`: nullable zwraca istniejącą wartość bez relokacji; zwykły handle klasy/enuma również przechodzi bez zapewnienia życia wskazywanego obiektu.
- `store_owned_field` używa tej funkcji zarówno dla pól klas, jak i payloadów enumów.
- `src/memory.rs::Planner::after_expr`: konstrukcje rejestrują zależności przez bezpośrednie pola `Ref`, ale brakuje analogicznego śledzenia zagnieżdżonych handle'i klas/enumów. Plan zewnętrznego obiektu nie utrzymuje ich automatycznie przy życiu.
- `src/escape.rs::place` nie rozwiązuje tego na poziomie składania agregatu: nowy zewnętrzny obiekt może trafić do sinka, podczas gdy istniejący wewnętrzny obiekt pozostaje w starej arenie.

**Jak naprawić.** Domknąć regułę właściciel → zagnieżdżony obiekt w istniejącym planie pamięci i materializacji. Każdy zachowany handle musi wskazywać na arenę żyjącą co najmniej tak długo jak docelowy właściciel. Dla klas zachować tożsamość obiektu i użyć istniejących mechanizmów dynamicznych aren/korzeni. Dla relokowanych slotów enumów/`Option` przenosić aktywny payload wraz ze slotem, z właściwym retain/drop `Ref`. Nie wystarczy skopiować dwóch słów handle'a albo przenieść samego zewnętrznego slotu.

Najpierw ustalić jedną działającą ścieżkę dla przechowywania istniejącego handle'a w nowym właścicielu. Następnie zastosować ją w konstrukcji, przypisaniu do pola i materializacji wyników. Samo odrzucenie przykładu w gate nie domyka obiecanej obsługi payloadów z #52.

**Zasięg i testy:** `memory.rs`, `escape.rs`, `managed_ref.rs`, materializacja wyników w `emit_fn.rs` i `region_emit.rs`; testy w `tests/build.rs` lub `programs/build/enums/` i `programs/build/option/`. Dodać powyższe dwa programy oraz zagnieżdżony enum. Testować wynik **po ponownym użyciu areny**. Wariant z `Ref` ma dodatkowo sprawdzać liczniki po wyjściu właściciela, aby naprawa UAF nie zamieniła się w wyciek lub podwójny drop.

**Jeśli pominąć:** poprawnie typowany program może odczytać cudze dane, błędny tag lub nieprawidłowy wskaźnik. To blokuje przejście dalej.

## T02. P1 — bindingi wzorców muszą istnieć w ownership na obu ścieżkach `when`

**Co jest źle.** Na badanym commicie typeck wiąże nazwy wzorców, ale `sema::walk_match` otwiera ramiona bez ich parametrów. `move s` widzi nieznaną nazwę albo błędnie operuje na zewnętrznym `s`.

```bork
enum E { Text(s: String) }

fun main(): i32 {
    when (E.Text("abc")) {
        Text(s) => {
            val t = move s
            println(t)
        }
    }
    return 0
}
```

**Commit `c95d813`:** `` ownership: cannot move unknown name `s` ``. Powinno kompilować się i drukować `abc`: payload tymczasowego enuma jest wiązany przez move zgodnie z dokumentacją.

**Poprawka `b9afb94`:** rejestruje bindingi zwykłego `Expr::Match` przez `RegionFrame::bind_param`. Powyższy program przechodzi frontend, build i drukuje `abc`; wariant z przesłanianiem drukuje `inner`. Ta część jest już naprawiona. Pozostaje druga ścieżka, opisana poniżej.

```bork
fun main(): i32 {
    when (Option.Some("abc")) {
        Some(s) => {
            val t = move s
            println(t)
        }
        None => { }
    }
    return 0
}
```

**Obecnie, także na `b9afb94`:** `` ownership: cannot move unknown name `s` ``. Kolejność `Some(s)`, potem `None` powoduje parse do `Expr::IfVal`; typeck zmienia go na HIR `Match`, ale `sema::walk_if_val` dla `option_if_vals` nadal wywołuje `walk_arms` bez bindingów.

Wariant z `val s = "outer"`, następnie `move (s) {}`, a potem takim `when (Option.Some("inner"))` z `println(s)` w ramieniu zgłasza ``use of `s` after move from fun main``. Powinien czytać nowe, wewnętrzne `s` i drukować `inner`.

**Przyczyna:** `src/sema/walk/branches.rs::{walk_match,walk_if_val,walk_arms}`. Typeck zapisuje typ bindingu pod jego `AstId` w `src/typeck/expr/pat.rs::bind`. Sema musi korzystać z tych rozstrzygnięć; sama nazwa AST może oznaczać również bezparametrowy wariant enuma.

**Jak naprawić.** Dokończyć rozwiązanie z `b9afb94`, używając `RegionFrame` i typów z `node_tys` również dla `if val`/pary `Some`–`None` przepisanej na `Option`. Bindingi muszą przesłaniać zewnętrzne nazwy, mieć właściwy tryb borrow/move i zostać zdjęte ze scope przed połączeniem stanów ramion. Nie wstawiać nazw bezparametrowych wariantów jako zmiennych. Zachować istniejący cleanup `Ref` w backendzie.

**Zasięg i testy:** `sema/walk/branches.rs`, przekazywanie wyników typeck do sema, testy sema i frontendu, goldeny ownership/hover. Sprawdzić zwykły enum, `Option` w obu kolejnościach ramion, `if val opt ... else`, zagnieżdżone wzorce, przesłanianie, drugi move tej samej nazwy oraz brak wycieku bindingu poza ramię. Ponownie uruchomić test zgodności harmonogramu regionów.

**Jeśli pominąć:** zamiana równoważnego `when` na składniowy skrót może zmienić poprawny program w błąd ownership.

## T03. P1 — `when` bez oczekiwanego typu daje `unit`

```bork
enum E { A, B }

fun main(): i32 {
    val n = when E.A {
        A => { 42 }
        B => { 0 }
    }
    println(n)
    return 0
}
```

**Obecnie:** ``argument 1 to `println` has type unit, expected i32``. Zamiana na `val n: i32 = ...` kompiluje się i drukuje `42`.

**Powinno być:** inferencja `n: i32` i wynik `42`, bez adnotacji.

**Przyczyna:** `src/typeck/expr/control.rs::match_arms`, w bazowym commicie linia 192, wybiera `unit` zawsze, gdy `expected.is_none()`. W tym samym trybie nie porównuje też typów ramion, mimo że wylicza ich `body_ty`. To gubi wynik w inicjalizatorze i przy inferencji argumentu wywołania.

**Jak naprawić.** Ustalać typ wyniku z ramion mogących dojść do końca i unifikować ich typy także bez adnotacji. Rozróżnić użycie wyniku od zwykłego statementu: obecne programy używają `when` tylko dla efektów ubocznych, więc nie wolno przypadkiem odrzucić ich wyłącznie dlatego, że nieużywane ramiona kończą się różnymi wartościami. W razie potrzeby jawnie przekazać kontekst statement/value do istniejącego dispatch, zamiast utożsamiać „brak oczekiwanego typu” z „wynik nieużywany”.

**Testy:** powyższy program, wynik przekazany do funkcji generycznej, wynik z payloadu i niezgodne typy ramion w inicjalizatorze. Zachować test statementowego `when`. Obsługę ramion kończących się `return` uzgodnić z T04.

**Jeśli pominąć:** podstawowa inferencja wyrażeń przestaje działać dokładnie przy nowej konstrukcji języka; kolejne błędy solvera będą maskowane przez fałszywy `unit`.

## T04. P1 — analiza powrotów i typ wyniku nie rozumieją kończących ramion

```bork
enum E { A, B }

fun f(e: &E): i32 {
    when e {
        A => { return 1 }
        B => { return 2 }
    }
}

fun main(): i32 {
    val e = E.A
    println(f(&e))
    return 0
}
```

**Obecnie:** `function must return a value of type i32 on all paths`.

**Powinno być:** sukces i `1`. Exhaustiveness już zapewnia, że każde wejście wybierze jedno z tych ramion.

Drugi problem tej samej analizy:

```bork
enum E { A, B }

fun f(e: &E): i32 {
    return when e {
        A => { return 1 }
        B => { 2 }
    }
}

fun main(): i32 {
    val e = E.A
    println(f(&e))
    return 0
}
```

**Obecnie:** `` `when` arm has type i32, expected unit `` i `return value has type unit, expected i32`. Ramię z `return 1` nie daje wartości do połączenia, ale nie powinno narzucać pozostałym ramionom typu `unit`.

**Przyczyna:** `src/typeck/stmt/mod.rs::expr_always_returns` obsługuje tylko pełne `If`; brak `Match` i `PresenceMatch`. `check_value_block_in_current_scope` zwraca `unit` dla ostatniego statementu `return`. `src/sema/walk/branches.rs::expr_diverges` również nie obsługuje `Expr::Match`.

**Jak naprawić.** Rozszerzyć istniejącą analizę kończenia ścieżek o pełny match. Przy łączeniu typów brać pod uwagę wyłącznie ramiona z normalnym wyjściem. Informacja „to ramię kończy przepływ” wystarczy; ten fix nie wymaga od razu publicznego typu `Never`. Oddzielić powrót z funkcji od `break` i `continue`: te ostatnie kończą ramię, lecz nie dowodzą, że funkcja zwraca wynik.

**Testy:** oba przykłady, `return` w każdym ramieniu zagnieżdżonego matcha oraz `break`/`continue` wewnątrz pętli. Dodać przypadek ownership, w którym ścieżka z ruchem kończy się i nie powinna zatruwać dalszej, osiągalnej ścieżki. Potwierdzić cleanup aren przy wcześniejszych powrotach w `codegen_arena_push_pop_counts_match_schedule`.

**Jeśli pominąć:** użytkownik musi dopisywać martwe `return`, a analiza ownership nie ma poprawnej informacji o osiągalnych ścieżkach.

## T05. P1 — literał wzorca jest obcinany do szerokości typu

```bork
fun main(): i32 {
    val n: u8 = 0
    when n {
        256 => { println(1) }
        0 => { println(2) }
        _ => { println(3) }
    }
    return 0
}
```

**Obecnie:** frontend i build akceptują program. Wynik to `1`: wzorzec `256` dopasowuje się do `0`.

**Powinno być:** błąd typu z lokalizacją wzorca: `256` nie mieści się w `u8`. Nie potrzeba do tego analizy zakresów ani nowych range patterns.

**Przyczyna:** `src/typeck/expr/pat.rs::literal` sprawdza zgodność rodzaju typu, ale nie wartości literału. `src/codegen/llvm/match_emit.rs::emit_lit_test` przekazuje liczbę do `const_int` docelowego typu, co obcina `256` do 8 bitów. Exhaustiveness porównuje wcześniej liczby jako `i64`, więc nie widzi, że backend połączy te dwa wzorce.

**Jak naprawić.** Sprawdzić granice po ustaleniu typu wzorca, przed analizą pokrycia i codegenem. Wykorzystać istniejącą regułę `src/constants.rs::fits`, udostępniając ją w odpowiednim wspólnym miejscu; nie kopiować tabel zakresów do kolejnego modułu. Jeżeli typ scrutinee nadal jest zmienną inferencji, walidację wykonać po jej rozwiązaniu.

**Testy:** granice `u8` i `i8`, liczby tuż poza nimi, literał w zagnieżdżonym payloadzie oraz wzorzec przy później ustalanym typie. Po błędzie zakresu nie produkować dodatkowych, mylących komunikatów o pokryciu.

**Jeśli pominąć:** wykonuje się inne ramię niż wskazuje źródło programu.

## T06. P1 — frontend pomija walidację instancji generycznych enumów

Ten przykład jest niepoprawny i **powinien być odrzucony** już przez `check` i LSP:

```bork
enum E<T> { A(r: Ref<T>) }

fun main(): i32 {
    val e: E<i32> = E.A(None)
    return 0
}
```

**Obecnie:** frontend kończy się kodem `0`. Dopiero build zgłasza `` payload `r` of `E<i32>`: managed Ref target must be a non-nullable class or enum, got `i32` ``.

Dodanie nieużywanej funkcji `fun unused<T>(x: T): T { return x }` sprawia, że frontend zaczyna prawidłowo odrzucać program. Walidacja nie może zależeć od takiej niezwiązanej deklaracji. To samo pominięcie występuje przy skonstruowaniu bezpayloadowego `E.Empty`, jeżeli inny wariant tej instancji zawiera nieprawidłowe `Ref<i32>`.

**Przyczyna:** `src/frontend.rs::has_generics` sprawdza `hir.functions` i `hir.classes`, ale pomija `hir.enums`. W efekcie warunek uruchomienia `mono::check_instances` nie obejmuje programów z samymi generycznymi enumami. Walidator pól instancji w `mono.rs` już istnieje i potrafi zgłosić poprawny błąd.

**Jak naprawić.** Dodać enumy do istniejącego warunku. Nie pisać drugiego walidatora. Zachować dotychczasową regułę instancjonowania kodu osiągalnego z punktów wejścia.

**Testy:** frontend odrzuca powyższy przykład z nieużywaną funkcją generyczną i bez niej; poprawny `E<Node>` przechodzi. Sprawdzić też narastający rekurencyjnie typ enuma, korzystając z istniejącego limitu instancjonowania. LSP i build mają zgłaszać tę samą przyczynę.

**Jeśli pominąć:** solver traitów dostanie typy, które frontend uznaje za poprawne, chociaż po podstawieniu łamią reguły języka.

## T07. P1 — inferencja generyka przez `&Enum<T>` używa równości zamiast unifikacji

To wcześniejszy błąd wspólnej obsługi generyków, ujawniony na enumach. Jest zależnością przed #56/#58, a nie nową funkcją traitów.

```bork
enum E<T> { A(v: T) }

fun f<T>(e: &E<T>) {
    when e { A(v) => { } }
}

fun main(): i32 {
    val e: E<i32> = E.A(5)
    f(&e)
    return 0
}
```

**Obecnie:** `borrow has type &E<i32>, expected &E<?1>`.

**Powinno być:** inferencja `T = i32` i poprawny build. Kontrolne `f::<i32>(&e)` kompiluje się i wykonuje prawidłowo.

**Przyczyna:** `src/typeck/expr/borrow.rs::check_borrow` porównuje `expected != ref_ty` i `ref_inner(expected) != Some(inner.ty)` bez strukturalnej unifikacji. Typ z nierozwiązaną zmienną inferencji nie ma tego samego `TyId` co konkretny typ, nawet jeśli można je poprawnie uzgodnić.

**Jak naprawić.** Użyć istniejącej unifikacji `Env::same`/`InferCtxt`, zachowując obsługę reborrow i pojedynczy komunikat o niezgodności. Unifikację zagnieżdżonego `Enum` interner już obsługuje. Nie dodawać osobnej ścieżki specjalnej dla enumów.

**Zasięg i testy:** borrow i wywołania w typeck; testy generyków i reference. Powyższy przypadek, analogiczny `&Class<T>`, zagnieżdżony argument generyczny, reborrow parametru i rzeczywiście niezgodne typy. Sprawdzić konflikt dwóch argumentów wyznaczających inne `T`, aby fix nie zaczął akceptować błędnych wywołań.

**Jeśli pominąć:** zwykłe wywołanie funkcji z boundem będzie wymagać ręcznych argumentów typu mimo jednoznacznego receivera/argumentu.

## T08. P2 — `Some` i `None` nadal omijają zwykłe rozwiązywanie nazw

```bork
fun Some(x: i32): i32 { return x + 1 }

fun main(): i32 {
    val n = Some(5)
    println(n)
    return 0
}
```

**Obecnie:** wywołanie deklarowanej funkcji staje się konstrukcją nullable; użycie `n` zgłasza typ `?0?` zamiast `i32`. Oczekiwany wynik przy zwykłym rozwiązywaniu nazw to `6`.

```bork
fun main(): i32 {
    val None = 5
    println(None)
    return 0
}
```

**Obecnie:** deklaracja jest dozwolona, ale odczyt zgłasza `` cannot infer type of `None` ``. Powinien odczytać lokalny binding i wydrukować `5`.

**Przyczyna:** `src/ast.rs::{Expr::ident,Expr::call}` przepisuje te nazwy do `Expr::None`/`Expr::Some` przed rozwiązywaniem scope. Typeck nie może już zwyczajnie wybrać lokalnej nazwy lub funkcji. Zmiana leksera z keyword na identifier sama nie zapewnia semantyki zwykłej nazwy.

**Jak naprawić.** Zachować informację o nazwie do etapu rozwiązywania nazw. Lokalny binding i jawnie zadeklarowana funkcja powinny działać według tego samego pierwszeństwa co inne nazwy prelude. Dopiero brak przesłonięcia może uruchamiać konstrukcję wariantu/legacy `Ref` zależną od oczekiwanego typu. Zachować działanie `Option.Some`, `Option.None` i istniejących programów z `Ref<T>`.

**Zasięg i testy:** konstrukcja AST, numerowanie i indeks spanów, dispatch typeck, testy parsera/prelude/nazw. Dodać oba przykłady, parametr o nazwie `None`, funkcję `Some` oraz nieprzesłonięte konstrukcje `Option` i `Ref`. Nie usuwać mechanicznie `Expr::Some/None` bez prześledzenia ich odbiorców.

**Jeśli pominąć:** prelude pozostaje częściowo specjalną składnią i łamie normalne reguły przesłaniania nazw, mimo deklarowania go jako zwykłych enumów/funkcji.

## T09. P2 — dowolny użytkowy `enum Option` staje się typem systemowego nullable

```bork
enum Option<T> { Other(value: T) }

fun main(): i32 {
    val x: i32? = None
    return 0
}
```

**Obecnie:** frontend przyjmuje program. Build kończy się ``enum variant `Option<i32>.None` is not supported by codegen yet``.

**Powinno być według obecnej dokumentacji:** czytelny błąd frontendu, że `T?` nie jest dostępne po zastąpieniu prelude `Option` własną definicją. Zwykłe `Option.Other(5)` nadal powinno działać jako użytkowy enum. Nie może powstać nieistniejący wariant.

**Przyczyna:** `src/prelude.rs::add_prelude` poprawnie pomija chunk po przesłonięciu, ale `src/ty/mod.rs::TyCtx::new` ustawia `option_def = defs.lookup_enum("Option")`. W ten sposób nadaje tożsamość nullable dowolnej definicji o tej nazwie. `check_none` następnie akceptuje fallback presence-capable nawet przy braku wariantu `None`.

**Jak naprawić.** Przekazać do `TyCtx` tożsamość rzeczywiście dołączonego enuma prelude. Samo wyszukanie tekstu `Option` w tabeli definicji nie wystarcza. Przy braku tej definicji użycie `T?` powinno dać jedną sensowną diagnostykę zamiast cichego typu błędu lub nieprawidłowego HIR. Zachować jawnie wybraną w repo politykę: własna definicja użytkownika wygrywa.

**Testy:** własny `Option<T>` o innym kształcie, niegeneryczny `Option`, klasa `Option`, zwykłe prelude oraz `optionFromRef`/`optionToRef` w obecności przesłonięcia. W każdym przypadku frontend i build mają zgadzać się co do poprawności programu.

**Jeśli pominąć:** rozbudowa tabel definicji pod traity utrwali zależność semantyki typu od jego nazwy tekstowej.

## T10. P2 — DWARF opisuje nullable według usuniętego ABI

```bork
fun main(): i32 {
    val o: i32? = Some(42)
    return 0
}
```

Zbudować z `bork build -g`, następnie odczytać pierwszą jednostkę kompilacji Borka przez `readelf --debug-dump=info`.

**Obecnie:** DWARF opisuje `i32?` jako strukturę 16-bajtową z polami `present: bool` na offsecie 0 i `value: i32` na offsecie 8. Rzeczywista wartość to handle `{arena, object}`. Wersja `String?` dostaje z kolei pola `data` i `length`, również niezgodne z nowym handle'em.

**Powinno być:** metadane odpowiadające slotowi `Option<T>` i jego handle'owi. Sam alias wyświetlanej nazwy `i32?` jest dopuszczalny, ale pola i offsety muszą opisywać rzeczywiste dane.

**Przyczyna:** `src/codegen/llvm/debug/types.rs::ty` wybiera `nullable_di`/`buffer_di` przed ścieżką enuma. `nullable_di` nadal opisuje starą flagę obecności i inline payload. `Codegen::basic_type` zostało już zmigrowane na nowy handle, ale debug types nie.

**Jak naprawić.** Rozpoznawać rzeczywisty `TyData::Enum` przed dawnymi nullable/buffer queries i wykorzystać istniejący `object_di` dla właściwego enuma. Nie trzeba w tym zadaniu dodawać pełnej prezentacji wszystkich payloadów wariantów: aktualny skrót opisujący tylko tag jest osobną, jawną decyzją. Trzeba usunąć fałszywy opis pamięci.

**Testy:** rozszerzyć `tests/debug_build.rs` o `i32?`, `String?` i zwykły `Option`, porównując pola/metadane Borkowej jednostki DWARF. Dotychczasowy `debug_build_names_enums` sprawdza zwykły enum z `Ref`, więc nie wykrywa regresji aliasu nullable.

**Jeśli pominąć:** debugger pokazuje adres areny jako obecność albo pointer do slotu jako wartość/długość.

## T11. P2 — poprawny `Option<unit>` odpada podczas layoutu

```bork
fun main(): i32 {
    val o: Option<unit> = Option.None
    return 0
}
```

**Obecnie:** frontend akceptuje program. Build zgłasza `` enum payload `Option<unit>.value` has unsupported type `unit` `` — mimo że tworzony wariant nie zawiera payloadu.

**Powinno być:** poprawny build i exit `0`. `unit` jest prawidłowym typem argumentu generycznego; `Result<unit, E>` to również naturalny wynik operacji bez wartości sukcesu.

**Przyczyna:** `src/codegen/llvm/context.rs::enum_layout_of` żąda `basic_type` każdego pola wszystkich wariantów. `basic_type(unit)` daje `None`, bo funkcje zwracające `unit` nie zwracają wartości LLVM. Brak rozróżnienia między takim wynikiem funkcji a polem o zerowym rozmiarze.

**Jak naprawić.** Ustalić backendową reprezentację `unit` w payloadzie i obsłużyć ją w konstrukcji, odczycie oraz bindingu wzorca. Zachować `void` dla wyniku funkcji, gdzie jest wymagany. Jeżeli pole jest pomijane fizycznie, wszystkie offsety i indeksy payloadów muszą uwzględniać mapowanie pól logicznych na fizyczne; prosty znacznik payloadu może wymagać mniejszej zmiany. Nie dodawać reguły zakazującej `unit` w całym systemie typów tylko po to, żeby ukryć brak backendu.

**Testy:** powyższe `None`, konstrukcja i match wariantu z `unit` oraz wariant z `unit` przed zwykłym `i32`, żeby wykryć przesunięte indeksy. Dodać przypadek `Result<unit, String>`.

**Jeśli pominąć:** „zwykłe generyczne enumy” mają nieudokumentowany wyjątek, a frontend przyjmuje program niemożliwy do zbudowania.

## T12. P2 — uzgodnić końcowy kontrakt etapu #54

**Co jest źle.** To samo repozytorium opisuje różne generacje semantyki:

- `docs/language.md` w opisie #53 nadal mówi, że `when` po nullable jest odrzucane. Kilka akapitów niżej opis #54 poprawnie dopuszcza zwykły match po `Option`/`T?`.
- `docs/rfcs/0001-type-system-core.md`, akapit „Implementation (#54, part 3)”, nadal tłumaczy zgodność `MatchArm` z labelami `IfValSome`/`IfValNone`. Badany commit wprowadza już osobne rozstrzygnięcie `option_if_vals` i wymaga rzeczywistych `MatchArm`.
- Pierwotne kryteria issue #54 mówią o odrębności `Option<T>` i `T?`, a implementacja celowo je zrównała.
- `src/escape.rs` przy `HirExprKind::Match` nadal ma komentarz, że #53 nie jest obniżane i gate je odrzuca. Jest to fałszywe. Sama gałąź pomija analizę scrutinee i ciał ramion; przy pracach nad T01/T04 trzeba ustalić i przetestować jej rzeczywisty kontrakt, zamiast polegać na tym komentarzu.

**Jak naprawić.** Zapisać końcową semantykę `T? = Option<T>`, wyjątek `Ref<T>` i aktualną obsługę regionów w jednym spójnym opisie. W raporcie/PR zamykającym #54 jawnie wskazać odstępstwo od pierwotnego issue. Zaktualizować powiązane fragmenty dokumentacji i komentarzy. Nie przepisywać historycznych planów tak, jakby nigdy nie istniały; oznaczyć, które założenia zostały zastąpione.

Nie traktować zmiany samego komentarza w `escape.rs` jako naprawy analizy. Dalszy LLM ma sprawdzić, które decyzje o życiu wyników rzeczywiście wykonują escape i memory planner, zwłaszcza wewnątrz matcha. Z T01 wynika już, że całego kontraktu nie można uznać za bezpieczny tylko dlatego, że slot zewnętrzny ma sink.

**Kryterium zakończenia:** osoba wdrażająca #55–#59 może przeczytać aktualne language/RFC i uzyskać jedną odpowiedź na pytania o `Option`, pożyczanie payloadu, regiony ramion i wsparcie backendu. Testy parsera/typeck/ownership/codegen potwierdzają tę samą odpowiedź.

## Dwa dodatkowe, potwierdzone problemy wspólnego kodu

Poniższe przypadki nie uzasadniają dopisywania nowych możliwości do etapu enumów. Warto zapisać je osobno, żeby nie pomylić błędu wspólnego backendu z problemem przyszłego trait dispatch.

**Porównanie `f64` wyciągniętego z enuma:**

```bork
enum E { A(x: f64) }
fun main(): i32 {
    val e = E.A(1.5)
    when e {
        A(x) => {
            if (x > 1.0) { println(1) } else { println(0) }
        }
    }
    return 0
}
```

Frontend przechodzi, build zgłasza `integer operand is not supported by codegen yet`. W `src/codegen/llvm/binop.rs::combine_binary_values` wybór floatowej ścieżki zależy od typu **wyniku**. Dla porównania wynik jest `bool`, więc floatowe operandy trafiają do ścieżki integer. Naprawić wspólną selekcję na podstawie operandów. Oczekiwany wynik: `1`. To błąd porównań float, nie matchingu payloadu.

**Równość zwykłych enumów:**

```bork
enum E { A, B }
fun main(): i32 {
    if (E.A == E.B) { println(1) } else { println(2) }
    return 0
}
```

Frontend przechodzi, build ponownie dochodzi do `integer operand is not supported by codegen yet`. `binary_result_ty` uznaje zgodność typów za wystarczającą dla `==`, a gate przepuszcza ten operator bez sprawdzenia operandów. Jeśli równość enumów ma wymagać przyszłego traita, obecnym poprawnym zachowaniem jest czytelna diagnostyka typeck o braku obsługi `==` dla tego typu. Nie implementować automatycznie strukturalnego `Eq` jako części tego audytu. Zachować już działającą równość skalarów, tekstu, `Ref` i specjalne przypadki nullable.

## Czego nie trzeba dopisywać przed traitami

Nie znalazłem podstaw, by w ramach zamknięcia tego etapu żądać niche optimization, zoptymalizowanych drzew decyzyjnych, range patterns, vtable, trait objects ani operatora propagacji `?`. Sekwencyjne testowanie ramion jest oznaczonym i zgodnym z zakresem uproszczeniem. Istnieją już testy własności algorytmu exhaustiveness, więc nie wpisuję ich jako brakującego zadania.

Gate jawnie odrzuca m.in. move zwykłego enuma i część bezpośrednich wzorców nad `Ref`. Raport nie uznaje każdego jawnego ograniczenia backendu za nowy błąd. T01 jest inną sytuacją: program **przechodzi** gate i wykonuje nieprawidłowy odczyt.

## Warunek przejścia dalej

1. T01 ma testy wykonywalne, które zachowują prawidłowe wartości po ponownym użyciu aren i nie psują cleanup `Ref`.
2. T02 działa również dla składni przepisanej z pary `Some`–`None` oraz `if val` nad `Option`.
3. T03–T07 mają małe testy regresji w istniejących zestawach, z pozytywnymi i negatywnymi przypadkami właściwymi dla danego błędu.
4. Kryteria T08–T12 zostają domknięte przy zamykaniu #54, bez niejawnego zmieniania reguł języka.
5. Na końcowej wersji uruchomiono `cargo test --workspace --features codegen`; aktualizacja goldenów wynika z zatwierdzonych zmian zachowania, a nie służy ukryciu regresji.

Wtedy #55–#59 mogą rozszerzać działający system typów. Sam zielony wynik obecnych 714 testów nie jest wystarczający: T01 i T05 pokazują poprawnie kompilujące się programy o błędnym wykonaniu.

Nie sprawdzono: sanitizerów/Valgrinda, architektur innych niż lokalne x86-64 i interaktywnej sesji GDB/LLDB; debug ABI sprawdzono przez DWARF. Audyt nie dowodzi braku pozostałych błędów. Najważniejsze znane ryzyko to odczyt zwolnionej pamięci opisany w T01.
