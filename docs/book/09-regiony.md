# Rozdział 8. Regiony, Copy, Shared i Move

## Ten rozdział obejmuje

- Drzewo aren i znaczniki w `--dump-arenas`
- Wyrażeniowy `move` i `promote`
- Regionalny `move (...)`, `move ()` i wnioskowany `move { }`
- Assign-up: przypisanie do zewnętrznego `var`
- Co jest błędem własności, a co dopiero błędem escape

## Dwa sprawdzania, jedna faza

Własność nazw sprawdza sema (`src/sema`). To, czy **bajty** przeżyją
użycie, sprawdza `escape::check_function`, ale tylko gdy typeck i sema
nie zgłosiły nic wcześniej. Oba źródła drukują się jako `ownership`.
Dlatego „przeszło semę” nie znaczy „przeszło książkowe reguły napisów”.
Znaczy: nazwy nie są użyte po `move` i nie-Copy nie wycieka przez gołe
imię. Escape jest drugim sitem.

Raport semy zostaje nawet przy błędach. HIR zostaje tylko przy pustej
liście diagnostyk, i to po escape. Na błędzie parsowania nie ma nawet
raportu.

## Znaczniki dumpu

| Znacznik | Znaczenie |
|---|---|
| `[Local]` | deklaracja w tym regionie albo parametr |
| `[Copy]` | odczyt prymitywu z regionu rodzica |
| `[Shared ← etykieta]` | odczyt `val` nie-Copy z rodzica |
| `[Moved ← etykieta]` | własność zabrana z tamtego regionu |

Etykiety węzłów: `fun nazwa`, `Block`, `Block (compacted K braces)`,
`ForLoop (i)`, `WhileLoop`, `IfThen`, `IfElse`, `MoveBlock (move)`,
`Closure`, `Closure (move)`.

## Wyrażeniowy `move`

`move nazwa` konsumuje wiązanie. Po nim użycie nazwy jest błędem
`use of nazwa after move from etykieta`.

Gołe przekazanie `var` nie-Copy w przypisaniu albo w argumencie nie
zgaduje `move` za ciebie.

**Listing 8.1.** Uruchomione checker.

```text
err_move.bork:3:13: error: ownership: use `move s` to transfer ownership
```

dla `var s: String = "hi"` oraz `var x = s`.

W wywołaniu tekst jest inny: `use move s to pass ownership`. Gdy sema
widzi przeniesienie między różnymi arenami w przypisaniu, potrafi powiedzieć
`` `s` is not Copy; move it into `Block` with `move` ``.

**Listing 8.2.** Move i druk. Uruchomione: stdout `ab\n42\n`.

```bork
fun main() {
    var s = "ab"
    val t = move s
    println(t)
    print(4)
    println(2)
}
```

Dump: `s [Moved ← fun main]`, `t [Local]`.

## `promote`

`promote nazwa` jest legalne tylko po prawej stronie przypisania do `var`
w **ściśle zewnętrznym** regionie. Kopiuje bajty do areny celu i
unieważnia źródło.

**Listing 8.3.** Uruchomione: stdout `temp\n`.

```bork
fun main() {
    var outer = "a"
    {
        var held = "temp"
        outer = promote held
    }
    println(outer)
}
```

Dump pokazuje `held` dwa razy w bloku: jako `Local` (deklaracja) i jako
`Moved ← Block` (obserwacja po promote).

Błędy, sprawdzone albo obecne w `sema/walk.rs`:

| Sytuacja | Komunikat |
|---|---|
| `promote` poza assign-up | `` `promote` is only valid when assigning to a binding in an outer region `` |
| źródło Copy | `` `promote` is not needed for Copy binding … `` |
| cel nie jest głębiej niż źródło | `` `promote held` cannot lift into `fun main`: value already lives in that arena or further out `` |

Ostatni dostaniesz, gdy `held` i cel są w tym samym regionie (`val x = promote held` na głębokości `main`). Sprawdzone.

`promote` na `return` nie istnieje. Nie ma takiej produkcji w roli
wyniku, która miałaby sink powrotny.

## Assign-up

`outer = rhs`, gdy `outer` jest `var` w przodku i nie jest moved, **nie**
jest „użyciem `var` rodzica w dziecku”. Sema pomija `note_use` na lewej
stronie. Prawa strona dostaje sink transferu równy arenie `outer`, nie
arenie bloku.

**Listing 8.4.** Uruchomione: stdout `b\n`. W bloku nie ma lokalnych nazw.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

To jest mutacja. Nie potrzebujesz `inner`. `docs/language.md` mówi to samo
i tutaj kompilator jest zgodny z dokumentem.

## Regionalny `move`

**Listing 8.5.** Uruchomione: stdout `A\nB\n`.

```bork
fun main() {
    var a: String = "A"
    var b: String = "B"
    move (a) {
        val x = a
        println(x)
    }
    move {
        val y = b
        println(y)
    }
}
```

Dump:

```text
├── a [Local]
├── b [Local]
├── MoveBlock (move)
│   ├── a [Moved ← fun main]
│   └── x [Local]
└── MoveBlock (move)
    ├── b [Moved ← fun main]
    └── y [Local]
```

Nazwy z listy przechwycenia są już lokalne w bloku. Nie pisze się
`val x = move a` dla nich drugi raz. Sema odrzuca ponowny move
przechwycenia w tym samym regionie: `cannot move nazwa: it was already
moved into this region as a capture`.

`move () { }` nie wnioskuje. Nic nie jest przeniesione. Odczyt
zewnętrznego `var String` wewnątrz jest wtedy zwykłym błędem „not Copy”.

`move { }` bez nawiasów przenosi każdą żywą, nie-Copy nazwę rodzica, której
ciało używa jako zmiennej wolnej. Nazwy nieczytane zostają. Copy się nie
przenosi. W listingu 8.5 drugi blok wspomina tylko `b`, więc `a` już i tak
było martwe po pierwszym bloku, a `b` pada ofiarą wnioskowania.

Jawna lista wygrywa z wnioskowaniem. Pusta jawna lista wyłącza wnioskowanie.
To jest różnica między `None` a `Some([])` w AST (`captures`).

## Pętla

Zakaz z rozdziału 2 dotyczy też regionalnego `move`. Lokal utworzony w
ciele wolno przenieść:

```bork
for (i in 1..10) {
    var a: String = "A"
    move (a) {
        val t = a
    }
}
```

Checker takich programów pilnuje test `move_loop_local_binding_is_ok`.
Każda iteracja ma świeże `a`. Zewnętrzne `a` byłoby błędem „inside a loop”.

## Shared kontra błąd

Odczyt `val` nie-Copy w dziecku jest Shared i jest poprawny (listing 2.2).
Odczyt `var` nie-Copy w dziecku bez `move` jest błędem. Nie ma trzeciej
możliwości „pożycz na chwilę `var`”. Albo przenosisz, albo trzymasz `val`.

`Some(s)` i argumenty zagnieżdżone podlegają tej samej regule: nazwany
`var` nie-Copy w środku konstruktora też chce `move`. Literał nie chce.

## Podsumowanie

- Sema pilnuje nazw. Escape pilnuje bajtów. Obie diagnostyki nazywają się `ownership`.
- `move nazwa` zużywa wiązanie. Gołe `var` nie-Copy w przypisaniu i w wywołaniu jest błędem z podpowiedzią `move`.
- `promote` podnosi bajty do zewnętrznego `var` i też zużywa źródło.
- Assign-up nie jest odczytem lewej strony.
- `move (a, b)`, `move ()` i `move { }` to trzy różne listy przechwycenia.
- Wnioskowanie nie rusza wartości Copy i nie rusza nazw, których ciało nie wspomina.
