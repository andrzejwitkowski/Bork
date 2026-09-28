# Rozdział 6. Warunki, pętle i powrót z funkcji

## Ten rozdział obejmuje

- kiedy warunek daje wartość, a kiedy tylko steruje
- jak działa pętla `for` po zakresie liczb
- jak działają `while`, `break` i `continue`
- dlaczego `&&` i `||` nie zawsze liczą prawą stronę
- czego nie wolno zwrócić z wewnętrznego bloku

## Warunek

Warunek zapisuje się jako `if (wyrażenie) { blok }`, opcjonalnie z `else { blok }`. Wyrażenie w nawiasie musi mieć typ `bool`. Obie gałęzie, jeśli druga istnieje, są blokami. Typ wyniku zależy od tego, czy ktoś oczekuje wartości, i od tego, czy jest `else`. Reguły są w funkcji `check_if` w pliku `src/typeck/expr/control.rs`.

Gdy nie ma `else` i nikt nie oczekuje typu, wynik ma typ `unit`. Gdy nie ma `else`, a oczekiwany typ jest, na przykład przez adnotację `val x: i32`, kompilator zgłasza `value-producing if expression requires an else branch`. Gdy `else` jest i obie gałęzie mają ten sam typ, wynik ma ten typ. Gdy typy gałęzi się różnią, a wynik jest potrzebny, dostaniesz `else branch has type …, expected …`. Gdy `else` jest, ale całe wyrażenie stoi w miejscu, które nie prosi o wartość, wynik i tak jest `unit`. Wartości gałęzi są sprawdzane, ale nie wypływają na zewnątrz.

**Listing 6.1.** Warunek użyty jako wartość. Program został zbudowany i kończy się kodem 42.

```bork
fun pick(flag: i32): i32 {
    val v = if (flag == 1) { 40 } else { 7 }
    return v + 2
}

fun main(): i32 {
    return pick(1)
}
```

Nazwa `v` nie ma adnotacji. Obie gałęzie mają typ `i32`, więc cały warunek też ma typ `i32`. W drzewie regionów gałęzie są osobnymi węzłami `IfThen` oraz `IfElse`.

**Listing 6.2.** Brak `else` przy oczekiwaniu liczby.

```bork
fun main(): i32 {
    val x: i32 = if (1 > 0) { 1 }
    return x
}
```

Komunikat nie wskazuje numeru linii:

```text
if_ann.bork: error: type: value-producing if expression requires an else branch
```

Bez adnotacji, przy `val x = if (n > 0) { n }`, błąd jest inny. Warunek staje się typem `unit`, bo nikt nie oczekiwał konkretnego typu, a dopiero `return x` mówi, że zwracana wartość ma typ `unit`, podczas gdy oczekiwano `i32`. Komunikat o brakującym `else` pojawia się wtedy, gdy oczekiwany typ jest znany.

Gałąź warunku jest regionem. Napis zbudowany tylko w jednej gałęzi nie może być wartością całego warunku, bo arena gałęzi ginie wraz z jej końcem. Analiza ucieczki mówi wtedy, że napis przeniesiony wewnątrz gałęzi nie może być wartością warunku, bo arena gałęzi jest zwalniana. Tekst komunikatu wymienia napis, choć warunek w kodzie patrzy szerzej, na każdą wartość trzymaną w arenie, a więc także na tablicę.

Analiza własności scala stan przeniesienia po warunku. Nazwa jest przeniesiona po całym `if` wtedy, gdy była przeniesiona już wcześniej albo gdy przeniosły ją obie gałęzie. Samo przeniesienie w gałęzi `then`, bez `else`, nie zostawia nazwy martwej po warunku. Inaczej gałąź, która może się nie wykonać, zabijałaby zmienną na stałe. Test `if_then_only_move_without_else_does_not_stick` zamyka tę zasadę.

## Pętla po zakresie

Pętlę po liczbach zapisuje się jako `for (nazwa in początek..koniec)`. Zakres jest otwarty z prawej strony. Liczba `koniec` nie wchodzi do obiegu. Granice mają typ `i32` i są liczone raz. Nazwa pętli ma w sprawdzaniu typów typ `i32`.

**Listing 6.3.** Suma liczb od zera do czterech włącznie. Program został zbudowany i kończy się kodem 10.

```bork
fun main(): i32 {
    var total = 0
    for (i in 0..5) {
        total = total + i
    }
    return total
}
```

W drzewie regionów ciało pętli jest węzłem `ForLoop (i)`. Nazwa `i` jest lokalna. Nazwa `total` wewnątrz pętli jest oznaczona jako kopiowana, bo `i32` jest kopiowane. Przypisanie `total = ...` zmienia zmienną z regionu funkcji. Nie jest przeniesieniem. Generator kodu wchodzi do regionu ciała raz, a przy każdym obiegu czyści arenę tego ciała, zamiast oddawać bufor do puli i brać nowy. Dzięki temu pętla nie zużywa nowego bufora na każdy obieg.

Zakres o granicach `i64` jest odrzucany. Komunikat mówi, że granice zakresu muszą mieć typ `i32`. Pętla, której prawa strona nie jest zakresem, dostaje `for-loop iterator must be a range`. Nie ma pętli po elementach tablicy.

## Pętla dopóki i sterowanie obiegiem

**Listing 6.4.** Przerwanie pętli, gdy licznik dojdzie do trzech. Program został zbudowany i kończy się kodem 3.

```bork
fun main(): i32 {
    var i: i32 = 0
    while (i < 10) {
        if (i == 3) {
            break
        }
        i = i + 1
    }
    return i
}
```

Warunek `while` musi mieć typ `bool`. `break` i `continue` poza pętlą są błędem fazy `type`, nie osobnej fazy sterowania. Dla samotnego `break` w `main` komunikat brzmi `` `break` outside of a loop ``. Sprawdzanie typów pamięta głębokość zagnieżdżenia pętli i podnosi ją zarówno przy `for`, jak i przy `while`.

**Listing 6.5.** `continue` pomija dodanie dwójki. Suma wynosi 0 + 1 + 3 + 4, czyli 8. Program został zbudowany i kończy się tym kodem.

```bork
fun main(): i32 {
    var s = 0
    for (i in 0..5) {
        if (i == 2) {
            continue
        }
        s = s + i
    }
    return s
}
```

`continue` działa także w `while`. Nie ma klauzuli `else` przy pętli. Pętlę bez warunku końca zapisuje się jako `while (true)`.

## Koniunkcja i alternatywa

**Listing 6.6.** Prawa strona koniunkcji nie powinna zmienić zmiennej, bo lewa strona jest fałszywa. Program został zbudowany i kończy się kodem 0.

```bork
fun side(): i32 {
    return 1
}

fun main(): i32 {
    var n: i32 = 0
    if (false && side() == 1) {
        n = 1
    }
    if (true || side() == 1) {
        return n
    }
    return 99
}
```

Funkcja `side` nie wypisuje nic, więc sam wydruk nie pokazuje, że prawe wywołanie zostało pominięte. Pokazuje to wynik: `n` zostaje zerem, a druga instrukcja wraca od razu, bo `true || ...` nie musi liczyć prawej strony. W kodzie generatora `&&` i `||` dostają osobne bloki. Spacer, który uzgadnia drzewo regionów z generowaniem kodu, nie wchodzi w prawy operand zwykłą ścieżką, gdy ta ścieżka jest już odłożona do osobnego bloku. Dzięki temu decyzja, czy region potrzebuje własnego bufora, pozostaje zgodna z tym, co naprawdę zostanie wyemitowane.

Operator `!` wymaga wartości `bool`. Program, który liczy `val flag = !false` i zwraca 1 albo 0 w zależności od `flag`, został zbudowany i kończy się kodem 1.

## Wczesny powrót i regiony

Powrót z wnętrza bloku musi zwolnić regiony, które funkcja otworzyła. Generator kodu zdejmuje je przy `return` i nie zdejmuje ich drugi raz na ścieżce, która już wróciła. Seria poprawek w historii repozytorium dotyczyła właśnie podwójnego zwolnienia albo zostawienia uchwytu bufora. Test `builds_nested_loops_with_regions_and_early_return` zwraca 63 i pilnuje, że wczesny powrót z zagnieżdżonych pętli daje oczekiwany wynik.

Dla programisty skutek jest taki. `return` w środku bloku jest legalny dla wartości kopiowanej i dla napisu, który żyje na poziomie funkcji. Nie jest legalny dla napisu, którego pamięć leży w głębszym regionie.

**Listing 6.7.** Zwrot napisu utworzonego w bloku wewnętrznym. Sprawdzenie i budowanie kończą się kodem jeden. Plik wykonywalny nie powstaje.

```bork
fun mk(): String {
    var s = "esc"
    {
        val x = move s
        return x
    }
}

fun main() {
    println(mk())
}
```

Komunikat fazy `ownership` mówi, że zwracany jest napis, którego pamięć leży w regionie wewnętrznym, i że ta arena jest zwalniana, zanim wartość zostanie zwrócona. Faza nazywa się tak samo jak faza analizy własności, bo analiza ucieczki dokłada błędy do tej samej grupy. Gdy analiza ucieczki zgłosi błąd, reprezentacja pośrednia jest wyrzucana. Drzewo regionów zostaje.

## Podsumowanie

- Warunek bez `else`, użyty tam, gdzie nikt nie oczekuje wartości, ma typ `unit`.
- Gdy oczekiwany typ jest znany, brak `else` jest osobnym błędem.
- Przeniesienie w jednej gałęzi nie zabija nazwy po warunku, jeśli druga gałąź jej nie ruszyła.
- Pętla `for` idzie po zakresie liczb `i32`, otwartym z prawej strony, i czyści arenę ciała przy każdym obiegu.
- `break` i `continue` poza pętlą są błędem typu.
- `&&` i `||` nie liczą prawej strony, gdy lewa już rozstrzyga wynik.
- Nie zwrócisz napisu z bloku wewnętrznego. Kompilator odrzuca taki program, zanim powstanie plik wykonywalny.
