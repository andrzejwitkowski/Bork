# Rozdział 1. Pierwszy program i narzędzia

## Ten rozdział obejmuje

- Co Bork jest, a czego w tej rewizji nie jest
- Program, który drukuje wiersz, i program, którego kodem wyjścia jest wynik
- Różnicę między `bork plik.bork` a `bork build`
- Jak czytać pierwszą diagnostykę
- Gdzie kończy się „język”, a zaczyna „podzbiór codegen”

## Jedno zdanie

Bork jest małym językiem natywnym. Składnia jest zbliżona do Kotlina: `fun`,
`val`, `var`, bloki w `{}`, `if` jako wyrażenie, trailing closure. Pamięcią
nie zarządza garbage collector. Bajty wartości, które nie mieszczą się w
rejestrze — dziś przede wszystkim `String` i tablice `[T; N]` — leżą w
arenach przypiętych do bloków. Kompilator, gdy program jest zły, wypisuje
diagnostykę i kończy się kodem 1. Stąd hasło z `README`: when your code is
invalid, the compiler borks.

Repozytorium jest kompilatorem napisanym w Ruście. Nie ma tu interpretera.
Ścieżka natywna idzie przez LLVM (biblioteka Inkwell, pin na LLVM 23) i małą
bibliotekę runtime `bork_runtime`, linkowaną statycznie do programu
użytkownika. Sprawdzenie programu — parse, typy, własność — nie potrzebuje
LLVM. Zbudowanie binarki potrzebuje.

## Dwa pierwsze programy

**Listing 1.1.** Drukuje `hi` i nową linię. Uruchomione: kod wyjścia 0, stdout `hi\n`.

```bork
fun main() {
    println("hi")
}
```

`fun main` bez `: Typ` zwraca `unit`. Taki `main` kompiluje się do funkcji
`main` w C ABI, która zwraca `i32` równe 0. `println` jest wbudowany. Nie
jest funkcją, którą wolno zdefiniować ponownie.

**Listing 1.2.** Wynik funkcji jest kodem wyjścia procesu. Uruchomione: kod 42.

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main(): i32 {
    return add(40, 2)
}
```

(1) Parametr bez `val`/`var` jest `val`.
(2) `main` z wynikiem `i32` przekazuje tę liczbę jako kod wyjścia. Innych
typów wyniku `main` codegen nie obniża: `i64`, `f64` i `bool` dostają
diagnostykę `main returning … is not supported by codegen yet`.

> **NOTE.** Nie ma instrukcji na poziomie pliku. Program to lista funkcji.
> `main` jest zwykłą funkcją o zastrzeżonej nazwie tylko na etapie `bork build`.
> Samo sprawdzenie pliku bez `main` przechodzi, jeśli reszta jest poprawna.

## Sprawdzić i zbudować

Z katalogu repozytorium, bez LLVM, wystarczy checker:

```text
cargo run --bin bork -- path/to/file.bork
cargo run --bin bork -- --dump-arenas path/to/file.bork
```

Pierwsza komenda uruchamia `frontend::check`: parse, typeck, analizę
własności, a przy czystym programie także hoist i escape. Druga dopisuje na
stdout drzewo aren. Diagnostyki idą na stderr. Kod wyjścia 0 oznacza brak
diagnostyk, 1 oznacza błąd programu, 2 oznacza złą linię poleceń albo brak
pliku.

Zbudowanie binarki wymaga feature `codegen`, LLVM 23, `clang` i bibliotek,
które ciągnięte są z `libLLVM` (`libz3`, `libedit`, `libxml2`, `libzstd`,
`libffi`). Na Ubuntu 24.04 pakietu LLVM 23 nie ma w domyślnym archiwum;
`README` wskazuje apt.llvm.org. Po złożeniu:

```text
cargo run --features codegen -- build file.bork
cargo run --features codegen -- build -o out file.bork
```

Domyślna nazwa wyjścia to ścieżka wejścia bez rozszerzenia. Kompilator
odmawia, gdy wyjście nadpisałoby plik źródłowy (`error: output path would
overwrite input file`, kod 2). Bez feature `codegen` podkomenda `build`
kończy się komunikatem, że `bork build` wymaga złożenia z tą flagą, też kodem 2.

**Listing 1.3.** Diagnostyka braku `main`. Uruchomione, `bork build`, kod 1, binarka nie powstaje.

```text
nomain.bork: error: codegen: `fun main` is required to build an executable
```

Źródło to jedna funkcja `helper`. Faza to `codegen`, nie `type`. Frontend
uważa plik za poprawny.

## Pierwszy błąd, który zobaczysz często

**Listing 1.4.** Dwie instrukcje w jednej linii. Uruchomione.

```bork
fun main() { val a = 1 val b = 2 }
```

```text
err_two_stmt.bork:1:24: error: parse: unexpected token `val`; expected "!!", "!=", ... "}", "NL"
```

Instrukcje rozdziela się znakiem nowej linii, nie średnikiem. Średnik jest
nieoczekiwanym tokenem. Lista `expected` jest surowa: LALRPOP wypisuje
terminale, których mógłby użyć w tym stanie parsera. Nie jest to przyjazny
tekst i książka nie będzie udawać, że jest.

**Listing 1.5.** Przypisanie do `val`. Uruchomione.

```bork
fun main() {
    val n = 1
    n = 2
}
```

```text
err_val.bork:3:5: error: type: cannot assign to immutable `val` binding `n`
```

Faza `type` przychodzi z typecku. Faza `ownership` przychodzi z semy i z
analizy escape. Obie mogą pojawić się w jednym uruchomieniu. Kolejność w
wyjściu jest taka, że komunikaty własności idą pierwsze, nawet gdy typeck
policzył się wcześniej. Rozdział 12 tłumaczy, dlaczego pipeline jest w tej
kolejności.

## Co już wolno, a czego `build` nie obniży

Następujący program przechodzi `bork plik.bork` (sprawdzone, kod 0) i jest
próbką z `src/lib.rs` (`MVP_SAMPLE`). `bork build` kończy się kodem 1 i
komunikatem zawierającym `codegen` oraz `not supported`. Test
`build_rejects_mvp_sample_at_codegen_gate` pilnuje dokładnie tego.

**Listing 1.6.** Trailing closure. Tylko check. Codegen odrzuca.

```bork
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main(): i32 {
    val threshold = 5
    var accumulator = 0
    for (i in 0..10) {
        accumulator = action(accumulator, i) { acc, current ->
            if (current > threshold) {
                acc + current
            } else {
                acc
            }
        }
    }
    return accumulator
}
```

To jest język, nie martwa gramatyka. Typeck sprawdza arność domknięcia i typ
wyniku. Sema buduje węzeł `Closure` w drzewie aren. Bramka w
`src/codegen/gate.rs` odrzuca `has_trailing_closure` tekstem `trailing
closures are not supported by codegen`.

> **WARNING.** Część programów, które check puszcza, nie dostaje grzecznej
> diagnostyki na `bork build`, tylko panic procesu kompilatora (kod 101).
> Najważniejszy przypadek: argument `String` do funkcji użytkownika.
> `println` i `concat` działają. Rozdział 17 pokazuje miejsce w
> `coerce_value_to_ty`.

## Drzewo aren, zanim zrozumiesz regiony

Dla listingu 1.2 `--dump-arenas` wypisuje:

```text
Arenas
├── fun add
│   ├── a [Local]
│   └── b [Local]
└── fun main
```

`Local` znaczy: nazwa powstała w tym regionie. `main` nie ma lokalnych nazw,
bo jedyną instrukcją jest `return`. W rozdziale 8 zobaczysz `Copy`,
`Shared ← …` i `Moved ← …`. Dump jest tym samym tekstem, który komenda
edytora **Bork: Dump Arenas** wkłada do kanału wyjścia.

## Podsumowanie

- Program Bork to lista funkcji. Instrukcje rozdziela nowa linia.
- `bork plik.bork` sprawdza. `bork build` obniża podzbiór do binarki przez LLVM.
- `main(): i32` zwraca kod wyjścia. `main()` bez typu kończy się zerem.
- Diagnostyka ma fazę: `parse`, `ownership`, `type` albo `codegen`.
- Trailing closures, nullable i `?:` są w języku frontendu. Codegen ich nie obniża.
- Przekazanie `String` do funkcji użytkownika jest dziś dziurą codegenu: panic, nie komunikat.
