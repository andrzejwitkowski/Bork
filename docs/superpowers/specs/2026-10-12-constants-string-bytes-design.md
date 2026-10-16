# Globalne stałe i podstawowe operacje na Stringach

Status: projekt do przeglądu; dokument nie oznacza zgody na implementację.

## Cel

Przygotować Borka do buforów HTTP i przyszłych kolekcji: nazwane stałe,
odczyt bajtów tekstu, równość tekstowa, kopiowanie UTF-8 z bufora i wewnętrzne
hashowanie Stringów. Bez sieci, kolekcji ani dynamicznych widoków w tym etapie.

Podstawa: `d7826ad` na `feat/debug-refs-arenas`; sprawdzony `origin/main`
to `d82e79f`. Implementację rozpocząć na osobnym branchu, po ustaleniu bazy
względem PR #24. Nie przenosić do niego lokalnych, nieśledzonych artefaktów.

## 1. Globalne stałe

```bork
const PORT: i32 = 8080
const ENABLED: bool = true
const RESPONSE: String = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK"
```

- `const NAME: Type = literal` wyłącznie na poziomie pliku; typ obowiązkowy.
- Typy: istniejące typy całkowite, `f32`, `f64`, `bool`, `String`, bez nullable.
- Inicjalizatory: istniejące literały liczbowe, bool i String. Bez wyrażeń,
  odwołań do innych stałych, wywołań, klas i tablic. Obowiązują istniejące
  ograniczenia składni literałów; ten etap nie dodaje ujemnych literałów ani
  rozszerzenia zakresu parsera liczb całkowitych ponad obecne `i64`.
- Walidować zakres wartości dla zadeklarowanego typu całkowitego; nie obcinać
  bitów. Konwersja literału float do `f32` nie może produkować nieskończoności.
- Stała jest widoczna we wszystkich funkcjach i metodach pliku, również
  zadeklarowanych wcześniej. Nie powstaje kolejność inicjalizacji w runtime.
- Duplikaty oraz kolizje z funkcją, klasą i nazwą builtinu są błędami.
- Parametr/lokalna zmienna przesłania stałą. W metodzie zachować istniejące
  pierwszeństwo lokalnych nazw i pól `this` przed globalną stałą.
- Przypisanie, `move` i `promote` globalnej stałej są błędami. Zwykłe odczyty
  nie zużywają stałej. `&RESPONSE` jest dozwolone tam, gdzie oczekiwano `&String`;
  nie znosi to istniejących ograniczeń przechowywania i zwracania `&T`.
- Skalary są wartościami kompilacyjnymi. Bajty stałego Stringa są niemutowalne
  i żyją przez cały proces. Użycie stałej nie alokuje ich w arenie.
- Stałą String można przekazać i zwrócić jak literał. Kopiowanie/przenoszenie
  lokalnej zmiennej zainicjalizowanej stałą zachowuje zwykłe reguły lokalnego Stringa.
- HIR użycia zawiera typowaną wartość literalną ze spanem użycia; backend nie
  potrzebuje zmiennych globalnych ani inicjalizatorów wykonywanych przed `main`.
- LSP: rozpoznaje deklarację i użycia stałej, pokazuje typ/wartość w hover,
  respektuje przesłanianie. Kolorowanie obejmuje `const`.

Nie dodajemy stałych lokalnych, pól statycznych klas ani nazwanych symboli stałych
do DWARF. Zachowujemy prawidłowe lokalizacje źródłowe przy `-g`.

## 2. Odczyt i równość

- `s[i]` na `String` i `&String` zwraca `u8`. Indeks ma typ `i32`.
- Indeks oznacza bajt UTF-8, nie znak. `.length` pozostaje liczbą bajtów.
- Ujemny indeks i indeks równy/większy od długości powodują ten sam kontrolowany
  abort co indeksowanie tablic. Nie dodajemy zapisu do Stringa ani jego slice'ów.
- `==` i `!=` dla dwóch nie-nullowalnych Stringów porównują bajty i długości,
  nigdy adresy. Obsłużyć również pary String / `&String` i `&String` / `&String`
  bez kopiowania payloadu i bez wymagania `move` na potrzeby porównania.
- Istniejąca semantyka nullable, zwłaszcza `text == None`, pozostaje bez zmian.
  Nie rozszerzamy tym zadaniem równości dwóch wartości `String?`.
- Pusty String i tekst zawierający NUL są poprawne. Żadna operacja nie używa
  `strlen` ani innych konwencji terminatora C.

## 3. Kopiowanie zakresu bajtów do Stringa

Proponowane publiczne API:

```bork
// N jest istniejącą, dowolną stałą długością tablicy, nie parametrem generycznym języka.
// stringFromUtf8(buffer: &[u8; N], start: i32, count: i32): String?
var bytes: [u8; 3] = [71, 69, 84]
val method: String? = stringFromUtf8(&bytes, 0, 3)
```

- To builtin sprawdzający konkretny typ argumentu, bez ogólnych generyków.
- Walidować `start >= 0`, `count >= 0`, `start <= length`,
  `count <= length - start`; nie używać przepełniającego się `start + count`.
- Zły zakres: abort jak przy indeksowaniu tablic. Niepoprawny UTF-8: `None`.
- Poprawny zakres: kopia bajtów do aktywnego allocation sink, zgodnie z
  regułami `concat`; późniejsza modyfikacja bufora nie zmienia Stringa.
- Pusty poprawny zakres daje obecny pusty String, odróżnialny od `None`.
  To ważne, ponieważ obecna reprezentacja `String?` używa null pointer jako None.
- Walidacja UTF-8 następuje przed alokacją; `None` nie zwiększa zużycia areny.
- Przypisanie wyniku z wewnętrznego bloku do zewnętrznego `var` wybiera arenę
  docelowej zmiennej. Nowy builtin musi być widoczny dla hoist/escape i planowania
  regionów, nie tylko dla LLVM.
- Nie dodajemy caller-owned return buffers. Zwracanie nowo alokowanego Stringa
  lub jego nullable opakowania z funkcji ma być odrzucone, jeśli bajty nie
  przeżyją areny wywołania. Sprawdzić również pośrednią lokalną zmienną,
  `Some`, `!!`, `?:`, `if` i presence binding; zachować bezpieczne istniejące zwroty.

## 4. Runtime i hashowanie

Nowy moduł `crates/bork_runtime/src/text.rs`, bez zależności zewnętrznych:

- `bork_string_equal(left: *const u8, left_len: usize, right: *const u8,
  right_len: usize) -> u8`: wynik 0/1, bez alokacji.
- `bork_utf8_valid(bytes: *const u8, len: usize) -> u8`: wynik 0/1,
  używa `std::str::from_utf8`, bez alokacji.
- `bork_string_hash(bytes: *const u8, len: usize) -> u64`: FNV-1a 64,
  offset `14695981039346656037`, prime `1099511628211`, wrapping multiplication.
  Helper wewnętrznego ABI runtime, bez publicznego builtinu Borka w tym etapie.

Wszystkie trzy funkcje przy `len == 0` dopuszczają null pointer i nie tworzą
Rust slice z null. Przy dodatniej długości ABI wymaga poprawnego, czytelnego
zakresu; kompilator dostarcza go z deskryptora. ABI używa docelowego `usize`,
LLVM konwertuje istniejącą długość deskryptora `i64` z walidacją dopasowania.

FNV nie przesądza algorytmu przyszłej HashMapy. Mapa przyjmująca klucze sieciowe
będzie wymagać osobnej decyzji o ochronie przed kolizjami.

## Kryterium zakończenia

Stała odpowiedź HTTP działa z wielu funkcji/metod; skaner czyta bajty Stringa;
tekst z bufora porównuje się po zawartości; kopia przeżywa koniec regionu bufora,
jeśli trafia do zewnętrznego sink. Niepoprawny UTF-8 daje `None`, pusta kopia jest
obecna, złe zakresy nie czytają pamięci. Zwykły build, `-g`, LSP i istniejący korpus
przechodzą. Runtime nie zmienia Ref ABI ani normalnej polityki aren.
