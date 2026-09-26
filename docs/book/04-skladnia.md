# Rozdział 3. Składnia, wiązania i układ pliku

## Ten rozdział obejmuje

- Kształt pliku: funkcje, komentarze, nowe linie
- `val` i `var`, cień nazw, adnotacje
- Które słowa są słowami kluczowymi, a które tylko terminalami gramatyki
- Wyrażenia, które wyglądają jak instrukcje
- Layout nawiasów okrągłych, czyli dlaczego wywołanie może złamać linię

## Plik

Plik `.bork` to zero lub więcej funkcji. Pusta treść i sam znak nowej linii
parsują się do `Program { functions: vec![] }`. Komentarz `//` biegnie do
końca linii i jest zjadany przez lexer, zanim parser zobaczy tokeny.

**Listing 3.1.** Komentarz nie psuje instrukcji. Uruchomione: kod wyjścia 1.

```bork
fun main(): i32 {
    // komentarz
    val n = 1 // tez
    return n
}
```

Nie ma komentarza blokowego. Nie ma atrybutów, pragm ani `import`.

Słowa rozpoznawane w lekserze jako osobne tokeny to: `if`, `fun`, `val`,
`var`, `for`, `in`, `return`, `Some`, `None`, `move`, `promote`. Słowa
`while`, `break`, `continue`, `true`, `false` są zwykłymi terminalami
gramatyki: lekser oddaje je jako identyfikatory-słowa, a parser oczekuje
konkretnego napisu. Skutek praktyczny jest taki, że nie użyjesz ich jako
nazwy, bo produkcja identyfikatora nie pokrywa się z tymi literałami tam,
gdzie gramatyka oczekuje słowa. `else` ma osobną regułę: przed `else` wolno
złamanie linii i komentarz, i to nadal jest token `else`, nie `NL` plus
`else`. Dzięki temu `if` w następnej linii może mieć `else`.

Identyfikator to `[A-Za-z_][A-Za-z0-9_]*`. Nie ma unicode w nazwach.

## Jedna instrukcja na linię

**Listing 3.2.** Średnik. Uruchomione.

```text
semi.bork:2:14: error: parse: unexpected token `;`; expected ... "}" , "NL"
```

dla źródła `val n = 1;`.

Parser traktuje `NL` jako separator w ciele bloku. Pusta linia jest zlepiana
przez regułę „jednego lub więcej” złamań, więc kilka pustych linii między
instrukcjami jest w porządku. Dwie instrukcje bez `NL` nie są.

> **TIP.** Gdy komunikat mówi `expected "NL"`, prawie zawsze dwie rzeczy
> stoją w jednej linii albo został średnik przywieziony z C lub Rusta.

## Wiązania

**Listing 3.3.** `val` jest stałe, `var` można przypisać. Fragment uruchomiony w większych programach.

```bork
val n = 1
var count: i32 = 0
var s: String = "hi"
count = count + 1
```

Reguły, które typeck naprawdę egzekwuje:

- `: Typ` jest opcjonalne, gdy inicjalizator wyznacza typ.
- Przypisanie do `val` to `cannot assign to immutable val binding`.
- Nazwa w bloku przesłania zewnętrzną do końca bloku.
- Cień może czytać zewnętrzną nazwę w swoim inicjalizatorze. Program
  `val n = 1` / w bloku `val n = n + 1` / `return n` daje kod wyjścia 2.
  Dump pokazuje w bloku lokalne `n` oraz obserwację `n [Copy]` — odczyt
  zewnętrznej wartości Copy.

Przypisanie po lewej stronie może być tylko nazwą albo `nazwa[indeks]`.
Pole, wywołanie i dowolne inne wyrażenie dostają błąd użytkownika gramatyki:
`assignment target must be a name or name[index]`. Ten błąd, jako
`ParseError::User`, dostaje span **na końcu pliku**, nie na lewym składniku.
To jest ograniczenie `diag::from_parse`, nie decyzji językowej.

`var` i `val` przy parametrze omawia rozdział 5. Przy lokalnej zmiennej
słowo stoi na początku instrukcji i jest obowiązkowe: nie ma gołego
`n := 1` ani `n = 1` jako deklaracji.

## Blok jest instrukcją i regionem

```bork
{
    val m = n
}
```

Blok zagnieżdżony otwiera region semy z etykietą `Block`. Kilka warstw
nawiasów, które nie zawierają nic poza jednym wewnętrznym blokiem, sema
skleja. `peel_blocks` zdejmuje takie opakowania.

**Listing 3.4.** Trzy warstwy, jeden region. Uruchomione checker.

```bork
fun main() {
    val n = 1
    {
        {
            {
                val m = n
            }
        }
    }
}
```

```text
└── fun main
    ├── n [Local]
    └── Block (compacted 2 braces)
        ├── m [Local]
        └── n [Copy]
```

Zewnętrzny `{` zostaje regionem. Dwa kolejne, które tylko owijają, znikają
z drzewa i zostają licznikiem `compacted_braces`. To nie jest optymalizacja
„na oko”: codegen i sema muszą zdejmować tak samo, inaczej rozjadą się
dzieci `ArenaNode` z miejscami w HIR. Funkcja `hir::peel_blocks` jest
bliźniacza i komentarz w `sema` każe je trzymać w zgodzie.

`if`, `for`, `while`, `fun` i domknięcie nie sklejają się z otoczeniem.
Każde ma własną etykietę (`IfThen`, `IfElse`, `ForLoop (i)`, `WhileLoop`,
`fun nazwa`, `Closure`).

## Wyrażenia, których używasz jako instrukcji

Każde wyrażenie może być instrukcją (`Stmt::Expr`). Wartość jest wtedy
wyrzucana. `if` bez `else` jest typowym przypadkiem: służy do sterowania, a
jego typem wyniku przy braku oczekiwanego typu jest `unit`.

**Listing 3.5.** `if` sterujący. Uruchomione: kod 1.

```bork
fun main(): i32 {
    var n = 0
    if (true) {
        n = 1
    }
    return n
}
```

Gdy `if` ma produkować wartość, obie gałęzie muszą być blokami i musi być
`else`. Szczegóły typów są w rozdziale 6. Tu ważna jest składnia: warunek
jest w nawiasach okrągłych, gałęzie są blokami, nie pojedynczymi
wyrażeniami. Nie napiszesz `if n > 0 n else 0`.

## Łamanie linii w nawiasach

Wywołanie i każda para `(...)` może złamać linię. Przed parserem
`layout::normalize_parenthesized_newlines` zamienia `\n` na `\r` wewnątrz
nawiasów okrągłych na tej samej głębokości klamer, pomijając stringi i
komentarze. Lexer traktuje `\r` jak zwykły biały znak, więc `NL` nie
rozdziela instrukcji w środku listy argumentów.

**Listing 3.6.** Uruchomione: `add(1, 2)` zwraca 3.

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main(): i32 {
    return add(
        1,
        2
    )
}
```

> **NOTE.** Normalizacja nie przesuwa offsetów diagnostyk w sposób, który
> gubiłby miejsce błędu: `\n` i `\r` mają po jednym bajcie, więc spany
> zostają. Testy w `tests/parser.rs` pilnują wieloliniowego wywołania i
> stabilnego miejsca błędu.

Poza nawiasami złamanie linii jest znaczące. Nie ma kontynuacji wyrażenia
„bo linia skończyła się operatorem”, w stylu niektórych języków. Operator na
końcu linii kończy wyrażenie albo jest błędem parsowania.

## Czego parser nie ma

Sprawdzone, kod 1, faza `parse`:

| Źródło | Komunikat |
|---|---|
| `struct Point { x: i32 }` | `unexpected token struct; expected "NL", "fun"` |
| `module foo` | `unexpected token module; expected "NL", "fun"` |
| `return -1` | `unexpected token -` wśród atomów |

Minus jest tylko operatorem binarnym (`+` i `-` na tym samym piętrze).
Liczbę ujemną da się dostać odejmowaniem: `0 - 1`. Nie ma literału
ujemnego. To ogranicza też testy dzielenia `i32::MIN / -1`, które codegen
umie obsłużyć w `guard_int_div`, ale których nie da się zapisać jako
literału.

`fun` zagnieżdżone w funkcji nie istnieje. Funkcje są tylko na poziomie
programu.

## Podsumowanie

- Plik to lista `fun`. Komentarz to tylko `//`.
- Instrukcję kończy nowa linia. Średnik jest błędem parsowania.
- `val` nie przyjmuje przypisania. `var` przyjmuje. Cień nazw działa i może
  czytać przesłaniane imię w inicjalizatorze.
- Gołe wielokrotne `{ { { } } }` sklejają się do jednego regionu.
- Wewnątrz `(...)` nowe linie są białym znakiem. Poza nimi są separatorami.
- Nie ma `struct`, modułów ani jednoargumentowego minusa.
