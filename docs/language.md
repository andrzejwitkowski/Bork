# Bork

Bork is a small language with curly-brace regions, no garbage collector, and arena memory. A program is a list of functions. Statements are separated by newlines, not semicolons. `//` comments run to the end of the line.

This page is the surface language as the frontend accepts it today. The chapter **Where string bytes are allocated** explains sink allocation, hoist, `promote`, and escape checking. Lower-level arena layout and codegen schedules are in [memory-model.md](memory-model.md). The last section lists what native codegen still rejects.

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
| `f32` `f64` `bool` `unit` | float, bool, unit |
| `String` | owned text, not Copy |
| `[T; N]` | fixed-length array with **N** elements (`N` is a non-negative integer literal) |
| `T?` | nullable form of `T` |
| `(A, B) -> R` | function type |
| `((A, B) -> R)?` | nullable function type |

Integer literals adopt the expected integer type. Non-null primitives are **Copy**. `T?`, `String`, and `[T; N]` are not.

`String` and `[T; N]` have one field: `.length` (`i32`, always **N** for arrays of that type). Unknown fields are type errors.

Array literals use Rust-style brackets: `[1, 2, 3]` has type `[i32; 3]`. Index from zero: `a[i]`. A slice `a[lo..hi]` requires **integer literal** bounds and has type `[T; hi - lo]` (a view of the same buffer, no copy). Codegen uses that length as-is and does not re-check the slice at runtime. An out-of-range slice is a type error. An out-of-range index aborts at runtime, because the index is not part of the type. Assigning or moving whole arrays requires matching `[T; N]` (same `T` and **N**). Empty `[]` needs an annotation such as `[i32; 0]`. There is no per-element `move` and no `a[i] = …`.

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
- `move (a, b) { ... }` moves the listed non-Copy bindings into the block. `move () { ... }` moves nothing. `move { ... }` infers which parent `var`s the block consumes.
- Names in an explicit capture list are already local inside the block. Do not write `move` again for them.

## Expressions

`if` is an expression. Both branches are blocks. Without `else`, the `if` is a statement-like expression used for control, not a value.

```bork
val t = if (n > 0) { n } else { 0 }
val label: String = name ?: "Guest"
val text: String = maybe!!
```

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
| `promote name` | relocate an owned value into the arena of an outer `var` you are assigning to |
| `"..."` | string; escapes `\n` `\r` `\t` `\\` `\"` |

Comparisons: `>` `<` `>=` `<=` `==` `!=`. Arithmetic: `+` `-` `*` `/` on integers. `+` does not concatenate strings; use `concat`.

## Ownership

Each `{ ... }` region has an arena. Leaving the region frees that arena in one shot. Scalars live in registers. `String` bytes live in an arena.

| Situation | What to write |
|---|---|
| Copy value (`i32`, `bool`, …) | use the name; it is copied |
| Read a parent `val` of `String` | use the name; the child observes it in place (**Shared**) |
| Read or pass a parent `var` of `String` | `move name` |
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

`print` / `println` accept those types even though the builtin table lists `i32` as the nominal parameter.

## Codegen today

`bork build` lowers a checked program to a native executable only for a subset of the frontend.

Supported: integer and float arithmetic and comparisons, `for` over `..`, `if`/`else`, `String` literals, `[T; N]` literals with index, literal-bounds slice, and `.length`, `move` / `promote` of strings and whole arrays, `concat`, `print` / `println`, calls to user functions without trailing closures.

Rejected by codegen (the frontend still accepts them): trailing closures, `None`, `Some`, `!!`, `?:`, and field access other than `.length` on `String` or `[T; N]`.

Division by zero, and signed division of the minimum value by `-1`, abort at runtime.
