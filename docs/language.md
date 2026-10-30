# Bork

Bork is a small language with curly-brace regions, no garbage collector, and arena memory. A program is a list of functions, classes and enums. Statements are separated by newlines, not semicolons. `//` comments run to the end of the line.

This page is the surface language as the frontend accepts it today. The chapter **Where string bytes are allocated** explains sink allocation, hoist, `promote`, and escape checking. Lower-level arena layout and codegen schedules are in [memory-model.md](memory-model.md). The last section lists what native codegen still rejects. For a step-by-step trip of one class-and-function program through every compiler phase (in Polish), see [walkthrough-classes-functions.md](walkthrough-classes-functions.md).

## Program

```bork
fun add(a: i32, b: i32): i32 {
    return a + b
}

fun main() {
    println(add(1, 2))
}
```

- `fun name(params) body` returns `unit` when `: Type` is omitted.
- A parameter is `val` unless written `var`. `name: Type` and `val name: Type` are the same.
- There is no top-level statement. `main` is an ordinary function.

## Types

| Write | Means |
|---|---|
| `i8` `i16` `i32` `i64` `u8` `u16` `u32` `u64` | integers |
| `Int` `Long` `Byte` `Float` `Double` | aliases of `i32` `i64` `u8` `f32` `f64` |
| `f32` `f64` `bool` `unit` | float, bool, unit (`true` / `false` literals) |
| `String` | owned text, not Copy; `s[i]` reads one byte as `u8`, `==` / `!=` compare contents |
| `Point` | instance of a declared `class Point`, not Copy |
| `Ref<Point>` | presence-capable managed handle to a non-nullable class |
| `[T; N]` | fixed-length array with **N** elements (`N` is a non-negative integer literal) |
| `&T` | borrow of non-Copy `T` (`String`, `[U; N]`, …) for the current region — **parameters** and **`val` locals** only; not `&i32`; not on fields or return types |
| `T?` | `Option<T>` (see [Option and Result](#option-and-result-54)) |
| `(A, B) -> R` | function type |
| `((A, B) -> R)?` | nullable function type |

Integer literals adopt the expected integer type. An untyped `val`/`var` initialised with a bare literal takes its type from later uses in the same function (`val x = 1; takes_u8(x)` makes `x: u8`); if nothing fixes it, it defaults to `i32`/`f64` at the end of the function. Non-null primitives are **Copy**. Classes, `T?`, `String`, and `[T; N]` are not. `Ref<T>` handles can be copied without making their class targets Copy.

## Classes and `Ref<T>`

```bork
class Node {
    name: String
    next: Ref<Node>
}

fun main() {
    val a = Node("A")
    val b = Node("B", a)
    a.next = b
    if val live = b.next {
        println(live.name)
    }
    val label = b.next?.name ?: "root"
}
```

- `class` is the only record declaration: fields in declaration order, methods, no inheritance. `Node(...)` builds one. Omitted `Ref` fields are `None`.
- Fields use `name: Type` without `val` or `var` modifiers and are mutable. Fields cannot store borrow views (`&T`); their lifetimes are not tracked inside objects. `val` keeps the binding fixed; fields of that class instance can still be assigned. A `Vec` is the exception: a `val` `Vec` cannot be mutated (`push`, `pop`, `set`, `clear`), either directly or through a field of a `val` object (`h.items.push(1)` is an error). `this`, borrows and `Ref` handles are exempt; a call result such as `getVec().push(1)` is a temporary and is not checked.
- Class values are not Copy and do not have structural `==` or `!=`.
- `Ref<T>` is a managed handle to a **non-nullable class** declared with `class` (or an enum, see [Enums](#enums); the frontend accepts it, codegen does not yet). `Ref<String>`, arrays, primitives, nullable classes, and nested `Ref` targets are rejected. It is always presence-capable (`None` or a live target), and is not `&T` or `T?`.
- `Ref<T>` copies without `move`. After `move`, the source reads as moved.
- Field access on a `Ref` goes through `if val`, `when`, or `?.`. The bound name inside `if val` / `when Some` is `&T` for that guard only.
- `?.` on a class field requires `Ref<T>`; an ordinary class value or `T?` cannot use it.
- `when` is exhaustive `Some(name)` / `None`. `?.` yields `T?` for an ordinary field and a flat `Ref<U>` for a `Ref` field. `?:` on a `Ref` keeps the handle; it does not unwrap to `&T`.
- `[Ref<T>; N]` stores handles. Element assignment and reads use ref glue, not a byte copy of the array. When a String or array is written into a class field, the record owns a copy of its descriptor payload; Ref elements are cloned and registered for drop in that owner arena.

### Methods and `this`

```bork
class Point {
    x: i32
    y: i32

    fun sum(): i32 {
        return x + y          // this.x + this.y
    }
    fun shift(x: i32) {
        this.x = this.x + x   // parameter shadows the field
    }
}

fun main(): i32 {
    val p = Point(3, 4)
    p.shift(1)
    return p.sum()
}
```

- A class body mixes fields and `fun` methods in any order. The constructor still takes fields only.
- `this` is a borrow `&Self` of the receiver. Fields are read and assigned through it; `move this`, `this = …`, and returning `this` as an owned value are errors. `this` outside a method is an error.
- A bare name resolves to a local or parameter first, then a field or method of the class, then a top-level function or class. Write `this.x` to reach a shadowed field.
- `recv.m(args)` borrows the receiver implicitly (no `&`). The receiver is a named value, a `&T`, or a field chain such as `this.inner.m()`. `Ref<T>` receivers need `if val` / `when`; nullable receivers and temporaries (`Point(1, 2).sum()`) are errors.
- No overloading. Methods in different classes may share a name; inside a class, a method shadows a top-level function of the same name.
- Not supported: static/companion methods, visibility, inheritance, extension functions, method references, `?.m()`. Shared behaviour is expressed with traits (below), not inheritance.

### Traits

```
trait Shape {
    fun area(): f64
    fun scaled(k: f64): f64
}

impl Shape for Circle {
    fun area(): f64 { return r * r }
    fun scaled(k: f64): f64 { return k * r }
}

impl<T> Show for Box<T> { ... }
```

- A trait lists method signatures (no bodies, no default methods). `Self` names the implementing type; methods take `this: &Self` implicitly, like class methods. Traits and impls may be generic (`trait Conv<U>`, `impl<T> Conv<i32> for Box<T>`).
- Each impl must provide exactly the trait's methods with the same parameter count and types (after substituting `Self` and the trait arguments). Missing, extra, duplicate and mismatched items are errors at the impl.
- Two impls of one trait may not overlap: if their headers unify (`impl<T> Show for Box<T>` and `impl Show for Box<Circle>`), the later one is an error naming both. Bounds are not considered (`impl<T: A> ..` and `impl<T: B> ..` for `Box<T>` overlap). Impls of different traits never overlap, and neither do impls of one trait whose trait arguments cannot unify (`Conv<i32>` and `Conv<bool>`); arguments that can unify do overlap (`impl<T> Conv<T> for Box<T>` and `impl Conv<Circle> for Box<Circle>`).
- Only classes and enums can implement a trait for now. `Self` is usable in impl signatures too.
- `trait` and `impl` are now reserved words: they can no longer be used as variable, function or field names.
- Not yet: calling trait methods (#58), associated types (#59). Impl bodies are parsed but not type-checked (no errors for bad names or return values inside them until calls can reach them, #58).

#### Bounds

```
fun show_all<T: Show + Eq>(item: &T) { ... }
fun pair<A, B>(a: &A, b: &B) where A: Show, B: Conv<i32> { ... }
impl<T: Show> Show for Box<T> { ... }
```

- A type parameter of a function, method or impl may carry bounds, written `T: A + B<i32>` or in a `where` clause (`where` is a reserved word; it may only name the item's own type parameters). Class, enum and trait parameters cannot have bounds yet.
- Every call checks the callee's bounds against the type arguments. A class or enum satisfies `Trait` if an impl matches; an impl's own bounds are then required of its type arguments (`Box<Square>: Show` needs `Square: Show`). Inside a generic body, `T` implements exactly the traits in its bounds.
- A requirement whose type is still unknown waits until it is fixed; at the end of the function an unfixed one is an error. Following impl bounds stops at depth 32.
- A type parameter that appears only in a bound (`fun g<T: Show>()`) is not monomorphized yet: calling it is rejected until dispatch lands (#58). A method in an impl may not add bounds the trait method does not declare, and a repeated bound is an error.
- Primitives, `Vec` and other non-class types implement no traits yet, so a bound such as `T: Show` is never satisfied by an `i32`.

For the complete strong/weak classification rules, arena-ID algorithm, and
cleanup diagrams, see [references.md](references.md).

`String` and `[T; N]` have one field: `.length` (`i32`, always **N** for arrays of that type). Unknown fields are type errors.

`strongCount(r)` and `weakCount(r)` return the reference counters of the arena a `Ref<T>` points into (diagnostics, see [memory-model.md](memory-model.md#strongcount--weakcount)). `Vec<T>` is a built-in growable array: `Vec<i32>()`, `Vec<String>(16)`; see [memory-model.md](memory-model.md#vect). Array literals use Rust-style brackets: `[1, 2, 3]` has type `[i32; 3]`. Index from zero: `a[i]` on `[T; N]` and on **`&[T; N]`** (including reference parameters). A slice `a[lo..hi]` requires **integer literal** bounds and has type `[T; hi - lo]` (a view of the same buffer, no copy). Codegen uses that length as-is and does not re-check the slice at runtime. An out-of-range slice is a type error. An out-of-range index aborts at runtime, because the index is not part of the type. Assigning or moving whole arrays requires matching `[T; N]` (same `T` and **N**). Empty `[]` needs an annotation such as `[i32; 0]`. A `var` array binding supports element assignment `a[i] = v` when `i` is `i32` and `v` has the element type; `val` arrays cannot be mutated. A `&[T; N]` parameter supports `p[i] = v` in the callee. `String` elements require `move` or `promote` on the right-hand side. There is no per-element `move` (only whole-array `move` / `promote`).

## Enums

```bork
enum Shape { Circle(r: f64), Rect(w: f64, h: f64), Empty }

enum Tree<T> {
    Leaf,
    Node(l: Ref<Tree<T>>, v: T),
}

fun main() {
    val a = Shape.Circle(1.0)
    val b: Shape = Shape.Empty
    val t: Tree<i32> = Tree.Node(None, 5)
}
```

- A variant has no payload (`Empty`) or named payload fields (`Circle(r: f64)`). Variants are separated by `,` or a newline, with an optional trailing separator.
- A variant is built with `Enum.Variant(args)`, or `Enum.Variant` for a unit variant. Every payload field takes exactly one argument; arity and field types are checked (`variant `Shape.Rect` takes 2 argument(s), got 1`). A local binding with the enum's name shadows the path.
- Enums may be generic. Like a generic class constructor, a variant is instantiated with fresh variables fixed by its arguments or the expected type (`Tree.Leaf` needs `val t: Tree<i32> = ...`); a parameter nothing fixes is an error, and explicit type arguments on a variant are not supported yet.
- Enums share the type-name namespace with classes, functions and constants: `enum Foo` next to `class Foo`, `fun Foo` or `const Foo` is an error, as is a duplicate enum or variant.
- A by-value cycle through payloads is rejected with its path, the same as for classes; `Ref<Tree<T>>` is the sanctioned recursion, so `Ref<_>` accepts a class or an enum.
- `T?`, `Some` and `None` are unchanged for nullables and `Ref<T>`. `Some` and `None` are ordinary names, not keywords: where the expected type (or the matched value) is an enum with a variant of that name, they are that variant, so an enum may declare `Some` and `None` variants.

### Patterns in `when`

`when value { pattern => { ... } ... }` picks the first arm whose pattern matches. Patterns: `_`, a name (binds the value), a literal (`1`, `true`, `"s"`), `Variant(p, ..)` or `Enum.Variant(p, ..)` (one sub-pattern per payload field, nested freely), a bare `Variant` for a payload-free variant, and `Some(p)` / `None`. A bare name that spells a payload-free variant of the matched enum is that variant. Over a `Ref<T>`, the exact pair `Some(name)` + `None` is `if val`; over a `T?` or `Option` it is an ordinary `when`, in either arm order.

Binding mode: over a named value (a variable, a field, `this`) or any `&T`, a bound name is a borrow `&T`, except `Copy` scalars (numbers, `bool`), which bind by value, and `Ref<T>` payloads, which bind as a counted copy (the same as reading a `Ref` field of a class) so the name can go through `if val`. Over a temporary it binds by move. Nothing is moved out of a named value. The arms must have one type when the `when` is used as a value.

Coverage is checked at compile time. A `when` over enums, `bool` or `Some`/`None` must cover every value, and the error lists what is missing (``non-exhaustive `when`: missing `Shape.Rect(_, _)` ``). Integers and strings are never fully covered by literals, so they need a `_` or a binding arm. An arm that earlier arms already cover is an error (`unreachable pattern`); this also catches a misspelled unit variant, which binds as a catch-all. The check is skipped when a pattern in the `when`, or its scrutinee, already has an error.

Codegen (#52): an enum value is a slot in its arena, `{ i32 tag, payload.. }`, as large and as aligned as the biggest variant (no niche optimization); the tag is the variant's declaration index. Payloads are owned like class fields: `String`, arrays and `Vec` are copied into the enum's arena and a `Ref` payload is retained and released with it, never copied bitwise. Variants can be built, passed, returned, stored in class fields and read back with `when`.

`bork build` lowers `when` over enums, integers, `bool` and `String` (#53). The scrutinee is evaluated once; the arms are tested in order (first match wins) and each arm is a region of its own. A variant pattern compares the tag and then tests the payload sub-patterns; literals compare with `==`. Every binding is a view into the matched value, so an arm never owns a payload and the enum's drop glue stays the only releaser; a `Ref` payload is the exception, it is a counted copy released when the arm ends. `when` over a `Ref` or a nullable value, and `Some`/`None` patterns other than the `if val` pair, are rejected by codegen for now.

### `Option` and `Result` (#54)

`Option<T>` and `Result<T, E>` are ordinary generic enums written in Bork (`src/prelude/*.bork`). A program that defines its own `Option` or `Result` keeps its own (and then has no `T?`):

```bork
enum Option<T> { Some(value: T), None }
enum Result<T, E> { Ok(value: T), Err(error: E) }
```

**`T?` is another spelling of `Option<T>`.** They are the same type: a `String?` can be passed where an `Option<String>` is expected and the other way round, `None` and `Some(x)` build either, and `T?` has no special representation any more: a nullable value is an enum slot in an arena, `{ tag, payload }`, like any other enum (a `Some` allocates; the old inline `{ i1, T }` form is gone). `Option<Option<T>>` is allowed, so `Some(None)` and `None` are different values. `Ref<T>` is the exception for now: it keeps its own null handle and its own `if val` / `?.` / `?:`, and it is **not** an `Option` (see the table).

```bork
fun half(n: i32): i32? {            // the same as Option<i32>
    if (n > 5) { return Some(n / 2) }
    return None
}

when (half(10)) {
    None => { println(0) }
    Some(v) => { println(v) }
}
```

`Some(x)` and `None` are plain names, not keywords: they build the `Option` variants wherever an `Option` / `T?` is expected (`Option.Some(x)` / `Option.None` always do), and they keep their old meaning only for a `Ref<T>`. A `when` over a `T?` or `Option` works like any other `when` over an enum, with the arms in any order. `if val x = opt { ... } else { ... }` reads an `Option` the same way (the `else` is required; without it `if val` still needs a `Ref<T>`). `Ref<T>` has an absence of its own (its target may be gone) and `Option` is an ordinary value, so nothing converts between them implicitly: use `optionFromRef` / `optionToRef`.

Nothing converts implicitly between `Ref<T>` and `Option<Ref<T>>` (an absent reference is `None` of a `Ref`, an `Option<Ref<T>>` is an enum that holds one):

| from | to | call |
|---|---|---|
| `Ref<T>` | `Option<Ref<T>>` | `optionFromRef::<T>(r)` |
| `Option<Ref<T>>` | `Ref<T>` | `optionToRef::<T>(&o)` (a counted copy; absent gives `None`) |

Use `T?` for a value that may be absent, `Ref<T>` for an optional link to an object, and `Option<T>` / `Result<T, E>` in signatures and enum payloads where you want the enum spelled out. Rust comparison: `Option` is an ordinary enum there too (`Some` / `None` re-exported by the prelude), there is no `T?`, and its `?` operator is sugar over `match`; Bork's `?` propagation is a later ticket (TS-P8-01).


## Constants

`const NAME: Type = literal` at file level, with a mandatory type. Types: integers, `f32`, `f64`, `bool`, `String`. The initializer is a single literal of that type; it is range-checked (`const B: u8 = 255` is fine, `256` is an error). Constants are visible in every function and method, including ones declared earlier; there is no runtime initialization order.

```bork
const PORT: i32 = 8080
const RESPONSE: String = "HTTP/1.1 200 OK\r\n\r\nOK"
```

- Parameters, locals and bare fields shadow a constant (the same precedence as elsewhere).
- Duplicate names and clashes with a function, class or builtin are errors.
- Assigning to a constant, or `move`/`promote` of one, is an error. Reads do not consume it, and `&RESPONSE` works where `&String` is expected.
- A `String` constant's bytes live for the whole process. They are copied into an arena only when a copy is made explicitly.

## Bindings

```bork
val n = 1
var count: i32 = 0
var s: String = "hi"
count = count + 1
```

- `val` is immutable. Assigning to it is a type error.
- `var` is mutable.
- `: Type` is optional when the initializer determines the type.
- A name shadows an outer name for the rest of the block.

## Reserved words

These are lexer tokens, so they cannot name a binding, parameter, field, function, class or constant:

- declarations: `class`, `enum`, `const`, `fun`, `val`, `var`, `this`
- control flow: `if`, `else`, `when`, `for`, `in`, `while`, `break`, `continue`, `return`
- values: `true`, `false`
- ownership: `move`, `promote`
- type constructors: `Ref`, `Vec`

`const` is an ordinary keyword. `Ref` and `Vec` stay reserved only until `Vec` becomes a regular generic type (#68); until then they are parsed specially (`Vec<T>(cap)` is its own expression). The type-system RFCs' assumption that the lexer does not special-case type names is the target, not today's behaviour. Near misses such as `vec`, `constant` or `Vector` are ordinary identifiers.

## Statements

One statement per line. A `{ ... }` block is a statement and opens an arena region.

```bork
return
return 1
for (i in 0..10) {
    count = count + i
}
move (s) {
    println(s)
}
```

- `for (name in lo..hi)` is a half-open integer range. Bounds are evaluated once. The loop body arena resets every iteration.
- `while (cond) { ... }` repeats while `cond` is true. `break` exits the innermost loop; `continue` starts the next iteration.
- `move (a, b) { ... }` moves the listed non-Copy bindings into the block. `move () { ... }` moves nothing. `move { ... }` infers which parent `var`s the block consumes.
- Names in an explicit capture list are already local inside the block. Do not write `move` again for them.

## Expressions

`if` is an expression. Both branches are blocks. Without `else`, the `if` is a statement-like expression used for control, not a value.

```bork
val t = if (n > 0) { n } else { 0 }
val label: String = name ?: "Guest"
val text: String = maybe!!
```

`if val` observes managed references from left to right. The original single binding
form remains valid; parentheses allow a nonempty comma-separated list, with an
optional trailing comma and multiline layout:

```bork
if val (
    manager = cache_ref,
    root = manager.root,
) {
    println(root.id)
} else {
    println(0)
}
```

Each RHS must have type `Ref<T>`; its binding has type `&T`. Later RHS expressions
can use earlier bindings. Each RHS runs once, and failure skips the remaining
RHS expressions and body, then runs `else` once if present. Effects already
performed are retained. Pins acquired by a successful prefix are released before
`else`. All bindings and the body share one success scope. A binding may shadow an
outer name (its RHS still sees the outer name); duplicate header names are errors.
`else` and code after the guard see the outer scope. Guard borrows cannot escape.
This does not unwrap nullable `T?` values. A value-producing guard requires `else`
and follows the existing branch type rules.

Precedence, tightest last: calls and suffixes, `*` `/`, `+` `-`, comparisons, `..`, `?:`.

| Form | Meaning |
|---|---|
| `f(a, b)` | call |
| `f(a) { x -> ... }` | call with a trailing closure |
| `f(a) move (s) { ... }` | trailing move-closure |
| `.name` `?.name` | field, optional field |
| `!!` | assert non-null |
| `?:` | if the left is null, use the right |
| `Some(x)` `None` | nullable constructors |
| `move name` | consume a binding |
| `&name` | borrow an outer `var` for this region (view, not a copy); required at call sites when the formal is `&T` |
| `promote name` | relocate an owned value into the arena of an outer `var` you are assigning to |
| `"..."` | string; escapes `\n` `\r` `\t` `\\` `\"` |

Comparisons: `>` `<` `>=` `<=` `==` `!=`. Logical: `&&` `||` `!` on `bool` (`&&` and `||` short-circuit). Arithmetic: `+` `-` `*` `/` on integers. `+` does not concatenate strings; use `concat`.

## Ownership

Each `{ ... }` region has an arena. Leaving the region frees that arena in one shot. Scalars live in registers. `String` bytes live in an arena.

| Situation | What to write |
|---|---|
| Copy value (`i32`, `bool`, …) | use the name; it is copied |
| Read a parent `val` of `String` | use the name; the child observes it in place (**Shared**) |
| Read or pass a parent `var` of `String` | `move name`, or `&name` when the callee takes `&String` (read-only in callee) |
| Borrow a parent `var` array in a child or loop | `&name` at use or call; formal `p: &[T; N]` requires `f(&a)` |
| Pass a `&[T; N]` parameter into another call from `if` / `while` / `for` | `other(&param)` — re-borrow at the call site only |
| Mutate parent array elements from a child | `a[i] = v` or `(&a)[i] = v`; inside `fun f(p: &[T; N])`, `p[i] = v` |
| Pass a `val` `String` into a `var` parameter | `move name` |
| Fresh expression into a `var` parameter (`f("hi")`, `f(concat(a, b))`) | no `move` |
| Assign a `String` up to an outer `var` | `outer = move inner` or `outer = promote held` |
| Return a `String` from this function | `return "literal"`, `return name` (parameter or local at function depth); not `return move` or `return concat(...)` yet |
| Return a parameter or other depth-0 string | `return name` |
| Move or promote a whole array | `move a` / `outer = promote inner` (types must match, including **N**) |
| Read an element | `a[i]` — Copy elements are copied; `String` is a shared view of the array buffer |

After `move`, the source binding is dead. Using it is an ownership error. Array elements are not moved on their own; `move` and `promote` apply to the whole `[T; N]` binding.

`promote name` is only valid as the right-hand side of an assignment to a `var` in an outer region. It is for a value that already exists in a shorter-lived arena. It is not a return form.

A `String` produced in only one arm of `if` cannot be the value of that `if`. The branch arena dies when the arm ends.

```bork
fun greet(name: String): String {
    return name
}

fun shout(): String {
    var s = "hi"
    return s
}

fun main() {
    var outer = "a"
    {
        var inner = "bc"
        outer = move inner
    }
    println(outer)
}
```

For the full story of how the compiler picks an arena for each `String`, see the next chapter. One-line reminder: `outer = concat(left, right)` puts only the **result** in `outer`'s arena, not the argument strings.

### References (`&T` and `&name`)

Use-site `&name` borrows an outer **`var`** without moving it. The owner stays alive until its region ends.

```bork
fun peek(msg: &String) {
    println(msg)
}

fun bump(buf: &[i32; 3]) {
    buf[0] = buf[0] + 1
}

fun main() {
    var s: String = "hi"
    var a: [i32; 3] = [1, 2, 3]
    {
        val view = &s
        peek(&s)
        bump(&a)
    }
}
```

- **`&String`:** read and pass to callees; **no** assigning a new string through the borrow (no `&mut String` yet).
- **`&[T; N]`:** read `buf[i]` and **assign** `buf[i] = v` in the callee (mutates the owner’s buffer).
- **Nested control flow:** inside `if`, `while`, or `for`, you may call `helper(&param)` when `param` is a `&[T; N]` (or other `&T`) parameter and `helper` takes the matching reference type. You may **not** write `val view = &param` in that inner region (lifting the borrow into the child).
- **`val view = &a`:** read through `view`; do not write `view[i] =` — use `(&a)[i]` or a `&[T; N]` parameter.
- **`var view = &a`:** error.

See [memory-model.md](memory-model.md) (**Borrow**) for `T` vs `&T` formals and the re-borrow table. A plain `[T; N]` parameter requires ownership (`move` at the call site); `&a` is only for parameters declared `&[T; N]` (or `&String`, etc.).

## Where string bytes are allocated

This chapter answers one question: when the program builds a `String`, **in which memory pool do the characters live**, and how does the compiler know that pool is still valid when you use the string later?

There is no garbage collector. Each `{ ... }`, each `for` body, and each arm of an `if` has its own memory pool (an arena). When you leave that region, the pool is wiped in one go. A `String` in the program is only a pointer and a length; the characters sit in some arena. If the arena is gone, the pointer is useless.

### Rule 1 — usually, you allocate where you stand

Code inside a block uses that block's arena for new string data (unless one of the special cases below applies).

```bork
{
    var s = "hello"
}
// When the closing `}` runs, this block's arena is cleared.
// You must not keep using `s` outside unless ownership rules allowed a safe move out.
```

Text in quotes can also be stored in read-only program data; then the pointer does not point into a block arena at all. The compiler tracks that separately.

When the compiler creates a local variable `String`, it remembers **which arena was active at that moment**. That remembered arena is the one used later when you assign *into* that variable from an inner block.

### Rule 2 — assigning into an outer `var` from an inner block

You are allowed to write:

```bork
var outer = "a"
{
    var inner = "b"
    outer = move inner
}
```

Here `outer` was created outside the inner `{ }`, but the assignment runs inside it.

Two things happen:

1. **Ownership (sema):** changing `outer` is not treated as “using the parent's `var` inside the child in a forbidden way”. You are just updating the outer variable.

2. **Where new bytes go (codegen):** while the right-hand side of `outer = …` is evaluated, the compiler pretends the **destination** is `outer`'s arena, not the inner block's arena. So the string that ends up in `outer` is allocated in the same pool as `outer` itself, even though your source code sits inside `{ }`.

Important detail: this “pretend the destination's arena” applies to the **value you store**, not to every little sub-expression on the right. If you write `outer = concat(left, right)`, only the **new** buffer for the concatenated result is created in `outer`'s arena. The strings `left` and `right` are read where they already live; they are not copied into `outer`'s pool just because they appear as arguments.

The same splitting applies when you call a normal function on the right-hand side of an assignment: arguments are evaluated in the inner block's arena; only the final stored result follows rule 2 when the assignment target is an outer `var`.

### Rule 3 — two lines in a row: avoid a double copy

Sometimes you write:

```bork
var outer = "a"
{
    var piece = concat("b", "c")
    outer = move piece
}
```

Naively, `piece` would be built in the inner arena, then `move piece` would copy everything into `outer`'s arena. That is correct but wasteful.

If **all** of this is true:

- the first line is `var piece = …` where `…` is a string literal or a function call (including `concat`);
- the very next line is `outer = move piece` (nothing in between);
- `outer` already exists (parameter or earlier `var` in the same function or block);

then the compiler builds `piece` **directly in `outer`'s arena** on the first line. The second line is then mostly bookkeeping, not a second full copy of the characters.

This optimization does **not** apply if there is a gap between the lines, if `outer` is not visible yet, if you are inside `if`/`for` logic the pass does not handle, and so on. Then the characters are built in the child's pool and copied into `outer`'s pool on `move` (or you assign with `promote`, which performs that copy explicitly — it does not skip the copy).

### When a temporary `inner` helps, and when it is pointless

A common pattern looks like “create in the child block, then hand upward”:

```bork
var outer = "a"
{
    var inner = "b"
    outer = move inner
}
```

**You do not need `inner` here.** Inside the block you can write `outer = "b"` instead. Rule 2 already stores the result in `outer`'s pool even though the assignment sits in `{ }`. The literal `"b"` does not need a stopover variable.

`inner` is useful when the right-hand side is **not** a single literal you can assign directly, for example:

```bork
var outer = "a"
{
    var piece = concat(left, right)
    outer = move piece
}
```

Here `piece` names a non-trivial value. Rule 3 may initialize `piece` **already in `outer`'s arena** because the next line is `outer = move piece` and `outer` is in scope. Without that pass, `piece` would be built in the child's pool and `move piece` would copy the characters up — correct, but two steps.

**`move` is not “zero-copy” by definition.** For `String` it always ends ownership of the source and, when needed, copies bytes into the pool chosen for the assignment (rule 2). What people call zero-copy here is an **effect** of rule 2 (assign straight to `outer`) or rule 3 (build the temporary in `outer`'s pool), not a separate meaning of the `move` keyword.

The compiler does **not** look at the whole function and infer “`inner` only exists to feed `outer`”. It only recognizes the **two adjacent lines** in rule 3. If you separate the lines, rename variables, or put logic between them, you pay a full copy from the child pool into `outer`'s pool (`move` on assign, or `promote`).

| You write | Typical outcome |
|---|---|
| `outer = "x"` inside `{ }` | No `inner`; result follows rule 2 |
| `var inner = "x"` then next line `outer = move inner` | `inner` optional; rule 3 may avoid building `"x"` in the child pool |
| `var piece = concat(...)` then next line `outer = move piece` | `piece` is the usual style; rule 3 avoids concat in child + copy up |
| `var piece = concat(...)` … later … `outer = move piece` | Copy from child pool to `outer` (`move` or `promote` on assign) |

### Rule 4 — `promote` when the value already exists in the wrong pool

```bork
var outer = "a"
{
    var held = "temp"
    outer = promote held
}
```

`held` was already allocated in the inner arena. `promote` means: copy the characters into `outer`'s arena, then treat `held` as moved (dead). You may only use `promote` on the right-hand side of an assignment to an outer `var`, not on `return`.

### Rule 5 — what the compiler rejects (Ownership errors)

After typing, a separate check walks the program and asks: “when this `String` is used, is its arena still alive?”

Typical rejections:

- You build a string inside an inner `{ }` and try to return it without moving it to a place that survives the inner block.
- You use `move` on a string inside only one branch of an `if` and then use that `if` as an expression value — the branch pool is destroyed when the branch ends.
- You assign to an outer `var` a string that still lives only in a deeper pool, and neither rule 2, nor 3, nor `promote` fixes it.
- You write `return move …` or `return concat(…)` today. When the function returns, its arenas are torn down; there is not yet a dedicated “return buffer in the caller's pool”. Use `return name` for parameters or locals that already live at function level, or `return "literal"`.

These messages show up as **Ownership** in the editor even if you never run `bork build`.

### One worked example

```bork
fun main() {
    var out = "prefix"
    var left = "L"
    var right = "R"
    {
        var piece = concat(left, right)
        out = move piece
    }
    println(out)
}
```

- `out` belongs to the function body; its slot outlives the inner `{ }`.
- Inside the block, `concat` builds the result. Because the next line is `out = move piece` and `out` is already in scope, rule 3 may place `piece`'s buffer in `out`'s pool from the start instead of building in the child pool and copying up.
- Returning a `String` from a user function after arena-backed updates like this is still limited (rule 5): there is no caller-owned return buffer yet, so stick to `return name` for parameters, `return "literal"`, or keep the value in `main` as here.
- `return if (c > 0) { concat("a", "b") } else { "x" }` is rejected: a branch-local `concat` cannot be returned even when the other arm is a literal.

### What is not implemented yet

- Guessing more patterns than “`var` then next line `outer = move`” for rule 3.
- A caller-provided buffer for string return values that must be freshly allocated.
- `promote` on `return`.

For how arenas are nested in the report and how LLVM pushes and pops pools, see [memory-model.md](memory-model.md).

## Builtins

Intrinsics are not user functions. Redefining them is a type error.

| Name | Signature | Effect |
|---|---|---|
| `print` | one `i32`, `i64`, or `String` | write it |
| `println` | same | write it and a newline |
| `concat` | `(String, String) -> String` | one new string; buffer in the arena of whatever assignment target is active, else the current block |
| `stringFromUtf8` | `(&[u8; N], i32, i32) -> String?` | copy `count` bytes from `start`; `None` when the range is not valid UTF-8; a bad range (negative, or past the array end) aborts like an out-of-bounds index |

`stringFromUtf8` takes an array of any length; its signature is checked by the compiler, not a generic parameter. An empty valid range gives an empty present `String`, distinct from `None`. Like `concat`, the result cannot be returned from the function that creates it.

`print` / `println` accept those types even though the builtin table lists `i32` as the nominal parameter.

## Codegen today

`bork build` lowers a checked program to a native executable only for a subset of the frontend. Linked objects are optimized with LLVM **`-O3`** by default. Runtime arenas use a **per-thread** slab pool (no global lock on push/pop).

Supported: integer and float arithmetic and comparisons, `for` over `..`, `while`, `if`/`else`, `String` literals, `[T; N]` literals with index and slice, `.length`, `move` / `promote` of strings and whole arrays, `concat`, `print` / `println`, reference parameters with index read/write, recursive calls that re-borrow `&param` from inside control flow, calls to user functions without trailing closures, nullable `T?` (`None`, `Some`, `?:`, `!!`, `?.length`, `==` / `!=` with `None`), records, `Ref<T>`, `if val`, `when`, `?.` on record fields, and `?:` on `Ref`.

A `T?` is an `Option<T>` enum slot `{ i32 tag, payload }` in the sink arena (`None` too); `!!` on `None` aborts.

Rejected by codegen (the frontend still accepts them): trailing closures. Safe navigation of an owned nested record (as opposed to a scalar, `String`, array, or `Ref` field) is also rejected.

For debugging LLVM output, set `BORK_DUMP_IR` to a file path before `bork build` (writes the module before the O3 pipeline). See [memory-model.md](memory-model.md) (**Native codegen**).

Division by zero, and signed division of the minimum value by `-1`, abort at runtime.

## Generics

`fun f<T>(x: T): T`, `class Box<T>`, type arguments `Box<i32>` and explicit call arguments `f::<i32>(x)` parse. Bare `Box<i32>(x)` in an expression is still a comparison; use `Box::<i32>(x)`. Do not add a `>>` token: nested `Vec<Vec<T>>` relies on its absence.

Type parameters are scoped to their `fun`/`class` (a method sees its class's parameters first) and must not repeat, shadow a class parameter, or reuse a type name. Type-argument arity is checked (`Box<i32, i32>` on `class Box<T>` is an error). Generic items lower to HIR with `Param` types; their checking is described below.

Generic functions are checked once with their type parameters opaque: a `T` value can be passed, stored, returned and compared by type, but operators, members and `==` need a bound, which does not exist yet. A call instantiates the signature with fresh variables fixed by the arguments, the expected type or explicit arguments (`id::<i64>(1)`); a parameter nothing fixes is an error (`cannot infer type parameter`). Generic functions are compiled by monomorphization: `bork build` emits one native function per distinct argument list that `main` reaches (`id<i32>`, `id<String>`). Generic classes and methods are checked as well (see below).

Generic classes are checked the same way (#45): `class Pair<A, B>` has type `Pair<A, B>`, field and method types are substituted by the receiver's arguments, and methods see the class parameters first, then their own. A constructor call instantiates the class with fresh variables fixed by its arguments or the expected type (`Pair(1, "a")` is `Pair<i32, String>`, `val p: Pair<i64, String> = Pair(1, "b")` is `Pair<i64, String>`); a parameter nothing fixes is an error, and explicit type arguments on a constructor are not supported yet. A by-value cycle through generic classes is rejected with its path (`Bad<T> -> Bad<T>`, `A -> Box<A> -> A`); `Ref<Node<T>>` is the sanctioned recursion. Generic classes are compiled the same way: each `Class<Args>` used becomes its own layout (`Pair<i32,String>`), and so does each method instance. Only what `main` reaches is emitted, so a generic function nobody calls produces no code. The types an instance produces are checked again with the rules written types obey (`Ref<T>` with `T = i32` or `Vec<T>` with a class `T` are errors, reported at the call), and `bork check` reports that, a type parameter no signature type mentions, and polymorphic recursion the same way `bork build` does.
