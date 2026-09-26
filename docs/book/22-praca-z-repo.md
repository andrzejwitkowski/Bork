# Rozdział 21. Testy, debugowanie i nowa funkcja języka

## Ten rozdział obejmuje

- Którą komendą odpalić który zestaw
- Gdzie szukać, gdy binarka kłamie, a checker milczy
- Ścieżkę dodania konstrukcji, na przykładzie `while`
- Dług z `TODO.md`, którego nie trzeba odkrywać drugi raz
- Czego nie robić przy pierwszym patchu

## Testy

Z `README` i z `.github/workflows/ci.yml`:

```text
cargo test --workspace
cargo test --workspace --features codegen
```

Pierwsza komenda **nie** składa LLVM. Łapie parser, semę, typeck, escape
przez `frontend::check`, LSP, runtime (`bork_runtime` ma własne testy
jednostkowe puli) i bramkę o tyle, o ile test bramki jest za
`cfg(feature = "codegen")`. Job CI `codegen` jest osobny: instaluje
LLVM 23 i `clang`, puszcza workspace z feature i jeszcze smoke
spakowanego `libLLVM`.

| Test | Co utwierdza |
|---|---|
| `tests/parser.rs` | tokeny, NL, `move`, pierwszeństwo, spany błędów |
| `src/sema/tests/*` | własność nazw |
| `src/typeck/tests/*` | typy, często przez pełny `check` |
| `tests/arrays.rs` | tablice bez uruchamiania binarki |
| `tests/build.rs` | `bork build` i kod wyjścia albo stdout |
| `src/codegen/gate.rs` | teksty `not supported` |
| `src/lsp.rs` | pozycje, hover, dump |
| `crates/bork_runtime` | bump, align, overflow |
| `tools/bork-lsp-extension/test/server.test.js` | ścieżka binarki kontra `cargo run` |

`tests/build.rs` jest za `#![cfg(feature = "codegen")]`. Bez feature
plik znika z kompilacji. Nie dziw się, że `cargo test` „nie widzi”
`builds_for_loop_sum`.

Testy build piszą źródło do `CARGO_TARGET_TMPDIR`, wołają
`CARGO_BIN_EXE_bork` z `build -o`, potem uruchamiają binarkę. Gdy
dodajesz zachowanie runtime, dopisuj tu oczekiwany kod albo stdout, nie
tylko test jednostkowy emisji. Książka oparła listingi na tym samym
kontrakcie.

## Debugowanie

**Checker.** `cargo run --bin bork -- --dump-arenas plik.bork`. Gdy drzewo
nie ma węzła, którego oczekujesz, błąd jest w semie (`walk.rs`), nie w
LLVM. Gdy drzewo jest, a `bork build` panikuje, błąd jest w emisji albo w
`coerce_value_to_ty`.

**Faza.** Prefiks `parse` / `ownership` / `type` / `codegen` mówi, którego
katalogu nie czytać. `ownership` bez słowa `after move` i bez `not Copy`
często jest z `escape.rs`, nie z `policy.rs`. Słowa `inner region`,
`concat`, `moved or promoted` są z escape.

**Rozjazd spaceru.** Komunikat `internal arena schedule mismatch` znaczy:
`region_walk` nie dostał dziecka `ArenaNode` albo dostał je w złym
miejscu. Porównaj etykietę w semie z `RegionSite`. Najczęstsza przyczyna
przy nowej konstrukcji: sema otwiera region, a visitor LLVM o nim nie wie,
albo odwrotnie.

**Panic `into_int_value`.** Wartość LLVM nie jest intem. Patrz, jaki `ty`
ma wyrażenie. `String` i float na ścieżce `emit_call_with_values` są
znanymi ofiarami. Naprawa należy do `coerce_value_to_ty`: dopasowanie
`BasicValueEnum`, a dla struktur przekazanie deskryptora bez kastu na
`i64`. Nie maskuj tego `catch_unwind`.

**Runtime abort bez diagnostyki.** Indeks albo dzielenie. Odpal binarkę
pod debuggerem i zobacz, czy stanąłeś w `abort`. Przepełnienie areny daje
panic Rusta z tekstem `arena overflow`, bo runtime jest pisany w Ruście i
`panic` nie jest łapany.

**LLVM verify.** `module.verify()` po emisji wywraca build, gdy IR jest
zepsuty (zły typ PHI, brak `ret`). To lepsze niż cichy zły kod. Gdy
verify pada, IR jest już zły; nie szukaj błędu w `clang`.

> **TIP.** `bork` bez feature `codegen` jest szybki do iteracji po
> komunikatach. Przepinaj feature dopiero, gdy check jest czysty i chcesz
> zobaczyć bramkę albo proces.

## Ścieżka nowej konstrukcji

Poniżej jest ścieżka, którą widać po fakcie na `while` / `break` /
`continue`. Nie jest to przepis z planu, tylko kolejność warstw, które
musiały ruszyć, bo każda z nich w kodzie o `while` wie.

1. **Gramatyka.** Terminal i produkcja `Stmt`. Dla `while`: warunek w
   nawiasach, ciało blokiem. `break` i `continue` jako instrukcje ze
   spanem. Test w `tests/parser.rs`, że NL działa i że słowo nie jest
   identyfikatorem w złym miejscu.

2. **AST.** Wariant `Stmt::While`, `Break`, `Continue`. Bez typów.

3. **Typeck.** Warunek oczekuje `bool`. `loop_depth` rośnie tak samo jak
   przy `for`. `break` poza pętlą to błąd typu. Nowy `HirStmt`. Test
   tekstu diagnostyki.

4. **Sema.** Region `WhileLoop` przez `open_ordinary`. Ten sam
   `loop_move_ban` co `for`. Jeśli zapomnisz zakazu, test
   `move_outer_binding_inside_loop_errors` nie pokryje `while`, dopóki go
   nie skopiujesz. Warto skopiować.

5. **HIR a peel.** Jeśli ciało jest blokiem, nie dokładaj drugiego
   regionu w typecku. Sema i HIR muszą mieć po jednym dziecku.

6. **Escape / hoist.** Nowa instrukcja w spacerze `escape` i w
   `hoist`, inaczej deklaracja w `while` nie będzie widziana. Dla `while`
   wystarczy zejść w ciało jak w `for`.

7. **`region_walk`.** `RegionSite` i hak `while_loop`. Stempel
   `codegen_push` zacznie widzieć alokacje w ciele. Bez tego LLVM i
   raport się rozjadą w chwili, gdy ciało alokuje napis.

8. **Bramka.** `while` nie potrzebuje odmowy, bo emisja go umie. Nowa
   konstrukcja, której emisja nie umie, **musi** dostać `reject` w
   `gate.rs`, żeby użytkownik dostał diagnostykę zamiast paniki. To jest
   lekcja z `String` jako argumentu: bramka go nie zna, emisja panikuje.

9. **Emisja.** Bloki podstawowe: nagłówek, ciało, zatrzask, `reset` jeśli
   region wypchnął płytę, `break` jako skok do bloku wyjścia. Potem test
   w `tests/build.rs` z konkretnym kodem wyjścia. Dla `while`+`break` jest
   to 3.

10. **Dokument.** `docs/language.md` i, jeśli ruszasz pamięć,
    `docs/memory-model.md`. `TODO.md` prosi, żeby dokument nadążał za
    bramką. Ta książka jest trzecim opisem. Przy zmianie semantyki
    aktualizuj co najmniej `language.md`. Książka w `docs/book` opisuje
    jedną rewizję; nie zaktualizuje się sama.

Konstrukcja, która jest tylko cukrem składniowym, może skończyć się w
parserze desugaringiem do istniejącego AST. W tym repozytorium prawie nic
się tak nie dzieje. `move` wyrażeniowy i `move` regionalny są osobnymi
węzłami. Nowy cukier lepiej zostawić osobnym węzłem, jeśli ma własną
diagnostykę.

## Dług, który już jest zapisany

Z `TODO.md`, skrót bez udawania, że to robimy w tej książce:

- rozciąć visitor codegenu, żeby `FnEmitter` nie był jednocześnie
  `RegionVisitor` (cykl `codegen_walk` ↔ `expr`),
- kursor aren zamiast makra, gdy makro dalej urośnie,
- test, że harmonogram pętli to jeden enter, wiele resetów, jeden exit,
- bramka zsynchronizowana z frontendem,
- błędy spaceru zawsze jako `WalkError::from_diagnostic`, bez gubienia
  tekstu użytkownika w opakowaniu `schedule_error`,
- więcej testów push/pop na zagnieżdżonym `if` i sinkach napisów,
- nie commitować luźnych `smoke.bork` w korzeniu repo.

`internal arena schedule mismatch` wokół floatów jest przykładem punktu
drugiego i czwartego: tekst użytkownika („float binary…”) utonął w
opakowaniu, a bramka w ogóle nie powiedziała „float nie jest obniżany”.

## Pierwszy patch

Nie zaczynaj od LLVM. Dopisz test parsera albo typeck, który na czerwono
nazywa zachowanie, i doprowadź `frontend::check` do tego tekstu. Bramkę
dodaj w tym samym patchu, jeśli emisji nie będzie. Pusta bramka przy nowej
składni jest tym, co zamienia przyszłą panikę w diagnostykę.

Nie zmieniaj `peel_blocks` tylko w jednym z dwóch miejsc.

Nie „naprawiaj” paniki `into_int_value` przez odrzucenie wszystkich
`String` w bramce, jeśli w `main` napisy działają. To obcięłoby listingi,
które są legalne. Wąskie miejsce to kasta argumentu wołania.

## Podsumowanie

- `cargo test --workspace` nie uruchamia LLVM. Feature `codegen` uruchamia `tests/build.rs`.
- Dump aren debuguje semę. Panic `into_int_value` debuguje emisję. `schedule mismatch` debuguje `region_walk`.
- Nowa konstrukcja idzie warstwami: gramatyka, AST, typeck, sema, walk, bramka, emisja, test binarki.
- Jeśli emisji nie ma, bramka jest obowiązkowa.
- `TODO.md` jest listą znanych cięć. Sprawdź ją, zanim opiszesz rozjazd jako odkrycie.
