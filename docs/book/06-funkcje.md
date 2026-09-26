# Rozdział 5. Funkcje, wywołania i domknięcia

## Ten rozdział obejmuje

- Deklarację `fun`, parametry `val`/`var`, typ wyniku
- Wywołania, trailing closure i trzy formy `move` przy domknięciu
- Wbudowane `print`, `println`, `concat` — i zakaz ich przesłaniania
- Co codegen naprawdę umie wywołać
- Panicę przy przekazaniu `String` do funkcji użytkownika

## Kształt funkcji

```text
fun nazwa(parametry): TypWyniku { ciało }
```

Pominięty typ wyniku to `unit`, nie nazwa `Unit`. Parametr to
`nazwa: Typ`, `val nazwa: Typ` albo `var nazwa: Typ`. Goła forma jest
`val`.

**Listing 5.1.** Oba rodzaje parametrów. Uruchomione checker; HIR ma `Val` i `Var`, wynik `i32`.

```bork
fun add(val a: Int, var b: Int): Int {
    return a
}
```

`var` na parametrze znaczy: wołający, który przekazuje wiązanie nie-Copy,
musi je oddać (`move`). Dla `i32` różnica nie boli, bo `i32` jest Copy.
Dla `String` boli i jest sprawdzana.

Ciało jest blokiem. Ostatnie wyrażenie **nie** jest niejawnym wynikiem
funkcji. Wynik idzie przez `return`. Typeck wymaga, żeby każda ścieżka
funkcji o wyniku innym niż `unit` wykonała `return`.

**Listing 5.2.** Uruchomione.

```text
missing_ret.bork: error: type: function must return a value of type i32 on all paths
```

dla funkcji, która ma tylko `val n = 1`.

`return` bez wartości ma typ `unit`. W funkcji `i32` dostaniesz
`empty return has type unit, expected i32`. `return` z wartością sprawdza
zgodność z zadeklarowanym wynikiem.

Nie ma rekurencji wzajemnej opisanej osobno: funkcje są w jednej mapie
`fun_sigs` po nazwie, więc wołanie wstecz działa o tyle, o ile typeck
widzi sygnaturę zebraną z całego programu przed ciałami. Nie ma przeciążeń.
Dwie funkcje o tej samej nazwie nie są modelem, który książka może obiecać;
mapa jest `HashMap` po gołej nazwie i komentarz w `sema/env.rs` mówi wprost:
„MVP: keyed by bare function name (no local shadowing of callees yet)”.

## Własność na granicy wywołania

Macierz, którą sema egzekwuje dla typu nie-Copy:

| Źródło | Parametr `val` | Parametr `var` |
|---|---|---|
| `val` nazwa | użycie w miejscu (Shared, jeśli region na to pozwala) | trzeba `move` |
| `var` nazwa | trzeba `move` | trzeba `move` |
| literał, `concat(...)`, inne świeże wyrażenie | bez `move` | bez `move` |

**Listing 5.3.** Świeży literał do `var`. Checker: tak. `bork build`: panic kompilatora w `value_as_int`.

```bork
fun f(var a: String) {
    println(a)
}

fun main() {
    f("hello")
}
```

To jest zgodne z regułą frontendu (świeże wyrażenie nie wymaga `move`) i
niezgodne z tym, co backend umie obniżyć. Panic, sprawdzony:

```text
thread 'main' panicked at src/codegen/llvm/expr.rs:901:14:
Found StructValue(...) but expected the IntValue variant
```

Ta sama panica jest na `f(move s)` i na `show(s)` gdy `s` jest `val String`.
`println(s)` i `concat` działają, bo są intrinsicami, nie wywołaniami
`bork.f`. Dopóki ta dziura istnieje, funkcja użytkownika w programie
`bork build` powinna przyjmować i zwracać prymitywy. Napisy zostaw w
`main` i przekazuj je do `print` / `println` / `concat`.

Zwracanie napisu, który już żyje na głębokości funkcji, checker i codegen
akceptują.

**Listing 5.4.** Uruchomione: stdout `hi\n`.

```bork
fun shout(): String {
    var s = "hi"
    return s
}

fun main() {
    println(shout())
}
```

`return "hi"` też działa (stdout `hi\n`). `return name` dla parametru
przechodzi checker (`fun greet(name: String): String { return name }`).
Wywołanie `println(greet("Ada"))` znowu panikuje na argumencie `String` do
`greet`, nie na `return`.

`return concat("a", "b")` checker odrzuca bez numeru linii, bo diagnostyka
escape nie ma spanu:

```text
err_ret_concat.bork: error: ownership: returning the result of `concat` is not supported yet: returned `String` bytes must outlive the callee arena
```

`return move s` jest w tej samej rodzinie: `returning a moved or promoted
String is not supported yet: use return s for parameters and locals, or
return a string literal`.

## Wbudowane

Trzy nazwy są intrinsicami (`src/builtins.rs`). Ponowna deklaracja jest
błędem typu, zanim ciało w ogóle ma znaczenie.

**Listing 5.5.** Uruchomione.

```text
err_builtin.bork:1:13: error: type: cannot redefine builtin function `println`
```

| Nazwa | Efekt | Sygnatura nominalna |
|---|---|---|
| `print` | pisze na stdout i robi flush | w tabeli `(i32) -> unit`, ale przyjmuje też `i64` i `String` |
| `println` | to samo plus `\n` | j.w. |
| `concat` | jeden nowy `String` | `(String, String) -> String` |

**Listing 5.6.** Kolejność i flush. Uruchomione, stdout dokładnie `1\ntail7` (bez końcowej nowej linii).

```bork
fun main() {
    println(1)
    print("tail")
    print(7)
}
```

`print` i `println` biorą jeden argument. `concat` bierze dwa. Wynik
`concat` ląduje w arenie aktualnego sinku, a gdy sinku nie ma — w arenie
bieżącego bloku. Rozdział 9 rozbiera to na reguły.

## Trailing closure

Ostatni argument może być blokiem po liście nawiasów. Parametry domknięcia
stoją przed `->`.

**Listing 5.7.** Tylko check. Ostatni parametr `action` ma typ
`(Int, Int) -> Int`.

```bork
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main(): i32 {
    val threshold = 5
    return action(1, 2) { acc, current ->
        if (current > threshold) { acc + current } else { acc }
    }
}
```

Błędy typeck, które tu pilnują kontraktu:

- `function … requires a function type as its last parameter when called with a trailing closure`,
- `trailing closure expects N parameters, got M`,
- `trailing closure body has type …, expected …`,
- `only named functions can be called` — nie wywołasz wartości typu funkcji, nawet jeśli typ jest w HIR; callee musi być nazwą funkcji albo, w typecku, wiązaniem typu funkcyjnego, ale codegen i tak odrzuca wywołanie pośrednie tekstem `indirect calls`.

Sema dla parametrów domknięcia wpisuje `Ty::Unknown`. Nie dostaje typów z
sygnatury. Skutek: wewnątrz domknięcia polityka Copy/Move dla tych parametrów
jest zachowawcza. Parametry z listingu 5.7 są w dumpie `Local`, a odczyt
`acc` w gałęzi `if` jest `Shared ← Closure`, nie `Copy`, mimo że `Int` jest
Copy. To rozjazd semy z typeckiem, widoczny w drzewie aren, a nie błąd
użytkownika. Program check przechodzi.

Domknięcie nie jest wartością pierwszej kategorii, którą można zapisać w
`val f = { ... }` poza wywołaniem. Gramatyka ma closure tylko jako trailing
albo jako blok `move`. Typ funkcyjny istnieje, ale nie ma literału funkcji
poza tym miejscem.

## Trzy formy domknięcia z `move`

| Składnia | Co przenosi |
|---|---|
| `f(...) { ... }` | nic; odczyt zewnętrznego `var String` jest błędem |
| `f(...) move () { ... }` | jawnie nic |
| `f(...) move (a, b) { ... }` | dokładnie wymienione nazwy |
| `f(...) move { ... }` | wnioskuje: każde żywe, nie-Copy imię z rodzica, którego ciało używa |

To samo da się napisać jako instrukcję, bez wywołania: `move (a) { ... }`,
`move () { ... }`, `move { ... }`. Rozdział 8 ma uruchomione binarki dla
form instrukcji. Formy trailing są w języku frontendu i odpadają na bramce:

```text
trail_move.bork:6:12: error: codegen: trailing closures are not supported by codegen
```

Wnioskowanie nie patrzy na `move nazwa` i `promote nazwa` jako na
swobodne zmienne (`free_vars` je pomija). Inaczej wyrażeniowy `move` byłby
liczony drugi raz jako przechwycenie regionu.

## Wywołanie a region

Ciało wołanej funkcji ma własne regiony i własne `bork_arena_push` na czas
aktywacji. Argumenty są liczone w regionie wołającego. Wynik, jeśli jest
prymitywem, wraca w rejestrze. Wynik `String` nie dostaje dziś bufora
własności wołającego, dlatego escape zabrania `return` świeżych bajtów.
Listing 5.4 działa, bo `"hi"` jest literałem w stałych albo lokalną wartością
na głębokości 0, a `println` tylko czyta deskryptor.

> **TIP.** W programie, który ma przejść `bork build`, trzymaj funkcje przy
> liczbach. Napisy drukuj z `main`. To nie jest styl na zawsze. To jest
> obwód dziury w `coerce_value_to_ty`, która każdą wartość nie-bool i
> nie-float przepycha przez `into_int_value`.

## Podsumowanie

- Wynik funkcji jest jawny (`return`). Brak ścieżki to błąd typu.
- Goły parametr jest `val`. `var` wymaga `move` przy przekazaniu wiązania nie-Copy.
- Świeży literał nie wymaga `move`, ale `String` jako argument funkcji użytkownika wywraca codegen.
- `print`, `println` i `concat` są wbudowane i nie wolno ich zadeklarować.
- Trailing closure jest w typecku i semie. Bramka codegen ją odcina.
- Sema nie typuje parametrów domknięcia; w dumpie potrafią wyglądać jak nie-Copy.
