# Klasy i funkcje: przejście przez kompilator od źródła do binarki

Ten dokument prowadzi jeden mały program przez cały kompilator Borka: parser,
AST, typeck i HIR, analizę własności, analizę pamięci, bramkę codegenu, LLVM
i linkowanie. Na każdym etapie pokazuje, który plik i która funkcja pracują
oraz jak wygląda stan pośredni dla przykładu.

Model aren opisuje [memory-model.md](memory-model.md), a MIR, na którym
pracuje plan pamięci i codegen, [mir/example.md](mir/example.md).

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

Ten przykład używa funkcji top-level `sum`. Klasy mogą też mieć metody z `this`;
opisuje je [language.md](language.md#methods-and-this). Typeck zamienia metodę
`Point.sum` na zwykłą funkcję z parametrem `this: &Point`, więc dalsza część
pipeline'u wygląda tak samo jak dla `sum` poniżej.

## Mapa pipeline'u

~~~text
src/main.rs                     bork build plik.bork
└── frontend::check             src/frontend.rs
    ├── parse                   src/lib.rs + src/parser.lalrpop  -> ast::Program
    ├── typeck::check           src/typeck/                      -> HirProgram + NodeTypes
    ├── sema::analyze_with_members   src/sema/                   -> ArenaReport (+ błędy własności)
    ├── hoist::annotate         src/hoist.rs                     (mutuje HIR)
    └── escape::check_function  src/escape.rs                    (tylko diagnostyki)
└── codegen::build              src/codegen/mod.rs
    ├── mono::monomorphize      src/mono.rs                      -> instancje + plan z MIR
    ├── mir::build              src/mir/build/                   -> Body na funkcję
    ├── gate                    src/codegen_gate.rs              (co MIR nie umie zbudować)
    ├── llvm::emit_module       src/codegen/llvm/lower.rs        -> inkwell Module
    ├── llvm::write_object      -> plik .o
    └── link::link_executable   src/codegen/link.rs              clang + bork_runtime
~~~

`frontend::check` jest wspólny dla `bork plik.bork` (tylko sprawdzenie),
`bork build`, LSP i testów. Kolejność jest ważna: HIR trafia dalej tylko wtedy,
gdy typeck i sema nie zgłosiły żadnej diagnostyki.

~~~rust
let (hir, node_tys, members, mut type_diagnostics) = typeck::check_with_members(&program);
let (mut report, ownership_errors) =
    crate::sema::analyze_with_members(&program, node_tys, members.clone());
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

Wejście to `typeck::check` w `src/typeck/mod.rs`. Najpierw buduje tablicę definicji (`DefTable`: `DefId` dla każdej klasy, funkcji, metody i stałej; to ona wykrywa duplikaty i kolizje z wbudowanymi nazwami), potem pracuje w czterech krokach:

~~~rust
let defs = DefTable::build(program); // src/defs.rs: DefId i konflikty nazw
let classes = build_classes(program, &defs, &mut diagnostics);
validate_owned_layouts(&classes, &mut diagnostics);
let fun_sigs = collect_fun_sigs(program, &defs, &classes, &mut diagnostics);
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

Span konstruktora jest ważny. plan pamięci z MIR i codegen używają go jako
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
Każde `HirExpr` niesie `ty` i `span`. Oprócz HIR typeck zwraca `NodeTypes`,
czyli typy deklaracji według `AstId` węzła AST (numeruje je `ast_number::number_nodes`
zaraz po parsowaniu), z których korzysta sema.

## 3. Sema: własność i drzewo aren

Sema (`src/sema/`) pracuje na AST, nie na HIR, ale dostaje typy z typecku:

~~~rust
pub fn analyze_with_node_tys(program: &Program, node_tys: NodeTypes)
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

## 4. Hoist i escape

Te kroki działają na HIR i tylko wtedy, gdy wcześniej nie było błędów.

### `hoist::annotate`

`src/hoist.rs` szuka inicjalizatorów, których alokację można od razu umieścić
w arenie zewnętrznego wiązania (`alloc_in_binding`). W przykładzie nie ma
takiego wzorca, więc HIR się nie zmienia.

### `escape::check_function`

`src/escape.rs` odrzuca wartości `String`, których bajty przeżyłyby arenę,
w której leżą. Pole `name: String` jest kopiowane do areny obiektu przy
konstrukcji (zobacz krok 6), więc nic nie ucieka.

Plan pamięci nie powstaje już na HIR. `mono::plan_instances` buduje MIR
dla każdej instancji (`mir::build`) i wyprowadza z niego `MemoryPlan` oraz
flagi `codegen_push` (`src/mir/plan.rs`: `derive_memory_plan`, `stamp_push`).
Wywołania nie mają podsumowań ucieczki, więc rekord przekazany do funkcji,
także przez `&`, dostaje arenę `Dynamic` z `lifetime_domain: Program`. Widać to
w IR w kroku 6 jako `bork_arena_create_dynamic` podpięte pod `bork_arena_root`.
Treść MIR dla dowolnego pliku pokaże `bork build --emit-mir`.

## 5. Bramka codegenu

`codegen::build` (`src/codegen/mod.rs`) buduje MIR raz i przekazuje wyniki do
bramki (`src/codegen_gate.rs`). Bramka nie ma listy odrzucanych kształtów:
każda konstrukcja, której MIR nie umie zbudować, zgłasza nazwaną
`Capability` (`src/mir/capability.rs`), a bramka zamienia ją w diagnostykę ze
spanem. Błąd bramki kończy `bork build` bez binarki.

Klasa z polami `String`/`i32`, borrow parametru i odczyt pól przechodzą bramkę
bez uwag.

## 6. LLVM: `emit_module`

`llvm::emit_module` (`src/codegen/llvm/mod.rs`) wymaga `fun main`, a potem:

~~~rust
let mut cx = Codegen::new(context, "bork", &hir.classes, &hir.enums, tcx)?;
let callees = hir.functions.iter()
    .map(|function| { let value = declare::declare_function(&cx, function)?; ... })
    .collect()?;
for (function, body) in hir.functions.iter().zip(bodies) {
    lower::lower_function(&cx, &callees, function, &body?)?;
}
cx.module.verify()?;
~~~

Każda funkcja jest zamieniana z MIR (`src/codegen/llvm/lower.rs`); emitera
HIR już nie ma.

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
| Plan pamięci | `src/mir/plan.rs`, `src/mono.rs` | `derive_memory_plan`, `stamp_push`, `plan_instances` |
| MIR | `src/mir/build/`, `src/mir/body.rs` | `build_program`, `Body`, `StmtKind` |
| Bramka | `src/codegen_gate.rs`, `src/mir/capability.rs` | `gate`, `Capability` |
| Moduł LLVM | `src/codegen/llvm/mod.rs`, `context.rs` | `emit_module`, `Codegen::new` |
| Funkcje / ABI | `src/codegen/llvm/declare.rs`, `call_abi.rs` | `declare_function`, `CallAbi` |
| Ciała z MIR | `src/codegen/llvm/lower.rs`, `ref_ops.rs`, `value_ops.rs` | `lower_function` |
| Link | `src/codegen/link.rs` | `link_executable` |

## Ograniczenia widoczne w tym przykładzie

- Klasy nie mają metod. Operacje na klasie to funkcje top-level.
- Pole klasy nie może być borrowem `&T`.
- Wywołania nie mają podsumowań ucieczki, więc każdy rekord przekazany do
  funkcji, także przez `&`, dostaje w `MemoryPlan` arenę `Dynamic` z
  `lifetime_domain: Program`.
- Codegen nie obsługuje jeszcze przenoszenia (`move`) obiektów klasowych.
