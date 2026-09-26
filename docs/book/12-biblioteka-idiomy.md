# Rozdział 11. Biblioteka, brak modułów i idiomy

## Ten rozdział obejmuje

- Dlaczego nie ma rozdziału o module w sensie językowym
- Całą bibliotekę standardową, która naprawdę istnieje
- Idiomy, które check i `bork build` udźwigną
- Idiomy tylko frontendu
- Krótki katalog „tak się tego nie pisze”

## Modułów nie ma

Parser na poziomie programu oczekuje `NL` albo `fun`. Token `module` i
token `struct` są błędami `expected "NL", "fun"`. Nie ma `import`, ścieżek,
widoczności `pub` ani crate'ów użytkownika. Jeden plik, jedna lista
funkcji, jedna mapa nazw.

Słowo „moduł” w tej książce dalej występuje, ale o crate'ach Rusta:
`bork` i `bork_runtime`. To nie jest mechanizm języka Bork.

LSP i rozszerzenie edytora (`tools/bork-lsp-extension`) są narzędziami
obok języka. Nie dodają składni.

## Cała biblioteka

Trzy intrinsics. Nic więcej nie jest zarejestrowane w `builtins::resolve`.

| Piszesz | Dostajesz |
|---|---|
| `print(x)` | `x` na stdout, flush, `x` to `i32`, `i64` albo `String` |
| `println(x)` | to samo i `\n` |
| `concat(a, b)` | nowy `String` w sinku albo w bieżącej arenie |

Nie ma `assert`, `format`, wejścia z stdin, plików, alokacji poza areną,
zegara ani argumentów wiersza poleceń. `main` nie dostaje `argv`. Codegen
odrzuca `main` z parametrami przy deklaracji funkcji LLVM (`emit_fn`).

Runtime C ABI, którego nie wołasz z Borka wprost, to `bork_arena_push`,
`bork_arena_pop`, `bork_arena_reset`, `bork_arena_alloc`, `bork_print_i64`,
`bork_println_i64`, `bork_print_str`, `bork_println_str`. Rozdział 17.

## Idiomy, które warto kopiować

**Wynik liczbowy jako kod wyjścia**, gdy ćwiczysz codegen bez drukowania.

```bork
fun main(): i32 {
    return 7
}
```

Uruchomione: kod 7. Tak robi test `builds_return_constant`.

**Pętla i akumulator Copy.** Listing 6.3. Indeks i akumulator są `i32`,
więc ciało nie walczy z własnością.

**Napis w `main`, druk na końcu.** Listing 9.1. Nie zwracaj świeżego napisu
z pomocnika, dopóki nie ma areny wyniku.

**`val` dla napisu, który dziecko tylko czyta.** Listing 2.2. Shared jest
tańsze pojęciowo niż `move` wte i wewte.

**Dwie linie, gdy hoist ma pomóc.** `var piece = concat(...)` i od razu
`outer = move piece`. Nie wstawiaj między nie `println`.

**Tablica i wycinek o stałych granicach**, gdy długość jest znana.
Listing 7.4. Nie używaj wycinka jako „vec slice” z granicami z zmiennych:
typeck tego nie przyjmie.

**`if` z dwoma blokami**, gdy potrzebujesz wartości. Listing 6.1.

## Idiomy tylko frontendu

Pisz je, gdy ćwiczysz checker albo LSP. Nie wkładaj ich do programu, który
ma być binarką.

- Trailing closure i `move { }` po wywołaniu.
- `Some`, `None`, `?:`, `!!`, `?.`.
- Wartość typu funkcji w `val` i próba wywołania pośredniego.
- `String` jako parametr funkcji użytkownika. To nawet nie jest „tylko
  frontend”: frontend jest za, codegen panikuje. Trzymaj się z daleka od
  tej kombinacji w `bork build`.

**Listing 11.1.** Fragment próbki `PROCESS_USER_SAMPLE` z `src/lib.rs`. Cała próbka przechodzi check (uruchomione). `bork build` odrzuca `?:`, `Some` i `None`.

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

## Tak się tego nie pisze

| Pokusa | Co się stanie |
|---|---|
| `val s = "a" + "b"` | błąd typu, `+` nie jest konkatenacją |
| `return -1` | błąd parsowania, minus nie jest jednoargumentowy |
| `val n = 1;` | błąd parsowania, średnik |
| `var x = s` dla `var s: String` | `use move s to transfer ownership` |
| `concat(left, right)` gdy oba są `var` w zagnieżdżonym bloku | `not Copy; move it into Block` |
| `return concat(a, b)` | escape, „not supported yet” |
| `a[i]` z indeksem spoza `N` | kompiluje się, proces aboruje |
| `1 / 0` | kompiluje się, proces aboruje |
| drugie `fun println` | `cannot redefine builtin` |
| `struct` albo `module` | błąd parsowania na poziomie programu |

## Styl, który pasuje do regionów

Krótki blok robi jedną pulę tymczasową. Długi blok funkcji trzyma dane,
które mają przeżyć pętlę. Pętla nie powinna przenosić stanu z zewnątrz;
powinna czytać Copy albo Shared `val` i zapisywać wynik do zewnętrznego
`var` przez przypisanie.

Nazwy `val` dla progów i napisów tylko do odczytu, `var` dla akumulatorów.
To nie jest konwencja formatowania. To jest różnica, którą sema sprawdza.

Nie ma rustowego `clone()`. Świadoma kopia napisu to `concat(s, "")` tylko
wtedy, gdy oba argumenty są legalne w tym regionie, albo przypisanie
literału. Nie ma ogólnego głębokiego kopiowania tablicy poza zbudowaniem
nowego literału. Wycinek nie jest kopią.

## Podsumowanie

- Biblioteka standardowa to `print`, `println` i `concat`.
- Modułów, struktur i `import` nie ma. Jeden plik.
- Idiom binarki: liczby w funkcjach, napisy w `main`, `val` do Shared, `var` do akumulatora.
- Nullable i trailing closures są prawdziwym językiem frontendu i nie są językiem `bork build`.
- `+` na napisach, średnik, minus jednoargumentowy i gołe użycie `var String` są błędami, nie lukami stylu.
