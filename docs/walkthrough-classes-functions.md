# Klasy i funkcje: przejście przez kompilator od źródła do binarki

Ten dokument prowadzi jeden mały program przez cały kompilator Borka: parser,
AST, typeck i HIR, analizę własności, analizę pamięci, bramkę codegenu, LLVM
i linkowanie. Na każdym etapie pokazuje, który plik i która funkcja pracują
oraz jak wygląda stan pośredni dla przykładu.

Szczegóły `RegionVisitor` i `MemoryPlan` opisuje
[memory-region-visitor.md](memory-region-visitor.md). Model aren opisuje
[memory-model.md](memory-model.md).

## Przykładowy program

Program leży w korpusie jako
[`programs/build/classes/walkthrough_point.bork`](../programs/build/classes/walkthrough_point.bork),
więc `cargo test --features codegen` buduje go i sprawdza kod wyjścia.

~~~bork
// exit: 8
class Point {
    name: String
    x: i32
    y: i32
}

fun sum(p: &Point): i32 {
    return p.x + p.y
}

fun main(): i32 {
    val p = Point("a", 3, 4)
    return sum(&p) + p.name.length
}
~~~

Program używa:

- deklaracji klasy z polami `String` i `i32`,
- funkcji top-level z parametrem pożyczonym `&Point`,
- konstruktora `Point(...)`, który składniowo jest zwykłym wywołaniem,
- odczytu pól przez `.` (`p.x`, `p.name.length`),
- wywołania funkcji z borrowem `&p`.

Klasy w tym branchu mają tylko pola. Metod klasowych nie ma, więc operacje na
klasie są funkcjami top-level, tak jak `sum`.

## Mapa pipeline'u

~~~text
src/main.rs                     bork build plik.bork
└── frontend::check             src/frontend.rs
    ├── parse                   src/lib.rs + src/parser.lalrpop  -> ast::Program
    ├── typeck::check           src/typeck/                      -> HirProgram + SpanTypes
    ├── sema::analyze_with_span_tys  src/sema/                   -> ArenaReport (+ błędy własności)
    ├── hoist::annotate         src/hoist.rs                     (mutuje HIR)
    ├── escape::check_function  src/escape.rs                    (tylko diagnostyki)
    ├── memory::plan            src/memory.rs                    -> MemoryPlan
    └── region_walk::stamp_codegen_push                          (stempluje ArenaReport)
└── codegen::build              src/codegen/mod.rs
    ├── gate                    src/codegen_gate.rs              (co LLVM jeszcze nie umie)
    ├── llvm::emit_module       src/codegen/llvm/                -> inkwell Module
    ├── llvm::write_object      -> plik .o
    └── link::link_executable   src/codegen/link.rs              clang + bork_runtime
~~~

`frontend::check` jest wspólny dla `bork plik.bork` (tylko sprawdzenie),
`bork build`, LSP i testów. Kolejność jest ważna: HIR trafia dalej tylko wtedy,
gdy typeck i sema nie zgłosiły żadnej diagnostyki.

~~~rust
let (hir, span_tys, mut type_diagnostics) = typeck::check(&program);
let (mut report, ownership_errors) =
    crate::sema::analyze_with_span_tys(&program, span_tys);
...
let mut hir = diagnostics.is_empty().then_some(hir);
~~~

## 1. Parser: tekst → `ast::Program`

Wejście to `bork::parse` w `src/lib.rs`:

~~~rust
pub fn parse(source: &str) -> Result<Program, Error> {
    let normalized = normalize_parenthesized_newlines(source);
    let input = normalized.as_deref().unwrap_or(source);
    parser::ProgramParser::new()
        .parse(input)
        ...
}
~~~

`normalize_parenthesized_newlines` (`src/layout.rs`) usuwa nowe linie wewnątrz
nawiasów. Gramatyka jest w LALRPOP (`src/parser.lalrpop`). Koniec linii to
token `"NL"`, więc nowa linia rozdziela itemy, pola klasy i statementy.

### Itemy najwyższego poziomu

~~~text
Item: Item = {
    <class:ClassDecl> => Item::Class(class),
    <f:Function> => Item::Fun(f),
};
~~~

`Program::from_items` (`src/ast.rs`) rozdziela itemy na dwie listy:
`classes` i `functions`. Kolejność deklaracji między klasami i funkcjami nie ma
później znaczenia.

### Klasa

~~~text
ClassDecl: Class = {
    "class" <name:Ident> "{" <fields:ClassBody> "}" => Class { name, fields },
};

ClassFieldDecl: ClassField = {
    <name:SpannedIdent> ":" <ty:Type> => ClassField { name, ty },
};
~~~

`ClassBody` i `ClassFieldRest` zbierają pola rozdzielone `"NL"`. Nazwa pola
jest `SpannedName`, bo typeck i LSP raportują błędy na pozycji pola.

### Funkcja

~~~text
Function: Function = {
    "fun" <name:Ident> "(" <params:Comma<Param>?> ")" <ret:ReturnType> <body:Block> => ...
};

ReturnType: Type = {
    ":" <ty:Type> => ty,
    => Type::unit(false),
};
~~~

Brak `: Typ` oznacza `unit`. Parametr bez `val`/`var` dostaje
`BindingKind::Val`. Typ `&Point` parsuje reguła `"&" <inner:Type>` jako
`Type::Ref { inner: Named("Point") }`.

### Wywołanie i pole

Konstruktor nie ma osobnej reguły. `Point("a", 3, 4)` to zwykły sufiks
wywołania:

~~~text
ExprSuffix: Expr = {
    <callee:ExprSuffix> "(" <args:Comma<Expr>?> ")" <trailing:TrailingClosure?> => Expr::Call { ... },
    ...
    <start:@L> <receiver:ExprSuffix> <safe:FieldOp> <name:Ident> <end:@R> => Expr::Field { ... },
    ...
};
~~~

Rozróżnienie „konstruktor czy funkcja” zapada dopiero w typecku, który zna
tablicę klas.

### Wynikowe AST

~~~text
Program
├── classes
│   └── Class Point
│       ├── name: Primitive String
│       ├── x:    Primitive i32
│       └── y:    Primitive i32
└── functions
    ├── Function sum(p: Ref<Named Point>): i32
    │   └── Return
    │       └── Binary(+)
    │           ├── Field(Ident p, "x")
    │           └── Field(Ident p, "y")
    └── Function main(): i32
        ├── VarDecl val p = Call(Ident Point, [Str "a", Int 3, Int 4])
        └── Return
            └── Binary(+)
                ├── Call(Ident sum, [Unary Borrow (Ident p)])
                └── Field(Field(Ident p, "name"), "length")
~~~

AST nie zna jeszcze typów wyrażeń. `Point(...)` i `sum(...)` są tym samym
wariantem `Expr::Call`.

## 2. Typeck: AST → typowany HIR

Wejście to `typeck::check` w `src/typeck/mod.rs`. Pracuje w czterech krokach:

~~~rust
reject_builtin_redefines(program, &mut diagnostics);
let classes = build_classes(program, &mut diagnostics);
validate_owned_layouts(&classes, &mut diagnostics);
let fun_sigs = collect_fun_sigs(program, &classes, &mut diagnostics);
// potem check_function dla każdej funkcji
~~~

### 2a. `build_classes`: tablica klas

`build_classes` robi dwa przejścia. Pierwsze rejestruje każdą nazwę z pustą
listą pól, więc pole może wskazywać klasę zadeklarowaną później. Drugie
wypełnia pola i sprawdza:

- duplikat klasy i duplikat pola,
- kolizję z nazwą typu wbudowanego (`String`, `i32`, ...),
- pole typu borrow (`&T`), które jest odrzucane:

~~~rust
if field.ty.is_reference() {
    diagnostics.push(type_error(
        "borrowed references cannot be stored in class fields",
        Some(field.name.span),
    ));
}
~~~

Typ AST zamienia się na typ HIR przez `lower::lower_type`
(`src/typeck/lower.rs`). Po tym kroku:

~~~text
ClassTable
└── Point
    ├── [0] name: String   (span pola)
    ├── [1] x:    i32
    └── [2] y:    i32
~~~

Indeks pola jest tu ustalony raz na zawsze. Codegen użyje go jako indeksu GEP.

`validate_owned_layouts` odrzuca rekurencyjny układ przez wartość, np. klasę,
która przez pola (bezpośrednio lub pośrednio) zawiera samą siebie. Komunikat
podpowiada `Ref<T>`.

### 2b. `collect_fun_sigs`: sygnatury

Każda funkcja dostaje `FunSig { params, return_ty }` z typami HIR. Do mapy
trafiają też sygnatury builtinów (`crate::builtins::signatures()`). Klasa
o tej samej nazwie co funkcja to błąd, bo obie są wywoływane tą samą składnią.

~~~text
fun_sigs
├── sum:  (&Point) -> i32
├── main: () -> i32
└── println, concat, ... (builtiny)
~~~

Sygnatury są zebrane przed sprawdzaniem ciał, więc funkcje mogą wywoływać się
w dowolnej kolejności deklaracji.

### 2c. `check_function`: ciało funkcji

~~~rust
let mut env = Env::new(fun_sigs, classes);
let params = bind_params(function, signature.params, &mut env);
let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
if return_ty != Ty::unit() && !stmt::block_always_returns(&body) {
    env.error(...);
}
~~~

`bind_params` wiąże `p: &Point` w `Env`. Parametr referencyjny musi być `val`.
Potem `check_block` przechodzi statementy, a wyrażenia trafiają do
`typeck::expr::check`.

### 2d. `Point("a", 3, 4)`: konstruktor

`check_call` (`src/typeck/expr/call.rs`) najpierw sprawdza, czy callee to nazwa
klasy:

~~~rust
if let Expr::Ident { name, span } = callee {
    if let Some(class) = env.class(name).cloned() {
        return check_construct(class, *span, args, trailing, return_ty, env);
    }
}
~~~

`check_construct` (`src/typeck/expr/object.rs`) dopasowuje argumenty do pól
pozycyjnie. Każdy argument dostaje oczekiwany typ pola, a za dużo argumentów
lub brak argumentu to błąd. Brakujące pole `Ref<T>` dostaje `None`. Wynik:

~~~text
HirExpr {
    kind: ObjectConstruct {
        class_name: "Point",
        fields: [Str "a": String, Int 3: i32, Int 4: i32],
    },
    ty: Point,
    span: Some(span konstruktora),
}
~~~

Span konstruktora jest ważny. `memory::plan` i codegen używają go jako
identyfikatora alokacji.

### 2e. `sum(&p)`: wywołanie funkcji

Ta sama funkcja `check_call` idzie dalej, do `callee_signature`, które
znajduje `sum` w `fun_sigs`. Każdy argument jest sprawdzany względem
parametru. Kluczowe reguły dla klas:

- parametr `&Point` wymaga argumentu w postaci `&…`
  (`argument N to sum expects borrow`),
- borrow przekazany do parametru posiadanego to błąd
  (`parameter expects owned ... use move`),
- typ musi pasować (`arg_matches_param`).

`&p` sprawdza `check_borrow` (`src/typeck/expr/borrow.rs`) i daje
`Unary { op: Borrow }` typu `&Point`. Wynik wywołania:

~~~text
Call {
    callee: Ident "sum" : (&Point) -> i32,
    args:   [Unary Borrow (Ident p : Point) : &Point],
} : i32
~~~

### 2f. `p.x` i `p.name.length`: pola

`check_field` (`src/typeck/expr/field.rs`) rozpoznaje dwa przypadki:

1. `length` na `String` lub tablicy daje `HirExprKind::Field` typu `i32`.
2. Pole klasy jest wyszukiwane w tablicy klas i daje `ObjectField` z indeksem
   pola.

Receiver może być `Point`, `&Point` albo `Ref<Point>`. Dla `Ref<Point>` zwykła
kropka jest błędem: potrzebny jest `if val`, `when` albo `?.`.

~~~text
p.x            -> ObjectField { receiver: p : &Point, class_name: "Point", field_index: 1 } : i32
p.y            -> ObjectField { ..., field_index: 2 } : i32
p.name         -> ObjectField { receiver: p : Point, field_index: 0 } : String
p.name.length  -> Field { receiver: <p.name>, name: "length" } : i32
~~~

### Wynikowy HIR

`typeck::check` składa `HirProgram { classes, functions }` (`src/hir/mod.rs`).
Każde `HirExpr` niesie `ty` i `span`. Oprócz HIR typeck zwraca `SpanTypes`,
czyli typy deklaracji według spanów, z których korzysta sema.

## 3. Sema: własność i drzewo aren

Sema (`src/sema/`) pracuje na AST, nie na HIR, ale dostaje typy z typecku:

~~~rust
pub fn analyze_with_span_tys(program: &Program, span_tys: SpanTypes)
    -> (ArenaReport, Vec<SemaError>)
~~~

Dla każdej funkcji zapisuje rodzaje parametrów i ich typy, a potem otwiera
region funkcji przez `open_ordinary` (`src/sema/walk.rs`). Parametr `&Point`
dostaje `borrow_from: REF_PARAM_BORROW_FROM`, czyli `"call site"`: borrow jest
aktywny przez całe ciało, a właściciel żyje u wywołującego.

Sema pilnuje m.in.:

- czy po `move` kod nie czyta przeniesionej nazwy,
- czy borrow nie przeżywa właściciela,
- jak rozgałęzienia łączą stan przeniesienia (PR 22: `if` bez `else` też
  zostawia nazwę jako przeniesioną).

`p` w `main` jest tylko pożyczane (`&p`) i potem czytane (`p.name`), więc
nie ma błędu. Wynik widać przez `--dump-arenas`:

~~~text
$ bork --dump-arenas programs/build/classes/walkthrough_point.bork
Arenas
├── fun sum
│   └── p [Borrow ← call site]
└── fun main
    └── p [Local]
~~~

`ArenaReport.roots` to drzewo regionów i zmiennych. Codegen przechodzi po tym
samym drzewie przez `RegionVisitor`, więc kształt regionów jest wspólny dla
analizy i emisji.

Gdyby `sum` przyjmowało `p: Point` i wywołanie brzmiało `sum(move p)`,
późniejsze `p.name` dałoby błąd:

~~~text
error: ownership: use of `p` after move from fun main
~~~

## 4. Hoist, escape, memory plan

Te kroki działają na HIR i tylko wtedy, gdy wcześniej nie było błędów.

### `hoist::annotate`

`src/hoist.rs` szuka inicjalizatorów, których alokację można od razu umieścić
w arenie zewnętrznego wiązania (`alloc_in_binding`). W przykładzie nie ma
takiego wzorca, więc HIR się nie zmienia.

### `escape::check_function`

`src/escape.rs` odrzuca wartości `String`, których bajty przeżyłyby arenę,
w której leżą. Pole `name: String` jest kopiowane do areny obiektu przy
konstrukcji (zobacz krok 6), więc nic nie ucieka.

### `memory::plan`

`src/memory.rs` przechodzi po HIR przez `region_walk::walk_program_readonly`.
`Planner` dostaje `after_expr` dla każdego `ObjectConstruct` i zakłada
alokację:

~~~text
ArenaPlan {
    allocation_site: span `Point("a", 3, 4)`,
    class_name: "Point",
    class: Lexical,
    lifetime_domain: LexicalScope(main),
}
~~~

Potem zmienia ją ten fragment dla wywołań:

~~~rust
HirExprKind::Call { args, .. } => {
    for argument in args.iter().filter(|argument| {
        argument.ty.record_name().is_some() || argument.ty.is_managed_ref()
    }) {
        // ponytail: calls have no escape summaries, so record arguments are program-lived.
        ...
~~~

`&p` ma typ `&Point`, a `record_name` przechodzi przez `Ref`, więc argument
liczy się jako rekord. Wywołania nie mają jeszcze podsumowań ucieczki, więc
planner konserwatywnie oznacza obiekt jako dynamiczny i żyjący na poziomie
programu:

~~~text
ArenaPlan { class: Dynamic, lifetime_domain: Program, ... }
~~~

To jest świadome uproszczenie. Widać je w IR w kroku 6 jako
`bork_arena_create_dynamic` podpięte pod `bork_arena_root`.

Na koniec `region_walk::stamp_codegen_push` stempluje w `ArenaReport`, które
regiony codegen musi naprawdę otworzyć jako areny.

## 5. Bramka codegenu

`codegen::build` (`src/codegen/mod.rs`) zaczyna od `gate(hir)`
(`src/codegen_gate.rs`). Bramka odrzuca konstrukcje, które frontend akceptuje,
ale LLVM jeszcze nie umie wyemitować, np. nieobsługiwane typy nullable. Błąd
bramki kończy `bork build` diagnostyką i bez binarki.

Klasa z polami `String`/`i32`, borrow parametru i odczyt pól przechodzą bramkę
bez uwag.

## 6. LLVM: `emit_module`

`llvm::emit_module` (`src/codegen/llvm/mod.rs`) wymaga `fun main`, a potem:

~~~rust
let cx = Codegen::new(context, "bork", &hir.classes, memory_plan)?;
let callees = hir.functions.iter()
    .map(|function| { let value = emit_fn::declare_function(&cx, function)?; ... })
    .collect()?;
let mut regions = RegionEmitter::new(report, ArenaCalls::new(&cx));
for function in &hir.functions {
    emit_fn::emit_function(&cx, &callees, &mut regions, &report.roots, function)?;
}
regions.finish()?;
cx.module.verify()?;
~~~

### 6a. Typ klasy

`Codegen::new` (`src/codegen/llvm/context.rs`) tworzy dla każdej klasy
nazwaną strukturę `bork.<Nazwa>` i zapamiętuje typy pól. Kolejność pól to
kolejność z deklaracji, czyli indeksy z typecku:

~~~llvm
%bork.Point = type { { ptr, i64 }, i32, i32 }
;                    name: String  x    y
~~~

`String` to para `{ ptr, i64 }`: wskaźnik na bajty i długość.

Obiekt klasy nie jest przekazywany przez wartość. Wartość typu `Point` w
rejestrach to uchwyt `object_handle_type`:

~~~text
{ ptr arena_control, ptr object }   // arena, w której leży obiekt + wskaźnik na struct
~~~

### 6b. Deklaracje funkcji i ABI

`declare_function` (`src/codegen/llvm/emit_fn.rs`) wybiera ABI przez
`CallAbi::of` (`src/codegen/llvm/call_abi.rs`):

| Funkcja | ABI | Ukryte parametry |
|---|---|---|
| `main` | `Main` | brak, symbol C `main`, zwraca `i32` |
| zwraca rekord, `String` lub `Ref<T>` | `OwningReturn` | arena rodzica + result sink |
| pozostałe | `Callee` | arena rodzica |

`sum` zwraca `i32`, więc dostaje jeden ukryty parametr: arenę wywołującego.
Funkcje inne niż `main` mają linkage `internal` i prefiks `bork.`, żeby nie
kolidować z libc ani runtime'em.

~~~llvm
define internal i32 @bork.sum(ptr %0, { ptr, ptr } %1)
;                             arena   p: &Point jako uchwyt obiektu
define i32 @main()
~~~

### 6c. Ciało `main`: konstrukcja obiektu

`emit_function` ustawia blok `entry` i oddaje sterowanie
`region_emit::emit_function_body`, który przechodzi HIR tym samym
`RegionVisitor` co planner. Region funkcji otwiera arenę
(`bork_arena_push`).

Przed dziećmi `ObjectConstruct` wywoływany jest `before_expr`
(`src/codegen/llvm/region_emit.rs`), który woła `begin_object_construct`
(`src/codegen/llvm/managed_ref.rs`). Ta funkcja czyta `ArenaPlan` po spanie:

~~~rust
let allocation = self.cx.memory_plan.allocation_at(span)?;
let arena = if allocation.class == ArenaClass::Dynamic {
    // Program -> rodzic to bork_arena_root, potem bork_arena_create_dynamic
    // i bork_arena_register_strong_root(owner, arena)
} else ...
~~~

Potem emitowane są argumenty pól, a na końcu `emit_object_construct`
alokuje struct w wybranej arenie i zapisuje pola przez GEP. Pole `String`
jest kopiowane do areny obiektu przez `materialize_owned_value`, więc literał
`"a"` nie jest współdzielony z obiektem:

~~~llvm
%arena          = call ptr @bork_arena_push()
%arena.root     = call ptr @bork_arena_root()
%arena.dynamic  = call ptr @bork_arena_create_dynamic(ptr %arena.root, i8 0)
call void @bork_arena_register_strong_root(ptr %arena, ptr %arena.dynamic)
%object = call ptr @bork_arena_alloc(ptr %arena.dynamic, i64 <sizeof Point>, i64 <alignof Point>)

%field    = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 0   ; name
%move.dst = call ptr @bork_arena_alloc(ptr %arena.dynamic, i64 1, i64 1)
call void @llvm.memcpy.p0.p0.i64(ptr align 1 %move.dst, ptr align 1 @str, i64 1, i1 false)
%moved    = insertvalue { ptr, i64 } { ptr @str, i64 1 }, ptr %move.dst, 0
store { ptr, i64 } %moved, ptr %field

%field1 = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 1   ; x
store i32 3, ptr %field1
%field2 = getelementptr inbounds nuw %bork.Point, ptr %object, i32 0, i32 2   ; y
store i32 4, ptr %field2

%object.arena = insertvalue { ptr, ptr } zeroinitializer, ptr %arena.dynamic, 0
%object.ptr   = insertvalue { ptr, ptr } %object.arena, ptr %object, 1
store { ptr, ptr } %object.ptr, ptr %p
~~~

`val p` jest slotem `alloca` z uchwytem `{ arena, object }`. Sam obiekt leży
w arenie, nie na stosie.

### 6d. Wywołanie `sum(&p)`

Borrow klasy przekazuje ten sam uchwyt. Pierwszym argumentem jest ukryta arena
wywołującego:

~~~llvm
%p4   = load { ptr, ptr }, ptr %p
%call = call i32 @bork.sum(ptr %arena, { ptr, ptr } %p4)
~~~

### 6e. Ciało `sum`: odczyt pól

`sum` otwiera własną arenę jako dziecko areny wywołującego
(`bork_arena_push_child`). `ObjectField` to `emit_object_field`: wyciąga
wskaźnik obiektu z uchwytu (`object_pointer`), robi GEP z `field_index`
z typecku i ładuje wartość:

~~~llvm
define internal i32 @bork.sum(ptr %0, { ptr, ptr } %1) {
entry:
  %p     = alloca { ptr, ptr }
  %arena = call ptr @bork_arena_push_child(ptr %0)
  store { ptr, ptr } %1, ptr %p
  %p1         = load { ptr, ptr }, ptr %p
  %object.ptr = extractvalue { ptr, ptr } %p1, 1
  %field      = getelementptr inbounds nuw %bork.Point, ptr %object.ptr, i32 0, i32 1  ; p.x
  %field2     = load i32, ptr %field
  ...                                                                                  ; p.y
  %add = add i32 %field2, %field6
  call void @bork_arena_pop(ptr %arena)
  ret i32 %add
}
~~~

`bork_arena_pop` przed `ret` zamyka arenę funkcji. Obiekt `Point` przeżywa,
bo leży w arenie `main`.

### 6f. `p.name.length` i powrót z `main`

`p.name` to GEP na pole 0 i load `{ ptr, i64 }`. `.length` to
`extractvalue ..., 1` i `trunc` do `i32`:

~~~llvm
%field7   = getelementptr inbounds nuw %bork.Point, ptr %object.ptr6, i32 0, i32 0
%field8   = load { ptr, i64 }, ptr %field7
%len      = extractvalue { ptr, i64 } %field8, 1
%len.i32  = trunc i64 %len to i32
%add      = add i32 %call, %len.i32
call void @bork_arena_pop(ptr %arena)
ret i32 %add
~~~

Wynik to `3 + 4 + 1 = 8`.

## 7. Obiekt i linkowanie

`write_object` ustawia triple i data layout maszyny docelowej i zapisuje plik
`.o`. Gdy ustawiona jest zmienna `BORK_DUMP_IR`, najpierw zapisuje tekstowy IR.
Fragmenty IR w tym dokumencie pochodzą właśnie z tego zrzutu.

`link::link_executable` (`src/codegen/link.rs`) woła `clang` z obiektem,
biblioteką `bork_runtime` (funkcje `bork_arena_*`) i libc.

## 8. Uruchomienie

~~~text
$ cargo build --features codegen
$ BORK_DUMP_IR=/tmp/point.ll ./target/debug/bork build -o /tmp/point \
      programs/build/classes/walkthrough_point.bork
$ /tmp/point; echo $?
8
$ ./target/debug/bork --dump-arenas programs/build/classes/walkthrough_point.bork
~~~

Korpus `programs/` buduje ten plik w `cargo test --features codegen` i
porównuje kod wyjścia z komentarzem `// exit: 8`.

## Gdzie szukać

| Etap | Plik | Kluczowe funkcje / typy |
|---|---|---|
| CLI | `src/main.rs` | `main`, `build_command`, `run_build` |
| Orkiestracja | `src/frontend.rs` | `check`, `CheckResult` |
| Parser | `src/parser.lalrpop`, `src/lib.rs` | `Item`, `ClassDecl`, `Function`, `ExprSuffix`, `parse` |
| AST | `src/ast.rs` | `Program::from_items`, `Class`, `ClassField`, `Function`, `Expr::Call`, `Expr::Field` |
| Typeck | `src/typeck/mod.rs` | `check`, `build_classes`, `collect_fun_sigs`, `check_function`, `bind_params` |
| Konstruktor / wywołanie | `src/typeck/expr/call.rs`, `object.rs` | `check_call`, `callee_signature`, `check_construct` |
| Pola | `src/typeck/expr/field.rs` | `check_field` |
| HIR | `src/hir/mod.rs`, `src/hir/ty.rs` | `HirProgram`, `HirExprKind::ObjectConstruct`, `ObjectField`, `Ty::record_name` |
| Własność | `src/sema/analyze.rs`, `walk.rs`, `region.rs` | `analyze_with_span_tys`, `open_ordinary`, `ArenaReport` |
| Hoist / escape | `src/hoist.rs`, `src/escape.rs` | `annotate`, `check_function` |
| Plan pamięci | `src/memory.rs` | `plan`, `Planner::after_expr`, `ArenaPlan`, `MemoryPlan::allocation_at` |
| Bramka | `src/codegen_gate.rs` | `gate` |
| Moduł LLVM | `src/codegen/llvm/mod.rs`, `context.rs` | `emit_module`, `Codegen::new`, `object_handle_type` |
| Funkcje / ABI | `src/codegen/llvm/emit_fn.rs`, `call_abi.rs` | `declare_function`, `emit_function`, `CallAbi` |
| Obiekty | `src/codegen/llvm/managed_ref.rs`, `region_emit.rs` | `begin_object_construct`, `emit_object_construct`, `emit_object_field` |
| Link | `src/codegen/link.rs` | `link_executable` |

## Ograniczenia widoczne w tym przykładzie

- Klasy nie mają metod. Operacje na klasie to funkcje top-level.
- Pole klasy nie może być borrowem `&T`.
- Wywołania nie mają podsumowań ucieczki, więc każdy rekord przekazany do
  funkcji, także przez `&`, dostaje w `MemoryPlan` arenę `Dynamic` z
  `lifetime_domain: Program`.
- Codegen nie obsługuje jeszcze przenoszenia (`move`) obiektów klasowych.
