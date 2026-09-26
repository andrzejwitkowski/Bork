# Rozdział 8. Kopiowanie, współdzielenie i przeniesienie wartości

## Ten rozdział obejmuje

- jak czytać drzewo regionów
- jak przenieść wartość słowem `move` i jak słowem `promote` skopiować ją do zmiennej z regionu zewnętrznego
- czym różnią się trzy formy bloku `move`
- dlaczego przypisanie do zmiennej z zewnątrz bloku nie jest odczytem tej zmiennej
- które błędy pochodzą z nazw, a które z czasu życia bajtów

## Jedna faza dla nazw i dla bufora

Własność nazw sprawdza analiza w katalogu `src/sema`. To, czy bajty przeżyją użycie, sprawdza osobne przejście, analiza ucieczki, w pliku `src/escape.rs`. Analiza ucieczki uruchamia się tylko wtedy, gdy sprawdzanie typów i analiza nazw nie zgłosiły wcześniej żadnego błędu. Oba źródła drukują fazę `ownership`. Dlatego program, który przeszedł analizę nazw, nie musi jeszcze spełniać wszystkich reguł napisów. Znaczy to tyle, że nazwy nie są użyte po przeniesieniu i że wartość niekopiowana nie ucieka przez gołą nazwę. Czas życia bajtów sprawdza dopiero analiza ucieczki.

Drzewo regionów zostaje nawet przy błędach. Reprezentacja pośrednia zostaje tylko wtedy, gdy lista błędów jest pusta, i to po analizie ucieczki. Przy błędzie składni nie ma nawet drzewa.

## Znaczniki w drzewie

Wypis `--dump-arenas` opisuje każdą nazwę krótkim znacznikiem. `Local` oznacza deklarację w tym regionie albo parametr. `Copy` oznacza odczyt wartości kopiowanej z regionu otaczającego. `Shared` z nazwą regionu oznacza odczyt stałej, która nie jest kopiowana. `Moved` z nazwą regionu oznacza, że własność została zabrana z tamtego regionu.

Węzły mają etykiety `fun nazwa`, `Block`, `ForLoop (i)`, `WhileLoop`, `IfThen`, `IfElse`, `MoveBlock (move)`, `Closure` i `Closure (move)`. Sklejone nawiasy dopisują do etykiety informację, ile par zostało scalonych.

## Przeniesienie zapisane przy nazwie

Zapis `move nazwa` zużywa nazwę. Późniejsze użycie daje `use of nazwa after move from etykieta`.

Sam zapis zmiennej napisowej po prawej stronie przypisania albo w argumencie nie dopisuje `move` za programistę.

**Listing 8.1.** Brak `move` przy przypisaniu napisu.

```bork
fun main() {
    var s: String = "hi"
    var x = s
}
```

Komunikat brzmi `use move s to transfer ownership`. W wywołaniu tekst jest inny: `use move s to pass ownership`. Gdy analiza widzi przeniesienie między różnymi regionami, potrafi powiedzieć, że nazwa nie jest kopiowana i trzeba ją przenieść do nazwanego regionu słowem `move`.

**Listing 8.2.** Przeniesienie i wypisanie. Program został zbudowany. Na wyjściu są wiersze `ab` oraz `42`.

```bork
fun main() {
    var s = "ab"
    val t = move s
    println(t)
    print(4)
    println(2)
}
```

W drzewie `s` jest przeniesione z regionu `fun main`, a `t` jest nazwą lokalną.

## Słowo promote

`promote nazwa` jest legalne tylko po prawej stronie przypisania do zmiennej z regionu ściśle zewnętrznego. Kopiuje bajty do areny tej zmiennej i unieważnia nazwę źródłową.

**Listing 8.3.** Podniesienie napisu, który już powstał w bloku wewnętrznym. Program został zbudowany. Na wyjściu jest `temp` oraz nowy wiersz.

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

W drzewie nazwa `held` pojawia się w bloku dwa razy. Raz jako deklaracja lokalna, raz jako przeniesienie po `promote`.

Są trzy typowe błędy. Użycie `promote` poza przypisaniem do zmiennej zewnętrznej daje komunikat, że `promote` jest legalne tylko przy takim przypisaniu, a użycie na wartości kopiowanej daje komunikat, że `promote` nie jest potrzebne. Użycie w tym samym regionie, na przykład `val x = promote held` obok deklaracji `held`, daje komunikat, że `promote held` nie może trafić do `fun main`, bo wartość już żyje w tej arenie albo jeszcze wyżej. Ostatni komunikat został potwierdzony uruchomieniem kompilatora.

`promote` nie występuje przy `return`. Nie ma miejsca przeznaczenia, do którego analiza mogłaby skopiować bajty wyniku.

## Przypisanie do zmiennej z zewnątrz

Zapis `outer = wartość`, gdy `outer` jest zmienną z regionu otaczającego i nie została wcześniej przeniesiona, nie jest odczytem tej zmiennej w bloku wewnętrznym. Analiza nie traktuje lewej strony jak użycia, które wymagałoby `move`. Prawa strona dostaje jako miejsce alokacji arenę zmiennej `outer`, a nie arenę bloku, w którym stoi przypisanie.

**Listing 8.4.** Przypisanie literału do zmiennej z zewnątrz. Program został zbudowany. Na wyjściu jest `b` oraz nowy wiersz. W bloku wewnętrznym nie ma żadnej nazwy lokalnej.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

To jest zmiana istniejącej zmiennej. Nie potrzebujesz nazwy pośredniej. W tym przykładzie dokument `docs/language.md` i kompilator mówią to samo.

## Blok, który przenosi kilka nazw naraz

**Listing 8.5.** Dwa bloki `move`. Program został zbudowany. Na wyjściu są wiersze `A` oraz `B`.

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

Pierwszy blok wymienia `a` w nawiasie. Wewnątrz nazwa `a` jest już lokalna. Nie pisze się przy niej drugi raz `move`. Drugi blok nie ma listy. Kompilator przenosi te niekopiowane nazwy z zewnątrz, których blok używa. Używa `b`, więc przenosi `b`. W drzewie oba bloki nazywają się `MoveBlock (move)`. Przy każdym widać, skąd nazwa została przeniesiona.

Nazwy z listy są już w regionie bloku. Ponowne `move` takiej nazwy w tym samym regionie jest błędem. Komunikat mówi, że nazwa została już przeniesiona do tego regionu jako przechwycenie.

Zapis `move () { }` nie zgaduje listy. Nic nie jest przenoszone. Odczyt zewnętrznej zmiennej napisowej wewnątrz takiego bloku jest zwykłym błędem mówiącym, że wartość nie jest kopiowana.

Jawna lista wygrywa ze zgadywaniem. Pusta jawna lista wyłącza zgadywanie. W drzewie składni różnica jest między brakiem listy a listą pustą. Nie wolno tych dwóch zapisów utożsamić, gdy będziesz czytał parser.

## Pętla jeszcze raz

Zakaz z rozdziału 2 dotyczy także bloku `move`. Nazwę utworzoną w ciele pętli wolno przenieść.

```bork
for (i in 1..10) {
    var a: String = "A"
    move (a) {
        val t = a
    }
}
```

Test `move_loop_local_binding_is_ok` pilnuje, że takie przeniesienie przechodzi. Każdy obieg ma świeże `a`. Ta sama nazwa utworzona przed pętlą byłaby błędem.

## Stała i zmienna

Odczyt stałej niekopiowanej w regionie wewnętrznym jest współdzieleniem i jest poprawny. Pokazuje to listing 2.2. Odczyt zmiennej niekopiowanej bez `move` jest błędem. Nie ma trzeciej możliwości w rodzaju chwilowego pożyczenia zmiennej. Albo przenosisz własność, albo trzymasz wartość w stałej.

Ta sama zasada dotyczy wartości schowanej w `Some` i argumentów zagnieżdżonych w większym wyrażeniu. Zmienna napisowa użyta w środku konstruktora też chce `move`. Literał nie chce.

## Podsumowanie

- Analiza nazw pilnuje nazw. Analiza ucieczki pilnuje bajtów. Obie wypisują fazę `ownership`.
- `move nazwa` zużywa nazwę. Zwykłe użycie zmiennej niekopiowanej w przypisaniu i w wywołaniu jest błędem, a komunikat podpowiada `move`.
- `promote` kopiuje bajty do zmiennej z regionu zewnętrznego i też zużywa nazwę źródłową.
- Lewa strona przypisania do zmiennej z zewnątrz nie jest odczytem tej zmiennej.
- `move (a, b)`, `move ()` i `move` bez listy to trzy różne polecenia. Pusta lista nie jest tym samym co brak listy.
- Zgadywanie listy nie rusza wartości kopiowanych i nie rusza nazw, których blok nie wspomina.
