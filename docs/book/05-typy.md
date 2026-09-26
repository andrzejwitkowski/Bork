# Rozdział 4. Typy

## Ten rozdział obejmuje

- Typy prymitywne i aliasy `Int`, `Long`, `Byte`, `Float`, `Double`
- Które typy są Copy
- Literały i wnioskowanie
- `String`, funkcje, nullowalność
- Błędy, które typeck wypisuje, gdy typy się nie zgadzają

## Tabela powierzchni

| Piszesz | Po obniżeniu |
|---|---|
| `i8` `i16` `i32` `i64` | ten sam typ całkowity ze znakiem |
| `u8` `u16` `u32` `u64` | ten sam typ całkowity bez znaku |
| `Int` | `i32` |
| `Long` | `i64` |
| `Byte` | `u8` |
| `Float` | `f32` |
| `Double` | `f64` |
| `f32` `f64` | floaty |
| `bool` | `bool`, literały `true` i `false` |
| `unit` | brak wartości użytecznej; pominięty typ wyniku funkcji |
| `String` | jedyny dozwolony typ nazwany |
| `[T; N]` | tablica o długości stałej |
| `T?` | wariant nullowalny |
| `(A, B) -> R` | typ funkcji |

Alias jest rozwiązywany w `Type::from_ident` w parserze, zanim typeck
zobaczy drzewo. W HIR nie ma osobnego konstruktora `Int`. Jest `Prim::I32`.

**Listing 4.1.** Aliasy. Uruchomione: `bork build`, kod wyjścia 1 (zwracane jest `n`).

```bork
fun main(): i32 {
    val n: Int = 1
    val wide: Long = 2
    val byte: Byte = 3
    val f: Float = 1.5
    val d: Double = 2.25
    val flag: bool = true
    return n
}
```

Sam zapis tych lokalnych wartości codegen obniża. Użycie floatu w
arytmetyce albo porównaniu jest osobną, niedokończoną ścieżką (rozdział 17).
Ten listing ich nie używa, dlatego binarka powstaje.

Każda inna nazwa typu, na przykład `Point` albo `Unit` z dużej litery, daje
`unknown named type`. Słowo `unit` małymi literami jest prymitywem. Wielka
litera `Unit` nie jest aliasem.

## Copy

`Ty::is_copy` jest prawdziwe tylko dla prymitywu, który nie jest
nullowalny. Spoza Copy wypadają:

- `String`,
- każda tablica `[T; N]`,
- każdy typ funkcji,
- cokolwiek z `?`, także `i32?`.

To jest ta sama odpowiedź w HIR i w semie (`sema` mapuje typ HIR z powrotem
na `ast::Type` albo zostawia `Unknown`, a `Unknown` **nie** jest Copy).
Dlatego błąd typu, który zostawia typ nieznany, potrafi pociągnąć dodatkowy
błąd własności. Nie ignoruj drugiej diagnostyki tylko dlatego, że „to już
był błąd typu”.

## Literały

Literał całkowity jest w AST liczbą `i64`, a typ przyjmuje z kontekstu, jeśli
kontekst jest całkowity. Bez kontekstu staje się `i32`.

**Listing 4.2.** Literał dopasowany do `i64`. Uruchomione checker. `bork build` odrzuca `main` zwracające `i64`.

```bork
fun main(): i64 {
    val n: i64 = 1
    return n
}
```

```text
int_widen.bork: error: codegen: `main` returning `i64` is not supported by codegen yet
```

Checker jest zadowolony. Bramka wyniku `main` nie.

Literał `1.5` bez adnotacji staje się `f64`. Z adnotacją `Float` albo `f32`
zostaje `f32`. Nie ma sufiksów `1.5f32`.

Literał napisowy używa cudzysłowów i escape'ów `\n`, `\r`, `\t`, `\\`, `\"`.
**Listing 4.3.** Uruchomione, stdout to znak `a`, nowa linia, `b`, tabulacja, `"c"`, nowa linia.

```bork
fun main() {
    println("a\nb\t\"c\"")
}
```

`+` nie skleja napisów.

```text
str_plus.bork: error: type: arithmetic operands must have the same numeric type, got String and String
```

Do sklejania jest `concat` (rozdział 9 i 11).

## Arytmetyka i porównania

Operatory `+ - * /` wymagają dwóch operandów tego samego typu liczbowego.
Nie ma promocji `i32` do `i64`.

**Listing 4.4.** Uruchomione checker. Dwa błędy na tym samym miejscu.

```bork
fun main(): i32 {
    val a: i64 = 1
    val b: i32 = 2
    return a + b
}
```

```text
err_div_types.bork:4:12: error: type: arithmetic operands must have the same numeric type, got i64 and i32
err_div_types.bork:4:12: error: type: return value has type i64, expected i32
```

Drugi komunikat jest skutkiem ubocznym: przy niezgodnych operandach typeck
zostawia typ lewej strony jako typ wyrażenia, a ten nie pasuje do `i32`.

Porównania `> < >= <=` wymagają tego samego nienullowalnego typu
liczbowego. `bool` odpada:

```text
cmp_bool.bork: error: type: ordered comparison operands must have the same non-nullable numeric type, got bool and bool
```

`==` i `!=` wymagają identycznego typu, ale typ może być nullowalny.
`val n: i32? = None` oraz `n == None` przechodzi checker. Codegen odrzuca
`None`.

`&&`, `||` i `!` działają na `bool`. Koniunkcja i alternatywa są
zwierające. To widać w codegenie: prawa strona `false && side()` nie jest
wywoływana. Test `builds_logical_short_circuit` i listing 6.4 to
uruchamiają.

Dzielenie całkowite przez zero oraz, w kodzie backendu, dzielenie minimum
typu ze znakiem przez `-1` woła `abort`. Uruchomione: `return 1 / 0` buduje
się, a proces kończy sygnałem SIGABRT (kod powłoki 134). Komunikatu Bork na
stderr nie ma. To `abort` z libc, nie panic Rusta z treścią.

> **NOTE.** Floaty w typecku wolno dodawać i porównywać. W codegenie
> dodawanie `f32` daje diagnostykę `internal arena schedule mismatch: float
> binary after walk is not supported by codegen yet`. Porównanie `f64` na
> ścieżce, którą trafiliśmy, panikuje w `into_int_value`, bo wartość LLVM
> jest `double`, a `coerce` oczekuje inta. Traktuj arytmetykę float jako
> sprawdzaną, nie jako obniżaną.

## Nullowalne

`T?` istnieje dla prymitywów, dla `String` i dla typu funkcji. Tablica
nullowalna parsuje się i jest odrzucana: `nullable array types [T]? are not
supported`.

Konstruktory to `Some(x)` i `None`. `None` bez oczekiwanego typu nie
przechodzi:

```text
err_none.bork:2:13: error: type: cannot infer type of `None`
```

`?:` wymaga lewej strony nullowalnej. Prawa strona musi pasować do typu po
zdjęciu `?`.

```text
elvis_bad.bork:3:13: error: type: left operand of `?:` must be nullable, got i32
```

`!!` wymaga operandu nullowalnego i daje typ bez `?`. Na `i32` dostaniesz
`operand of !! must be nullable, got i32`.

**Listing 4.5.** Nullowalny `String` i `i32`. Tylko check. Codegen odrzuca `?:`, `Some` i `None`.

```bork
fun processUser(name: String?, score: i32): i32 {
    val fallbackName: String = name ?: "Guest"
    val verifiedUser: String? = Some(fallbackName)
    val emptyMiddle: String? = None
    val verifiedLength: i32 = verifiedUser?.length ?: 0
    if (verifiedLength > 0) {
        return score
    }
    return 0
}
```

`?.` jest bezpiecznym dostępem do pola. Na typie nienullowalnym typeck każe
użyć `.`. Jedyne znane pole `String` i `[T; N]` to `length`, typu `i32`.
Dla tablicy `length` jest zawsze `N` z typu, nie osobnym licznikiem.

```text
unknown_field.bork:3:13: error: type: unknown field `foo` on type String
```

> **WARNING.** `bork build` na listingu 4.5 wypisuje osobny błąd codegen dla
> każdego `?:`, `Some` i `None`. To nie jest „częściowy runtime nullable”.
> Bramka odcina całe konstrukcje, zanim LLVM je zobaczy.

Typ funkcji też bywa nullowalny: `val f: ((i32) -> i32)? = None` przechodzi
checker. Wywołanie takiego `f` bez `!!` nie jest tym, co codegen umie, i
nawet po `!!` bramka odrzuca asercję.

## Typ funkcji

`(A, B) -> R` zapisuje się w nawiasach. Nullable funkcja to
`((A, B) -> R)?`. Gramatyka odrzuca nullowanie nawiasów, które nie są typem
funkcji: `nullable parentheses require one function type` oraz `only
function types may be parenthesized for nullability`.

Nie ma krotek jako wartości. Nawias przy typie jest albo grupowaniem typu
funkcji, albo — w wyrażeniu — zwykłym nawiasem arytmetycznym. Nie ma
`(1, 2)` jako pary.

## Nieznane i trucizna

Gdy wyrażenie jest już błędne, typeck podstawia `TyKind::Unknown`. Kolejne
niezgodności z `Unknown` są tłumione, żeby jeden błąd nie produkował kaskady.
To nie dotyczy semy: typ, którego nie da się zmapować, jest tam
nie-Copy. Własność nadal może krzyczeć.

## Podsumowanie

- Aliasy `Int`/`Long`/`Byte`/`Float`/`Double` znikają już w AST.
- Copy to nienullowalny prymityw. `String`, tablice, funkcje i `T?` nie są Copy.
- Nie ma promocji liczbowych ani `+` dla napisów.
- `None` potrzebuje oczekiwanego typu. `?:` i `!!` wymagają `?`.
- `length` jest jedynym polem `String` i tablicy.
- Nullable i floaty są w typecku. Codegen nullable odcina bramką, a floaty
  albo diagnostyką „not supported yet”, albo panicą.
