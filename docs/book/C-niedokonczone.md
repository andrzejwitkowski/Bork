# Dodatek C. Niedokończone, rozjazdy i paniki kompilatora

Ten dodatek jest listą miejsc, w których kod jest węższy niż język
frontendu, dokumentacja jest węższa albo szersza niż kod, albo kompilator
zachowuje się w sposób, którego nie da się nazwać diagnostyką. Nic z tej
listy nie jest propozycją naprawy. Książka nie zmienia kompilatora.

## Świadomie niezaimplementowane

Z `escape.rs`, `docs/language.md` i `docs/memory-model.md`:

- arena wyniku po stronie wołającego, więc `return move`, `return concat`
  i `return` bajtów z wewnętrznego regionu są odrzucane,
- `promote` na `return`,
- hoist poza wzorcem dwóch sąsiednich linii,
- elizja `memcpy`, gdy bajty już leżą w puli celu,
- osobna arena tymczasowa na czas zwykłego wołania,
- codegen trailing closures, `Some`, `None`, `!!`, `?:`,
- codegen pól innych niż `.length`,
- `main` z parametrami oraz `main` zwracające typ inny niż `i32` i `unit`,
- wywołanie pośrednie,
- moduły, struktury, generyki, wyjątki, borrow checker, GC.

Z `TODO.md` (refactor, nie semantyka języka): rozcięcie visitora codegenu,
kursor aren zamiast makra, test zgodności bramki z frontendem, więcej
złotych testów push/pop, uporządkowanie `docs/superpowers/plans/`.

## Rozjazdy dokumentu z kodem

| Dokument | Mówi | Kod robi |
|---|---|---|
| `docs/language.md`, przykład `concat(left, right)` przy `var` | program jest wzorcem | checker: `` `left` is not Copy; move it into `Block` `` |
| `docs/language.md`, pierwszeństwo | pomija `&&` `\|\|` `!` | gramatyka ma te piętra |
| `docs/superpowers/specs/2026-09-23-typed-hir-llvm-design.md` | LLVM 18, kolejność „własność potem typy” | LLVM 23, typeck potem sema |
| specyfikacja LSP | diagnostyki parsera | `frontend::check` |
| specyfikacja MVP aren | brak pełnego typecku | typeck jest |
| `README`, nagłówek „Parse diagnostics” | parse | akapit niżej i kod: pełny check |
| specyfikacja parsera | wynik `Unit`, brak spanów | prymityw `unit`, spany są |
| `language.md` o `MIN / -1` | runtime abort | strażnik w `guard_int_div` jest; literału ujemnego nie da się zapisać |

## Paniki i ostre krawędzie, sprawdzone

Uruchomione `bork build` (LLVM 23.1.2, rustc 1.98.1):

1. **Argument `String` do funkcji użytkownika** (`f("hello")`, `f(move s)`,
   `show(s)` dla `val s`) panikuje w `src/codegen/llvm/expr.rs` w
   `value_as_int` (`into_int_value`). Frontend jest czysty. Bramka milczy.
   Kod procesu kompilatora 101.

2. **Porównanie `f64`** (`if (x > 1.0)` przy `x: f64`) panikuje w tym
   samym miejscu, bo wartość jest `FloatValue`. **Dodawanie `f32`** daje
   diagnostykę, nie panikę: `internal arena schedule mismatch: float
   binary after walk is not supported by codegen yet`. Dwa operatory
   float, dwa różne złe zakończenia.

3. **Indeks poza zakresem** i **dzielenie przez zero** kompilują się.
   Proces użytkownika kończy się SIGABRT, kod 134, bez tekstu Bork.

4. **Przepełnienie areny** nie było odpalane dużym literałem w książce
   (trudno przekroczyć 4096 krótkim przykładem bez pętli alokującej, a
   pętla resetuje płytę). Kod w `bork_runtime` panikuje tekstem `arena
   overflow: need {end} bytes, capacity 4096`. Test jednostkowy runtime
   to zamyka. To nie jest diagnostyka kompilacji.

5. **`return concat` i część błędów escape** nie mają spanu. CLI pomija
   linię i kolumnę.

6. **`ParseError::User`** (zły cel przypisania, za duża liczba) wskazuje
   koniec pliku.

7. **Sema parametrów trailing closure** używa `Ty::Unknown`. Dump pokazuje
   `Shared` dla parametrów `Int`, które typeck uważa za Copy. Check
   przechodzi. To mylące przy czytaniu `--dump-arenas`, nie jest błędem
   użytkownika.

8. **Komunikat escape o gałęzi `if`** mówi `String`, a warunek obejmuje
   każdy `uses_arena_storage` (także tablicę).

9. **`u32` / alias `Byte` w `main(): i32`** nie jest ograniczeniem codegenu
   szerokich intów. `i64` w funkcji pomocniczej i porównanie działają
   (kod wyjścia 7 na `sub`). Ograniczenie `main` dotyczy typu **wyniku**
   `main`, nie lokalnego `i64`.

10. **`src/arena.rs` kontra `bork_runtime`.** Dwie stałe 4096. Sema nie
    woła żadnej. Łatwo „naprawić” zły plik.

11. **Opakowanie `internal arena schedule mismatch`.** Potrafi ukryć
    zwykłe „not supported yet” w tekście, który brzmi jak bug wewnętrzny.
    Czasem jest bugiem wewnętrznym (rozjazd dzieci areny), czasem jest
    tylko prefiksem.

12. **Specyfikacje w `docs/superpowers`** opisują świat przed tablicami,
    przed `while` i przed LLVM 23. `TODO.md` każe je przenieść, gdy
    wchłoną się w ten plik albo w zamknięte PR. Nie są mapą kodu.

## Co jest stabilne

Żeby lista dziur nie przesłoniła reszty: parser, typeck liczb i tablic,
sema Copy/Shared/Move, dump aren, `for`/`while`/`break`/`continue`,
`println` liczb i napisów w `main`, `concat` i `promote` w `main`,
wycinki o literałowych granicach oraz linkowanie z runtime są pokryte
testami `tests/build.rs` albo `frontend::check` i zostały powtórzone przy
pisaniu tej książki. Dziury są na brzegach: nullable w codegenie, floaty,
`String` jako argument wołania, powrót świeżego napisu.

## Jak czytać niejasność

Gdy komentarz, specyfikacja i kod się różnią, kolejność zaufania przyjęta
w książce jest taka: test wykonawczy (`tests/build.rs`), potem
`frontend::check` na przykładzie, potem kod fazy, potem `docs/language.md`,
na końcu `docs/superpowers`. Hasło README o borrow checkerze i GC jest
zgodne z modelem. Przykład `concat` na dwóch `var` nie jest.
