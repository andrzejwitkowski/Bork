# Rozdział 11. Funkcje wbudowane i sposób pisania programów

## Ten rozdział obejmuje

- dlaczego w języku nie ma modułów
- jakie funkcje wbudowane naprawdę istnieją
- jak pisać programy, które dają się zbudować
- jak pisać programy, które na razie tylko przechodzą sprawdzenie
- których zapisów kompilator nie przyjmie, choć wyglądają znajomo

## Modułów nie ma

Parser na poziomie programu oczekuje nowej linii albo słowa `fun`. Słowa `module` i `struct` są błędami składni. Nie ma `import`, ścieżek plików, widoczności ani osobnych jednostek kompilacji po stronie języka. Jeden plik zawiera jedną listę funkcji i jedną mapę nazw.

Słowo moduł wraca w tej książce tylko przy opisie dwóch skrzynek Rusta, `bork` i `bork_runtime`. Skrzynka, po angielsku crate, jest jednostką kompilacji w Cargo. To podział kodu kompilatora, nie mechanizm języka Bork. Serwer edytora i rozszerzenie w `tools/bork-lsp-extension` są narzędziami obok języka. Nie dodają składni.

## Cała biblioteka, która istnieje

W `src/builtins.rs` są trzy nazwy. Nic więcej nie jest zarejestrowane.

`print` wypisuje jedną wartość `i32`, `i64` albo napis i opróżnia bufor standardowego wyjścia. `println` robi to samo i dodaje nowy wiersz. `concat` przyjmuje dwa napisy i zwraca nowy napis. Bufor powstaje w arenie miejsca, do którego wynik jest zapisywany, a gdy takiego miejsca nie ma, w arenie bieżącego bloku.

Nie ma asercji, formatowania, czytania standardowego wejścia, plików, zegara ani argumentów wiersza poleceń. Funkcja `main` nie dostaje `argv`. Generator kodu odrzuca `main` z parametrami już przy deklaracji funkcji LLVM.

Biblioteka wykonawcza ma funkcje, których nie wołasz z Borka wprost. Należą do nich pobranie bufora areny, jego zwrot, wyczyszczenie wskaźnika, alokacja w buforze oraz wypisywanie liczb i napisów. Rozdział 17 wymienia ich nazwy w C, bo taki jest sposób połączenia wygenerowanego kodu z biblioteką wykonawczą.

## Jak pisać programy, które kompilator potrafi zbudować

Wynik liczbowy jako kod wyjścia jest wygodny, gdy ćwiczysz budowanie bez wypisywania. Funkcja `main`, która zwraca 7, została zbudowana i kończy się kodem 7. Tak samo robi test `builds_return_constant`.

Pętla z liczbowym akumulatorem nie walczy z własnością. Listing 6.3 jest wzorem. Indeks i suma są typu `i32`, więc ciało pętli je kopiuje.

Napis, który ma przeżyć blok, trzymaj w `main` i wypisz na końcu. Listing 9.1 pokazuje przypisanie literału do zmiennej z zewnątrz. Nie zwracaj świeżego napisu z funkcji pomocniczej, dopóki nie ma bufora należącego do wywołującego.

Stała napisowa, którą blok wewnętrzny tylko czyta, jest prostsza niż przenoszenie w tę i z powrotem. Listing 2.2 jest wzorem współdzielenia.

Gdy chcesz, żeby kompilator zbudował napis od razu w arenie zmiennej docelowej, napisz deklarację i w następnym wierszu przeniesienie. Nie wstawiaj między nie `println`.

Tablicy używaj z wycinkiem o stałych granicach, gdy długość jest znana w typie. Listing 7.4 to pokazuje. Wycinka o granicach trzymanych w zmiennych sprawdzanie typów nie przyjmie, bo typ wyniku nie miałby znanej długości.

Warunek, który ma dać liczbę, zapisuj z dwiema gałęziami-blokami. Listing 6.1 jest wzorem.

## Programy tylko do sprawdzenia

Pisz je, gdy ćwiczysz kompilator albo edytor. Nie wkładaj ich do programu, który ma być plikiem wykonywalnym.

Należą tu funkcja dopisana na końcu wywołania, blok `move` stojący po wywołaniu, `Some`, `None`, `?:`, `!!` i `?.`. Należy tu także próba trzymania funkcji w stałej i wywołania jej pośrednio. Osobno należy tu napis jako parametr funkcji użytkownika. Sprawdzenie go przyjmuje, a budowanie przerywa kompilator. W `bork build` trzymaj się od tej kombinacji z daleka.

**Listing 11.1.** Fragment próbki `PROCESS_USER_SAMPLE` z `src/lib.rs`. Cała próbka przechodzi sprawdzenie. Budowanie odrzuca `?:`, `Some` i `None`.

```bork
fun processUser(name: String?, score: i32): i32 {
    val fallbackName: String = name ?: "Guest"
    val verifiedUser: String? = Some(fallbackName)
    val emptyMiddle: String? = None
    val verifiedLength: i32 = verifiedUser?.length ?: 0
    if (verifiedLength > 0) {
        return score
    }
    return 0
}
```

Oryginalna próbka w `lib.rs` jest dłuższa. Ma dodatkową nazwę i zagnieżdżony blok. Też przechodzi sprawdzenie. Zostawiam ją w źródle kompilatora jako test, nie jako program do zbudowania.

## Zapisy, które wyglądają znajomo i są błędami

Dodawanie dwóch napisów operatorem `+` jest błędem typu. Operator nie skleja tekstu. `return -3` jest błędem składni, bo minus nie jest operatorem jednoargumentowym. `val n = 1;` jest błędem składni. Przypisanie `var x = s` przy zmiennej napisowej `s` prosi o `move`. Wywołanie `concat` na dwóch zmiennych napisowych wewnątrz bloku prosi o przeniesienie każdej z nich. `return concat(a, b)` odpada w analizie ucieczki. Indeks spoza tablicy kompiluje się i przerywa program. Dzielenie `1 / 0` kompiluje się i przerywa program. Druga deklaracja `println` jest błędem typu. Słowa `struct` i `module` są błędami składni na poziomie programu.

## Styl, który pasuje do regionów

Krótki blok robi jedną arenę na wartości tymczasowe. Długi blok funkcji trzyma dane, które mają przeżyć pętlę. Pętla nie powinna przenosić stanu utworzonego przed nią. Powinna czytać wartości kopiowane albo stałe współdzielone i zapisywać wynik do zmiennej z zewnątrz przez przypisanie.

Stałych używaj do progów i do napisów, które są tylko czytane. Zmiennych używaj do akumulatorów. To nie jest rada stylistyczna obok kompilatora. To jest różnica, którą analiza własności sprawdza.

Nie ma odpowiednika rustowego `clone`. Świadoma kopia napisu wymaga zbudowania nowego napisu, na przykład przez `concat`, gdy oba argumenty są w danym regionie legalne, albo przez przypisanie literału. Nie ma ogólnego kopiowania tablicy poza zbudowaniem nowego literału. Wycinek nie jest kopią.

## Podsumowanie

- Biblioteka języka to `print`, `println` i `concat`.
- Modułów, struktur i `import` nie ma. Program jest jednym plikiem.
- Program, który ma się zbudować, trzyma funkcje przy liczbach, napisy w `main`, stałe przy samym odczycie i zmienne przy akumulatorach.
- Brak wartości i funkcja dopisana na końcu wywołania są prawdziwą częścią sprawdzanego języka i nie są częścią `bork build`.
- Operator `+` na napisach, średnik, minus jednoargumentowy i gołe użycie zmiennej napisowej są błędami kompilacji.
