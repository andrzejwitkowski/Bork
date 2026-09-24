# Bork

Bork is a small language with curly-brace regions, no garbage collector, and arena memory. A program is a list of functions. Statements are separated by newlines, not semicolons. `//` comments run to the end of the line.

This page is the surface language as the frontend accepts it today. Deeper arena mechanics live in [memory-model.md](memory-model.md). The last section lists what native codegen still rejects.

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
| `T?` | nullable form of `T` |
| `(A, B) -> R` | function type |
| `(A, B) -> R)?` | nullable function type |

Integer literals adopt the expected integer type. Non-null primitives are **Copy**. `T?` and `String` are not.

`String` has one field: `.length` (`i32`). Unknown fields are type errors.

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
| Return a `String` built in this function | `return move name` or `return "literal"` or `return concat(...)` |
| Return a parameter or other depth-0 string | `return name` |

After `move`, the source binding is dead. Using it is an ownership error.

`promote name` is only valid as the right-hand side of an assignment to a `var` in an outer region. It is for a value that already exists in a shorter-lived arena. It is not a return form.

A `String` produced in only one arm of `if` cannot be the value of that `if`. The branch arena dies when the arm ends.

```bork
fun greet(name: String): String {
    return name
}

fun shout(): String {
    var s = "hi"
    return move s
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

`outer = concat(left, right)` allocates the result in `outer`'s arena, not in a temporary that dies with the current block.

## Builtins

Intrinsics are not user functions. Redefining them is a type error.

| Name | Signature | Effect |
|---|---|---|
| `print` | one `i32`, `i64`, or `String` | write it |
| `println` | same | write it and a newline |
| `concat` | `(String, String) -> String` | one new string in the current sink arena |

`print` / `println` accept those types even though the builtin table lists `i32` as the nominal parameter.

## Codegen today

`bork build` lowers a checked program to a native executable only for a subset of the frontend.

Supported: integer arithmetic and comparisons, `for` over `..`, `if`/`else`, `String` literals, `move` / `promote` of strings, `concat`, `print` / `println`, calls to user functions without trailing closures.

Rejected by codegen (the frontend still accepts them): trailing closures, `None`, `Some`, `!!`, `?:`, and field access.

Division by zero, and signed division of the minimum value by `-1`, abort at runtime.
