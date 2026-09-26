# Dodatek A. Ściągawka składni

Stan: frontend tej rewizji. Gwiazdka `*` oznacza: checker przyjmuje, `bork build` nie obniża albo panikuje. Szczegóły w dodatku C.

## Program

```text
fun nazwa(parametry) { instrukcje }
fun nazwa(parametry): Typ { instrukcje }
```

Parametr: `nazwa: Typ`, `val nazwa: Typ`, `var nazwa: Typ`. Goły = `val`.
Instrukcje rozdziela `NL`. Komentarz: `//`.

## Typy

```text
i8 i16 i32 i64 u8 u16 u32 u64
Int Long Byte Float Double     // aliasy i32 i64 u8 f32 f64
f32 f64 bool unit String
[T; N]  T?  (A, B) -> R  ((A) -> R)?
```

Copy: nienullowalny prymityw. Reszta nie.

## Instrukcje

```text
val nazwa = wyrażenie
val nazwa: Typ = wyrażenie
var nazwa = wyrażenie
var nazwa: Typ = wyrażenie
nazwa = wyrażenie
nazwa[indeks] = wyrażenie
{ instrukcje }
for (nazwa in lo..hi) { instrukcje }
while (warunek) { instrukcje }
break
continue
return
return wyrażenie
move { instrukcje }
move () { instrukcje }
move (a, b) { instrukcje }
wyrażenie
```

`for`: `lo..hi` po `i32`, bez `hi`. `while`: warunek `bool`.

## Wyrażenia

Od najciaśniejszego: wywołanie i przyrostki, `!`, `*` `/`, `+` `-`,
porównania, `..`, `&&`, `||`, `?:`.

```text
nazwa
liczba
liczba.liczba
"napis"          // \n \r \t \\ \"
true false
[e, e, e]
nazwa[i]
nazwa[lo..hi]    // lo i hi literały i32
nazwa.pole
nazwa?.pole
wyrażenie!!
wyrażenie ?: wyrażenie
Some(wyrażenie) None
move nazwa
promote nazwa
f(a, b)
f(a) { x -> instrukcje }          *
f(a) move (s) { x -> instrukcje } *
- nie istnieje jako minus jednoargumentowy
```

`if (warunek) { } else { }` jest wyrażeniem. Bez `else`, gdy nikt nie
oczekuje typu, wynikiem jest `unit`.

## Własność w jednej tabeli

| Sytuacja | Zapis |
|---|---|
| Copy (`i32`, `bool`, …) | goła nazwa |
| rodzic `val` nie-Copy | goła nazwa (Shared) |
| rodzic `var` nie-Copy | `move nazwa` |
| `val` nie-Copy do parametru `var` | `move nazwa` |
| literał do parametru `var` | bez `move` (codegen `String` dziś panikuje) * |
| zapis do zewnętrznego `var` | `outer = …` albo `outer = move inner` albo `outer = promote held` |
| `return` napisu | `return nazwa` na głębokości funkcji albo `return "literał"` |

## Wbudowane

```text
print(i32 | i64 | String)
println(i32 | i64 | String)
concat(String, String) -> String
```

## Polecenia

```text
bork plik.bork
bork --dump-arenas plik.bork
bork build plik.bork
bork build -o wyjście plik.bork
cargo test --workspace
cargo test --workspace --features codegen
```

Kody: 0 sukces, 1 zły program, 2 złe wywołanie lub toolchain, 101 panic
procesu `bork`, 134 SIGABRT procesu użytkownika (`abort`).

## Fazy diagnostyki

`parse`, `ownership`, `type`, `codegen`.
