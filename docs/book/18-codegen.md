# Rozdział 17. Bramka, LLVM i runtime

## Ten rozdział obejmuje

- Co `gate` puszcza
- Kształt modułu LLVM i nazwy symboli
- Emisję funkcji, wyrażeń, aren i tablic
- C ABI `bork_runtime` i linkowanie
- Dziury, które kończą się panicą albo `not supported yet`

## Bramka

`src/codegen/gate.rs` schodzi po HIR i zbiera `Phase::Codegen`. Nie próbuje
obniżyć „trochę”. Pierwsze trafienie zostaje diagnostyką, reszta
konstrukcji i tak jest odwiedzana, więc jeden plik może dostać kilka
`not supported`.

Puszczane bez własnego komunikatu bramki: literały, identyfikatory,
tablice, indeks, wycinek, arytmetyka całkowita i porównania, `&&` `||`
`..`, `!`, wywołanie bez trailing closure, `if`, pole `length` na typie
`uses_arena_storage`.

Odrzucane teksty są w rozdziale 10. Bramka nie wie o panice
`into_int_value`. Program z argumentem `String` **przechodzi bramkę** i
wywraca się później, w emisji. To jest luka testu, o którą prosi
`TODO.md`: trzymać bramkę zsynchronizowaną z tym, co emisja umie, albo
mieć test, że każda próbka check-ok albo przechodzi bramkę, albo jest na
liście świadomych odmów. Tej listy dla „panic na String” nie ma.

## Moduł

`emit_module` (`src/codegen/llvm/mod.rs`):

1. Wymaga funkcji o nazwie `main`.
2. `Codegen::new(context, "bork")` — moduł LLVM i builder.
3. `declare_function` dla każdej funkcji HIR, zanim którekolwiek ciało.
   Dzięki temu wołanie wstecz ma symbol.
4. `RegionEmitter` trzyma raport i `ArenaCalls`.
5. `emit_function` dla każdej funkcji.
6. `regions.finish()`, `module.verify()`.

Symbol `main` jest C `main`, typ LLVM `i32 ()`, bez parametrów. Wynik
`unit` i tak zwraca stałe 0. Inna funkcja nazywa się `bork.{nazwa}` i ma
linkage internal. Nie jest eksportowana.

`main` zwracające coś poza `i32` i `unit` odpada komunikatem
`` `main` returning `…` is not supported by codegen yet ``. Sprawdzone dla
`i64`, `f64` i `bool`.

## Wartości

Prymityw całkowity jest wartością LLVM o odpowiadającej szerokości.
`bool` jest intem. Float, tam gdzie emisja w ogóle dojdzie do instrukcji
arytmetycznej sprzed walka, używa `build_float_*`. Ścieżka „after walk”
dla dodawania `f32` zwraca błąd „float binary after walk”. Porównanie
`f64`, które wpadło w `value_as_int`, panikuje. `i64` w funkcji pomocniczej
i porównanie z progiem **działa**: listing z `sub(a: i64, b: i64)` i
`if (sub(big, 50) > 40)` daje kod wyjścia 7. Ograniczenie floatów nie jest
ograniczeniem szerokich intów.

`String` i tablica to `{ ptr, i64 }`. Literał napisu jest prywatnym
globalem, a w wartości ląduje stały deskryptor. Długość pola w języku bywa
`i32` (`length`); w deskryptorze jest `i64`.

Lokalny slot to `alloca` w bloku wejścia funkcji. Dla typu z areną slot
pamięta `home_arena`: identyfikator regionu, w którym bajty powinny żyć.
`alloc_sink` na `FnEmitter` jest ustawiany na czas emisji **wyniku**
przypisania, `return` i `concat`, a czyszczony na czas argumentów. To jest
realizacja reguły „tylko wynik, nie podwyrażenia” z rozdziału 9.

`move` i `promote` deskryptora wołają `copy_into_arena` albo
`copy_array_into_arena`: alokacja w puli celu i `memcpy`. Hoist sprawia, że
kopii czasem nie trzeba, bo bajty już tam powstały. Elizji `memcpy` jako
osobnego passu nie ma (`TODO` w modelu pamięci: „memcpy elision”).

## Areny w LLVM

`ArenaCalls` implementuje zlew regionów:

- push → `bork_arena_push`, uchwyt na stos,
- reset → `bork_arena_reset`,
- pop → `bork_arena_pop`,
- na martwym punkcie wstawienia (po `return`) reset i pop są pomijane.

`return` robi `unwind`: zdejmuje uchwyty, które funkcja jeszcze trzyma.
Seria błędów naprawiana w historii (`skip region exit after return`,
`sync region stack`, `clear arena handles`) dotyczyła podwójnego `pop`
albo zostawienia uchwytu. Testy codegen liczą pary push/pop. Gdy ruszasz
`return`, odpal te testy, nie tylko binarkę.

Alokacja użytkowa to `bork_arena_alloc(arena, size, align)`. Wyrównanie
musi być potęgą dwójki. Runtime panikuje, gdy `end > 4096`.

## Wyrażenia, które emisja zna

`src/codegen/llvm/expr.rs` i `array/emit.rs`. Skrót zachowań sprawdzonych
binarką:

- arytmetyka `i32` i `i64`, porównania intów, dzielenie ze strażnikiem
  `abort`,
- `if` jako PHI albo jako sterowanie ze skokiem,
- `for` i `while` z `break` / `continue`,
- `&&` `||` ze zwieraniem,
- `println` / `print` liczb i literałów oraz lokalnych napisów,
- `concat` dwóch napisów, które da się załadować jako struktury (w `main`),
- literał tablicy, indeks ze strażnikiem, wycinek jako GEP plus nowy
  deskryptor, `.length`,
- przypisanie elementu, w tym `String` przez `move` w `main`.

`concat` alokuje jeden bufor w sinku i kopiuje dwa argumenty. Nie ma
osobnej areny tymczasowej wołania.

Wywołanie funkcji użytkownika idzie albo przez `emit_call` (liczy
argumenty `emit_value`), albo przez `emit_call_with_values` po spacerze.
Ta druga ścieżka woła `coerce_value_to_ty`. Dla `bool` zwęża do inta. Dla
floata zwraca błąd „float call argument after walk”. Dla reszty woła
`value_as_int`, które robi `into_int_value()` **bez** sprawdzenia
wariantu. Struktura `String` panikuje. To jest linia 901 w
`src/codegen/llvm/expr.rs` w tej rewizji. Nie cytuję śladu Inkwella jako
kontraktu. Cytuję go jako objaw.

Wywołanie pośrednie (callee nie jest identyfikatorem) daje diagnostykę
`indirect calls is not supported by codegen yet`, o ile ścieżka w ogóle
tam wejdzie. Bramka trailing closure zwykle odpada wcześniej.

## Runtime

`crates/bork_runtime/src/lib.rs`. Pula to `Mutex<ArenaPool>` w `OnceLock`.
`acquire` zdejmuje `Box<Arena>` z wektora albo alokuje nowy. `release`
robi `reset` i odkłada. Zagnieżdżone żywe regiony mają różne płyty.
Sąsiednie w czasie mogą dostać tę samą.

API `extern "C"`:

| Symbol | Rola |
|---|---|
| `bork_arena_push` | `*mut c_void`, płyta z puli |
| `bork_arena_reset` | offset = 0, płyta zostaje |
| `bork_arena_pop` | zwrot płyty do puli |
| `bork_arena_alloc` | bump, wynik `*mut c_void` |
| `bork_print_i64` / `bork_println_i64` | stdout, flush |
| `bork_print_str` / `bork_println_str` | `ptr` + `len` |

Nie ma symbolu `bork_abort`. Codegen deklaruje libc `abort`.

Testy runtime sprawdzają wyrównanie i przepełnienie. Są w tym samym pliku,
pod `cfg(test)`, i wchodzą w `cargo test --workspace` nawet bez feature
`codegen` kompilatora. Sam archiwum do linkowania powstaje dopiero przy
`codegen`, w `build.rs`.

## Linkowanie

`src/codegen/link.rs`. Polecenie to w skrócie:

```text
clang -o wyjście obiekt.o libbork_runtime.a -lgcc_s -lutil -lrt -lpthread -lm -ldl
```

Ścieżka archiwum: zmienna środowiskowa `BORK_RUNTIME_LIB` albo wartość
wkompilowana `env!("BORK_RUNTIME_LIB")`. `build.rs` ustawia rpath
`$ORIGIN` na binarce `bork`, żeby `libLLVM` mogło leżeć obok niej.
`scripts/bundle-llvm.sh` kopiuje SONAME `libLLVM` używany przez `bork`.
Maszyna, która tylko **uruchamia** skompilowany program Bork, nie potrzebuje
LLVM. Maszyna, która uruchamia `bork build`, potrzebuje `clang` i, jeśli
nie spakowano `libLLVM`, instalacji LLVM 23.

## Weryfikacja, którą ta książka zrobiła

`bork` złożony z `--features codegen` (LLVM 23.1.2) zbudował i uruchomił
między innymi: zwrot 42 z `add`, sumę pętli 10, `while`/`break` z wynikiem
3, `continue` z sumą 8, `println("hi")`, Shared `x`, `move` napisu `ab`,
`concat` dający `LR`, `promote` dający `temp`, przypisanie `"b"`, escape
`\n` i `\"`, dzielenie `8/2` z kodem 4, tablicę z wycinkiem i kodem 12,
wycinek drukujący `20` i `2`, indeks poza zakresem oraz `1/0` jako abort,
brak `main` jako diagnostykę. Tych wyników nie trzeba brać na wiarę:
`docs/book/przyklady/WYNIKI.md` je powtarza.

## Podsumowanie

- Bramka odcina nullable, `?:`, `!!` i trailing closures komunikatem.
- Nie odcina `String` jako argumentu funkcji. Emisja wtedy panikuje.
- `main` to C `main` zwracające `i32`. Inne funkcje to `bork.nazwa`.
- Areny w LLVM to stos uchwytów do `bork_runtime`. Pętla resetuje, nie zdejmuje płyty co obrót.
- Napis i tablica to `{ ptr, i64 }`. Literał napisu leży w globalu.
- Linkuje `clang` z statycznym runtime. Kompilator sam ładuje `libLLVM-23`.
