# Rozdział 13. Lexer, parser i AST

## Ten rozdział obejmuje

- LALRPOP i wbudowany lekser
- Piętra pierwszeństwa
- AST: które enumy naprawdę istnieją
- Normalizację nawiasów
- Jak błąd parsowania staje się `Diagnostic`

## Narzędzie

Gramatyka jest w `src/parser.lalrpop`. `build.rs` woła
`lalrpop::process_src()`. Moduł `parser` jest publiczny przez
`lalrpop_mod!`. Wejście do reszty kompilatora to `bork::parse` w
`src/lib.rs`: najpierw `normalize_parenthesized_newlines`, potem
`ProgramParser::new().parse`. Typ błędu to
`ParseError<usize, String, &'static str>`.

Nie ma osobnego pliku tokenów. Lekser jest blokiem `match` w gramatyce.

## Lekser

Pomijane: spacje, tabulatory, `\r`, `\f`, oraz `//` do końca linii.
Złamania linii, które nie stoją tuż przed `else`, stają się tokenem `NL`.
`else` wchłania poprzedzające puste linie i komentarze, żeby `if` mógł
złamać linię przed `else`.

Słowa kluczowe w `match`: `if`, `fun`, `val`, `var`, `for`, `in`,
`return`, `Some`, `None`, `move`, `promote`. Reszta, w tym `while` i
`true`, idzie ścieżką domyślną i jest porównywana jako literał terminala
w produkcji.

Liczba całkowita to `[0-9]+` i mieści się w `i64`, inaczej błąd użytkownika
`integer literal out of range for i64`. Float to `cyfry.cyfry`
(`invalid float literal` przy przepełnieniu parsowania). Napis przechodzi
przez `unescape_string_literal` w `ast.rs`.

## Piętra

Od najciaśniejszego do najluźniejszego, tak jak produkcje wołają siebie:

| Produkcja | Operatory |
|---|---|
| `ExprSuffix` | wywołanie, `[i]`, `[lo..hi]`, `.` / `?.`, przyrostkowe `!!` |
| `ExprUnary` | `!` |
| `ExprMul` | `*` `/` |
| `ExprAdd` | `+` `-` |
| `ExprCompare` | `> < >= <= == !=` |
| `ExprRange` | `..` |
| `ExprAnd` | `&&` |
| `ExprOr` | `\|\|` |
| `ExprElvis` | `?:` |

`?:` jest prawostronnie łączny, bo prawa produkcja woła z powrotem
`ExprElvis`. Reszta binarna jest lewostronna przez `BinTier`.

`docs/language.md` w jednym zdaniu o pierwszeństwie wymienia wywołania,
`*` `/`, `+` `-`, porównania, `..` i `?:`. Pomija `&&`, `||` i `!`, które
w gramatyce są. Książka trzyma się gramatyki. `&&` wiąże ciaśniej niż
`||`, oba luźniej niż porównanie i ciaśniej niż `?:`.

Indeks i granice wycinka używają `ExprCompare`, nie pełnego `Expr`. Bez
dodatkowych nawiasów nie włożysz `?:`, `||`, `&&` ani `..` w środek
`[...]`.

## AST

Plik `src/ast.rs`. Skrót, który wystarcza, żeby czytać typeck.

`Program` ma `functions`. `Function` ma `name`, `params`, `return_type`,
`body`. `Param` ma `BindingKind` (`Val` albo `Var`), `SpannedName` i
`Type`.

`Type` to `Primitive`, `Named`, `Array { elem, len, nullable }`,
`Func { params, ret, nullable }`.

`Stmt` to `Block`, `VarDecl`, `Assign`, `For`, `While`, `Break`,
`Continue`, `MoveBlock`, `Return`, `Expr`. `AssignTarget` to `Name` albo
`Index`. Cel inny niż te dwa ginie już w gramatyce.

`Expr` to literały (`Int`, `Float`, `Bool`, `Str`, `ArrayLit`), `Index`,
`Slice`, `Ident`, `Move`, `Promote`, `None`, `Some`, `Binary`, `Unary`,
`Field`, `Call`, `If`. `Call` niesie `trailing: Option<Closure>`.
`Closure` ma `params`, `body`, `is_move`, `captures`.

`BinOp` obejmuje arytmetykę, porównania, `RangeTo`, `Elvis`, `And`, `Or`.
`UnaryOp` to `Not` i `NotNullAssert` (`!!`).

Spany są `Span { start, end }` w bajtach. Nazwy często są
`SpannedName { name, span }`. Specyfikacja parsera MVP mówiła „bez spanów”.
To jest nieaktualne.

`MoveBlock.captures` i `Closure.captures` są `Option<Vec<...>>`. `None`
znaczy „wnioskuj”. `Some` puste znaczy „jawnie nic”. Tej różnicy nie wolno
spłaszczyć do pustego wektora.

## Layout

`src/layout.rs`. Wewnątrz `(...)` na głębokości klamer z momentu otwarcia
nawiasu, `\n` staje się `\r`. Pomiń stringi i komentarze. Długość pliku w
bajtach się nie zmienia, więc diagnostyka nie jedzie.

Testy: `tests/parser.rs` (pusta programa, próbka MVP, przypisanie indeksu,
`val`/`var` parametrów, `move`, komentarze, pierwszeństwo, `while`,
błędy lokalizacji). To jest pierwszy plik testowy, który warto czytać, gdy
ruszasz gramatykę.

## Z błędu parsowania w diagnostykę

`diag::from_parse` mapuje warianty `ParseError` na `Phase::Parse`.
`User { error }` dostaje span `Span::new(source.len(), source.len())`.
Dlatego „assignment target must be a name or name[index]” wskazuje koniec
pliku. Poprawka byłaby lokalna w `from_parse` albo w akcji `=>?`, która
dziś nie niesie pozycji. Nikt jej jeszcze nie zrobił.

`frontend::check` przy błędzie parse wraca natychmiast. Typeck i sema się
nie wykonują.

## Podsumowanie

- Gramatyka LALRPOP jest jedynym lekserem i parserem.
- Nowa linia jest tokenem `NL`, chyba że layout zamienił ją w `\r` wewnątrz nawiasów, albo stoi przed `else`.
- `Option` przy liście `move` odróżnia wnioskowanie od pustej listy.
- Pierwszeństwo obejmuje `!`, `&&` i `||`, nawet jeśli skrót w `language.md` je pomija.
- Błąd `User` z gramatyki ma span na EOF.
- AST nie niesie typów wywnioskowanych. To robota HIR.
