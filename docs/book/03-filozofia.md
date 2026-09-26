# Rozdział 2. Po co Borkowi regiony pamięci

## Ten rozdział obejmuje

- jaki problem z pamięcią Bork chce rozwiązać bez garbage collectora i bez sprawdzania pożyczek
- czym jest region i czym jest arena
- trzy legalne sposoby użycia wartości spoza bieżącego bloku
- co już działa w kompilatorze, a co jest jeszcze świadomie odłożone

## Dwa rodzaje danych

W programie systemowym są dane, które mieszczą się w rejestrze albo na stosie, i dane, które tam się nie mieszczą. Liczba, wartość logiczna i brak wartości użytkowej należą do pierwszej grupy. Napis i tablica należą do drugiej. Wartość napisu w gotowym programie jest adresem bajtów oraz długością. Same znaki leżą gdzie indziej. Trzeba wiedzieć, kto te bajty zwalnia i skąd kompilator wie, że adres jest jeszcze ważny.

Garbage collector odpowiada, że środowisko wykonawcze zwolni pamięć kiedyś, a programista nie wskazuje chwili. Sprawdzanie pożyczek w Rustcie odpowiada, że każda referencja ma czas życia dowiązany do konkretnego wypożyczenia i że to wypożyczenie nie może przeżyć właściciela. Oba rozwiązania są dobre w swoich miejscach. Bork nie chce płacić ich kosztu. Odśmiecanie odbiera programiście pewność, kiedy pamięć wróci do puli. Ogólne sprawdzanie pożyczek wymaga składni i komunikatów, których ten projekt świadomie nie buduje.

Odpowiedź Borka jest zapisana w `docs/memory-model.md` i zrealizowana w analizie własności oraz w bibliotece `bork_runtime`. Blok ograniczony nawiasami klamrowymi otwiera region. Region ma arenę, czyli jeden bufor, do którego dopisuje się kolejne bajty przez przesuwanie wskaźnika. Wyjście z bloku ustawia ten wskaźnik z powrotem na początek. Cały bufor wraca do puli buforów wolnych. Nie zwalnia się pojedynczego napisu w środku regionu.

Dalej używam słowa region w tym właśnie znaczeniu. Region jest jednocześnie fragmentem programu i czasem życia pamięci z nim związanej. Słowo arena oznacza bufor tego regionu. W kodzie kompilatora nazwa arena pojawia się jeszcze raz, w innym sensie. Wrócę do tego pod koniec rozdziału, bo pomieszanie tych dwóch rzeczy utrudnia czytanie źródeł.

## Bufor regionu, nie pojedynczy obiekt

**Listing 2.1.** Napis zapisany do zmiennej z zewnątrz bloku pozostaje dostępny po wyjściu z bloku.

```bork
fun main() {
    var outer = "a"
    {
        outer = "b"
    }
    println(outer)
}
```

Program został zbudowany i uruchomiony. Na standardowym wyjściu jest `b` oraz nowy wiersz, a kod wyjścia wynosi zero. Zmienna `outer` powstała w regionie funkcji. Przypisanie wewnątrz bloku zmienia tę zmienną. Nie jest próbą wyniesienia nazwy, która żyje tylko w bloku. Bajty nowego napisu trafiają do areny zmiennej `outer`, a nie do areny wewnętrznego bloku, który za chwilę zniknie. Ten mechanizm nazywa się alokacją w miejscu przeznaczenia. Rozdział 9 rozpisuje go na reguły. Tutaj widać skutek. Po zamknięciu nawiasu napis `"b"` nadal wolno wypisać.

Gdybyś wprowadził wewnątrz bloku nazwę `inner` i próbował użyć jej za zamykającym nawiasem, nazwa w ogóle nie jest widoczna. To jest zwykły zasięg leksykalny i nie trzeba do niego areny. Region nie przedłuża zasięgu nazwy. Nazwa umiera według bloków, a bajty umierają według areny, w której zostały położone. Te dwie rzeczy są złączone, ale nie są tym samym.

## Trzy sposoby przejścia granicy regionu

Wartość z regionu otaczającego można w regionie wewnętrznym użyć na trzy sposoby. Kompilator wybiera sposób według typu i według tego, czy nazwa jest stała, czy zmienna.

Kopiowanie dotyczy typów, które mieszczą się w wartości samej i nie są puste. Należą do nich `i32`, `bool` i pozostałe niepuste typy proste. Odczyt kopiuje bity. Nazwa w regionie zewnętrznym zostaje żywa.

Współdzielenie dotyczy stałej, której typ nie jest kopiowany, na przykład napisu wprowadzonego przez `val`. Region wewnętrzny patrzy na te same bajty. Nazwa zewnętrzna zostaje żywa. Jest to bezpieczne bez sprawdzania pożyczek, bo region wewnętrzny kończy się wcześniej niż region zewnętrzny, a język nie ma typu referencji, którą dałoby się zapisać w polu na dłużej.

Przeniesienie dotyczy zmiennej, której typ nie jest kopiowany, albo jawnego słowa `move`. Własność przechodzi na nowe użycie, a stara nazwa staje się martwa.

**Listing 2.2.** Stały napis jest w bloku wewnętrznym tylko oglądany.

```bork
fun main() {
    val s = "x"
    {
        println(s)
    }
}
```

Program został zbudowany. Wypisuje `x` i nowy wiersz. Drzewo regionów pokazuje w bloku wewnętrznym nazwę `s` ze znacznikiem współdzielenia z regionu `fun main`. Region wewnętrzny nie kopiuje znaków i nie odbiera własności.

**Listing 2.3.** Po przeniesieniu stara nazwa nie może być użyta.

```bork
fun main() {
    var s: String = "hi"
    val t = move s
    val u = s
}
```

Sprawdzenie kończy się błędem fazy `ownership`:

```text
err_use_after.bork:4:13: error: ownership: use of `s` after move from fun main
```

Przeniesienie nie wycina bajtów ze starej areny. Zostają one miejscem, do którego nie ma już dojścia, aż arena zostanie wyczyszczona. Kompilator unieważnia nazwę. To wystarcza, dopóki nie ma struktur, w których dałoby się przechować adres. Struktur definiowanych przez programistę nie ma.

Słowo `move` przy wartości kopiowanej nie odbiera nazwy. Program, który robi `val n = 1`, potem `val m = move n`, a potem zwraca `n + m`, przechodzi sprawdzenie i budowanie. Kod wyjścia wynosi 2. Drzewo regionów nie oznacza `n` jako przeniesionej.

## Pętla nie może przenieść nazwy z zewnątrz

Ciało pętli wykonuje się wiele razy. Własność zmiennej utworzonej przed pętlą da się oddać tylko raz. Analiza własności zapamiętuje nazwy widoczne przy wejściu do `for` albo `while` i zabrania przenosić je w ciele.

**Listing 2.4.** Przeniesienie napisu utworzonego przed pętlą jest błędem.

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

Kompilator wypisuje dwa komunikaty na tym samym miejscu. Pierwszy mówi, że `s` nie może być przeniesione wewnątrz pętli, bo przy kolejnym obiegu byłoby już przeniesione. Drugi mówi, że `s` nie jest kopiowane i trzeba je przenieść do bloku `move`. Dwa zdania wynikają z dwóch sprawdzeń, które widzą ten sam zapis. Nazwę utworzoną wewnątrz ciała pętli wolno przenieść, bo każdy obieg dostaje świeżą zmienną. Przykład jest w rozdziale 8.

Ciało pętli czyści swoją arenę przy każdym obiegu. Pamięć zajmowana przez wartości tymczasowe pętli nie rośnie razem z liczbą obiegów, o ile wynik nie jest zapisywany do zmiennej z zewnątrz. To jest osobna sprawa od zakazu przenoszenia. Poprawna pętla, która buduje tymczasowy napis i nie wynosi go na zewnątrz, zużywa stałą ilość pamięci regionu.

## Czego ten model świadomie nie robi

Dokument modelu pamięci wymienia rzeczy, których nie planuje. Nie będzie ogólnych referencji ani referencji zmiennych trzymanych w polach między regionami. Nie będzie niejawnego głębokiego kopiowania przy każdym odczycie spoza regionu. Nie będzie zapisywania wypożyczenia z regionu wewnętrznego do regionu zewnętrznego. Jedyna obserwacja krótsza niż właściciel to współdzielenie stałej z regionu otaczającego. Region wewnętrzny i tak jest w nim zagnieżdżony w tekście programu.

## Co już jest zrobione, a co jest odłożone

Bufor areny ma pojemność 4096 bajtów. Stała nazywa się `ARENA_CAPACITY` i leży w `crates/bork_runtime/src/lib.rs`. Taką samą pojemność ma model opisany w `src/arena.rs`. Wejście do regionu, który naprawdę alokuje, pobiera bufor z puli. Wyjście zwraca bufor do puli. Czyszczenie wskaźnika bez zwrotu bufora jest używane w pętli, żeby nie oddawać pamięci tylko po to, by za chwilę wziąć ją z powrotem. Przepełnienie bufora przerywa program komunikatem Rusta `arena overflow`, z podaną liczbą bajtów i pojemnością 4096. Nie jest to błąd, który program w Borku mógłby obsłużyć.

Odłożone są rzeczy opisane w tym samym dokumencie i widoczne w kodzie analizy ucieczki. Analiza ucieczki, po angielsku escape analysis, sprawdza, czy bajty wartości nie będą użyte po zwolnieniu areny, w której powstały. Dziś nie ma osobnego bufora na wynik zwracany do funkcji wywołującej, więc nie zwrócisz świeżo sklejonego napisu. Przeniesienie alokacji do zewnętrznej zmiennej rozpoznaje tylko bardzo prosty układ dwóch sąsiednich instrukcji. Słowa `promote` nie używa się przy `return`. Generator kodu nie pomija kopiowania pamięci nawet wtedy, gdy bajty już leżą we właściwej arenie.

> **NOTA.** Plik `src/arena.rs` opisuje bufor 4096 bajtów i pulę takich buforów. Analiza własności go nie wywołuje. Ta analiza buduje drzewo regionów. Prawdziwe bufory powstają w `bork_runtime`, gdy generator kodu wstawi wywołanie `bork_arena_push`. Czytając źródła, nie traktuj tych dwóch plików jako jednej struktury.

## Kompromis, który warto nazwać wprost

Region jest gruby. Napis i duża tablica utworzone w tym samym bloku dzielą jeden bufor 4096 bajtów i giną razem. Nie da się zwolnić napisu wcześniej. To jest cena braku `free` i braku garbage collectora. Stały rozmiar upraszcza generator kodu, bo region ma jeden wskaźnik końca danych i jedno czyszczenie. Gdy dane mają przeżyć blok, trzeba zapisać je do zmiennej z regionu zewnętrznego. Nie ma sterty, która żyłaby dłużej niż funkcja, poza literałami umieszczonymi w stałych programu.

Literał `"hi"` może w ogóle nie leżeć w arenie. Generator kodu kładzie znaki w stałej, a w wartości zostawia adres i długość. Koniec bloku nie unieważnia wtedy znaków. Mimo to nazwa po `move` jest martwa. Stały napis nie jest powodem, żeby używać nazwy po przeniesieniu.

## Podsumowanie

- Blok w nawiasach klamrowych jest regionem. Alokacja dopisuje bajty do bufora, a wyjście z bloku zwalnia bufor w całości.
- Arena w tej książce oznacza bufor regionu. W źródłach kompilatora podobna nazwa oznacza także osobny model tego bufora, którego analiza własności nie używa.
- Kopiowanie, współdzielenie i przeniesienie są trzema legalnymi sposobami użycia wartości spoza bieżącego regionu.
- Przeniesienie unieważnia nazwę. Nie porządkuje bajtów w starym buforze.
- Pętla nie przenosi nazwy utworzonej poza jej ciałem i czyści arenę ciała przy każdym obiegu.
- Pojemność jednego bufora wynosi 4096 bajtów. Przepełnienie przerywa program.
- Ogólnych referencji nie planuje się. Zwrot świeżo zbudowanego napisu z funkcji też jeszcze nie działa, bo brakuje bufora należącego do wywołującego.
