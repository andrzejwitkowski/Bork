# Dodatek B. Słowniczek

**Arena.** W runtime: płyta 4096 bajtów i offset. W semie: węzeł
`ArenaNode` w drzewie regionów. To nie jest ten sam obiekt.

**Arena pool.** Lista wolnych płyt w `bork_runtime`. `pop` oddaje płytę,
`push` ją bierze. `reset` nie oddaje.

**Assign-up.** Przypisanie do `var` z regionu przodka. Nie jest odczytem
lewej strony. Prawa strona może alokować w arenie celu.

**AST.** Drzewo z `src/ast.rs` po parserze, bez typów wywnioskowanych.

**Bramka (`gate`).** Przejście po HIR, które odrzuca konstrukcje spoza
podzbioru codegen, zanim powstanie moduł LLVM.

**Bump.** Alokacja przez przesunięcie offsetu. Brak zwalniania
pojedynczego obiektu.

**Check.** `frontend::check`: parse, typeck, sema, a przy braku błędów
także stempel, hoist i escape.

**Codegen.** Feature Cargo i faza diagnostyki. Obniżenie HIR do obiektu
LLVM i linkowanie.

**Copy.** Przejście granicy regionu dla nienullowalnego prymitywu. Źródło
zostaje.

**Deskryptor.** Wartość `{ ptr, len }` dla `String` i `[T; N]`. Bajty są
gdzie indziej.

**Escape.** Pass `escape::place`. Pilnuje, żeby bajty nie były użyte po
śmierci płyty. Diagnostyka ma fazę `ownership`.

**HIR.** Typowane drzewo. Nie niesie identyfikatora regionu.

**Hoist.** Rozpoznanie dwóch sąsiednich instrukcji i ustawienie
`alloc_in_binding`, żeby bajty powstały od razu w arenie celu.

**Local.** Nazwa zadeklarowana w tym regionie.

**Move.** Przeniesienie własności. Wiązanie źródłowe jest martwe. Bajty w
starej płycie zostają śmieciem aż do `reset`.

**Ownership.** Faza diagnostyki semy i escape. Nie jest osobnym passem o
tej nazwie w kodzie.

**Promote.** `promote nazwa` przy assign-up. Kopiuje bajty do areny
zewnętrznego `var` i unieważnia źródło.

**Region.** Blok, ciało funkcji, gałąź `if`, ciało pętli albo domknięcie,
o ile `peel_blocks` go nie skleił z otoczeniem.

**Shared.** Odczyt `val` nie-Copy z regionu rodzica. Dziecko nie kopiuje
bajtów i nie zabija nazwy.

**Sink.** Miejsce, które konsumuje wartość: cel przypisania, `return`,
albo wynik `concat`. Steruje, gdzie bump położy bajty wyniku.

**Span.** Para offsetów bajtowych w pliku źródłowym.

**UseKind.** Adnotacja HIR na identyfikatorze: Local, Shared, Copy, Move,
Promote. Nie zastępuje analizy semy.

**`codegen_push`.** Flaga na `ArenaNode`. LLVM woła `bork_arena_push`
tylko gdy jest ustawiona (korzeń funkcji zawsze).

**`decl_tys`.** Wektor typów deklaracji w kolejności źródła, przekazany z
typecku do semy.

**`loop_move_ban`.** Zbiór nazw, których nie wolno przenieść w ciele
pętli, bo były widoczne na wejściu.

**`peel_blocks`.** Zdjęcie zagnieżdżonych bloków, które zawierają tylko
jeden blok. Licznik ląduje w `compacted_braces`.

**`unit`.** Typ funkcji bez zadeklarowanego wyniku. Nie jest nazwą `Unit`.
