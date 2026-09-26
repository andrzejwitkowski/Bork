# O tej książce

Ta książka uczy języka Bork i jednocześnie jest mapą kodu kompilatora
napisanego w Ruście. Nie jest specyfikacją życzeń. Jeśli funkcja nie występuje
w parserze, typecku, semantyce albo codegenie, nie jest opisana jako część
języka. Jeśli jest tylko w specyfikacji z `docs/superpowers/specs/` albo w
`TODO.md`, jest oznaczona jako plan albo dług.

## Dla kogo

Książka zakłada, że piszesz już w jakimś języku programowania. Najwygodniej,
jeśli znasz choć jeden z trzech punktów odniesienia, których Bork używa w
dokumentacji projektu:

- Kotlin albo inny język z blokami, `if` jako wyrażeniem i trailing lambda,
- Rust albo C++, na tyle żeby słowa „własność”, „przeniesienie” i „alias”
  nie były puste,
- odrobinę kompilatorów: wiesz, że lexer, parser, AST i backend to kolejne
  fazy, nawet jeśli nigdy nie pisałeś LALRPOP ani LLVM IR.

Nie musisz znać Inkwella ani `tower-lsp`. Rozdziały o backendzie tłumaczą te
biblioteki w zakresie, w jakim repo ich używa.

Część I da się czytać bez zaglądania do `.rs`. Część II cytuje nazwy funkcji i
plików. Część III zakłada, że masz repo otwarte obok książki.

## Jak czytać

Są trzy sensowne przejścia.

**Ścieżka języka.** Przedmowa, rozdziały 1–11, dodatki A i C. Zatrzymaj się
na każdym listingu oznaczonym „uruchomione” i odpal go sam. Listingi
oznaczone „tylko check” przechodzą `bork plik.bork`, ale `bork build` ich nie
obniża albo obniżenie kończy się paniką.

**Ścieżka kompilatora.** Rozdział 2 (po co regiony), potem 12–18, potem 20.
Rozdział 20 przeprowadza jeden krótki program przez `parse`, typeck, semę,
hoist, escape, bramkę i LLVM. To jest najszybszy sposób zobaczyć, które
struktury danych naprawdę niosą informację.

**Ścieżka kontrybutora.** Rozdziały 19–22 i dodatek C. Rozdział 21 opisuje,
które testy łapią którą fazę i jak dodać nową konstrukcję, idąc śladem
`while` / `break` / `continue`, które w kodzie już są.

> **TIP.** Komunikaty w książce pochodzą z uruchomienia `target/debug/bork`.
> Format linii to `plik:linia:kolumna: error: faza: treść`. Kolumna liczy
> znaki Unicode od początku linii, nie bajty. Gdy diagnostyka nie ma spanu,
> CLI pomija numer linii. Tak jest na przykład przy `returning the result of
> concat`.

## Konwencje

**Listing 4.2.** tak podpisany jest numerowany w obrębie rozdziału. Pod
listingiem numer w nawiasie, `(1)`, odnosi się do komentarza w kodzie.

Ramki:

> **NOTE.** Fakt o modelu albo o kodzie, który łatwo przegapić.

> **TIP.** Co zrobić w praktyce.

> **WARNING.** Pułapka: program, który typeck przyjmie, a `bork build`
> odrzuci albo na którym kompilator spanikuje. Osobno: zachowanie runtime,
> które kończy proces.

Kod Bork jest w blokach `bork`. Kod Rusta, który jest częścią kompilatora,
jest w blokach `rust` i ma ścieżkę pliku w podpisie.

Diagramy są w Mermaid. W PDF są wyrenderowane do obrazków.

## Czego książka nie obiecuje

Bork w tej rewizji nie ma:

- modułów, `import`, pakietów ani przestrzeni nazw,
- struktur, enumów użytkownika, traitów, generyków,
- wyjątków, `panic` jako konstrukcji języka, typu `Result`,
- operatora jednoargumentowego minusa (literał ujemny nie parsuje się),
- średników jako separatorów instrukcji,
- garbage collectora,
- referencji `&` / `&mut` i borrow checkera w stylu Rusta,
- JIT, interpretera `bork run`, debuggera,
- codegenu dla trailing closures, `Some`, `None`, `?:` i `!!`.

`String` przekazywany do funkcji użytkownika jest akceptowany przez frontend,
ale `bork build` na tej ścieżce panikuje w `value_as_int`. To błąd
kompilatora, nie reguła języka. Szczegóły są w rozdziale 17 i dodatku C.

## O kodzie źródłowym książki

Rozdziały leżą w `docs/book/`. Przykłady, które uruchamialiśmy, są w
`docs/book/przyklady/` razem z plikiem `WYNIKI.md`. Historia commitów
projektu (parser, sema, typowany HIR, LLVM, sink/hoist/escape, tablice,
LLVM 23) jest tłem rozdziału 19; książka nie streszcza każdego pull requestu.

## O autorze języka i o tej książce

Język i kompilator są projektem Andrzeja Witkowskiego (historia `git` na
`main`). Ta książka jest opisem tego kodu dla innych programistów. Nie
zmienia semantyki. Gdy dokumentacja w `docs/language.md` rozjeżdża się z
kompilatorem, książka staje po stronie kompilatora i mówi o rozjeździe wprost.
Najważniejszy przykład: rozdział o konkatenacji w `docs/language.md` woła
`concat` na dwóch `var String` bez `move`. Frontend tego nie przyjmuje.
