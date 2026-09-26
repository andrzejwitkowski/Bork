# Rozdział 7. Tablice o znanej z góry długości

## Ten rozdział obejmuje

- jak zapisać typ tablicy i literał tablicy
- czym różni się odczyt elementu od wycinka o stałych granicach
- jak czytać długość
- jak własność dotyczy całej tablicy i pojedynczego napisu w tablicy
- co dzieje się w czasie działania, gdy indeks nie mieści się w tablicy

## Dlaczego długość należy do typu tablicy

Tablicę zapisuje się jako `[T; N]`. `T` jest typem elementu, a `N` jest liczbową długością. Długość nie jest wyrażeniem liczonym w czasie działania. Jest częścią typu, tak jak szerokość liczby `i32` jest częścią typu, a nie wartością trzymaną obok liczby. Tablica trzech liczb `i32` ma inny typ niż tablica dwóch liczb `i32`. Nie przypiszesz jednej do drugiej.

`N` w zapisie typu musi być literałem całkowitym. Za duża liczba, która nie mieści się w `u32`, daje błąd gramatyki `array length is too large`. Pusta tablica nie ma skąd wziąć ani typu elementu, ani długości.

**Listing 7.1.** Pusta tablica z jawnym typem. Program został zbudowany. Długość wynosi zero, więc kod wyjścia też wynosi zero.

```bork
fun main(): i32 {
    val a: [i32; 0] = []
    return a.length
}
```

Bez adnotacji zapis `val a = []` daje komunikat, że pusty literał wymaga jawnego typu, i podaje przykład `val a: [i32; 0] = []`.

Elementy literału muszą mieć jeden typ, a ich liczba musi równać się `N`.

**Listing 7.2.** Trzy elementy w tablicy, która według typu ma dwa.

Sprawdzenie wypisuje dwa błędy fazy `type`. Pierwszy mówi, że literał ma 3 elementy, a oczekiwano 2. Drugi mówi, że inicjalizator nazwy `a` ma typ `[i32; 3]`, a oczekiwano `[i32; 2]`. Źródło to `val a: [i32; 2] = [1, 2, 3]`.

Dozwolony element to wartość kopiowana albo niepusty napis. Napis, który może być pusty, nie może być elementem. Tablica, która sama może być pusta w sensie `?`, też jest odrzucana.

Tablica nie jest kopiowana, nawet gdy jej elementy są. Przypisanie całych tablic wymaga identycznego typu elementu i identycznej długości.

## Indeks

Indeks jest typu `i32` i jest liczony dopiero przy uruchomieniu, więc nie wchodzi do typu i sprawdzenie nie odrzuca `a[9]` na tablicy trzyelementowej. Taki program został zbudowany: plik wykonywalny powstaje, a uruchomienie kończy się sygnałem przerwania, z kodem powłoki 134, bo generator kodu wstawia sprawdzenie zakresu i woła `abort`.

**Listing 7.3.** Zapis elementu i wycinek, który widzi ten sam bufor. Program został zbudowany i kończy się kodem 12.

```bork
fun main(): i32 {
    var a: [i32; 3] = [1, 2, 3]
    a[1] = 9
    val b = a[0..2]
    return b[1] + a.length
}
```

Po zapisie pod indeksem 1 leży 9. Wycinek od zera do dwóch, bez dwójki, obejmuje dwa pierwsze elementy i nie kopiuje ich do nowego bufora. Dlatego `b[1]` też jest 9. Długość `a` wynosi 3. Suma wynosi 12 i tyle wynosi kod wyjścia.

Nazwa wprowadzona przez `val` nie przyjmuje zapisu elementu. Komunikat jest ten sam co przy zwykłym przypisaniu do stałej: nie można przypisać do niezmiennej nazwy `a`. Indeks złego typu daje `array index must be i32` albo, przy zapisie, informację, że indeks ma inny typ, a oczekiwano `i32`.

## Wycinek o stałych granicach

Zapis `a[początek..koniec]` wymaga, żeby obie granice były literałami typu `i32`. Typ wyniku to tablica o długości `koniec - początek`. Wycinek nie kopiuje bufora. Nowy opis tablicy wskazuje w środek starej, a długość bierze z typu.

Wycinek `[1..9]` na tablicy `[i32; 3]` jest błędem typu: `slice [1..9] is out of bounds`. Granice, które nie są literałami, dają komunikat, że granice muszą być literałami całkowitymi, żeby typ wyniku miał znaną długość. Generator kodu nie wstawia drugiego sprawdzenia wycinka w czasie działania. Ufa sprawdzeniu typów. Indeks jest sprawdzany w czasie działania, bo nie da się go wpisać do typu.

**Listing 7.4.** Wycinek i jego długość. Program został zbudowany. Na wyjściu są wiersze `20` oraz `2`.

```bork
fun main() {
    val a = [10, 20, 30]
    val b = a[0..2]
    println(b[1])
    println(b.length)
}
```

Analiza ucieczki traktuje wycinek jak tablicę, z której powstał. Wycinek nie może przeżyć areny, która trzyma bufor. Test `rejects_slice_escaping_inner_region` oczekuje fazy `ownership` i tekstu `inner region`.

## Napis jako element

Odczyt `a[i]`, gdy element jest napisem, jest oglądaniem napisu leżącego w buforze tablicy. Nie ma przeniesienia jednego elementu. Słowa `move` i `promote` dotyczą całej tablicy albo, przy zapisie do elementu, wartości po prawej stronie.

Zapis elementu, który nie jest kopiowany, wymaga `move` albo `promote` po prawej stronie. Arena tablicy jest miejscem, w którym lądują bajty nowego elementu.

**Listing 7.5.** Zapis przeniesionego napisu do tablicy. Program został zbudowany. Na wyjściu jest `z` oraz nowy wiersz.

```bork
fun main() {
    var a: [String; 2] = ["a", "b"]
    var s = "z"
    a[0] = move s
    println(a[0])
}
```

Bez `move` sprawdzenie mówi, że własność trzeba przenieść. Po `move` nazwa `s` jest martwa. W drzewie regionów `s` jest oznaczone jako przeniesione z regionu funkcji `main`.

## Jak tablica wygląda w gotowym programie

W reprezentacji LLVM tablica i napis mają ten sam kształt. Jest to para: adres bufora i długość zapisana na 64 bitach. Dla tablicy długość w tej parze równa się `N` z typu. Bajty elementów leżą w arenie, w której tablica powstała, albo w arenie zmiennej, do której tablica jest od razu zapisywana.

Pole `length` w języku ma typ `i32`. Generator kodu wycina długość z pary i obcina ją do 32 bitów. Dopóki `N` mieści się w `i32`, wynik jest zgodny z typem. Gramatyka przyjmuje większe `N`, bo długość w typie jest liczbą `u32`. Testy nie sprawdzają tablicy dłuższej niż maksymalna wartość `i32`. Nie zakładaj, że pole `length` takiej tablicy jest dobrze określone.

## Podsumowanie

- Długość tablicy jest częścią jej typu. Literał musi mieć dokładnie tyle elementów, ile mówi typ.
- Pusty literał `[]` wymaga adnotacji typu.
- Indeks spoza zakresu przerywa program. Zły wycinek jest błędem kompilacji.
- Wycinek o stałych granicach wskazuje w istniejący bufor i ma typ tablicy o długości równej różnicy granic.
- Element napisowy czyta się bez kopiowania. Zapis takiego elementu wymaga `move`.
- Całą tablicę wolno przenieść tylko do tablicy o tym samym typie elementu i tej samej długości.
