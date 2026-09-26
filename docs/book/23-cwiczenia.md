# Rozdział 22. Ćwiczenia

## Ten rozdział obejmuje

- Ćwiczenia na checker, bez LLVM
- Ćwiczenia na `bork build` i kod wyjścia
- Ćwiczenia czytania kodu kompilatora
- Odpowiedzi, które da się zestawić z komunikatem albo z plikiem

Odpowiedzi są w tym samym rozdziale, pod treścią. Spróbuj najpierw
odpalić kompilator. Komunikaty z części I wolno uznać za ściągę, nie za
porażkę.

## Część A. Checker

**A1.** Napisz funkcję `main`, która zwraca `i32` i nie ma ani jednego
`return`. Jaki jest dokładny początek komunikatu (faza i pierwsze słowa)?

**A2.** Zadeklaruj `var s = "hi"` i `var t = s`. Popraw program najmniejszą
zmianą tak, żeby check przeszedł, a `s` nie dało się użyć niżej.

**A3.** Wytłumacz, czemu ten program nie parsuje się, i podaj poprawną
wersję o tym samym wyniku:

```bork
fun main(): i32 {
    val n = 0 - 3
    return n
}
```

Ta wersja jest poprawna. Ćwiczenie: zamień `0 - 3` na `-3` i nazwij fazę
błędu.

**A4.** `val a: [i32; 2] = [1, 2, 3]`. Ile diagnostyk dostaniesz i jakich
faz? Nie poprawiaj programu, tylko je nazwij.

**A5.** Napisz `for` po zakresie `i64`. Jaki typ granic checker wymaga?

**A6.** Program z dokumentacji:

```bork
var left = "L"
var right = "R"
{
    var piece = concat(left, right)
    out = move piece
}
```

dopisz brakującą deklarację `out` i spraw, żeby check przeszedł na dwa
sposoby: przez `val` oraz przez `move`.

**A7.** `return concat("a", "b")` w funkcji zwracającej `String`. Czy
komunikat ma numer linii?

## Część B. Binarka

Uruchom `bork build` i proces.

**B1.** Ile wynosi kod wyjścia sumy `for (i in 0..5) total = total + i`?

**B2.** `while` podbija `i` od 0 i robi `break` przy `i == 3`, zanim
zwiększy. Co zwraca `main`?

**B3.** `println("hi")` w `main` bez typu wyniku. Co jest na stdout i jaki
jest kod wyjścia?

**B4.** `return 1 / 0`. Czy `bork build` kończy się błędem? Co robi proces?

**B5.** `val a = [10, 20, 30]` i `a[9]`. Czy check przechodzi?

**B6.** Weź listing 20.1. Zmień zakres na `0..4`. Jaki kod wyjścia
przewidujesz i dlaczego, zanim uruchomisz?

**B7.** `fun main(): i64 { return 1 }`. Jaka faza i czy pada checker, czy
dopiero build?

## Część C. Kod kompilatora

**C1.** Wskaż plik i funkcję, które ustawiają kolejność „najpierw
diagnostyki semy, potem typy”.

**C2.** Gdzie jest pojemność 4096? Podaj dwa pliki. Który wykonuje się w
procesie użytkownika?

**C3.** Czemu `--dump-arenas` przy `fun oops(` nie drukuje `Arenas`?

**C4.** `MoveBlock.captures` bywa `None` albo `Some([])`. Które
wnioskuje?

**C5.** Dlaczego HIR nie ma pola `region_id`? Gdzie codegen bierze region?

**C6.** Która funkcja w `sema/policy.rs` zwraca `Shared`?

**C7.** Co trzeba zmienić, żeby przekazanie `String` do funkcji
użytkownika dało diagnostykę zamiast paniki? Nie musisz pisać patcha.
Nazwij funkcję, która panikuje, i powiedz, czemu bramka tego nie łapie.

**C8.** Nowy operator binarny, którego LLVM jeszcze nie ma. Wymień
warstwy z rozdziału 21 w kolejności i zaznacz, która jest obowiązkowa,
żeby nie było paniki.

## Odpowiedzi A

**A1.** `type: function must return a value of type i32 on all paths`.
Span bywa pusty, więc linia może zniknąć z prefiksu.

**A2.** `var t = move s`. Komunikat przy gołym `s` to `use move s to
transfer ownership`.

**A3.** `return -3` jest fazą `parse`, `unexpected token`. `0 - 3` jest
odejmowaniem i dla `main(): i32` daje kod wyjścia, który owija się w
`i32` (wartość ujemna jako kod procesu). Checker jest zadowolony.

**A4.** Dwie diagnostyki fazy `type`: literał ma 3 elementy, oczekiwano 2;
inicjalizator ma typ `[i32; 3]`, oczekiwano `[i32; 2]`.

**A5.** `range bounds must have type i32`.

**A6.** `var out = "prefix"` na zewnątrz. Albo `val left` i `val right`,
albo `concat(move left, move right)`. Wynikowy napis to `LR`, nie
`prefixLR`.

**A7.** Nie. Escape stawia `span: None`. Zostaje `plik: error: ownership:
returning the result of concat...`.

## Odpowiedzi B

**B1.** 10.

**B2.** 3.

**B3.** Stdout `hi` plus nowa linia. Kod 0, bo `main` bez typu zwraca
`unit`, a codegen zamienia to na `i32` 0.

**B4.** Build się udaje. Proces kończy się SIGABRT (134).

**B5.** Check przechodzi. Proces aboruje. Indeks nie jest częścią typu.

**B6.** `0+1+2+3 = 6`.

**B7.** Checker przechodzi. Build: faza `codegen`, `` `main` returning `i64` is not supported by codegen yet ``.

## Odpowiedzi C

**C1.** `frontend::check` w `src/frontend.rs`. Typeck zwraca diagnostyki,
sema zwraca błędy, wektor buduje się z semy i dopiero `append` typów.

**C2.** `crates/bork_runtime/src/lib.rs` (stała `ARENA_CAPACITY`, ten plik
jest w procesie użytkownika) oraz `src/arena.rs` (model w kompilatorze,
nie jest wołany przez semę).

**C3.** `CheckResult.report` jest `None`, gdy parse pada. CLI drukuje dump
tylko przy `Some`.

**C4.** `None` wnioskuje. `Some([])` to jawne `move ()`.

**C5.** Komentarz w `src/hir/mod.rs`: regiony żyją w `ArenaReport`. Codegen
bierze je w `region_walk`, zsynchronizowanym z HIR.

**C6.** `classify_use`, gałąź `BindingKind::Val` po sprawdzeniu Copy i
tej samej areny.

**C7.** Panikuje `value_as_int` przez `into_int_value`, wołane z
`coerce_value_to_ty` przy `emit_call_with_values`. Bramka ogląda kształt
HIR (czy jest `Some`, trailing closure, operator), a nie „czy ten argument
jest strukturą LLVM”. Wywołanie z argumentem `String` wygląda jak zwykły
`Call`, więc bramka milczy.

**C8.** Gramatyka, AST, typeck, sema (jeśli operator rusza nazwy), escape
(jeśli rusza bajty), `region_walk` (jeśli otwiera region; operator zwykle
nie), **bramka zanim emisja istnieje**, potem emisja i `tests/build.rs`.
Obowiązkowa przeciw panice jest bramka, dopóki `expr.rs` nie ma gałęzi
dla operatora. Inaczej `match` na `BinOp` wpadnie w `not_yet_supported`
albo, gorzej, w ścieżkę int/float i zły `into_*_value`.

## Podsumowanie

- Ćwiczenia A kończą się na tekście diagnostyki. Ćwiczenia B na kodzie procesu.
- Ćwiczenia C wskazują pliki, nie ogólniki.
- `0..n` jest otwarty z prawej. Suma `0..5` to 10, suma `0..4` to 6.
- Bramka nie zastępuje `coerce_value_to_ty`. Stąd osobna odpowiedź C7.
