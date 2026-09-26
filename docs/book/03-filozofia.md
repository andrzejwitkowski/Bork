# Rozdział 2. Filozofia: regiony zamiast GC i borrow checkera

## Ten rozdział obejmuje

- Dlaczego Bork nie ma GC i nie ma ogólnych referencji
- Czym region różni się od lifetime'u jednej zmiennej
- Trzy sposoby przekroczenia granicy regionu: Copy, Shared, Move
- Co jest modelem docelowym, a co jest jeszcze długiem
- Czego język świadomie nie planuje

## Problem, który region ma rozwiązać

W programie systemowym są dwa rodzaje danych. Liczby, boole i `unit` mieszczą
się w rejestrze albo na stosie. Tekst i bufor tablicy nie: to adres plus
długość, a bajty leżą gdzie indziej. Pytanie brzmi, kto te bajty zwalnia i
skąd kompilator wie, że wskaźnik jeszcze żyje.

GC odpowiada: runtime kiedyś to zbierze, a programista nie wskazuje miejsca.
Borrow checker Rusta odpowiada: każda referencja ma lifetime dopięty do
konkretnego wypożyczenia, a checker dowodzi, że wypożyczenie nie przeżywa
właściciela. Oba rozwiązania są dobre w swoich niszach. Oba mają koszt, który
Bork nie chce płacić. GC psuje determinizm czasu życia. Ogólny borrow checker
wymaga składni i diagnostyk, których ten projekt celowo nie buduje.

Bork odpowiada trzecią regułą, zapisaną w `docs/memory-model.md` i zrealizowaną
w semie oraz w `bork_runtime`:

**Nawiasy klamrowe otwierają pulę.** Alokacja w puli to bump wskaźnika.
Wyjście z bloku ustawia offset na zero. Cała pula wraca na listę wolnych
płyt. Nie ma zwalniania pojedynczego obiektu w środku puli.

## Pula, nie obiekt

**Listing 2.1.** Dwa czasy życia. Uruchomione: stdout `b\n`, kod 0.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

`outer` powstał w regionie funkcji. Przypisanie w środku bloku nie „używa
zmiennej rodzica w sposób zabroniony”. To mutacja wiązania, które żyje
dłużej. Bajty nowego napisu lądują w puli `outer`, nie w puli wewnętrznego
bloku, który za chwilę zniknie. To jest reguła sink allocation z rozdziału 9.
Na tym listingu widać skutek bez słownictwa kompilatora: po `}` napis `"b"`
nadal wolno wydrukować.

Gdybyś napisał `var inner = "b"` i próbował użyć `inner` za zamykającym
nawiasem, nazwa w ogóle nie jest w zasięgu. To osobna sprawa od aren. Region
nie przedłuża zasięgu leksykalnego. Zasięg i pula są złączone, ale nie są
tym samym: nazwa umiera według bloków, a bajty umierają według areny, w
której zostały położone.

## Trzy przejścia

| Przejście | Kiedy | Co zostaje u rodzica |
|---|---|---|
| Copy | Nie-null typ prymitywny (`i32`, `bool`, `f64`, …) | Wartość żyje dalej, dziecko ma kopię bitów |
| Shared | `val` typu nie-Copy, czytany w dziecku | Wiązanie żyje, dziecko patrzy na te same bajty |
| Move | `var` typu nie-Copy albo jawne `move` | Wiązanie źródłowe jest martwe |

**Listing 2.2.** Shared. Uruchomione: stdout `x\n`.

```bork
fun main() {
    val s = "x"
    {
        println(s)
    }
}
```

Dump:

```text
Arenas
└── fun main
    ├── s [Local]
    └── Block
        └── s [Shared ← fun main]
```

Dziecko nie kopiuje znaków i nie przejmuje własności. Stos wywołań (tu: stos
regionów) gwarantuje, że rodzic żyje dłużej niż dziecko. Dlatego Shared jest
bezpieczne bez borrow checkera: nie da się schować wskaźnika z dziecka do
rodzica, bo język nie ma typu referencji, którą można zapisać w polu.

**Listing 2.3.** Move zabija nazwę. Uruchomione checker; diagnostyka cytowana dosłownie.

```bork
fun main() {
    var s: String = "hi"
    val t = move s
    val u = s
}
```

```text
err_use_after.bork:4:13: error: ownership: use of `s` after move from fun main
```

Move nie wycina dziury w arenie rodzica. Stare bajty zostają martwym
miejscem aż do `reset` puli. Kompilator unieważnia **wiązanie**. To jest
tańsze niż kompaktor areny i wystarcza, dopóki nie ma wskaźników w pola
struktur — a struktur użytkownika nie ma.

`move` na wartości Copy jest no-opem własności. Program `val n = 1` /
`val m = move n` / `return n + m` przechodzi check i `bork build`, kod
wyjścia 2. Nazwa `n` zostaje żywa. Dump nie oznacza jej jako `Moved`.

## Pętla nie może przenieść rodzica

Pętla wykonuje ciało wiele razy. Własność zewnętrznego `var` da się oddać
tylko raz. Sema trzyma zbiór `loop_move_ban`: nazwy widoczne przy wejściu do
`for` albo `while` nie mogą być źródłem `move` w ciele.

**Listing 2.4.** Uruchomione checker.

```bork
fun main() {
    var s = "Hello, World!"
    for (i in 1..10) {
        move {
            val t = s
        }
    }
}
```

```text
err_loop.bork:5:21: error: ownership: cannot move `s` inside a loop: it would already be moved on later iterations
err_loop.bork:5:21: error: ownership: `s` is not Copy; move it into `MoveBlock (move)` with `move`
```

Dwa komunikaty na ten sam span są uczciwe: pierwszy z `move_source` (zakaz
pętli), drugi z klasyfikacji użycia, bo wnioskowany `move { }` nie zdążył
przechwycić `s`. Lokalny `var` utworzony **wewnątrz** ciała pętli wolno
przenieść. Każda iteracja dostaje świeże wiązanie. Rozdział 8 ma działający
przykład.

Ciało pętli resetuje swoją arenę co iterację (`bork_arena_reset`), więc
pamięć pętli zostaje O(1) względem liczby obrotów, o ile nie ucieka do
zewnętrznego `var`. To jest osobna oś od zakazu move: nawet poprawna pętla,
która alokuje napis tymczasowy, nie rośnie liniowo po arenach.

## Czego model nie robi

`docs/memory-model.md` ma jawną listę „Not planned”:

- ogólne `&` / `&mut` i referencje w polach między regionami,
- niejawna głęboka kopia przy każdym odczycie spoza regionu,
- zapisanie wypożyczenia z dziecka w polu rodzica.

Nie ma też typu, który mówiłby „ta referencja żyje tyle co tamten obiekt”.
Jedyna dozwolona obserwacja krótsza niż właściciel to Shared z `val` rodzica
do dziecka, które i tak jest zagnieżdżone leksykalnie.

## Co jest modelem, a co długiem

Model docelowy puli jest zaimplementowany:

- pojemność jednej płyty to **4096** bajtów (`ARENA_CAPACITY` w
  `crates/bork_runtime/src/lib.rs` i bliźniaczy model w `src/arena.rs`),
- `bork_arena_push` bierze płytę z puli, `pop` oddaje, `reset` zeruje offset
  bez oddawania płyty (pętle),
- przepełnienie płyty **panikuje** runtime'u Rusta (`arena overflow: need N
  bytes, capacity 4096`), nie zwraca błędu do programu Bork.

Dług, opisany w tym samym pliku jako „Still open” i widoczny w `escape.rs`:

- brak areny wyniku po stronie wołającego, więc `return concat(...)` i
  `return move ...` są odrzucane,
- hoist działa tylko dla pary sąsiednich instrukcji `var` i zaraz potem
  `outer = move var`,
- `promote` nie działa na `return`,
- codegen nie eliduje `memcpy`, gdy bajty już leżą w docelowej puli.

> **WARNING.** `src/arena.rs` to model bump-allocatora używany jako opis i
> testy koncepcji. Sema go nie woła. Sema buduje drzewo `ArenaNode`. Prawdziwe
> płytki 4 KiB powstają w `bork_runtime`, gdy `bork build` wyemituje
> `bork_arena_push`. Nazwa „arena” w dwóch crate'ach znaczy dwie różne
> struktury. Rozdział 16 trzyma je osobno.

## Kompromis, który warto nazwać

Region jest gruboziarnisty. W bloku, który buduje jeden krótki napis i jeden
duży bufor, oba dzielą 4096 bajtów i umierają razem. Nie da się zwolnić
napisu wcześniej. To jest cena braku `free` i braku GC. Dla MVP jest przyjęta
świadomie: stały rozmiar płyty jest przyjazny cache'owi i upraszcza codegen
(jeden wskaźnik offsetu, jeden `reset`). Gdy program potrzebuje danych
dłuższych niż blok, jedyna droga to położyć je w `var` zewnętrznego regionu
przez przypisanie, `move` albo `promote`. Nie ma sterty „na zawsze” poza
regionem funkcji i poza literałami w stałych programu.

Literał `"hi"` może w ogóle nie leżeć w arenie. Codegen kładzie bajty w
prywatnym globalu LLVM, a w wartości zostaje deskryptor `{ ptr, len }`.
Wtedy koniec bloku nie unieważnia znaków. Kompilator i tak śledzi własność
wiązania. Stały napis nie jest pretekstem do używania nazwy po `move`.

## Podsumowanie

- Blok `{ }` jest regionem: bump alokacji, zbiorcze zwolnienie przy wyjściu.
- Copy, Shared i Move to trzy legalne przejścia granicy. Innych nie ma.
- Move unieważnia wiązanie, nie kompaktuje areny.
- Pętla nie przenosi wiązania spoza ciała i resetuje swoją pulę co iterację.
- Pojemność płyty to 4096 bajtów. Przepełnienie panikuje runtime.
- Ogólne referencje nie są w planie. Świeżo zaalokowany `String` zwracany z
  funkcji też jeszcze nie: brakuje areny wyniku po stronie wołającego.
