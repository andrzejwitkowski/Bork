# Constants and String Bytes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Globalne stałe i minimalne operacje tekst/bajty potrzebne przed kolekcjami i HTTP.

**Architecture:** Stałe rozwiązywane w frontendzie, użycia obniżane do typowanych literałów. Operacje tekstowe korzystają z obecnego deskryptora i małych helperów runtime. Kopiowanie UTF-8 respektuje istniejące allocation sinks i ograniczenia zwracania Stringów.

**Tech Stack:** Rust 2021, LALRPOP, istniejący HIR/sema, Inkwell/LLVM 23, runtime Rust std; bez nowych zależności.

**Spec:** [2026-10-12-constants-string-bytes-design.md](../specs/2026-10-12-constants-string-bytes-design.md)

Status: plan do przeglądu, bez zmian w kodzie produktu. Wykonanie sekwencyjne jest
zalecane ze względu na wspólne kontrakty frontend/ownership/backend.

## Global Constraints

- `const NAME: Type = literal` wyłącznie na poziomie pliku; typ obowiązkowy.
- Typy: istniejące typy całkowite, `f32`, `f64`, `bool`, `String`, bez nullable.
- Indeks oznacza bajt UTF-8, nie znak. `.length` pozostaje liczbą bajtów.
- Zły zakres: abort jak przy indeksowaniu tablic. Niepoprawny UTF-8: `None`.
- Walidacja UTF-8 następuje przed alokacją; `None` nie zwiększa zużycia areny.
- Nie dodajemy caller-owned return buffers.
- FNV-1a 64: offset `14695981039346656037`, prime `1099511628211`, wrapping multiplication.
- Bez sieci, kolekcji ani dynamicznych widoków w tym etapie.

## Review Focus

- Lokalne nazwy i bare fields przesłaniają globalne stałe identycznie w typeck, sema i hover (Task 1).
- Pusty poprawny UTF-8 jest Some, nigdy None; NUL nie kończy tekstu (Tasks 2, 3).
- Skopiowane bajty nie zależą od regionu źródła; nullable/unwrap nie omija escape check (Task 3).
- Złe indeksy i przepełniające zakresy kończą się przed odczytem pamięci (Tasks 2, 3).
- O3 i `-g` mają tę samą semantykę; nowy moduł trafia do obu archiwów runtime (Tasks 2–4).

## Mapa plików

- Stałe: `src/ast.rs`, `src/parser.lalrpop`, nowy `src/constants.rs`, eksport w
  `src/lib.rs`, `src/frontend.rs`, `src/typeck/{mod,env}.rs`,
  `src/typeck/expr/{mod,borrow}.rs`, `src/typeck/stmt/{mod,assign}.rs`,
  `src/sema/{analyze,env,walk,free_vars}.rs`.
- Tekst: nowy `crates/bork_runtime/src/text.rs`, `crates/bork_runtime/src/lib.rs`,
  `build.rs`, `src/builtins.rs`, `src/typeck/expr/{array,binary,call}.rs`,
  `src/codegen_gate.rs`, `src/codegen/llvm/{context,expr,binop}.rs`,
  `src/codegen/llvm/array/emit.rs`.
- Własność/alokacje: `src/escape.rs`, `src/hoist.rs`, `src/region_walk.rs`,
  `src/memory.rs` tylko w miejscach klasyfikujących alokujące wyrażenia/builtiny.
- Integracja: `src/lsp.rs`, `tools/bork-lsp-extension/syntaxes/bork.tmLanguage.json`,
  `tests/{parser,build,debug_build,programs}.rs`, testy modułowe sema/typeck,
  `programs/{check,check_fail,build,build_fail}/text/`, `docs/language.md`,
  `docs/memory-model.md`, `programs/README.md`.

Przed zmianami wyszukać wszystkie konstruktory `Program`, `Item`, `CheckResult`
i wywołania zmienianych API; poprawić dotknięte fixtures. Zachować obecne publiczne
wrappers `frontend::check`, `typeck::check*`, `sema::analyze*`, `build` i `emit_module`.

### Task 1: Globalne stałe w całym frontendzie

**Interfaces:** `constants::collect(program: &Program) -> (ConstantTable, Vec<Diagnostic>)`;
`ConstantTable` przechowuje zadeklarowany `Ty`, typowaną wartość i span deklaracji.
Wspólna tabela dostępna w typeck i sema; HIR użyć zawiera istniejące typowane
literały ze spanem użycia. Nie wprowadzać niezależnych resolverów nazw dla LLVM.

- [ ] Dodać parser/frontend tests `global_constants_literals_and_forward_use`,
  `global_constant_integer_range_checked`, `global_constant_float_overflow_rejected`.
  Sprawdzić `const B: u8 = 255` → OK, `256` → błąd typu, zgodne bool/float/String,
  obowiązkową adnotację oraz odrzucenie inicjalizatorów wyrażeniowych i nullable.
- [ ] Dodać `global_constant_names_and_shadowing`: duplikaty i kolizje z builtin,
  funkcją/klasą → błąd; parametr, lokalna zmienna i bare field wygrywają ze stałą.
  Odczyt przed deklaracją działa; assignment/move/promote stałej → błąd ze spanem.
- [ ] Uruchomić `cargo test global_constant` i odpowiednie nowe testy parsera;
  potwierdzić FAIL z powodu nieobsługiwanej deklaracji `const`.
- [ ] Rozszerzyć AST/grammar i wdrożyć wspólną tabelę literalnych stałych z walidacją.
  Wpiąć ją w frontend i istniejące wrappers. Dodać fallback do stałych dopiero
  po obecnych lokalnych/implicit-member lookups, także w borrow i assignment.
  Sema nie traktuje globalnego tekstu jako wartości z areny funkcji ani jako
  nieznanej wolnej zmiennej closure. Zachować obecne ograniczenia closures.
- [ ] Dodać golden build: stała String odczytana wielokrotnie, zwrócona z helpera,
  przekazana jako `&String`; stała `u8` zachowuje typ po lowering. Dodać hover test
  dla deklaracji, użycia i przesłoniętej nazwy; rozszerzyć kolorowanie `const`.
- [ ] Uruchomić `cargo test global_constant`, `cargo test --test parser`,
  `cargo test --features codegen programs_corpus`; oczekiwać PASS.
- [ ] Commit: `feat: add literal global constants` (stage wyłącznie pliki zadania).

### Task 2: Bajty, równość i wewnętrzny hash Stringów

**Interfaces:** `s[i]: u8` dla String/`&String`; `==`/`!=` zgodnie ze spec.
Runtime C ABI: `bork_string_equal(ptr, usize, ptr, usize) -> u8`,
`bork_string_hash(ptr, usize) -> u64`.

- [ ] Dodać testy runtime: puste wejścia z null, równe bajty w różnych buforach,
  różne długości/payloady, embedded NUL i UTF-8. Hash pustego →
  `14695981039346656037`, hash `a` → `12638187200555641996`; równe treści mają
  równy hash, niezależnie od adresu. Nie zakładać braku kolizji różnych treści.
- [ ] Dodać frontend/build tests `string_byte_index`, `string_content_equality`,
  `string_borrowed_equality`, `string_index_bounds_abort`.
  `"ą".length == 2`, bajty 196/133; równość literału z `concat`;
  oba operandy z efektami wykonywane raz; indeks `0 - 1` i length → abort.
  Nullable `== None` nadal działa; zapis `s[i] = ...` jest odrzucony.
- [ ] Uruchomić nowe testy; potwierdzić FAIL dla nieobsługiwanych operacji/helperów.
- [ ] Dodać `text.rs` i eksporty runtime, rerun-if-changed w `build.rs` dla obu
  archiwów. Obsłużyć zero length bez Rust slice z null. Emitować wywołanie helpera
  równości i negację dla `!=`; nie uruchamiać integer/nullable equality na Stringu.
  Współdzielić istniejącą kontrolę granic indeksowania; wynik `u8`, brak mutacji.
- [ ] Zsynchronizować typeck, gate i ownership: porównanie jest obserwacją operandów,
  nie ich przeniesieniem. Osobny test z zewnętrznym `var String` porównywanym
  w pętli, a następnie używanym ponownie. Zachować reguły pozostałych operatorów.
- [ ] Uruchomić `cargo test -p bork_runtime`, `cargo test --features codegen string_`,
  `cargo test --features codegen programs_corpus`; oczekiwać PASS.
- [ ] Commit: `feat: add String byte access and content equality`.

### Task 3: Walidowana kopia UTF-8 z tablicy bajtów

**Interfaces:** `stringFromUtf8(buffer: &[u8; N], start: i32, count: i32): String?`
dla dowolnego istniejącego N; runtime `bork_utf8_valid(ptr, usize) -> u8`.
Nowy builtin klasyfikowany jako alokujący do allocation sink, jak `concat`.

- [ ] Dodać runtime tests: valid ASCII/multibyte/NUL/empty → 1; niepełna sekwencja,
  samotny continuation byte, overlong encoding, surrogate → 0.
- [ ] Dodać frontend tests: rozmiary tablic 0/3/8192 akceptowane, element i8/i32,
  String zamiast tablicy, brak borrow, błędna arność/typ indeksu → diagnostyka.
- [ ] Dodać golden tests `utf8_copy_independent`, `utf8_invalid_is_none`,
  `utf8_empty_is_present`, `utf8_outer_sink_survives`, `utf8_invalid_ranges_abort`.
  Kopia GET zostaje GET po zmianie bufora; zakres pusty na końcu bufora jest obecny;
  ujemne i nadmierne zakresy oraz count=2147483647 przy start=1 → abort.
- [ ] Dodać escape regressions: bezpośredni i pośredni zwrot nowej kopii,
  nullable/`!!`/`?:`/`if`/presence binding nie mogą zwrócić zwolnionych bajtów.
  Bezpieczny zwrot literału/globalnej stałej pozostaje akceptowany.
- [ ] Uruchomić nowe testy; potwierdzić FAIL dla nieznanego builtinu.
- [ ] Wdrożyć intrinsic validation dla `&[u8; N]`, bounds check bez dodawania
  start+count, helper UTF-8 i warunkowe kopiowanie do sink. Dla pustej poprawnej
  kopii zapewnić nie-nullowy wskaźnik zgodny z obecną reprezentacją String?.
  Dla invalid UTF-8 zwrócić null/0 bez arena_alloc. W IR potwierdzić kolejność
  bounds → validation → conditional allocation/copy, bez odczytu na złej ścieżce.
- [ ] Wspólną klasyfikację builtinów alokujących zastosować w hoist, escape,
  region_walk i backendzie; nie kopiować kolejnych wyjątków `is_concat`.
  Zachować bezpieczne obecne zwroty, dodać precyzyjną diagnostykę dla nowego builtinu.
- [ ] Uruchomić nowe testy, `cargo test --features codegen` oraz testy runtime;
  oczekiwać PASS, w tym assignment-up z pętli i regionu źródła.
- [ ] Commit: `feat: copy validated UTF-8 buffers into arena Strings`.

### Task 4: Integracja, dokumentacja i przegląd

**Interfaces:** nie dodaje nowych API; sprawdza wspólny kontrakt Tasks 1–3.

- [ ] Dodać mały golden program: globalny token `GET`, bufor u8, UTF-8 copy,
  porównanie i byte indexing; wynik identyczny w ordinary build i `-g`.
  Rozszerzyć debug_build test o nowe operacje, nie wymagać GDB print globalnej stałej.
- [ ] Opisać const, byte indexing/length, equality, bounds, nullable UTF-8 result
  i allocation sink w language/memory docs. Dodać przykłady do korpusu i README.
- [ ] Uruchomić SEKWENCYJNIE `cargo test --workspace`,
  `cargo test --workspace --no-default-features`,
  `cargo test --workspace --features codegen`,
  `cargo test -p bork_runtime --features debug-runtime`.
  W tym środowisku ustawić ścieżki clang i bibliotek LLVM jak w poprzednim planie.
  Nie uruchamiać różnych feature-buildów równolegle w jednym target directory.
- [ ] Sprawdzić `git diff --check`; rustfmt tylko dotkniętych plików z edition 2021.
  Przejrzeć scope lookup, zakresy, zerowe bufory, własność kopii i ścieżki abort.
  Przeprowadzić review całej zmiany przed uznaniem zadania za ukończone.
- [ ] Commit dokumentacji/integracji; nie pushować ani tworzyć PR bez zlecenia.

## Świadomie poza zakresem

Globalne `var`, pola statyczne, const evaluation wyrażeń, pełny zakres u64
literałów, generics, dynamiczne slice'y/listy/mapy, Unicode character indexing,
normalizacja/case folding, formatowanie liczb i caller-owned String return buffers.
