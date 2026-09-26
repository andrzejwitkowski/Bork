# Dodatek A. Zestawienie składni

Ten dodatek zbiera składnię, którą przyjmuje czytanie programu w tej rewizji kompilatora. Gwiazdka przy wierszu znaczy, że sprawdzenie program przyjmuje, a `bork build` albo odrzuca go komunikatem, albo kończy się awarią kompilatora. Szczegóły są w dodatku C. Zdania wokół zestawienia mówią, jak czytać skrót. Sam skrót jest po to, żeby nie szukać reguły w całym rozdziale.

## Program i parametry

Funkcja ma nazwę, listę parametrów w nawiasach i ciało w nawiasach klamrowych. Typ wyniku, jeśli jest, stoi po dwukropku przed ciałem. Brak typu wyniku znaczy `unit`.

```text
fun nazwa(parametry) { instrukcje }
fun nazwa(parametry): Typ { instrukcje }
```

Parametr zapisuje się jako `nazwa: Typ`, `val nazwa: Typ` albo `var nazwa: Typ`. Goły parametr, bez `val` i bez `var`, jest stały. Instrukcje rozdziela przejście do nowego wiersza, a nie średnik. Komentarz zaczyna się od `//` i trwa do końca wiersza.

## Typy

Typy proste to `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `bool` i `unit`. Aliasy `Int`, `Long`, `Byte`, `Float` i `Double` znaczą odpowiednio `i32`, `i64`, `u8`, `f32` i `f64`. Napis to `String`. Tablica o znanej długości to `[T; N]`. Wartość, która może być pusta, to `T?`. Typ funkcji to `(A, B) -> R`. Może też sam być pusty, na przykład `((A) -> R)?`.

Kopiowalne są tylko typy proste, które nie mogą być puste. Napis, tablica, typ funkcji i wartość pusta kopiowalne nie są.

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

Pętla `for` idzie po zakresie `lo..hi` typu `i32` i nie wykonuje ciała dla wartości `hi`. Pętla `while` wymaga warunku typu `bool`. `break` i `continue` są legalne tylko wewnątrz pętli.

## Wyrażenia

Najmocniej wiążą się wywołanie, indeks, wycinek, odczyt pola i przyrostkowe `!!`. Potem negacja `!`. Potem mnożenie i dzielenie. Potem dodawanie i odejmowanie. Potem porównania. Potem zakres `..`. Potem koniunkcja `&&`. Potem alternatywa `||`. Najsłabiej wiąże się `?:`. Operator `?:` jest prawostronnie łączny. Pozostałe operatory dwuargumentowe są lewostronnie łączne.

```text
nazwa
liczba
liczba.liczba
"napis"          \n \r \t \\ \"
true false
[e, e, e]
nazwa[i]
nazwa[lo..hi]    lo i hi są literałami i32
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
```

Jednoargumentowego minusa nie ma. Liczbę ujemną zapisuje się odejmowaniem, na przykład `0 - 1`.

Warunek `if (warunek) { } else { }` jest wyrażeniem. Obie gałęzie są blokami. Bez `else`, gdy nikt nie oczekuje typu, wynikiem jest `unit`.

## Własność nazw w skrócie

Gdy typ jest kopiowalny, wystarcza goła nazwa. Gdy stała z regionu zewnętrznego nie jest kopiowalna, też wystarcza goła nazwa: to współdzielenie. Gdy zmienna z regionu zewnętrznego nie jest kopiowalna, trzeba napisać `move nazwa`. To samo dotyczy przekazania stałej niekopiowalnej do parametru zmiennego. Literał napisowy do parametru zmiennego nie wymaga `move`, ale generowanie kodu dla napisu jako argumentu funkcji użytkownika dziś kończy się awarią kompilatora. Zapis do zmiennej zewnętrznej ma postać `outer = wyrażenie`, `outer = move inner` albo `outer = promote held`. Zwrot napisu to `return nazwa` na głębokości funkcji albo `return` literału. Zwrot wyniku `concat` oraz zwrot przeniesienia z regionu wewnętrznego są odrzucane.

## Funkcje wbudowane i polecenia

`print` i `println` przyjmują `i32`, `i64` albo `String` i zwracają `unit`, mimo że ich nominalna sygnatura mówi o `i32`. `concat` bierze dwa napisy i zwraca napis.

```text
bork plik.bork
bork --dump-arenas plik.bork
bork build plik.bork
bork build -o wyjście plik.bork
cargo test --workspace
cargo test --workspace --features codegen
```

Kod zero oznacza sukces. Kod jeden oznacza zły program. Kod dwa oznacza złe wywołanie albo brak narzędzia. Kod 101 oznacza awarię procesu `bork`. Kod 134 w powłoce oznacza przerwanie procesu użytkownika przez `abort`.

Fazy komunikatów to `parse`, `ownership`, `type` i `codegen`.
