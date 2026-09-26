# Przedmowa

Bork powstał jako odpowiedź na konkretne tarcie. Rust daje natywny kod i
bezpieczeństwo pamięci, ale lifetime'y i borrow checker są osobnym językiem,
którego uczy się dłużej niż samej składni. Kotlin daje czytelne bloki,
trailing closures i składnię, w której DSL prawie nie wygląda jak DSL. C daje
deterministyczny czas życia i zero garbage collectora, ale każdy `free` jest
osobną decyzją. Bork stawia te trzy obserwacje obok siebie i wybiera czwartą
drogę: **nawiasy klamrowe są regionami pamięci**.

Region nie jest lifetime'em dopiętym do jednej zmiennej. Jest pulą. Wszystko,
co w bloku alokuje bajty `String` albo bufor tablicy, żyje w tej puli. Wyjście
z bloku zeruje offset puli jednym ruchem. Nie ma GC, nie ma ogólnego `&` i
`&mut`, nie ma ręcznego `free`. Przepływ danych między pulami jest jawny:
wartość Copy się kopiuje, `val` typu nie-Copy dziecko może obserwować w
miejscu (Shared), a `var` trzeba przenieść (`move`) albo świadomie podnieść
(`promote`).

Ta książka jest przewodnikiem po języku **takim, jaki akceptuje dzisiejszy
frontend**, i po kompilatorze **takim, jaki leży w tym repozytorium**. To
rozróżnienie jest ważne. Parser i typeck przyjmują nullable, `?:`, `!!`,
`Some`, `None` i trailing closures. `bork build` obniża do kodu maszynowego
węższy podzbiór i przy kilku konstrukcjach, które frontend uważa za poprawne,
albo odmawia diagnostyką fazy `codegen`, albo — co gorsza — panikuje w
procesie kompilatora. Dodatki C wymienia te miejsca bez owijania.

Nazwa diagnostyk jest częścią projektu. Gdy program jest niepoprawny, komunikat
mówi, że kompilator *borks*. Fazy nazywają się `parse`, `ownership`, `type` i
`codegen`. Nie ma osobnego rozdziału o wyjątkach, bo język nie ma wyjątków,
`Result` ani `try`. Błąd programisty jest albo błędem kompilacji, albo
`abort` w czasie wykonania (dzielenie przez zero, indeks poza tablicą,
przepełnienie areny 4096 bajtów).

Książka jest po polsku. Terminy, które w kodzie i w komunikatach kompilatora
zostają po angielsku — borrow checker, lexer, arena, IR, HIR, move, Copy,
Shared — zostają po angielsku. Tłumaczenie `move` na „przenieś” w tekście
objaśniającym nie zmienia tego, co trzeba napisać w pliku `.bork`.

Jeśli czytasz to jako autor kompilatora albo jako ktoś, kto chce dopisać
kolejną konstrukcję, część III jest właściwym wejściem. Jeśli chcesz najpierw
napisać program, zacznij od części I i trzymaj się programów, które
`bork build` naprawdę linkuje. Bramka codegen jest węższa niż gramatyką, i
książka powtarza to za każdym razem, gdy różnica ma znaczenie.
