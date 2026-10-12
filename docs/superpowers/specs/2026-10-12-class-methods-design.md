# Class methods and `this`

## Goal

Let classes declare methods, so operations on a class no longer have to be
top-level functions such as `fun sum(p: &Point)`. Inside a method, `this`
refers to the current instance, as in Kotlin. Methods work in `bork` (check),
`bork build` (LLVM, runnable binary), and the LSP.

~~~bork
class Point {
    x: i32
    y: i32

    fun sum(): i32 {
        return x + y              // implicit this.x / this.y
    }
    fun scaled(k: i32): i32 {
        return sum() * k          // implicit this.sum()
    }
    fun shift(x: i32) {
        this.x = this.x + x       // parameter shadows the field
    }
}

fun main(): i32 {
    val p = Point(3, 4)
    p.shift(1)
    return p.scaled(2)            // 16
}
~~~

## Decisions

1. **`this` is always an implicit borrow `&Self`.** A method reads and writes
   fields (fields are mutable through a borrow) and calls other methods. It
   cannot `move this`, return `this` as an owned value, or assign `this = …`.
2. **Implicit `this`, Kotlin style.** A bare name resolves to, in order:
   locals and parameters, then fields and methods of the enclosing class, then
   top-level functions and classes. A local or parameter shadows a field;
   `this.x` reaches the field.
3. **Receivers.** `recv.m(args)` accepts:
   - an owned class value (`val`/`var`), borrowed implicitly (no `&` at the call
     site);
   - a borrow `&C` (parameter, `if val`/`when` binding, `this`), re-borrowed;
   - a field chain rooted at a name: `this.inner.m()`, `outer.inner.m()`.

   Rejected: `Ref<C>` receivers (use `if val` / `when`), nullable receivers,
   and temporaries (`Point(1, 2).sum()`, `make().sum()`).

## Out of scope

- Methods without `this` (`companion` / static).
- Visibility (`private`), inheritance, interfaces, extension functions.
- Method references (`p::sum`).
- `?.m()` on `Ref<T>` or `T?`.
- Method calls on temporaries.
- Consuming methods (`this` taken by value).
- Overloading (no overloading of functions either).
- `this` inside trailing closures. Codegen does not lower trailing closures
  yet; the frontend treats `this` in a closure like any other captured borrow.
- LSP go-to-definition (the server has none today).
- Escape summaries for calls. A method receiver is a record argument, so
  `memory::plan` keeps marking it `Dynamic` / `Program`, the same as
  `sum(&p)` today.

## Architecture

Methods are methods in the AST and plain functions from HIR downwards.
Typeck, which knows receiver types, resolves member names and lowers each
method to a `HirFunction` with an explicit `this: &C` first parameter. Hoist,
escape, memory, the codegen gate, and LLVM emission see ordinary functions and
calls. Sema stays on the AST and reads a typeck side table to treat method
calls and bare field names correctly. The LSP reads the same side table for
hover.

~~~text
parse ──► ast::Class { fields, methods }
typeck ─► HirProgram.functions += "Point.sum"(this: &Point, …)
          recv.m(a)  ─► Call(Ident "Point.m", [Borrow(recv), a])
          bare x     ─► ObjectField(Ident this, i)
          MemberResolutions: Span ─► Field{class,index} | Method{class,name}
sema ───► uses MemberResolutions + span_tys; `this` is a REF param
memory / gate / LLVM ─► unchanged logic; symbol @bork.Point.sum
lsp ────► hover via sema bindings, then MemberResolutions
~~~

## 1. Syntax, parser, AST

- The class body is a `"NL"`-separated list of members. A member is a field
  (`name: Type`) or a method (the existing `Function` rule). Fields and
  methods may interleave.

  ~~~text
  ClassMember: ClassMember = {
      <ClassFieldDecl> => ClassMember::Field(<>),
      <Function>       => ClassMember::Method(<>),
  };
  ClassDecl: "class" <name:Ident> "{" <members:ClassBody> "}"
      => Class::from_members(name, members)
  ~~~

- `this` becomes a keyword with its own expression production. It produces
  `Expr::Ident { name: "this", span }`, so it flows through the existing
  identifier paths. `this` cannot be declared: `val this`, a parameter named
  `this`, or a field named `this` are parse errors.
- `ast::Class` gains `methods: Vec<Function>`. A method's AST has no explicit
  `this` parameter. `Program.functions` still holds only top-level functions.
- `ast::Function` gains `name_span: Span` for LSP hover on declarations.
- `p.sum(1)` already parses as `Call { callee: Field(p, "sum"), args }` and bare
  `sum()` as `Call { callee: Ident sum }`. The parser does not tell them apart;
  typeck does.
- Constructors are unchanged: positional `Point(...)` over fields only.

## 2. Typeck

**Tables.** `ClassInfo` gains `methods: HashMap<String, MethodSig>` with
`MethodSig { params: Vec<Ty>, return_ty: Ty, span: Span }`, where `params`
excludes `this`. `collect_fun_sigs` collects method signatures together with
function signatures before any body is checked, so declaration order does not
matter.

**Declaration errors:**
- `class `C` already has a field named `m`` for a method named like a field;
- `duplicate method `m` in class `C`` for two methods with the same name.

A method parameter named like a field is allowed and shadows it. Methods in
different classes may share a name. A method may share a name with a top-level
function; inside the class, the method wins.

**Checking a method body.** `check_method` mirrors `check_function`, but:
- `Env` gains `self_class: Option<String>`;
- the outermost scope binds `this: &C` as a `val` with `explicit_ref: true`.

**Bare-name resolution** (`check_ident`, assignment, and bare calls), in order:
1. local scopes and parameters;
2. a field of `self_class`, which gives `ObjectField { receiver: Ident this,
   field_index }`, or for a call, a method of `self_class`, which gives
   `Call(Ident "C.m", [Borrow(Ident this), args…])`;
3. top-level functions and classes (unchanged).

Bare `x = v` with a field resolution is a field assignment on `this`. Typeck
records the field type in `span_tys` under the identifier span, as it already
does for `AssignTarget::Field`.

**Errors on `this`:**
- `this` outside a method: ``this` is only available inside a class method`;
- `this = …`: `cannot assign to `this``;
- `move this`: `cannot move `this`; it is a borrow`.

**Method call `recv.m(args)`.** `check_call` with a `Field { receiver, name }`
callee:
1. Check `receiver`. If its type is `Named(C)` or `Ref(Named(C))` and `C` has
   method `m`, this is a method call. If `C` has no method `m`, the error is
   `class `C` has no method `m``. Otherwise keep today's
   `only named functions can be called`.
2. Receiver errors:
   - `ManagedRef(C)`: `method call on managed Ref requires `if val` or `when``;
   - nullable type: `method call on a nullable receiver`;
   - not a place, meaning not a `name(.field)*` chain:
     `method receiver must be a named value; bind it to a `val` first`.
3. Arguments go through the existing `check_call` rules, shifted by one for
   the receiver: count, `&…` for `&T` parameters, owned-vs-borrow, and type
   matching. Error messages name the method as `C.m`.
4. Result:

   ~~~text
   Call { callee: Ident "Point.sum" : Func(&Point, i32) -> i32,
          args: [Unary Borrow (<receiver>) : &Point, <a>] } : i32
   ~~~

   A receiver that is already `&C` is still wrapped in `Borrow`; the type stays
   `&C`, matching `check_borrow` today. `Borrow` over a field chain is new;
   `check_borrow` currently takes only a name, so the method path builds the
   `Unary Borrow` node directly.

A trailing closure on a method call follows the function rules: the frontend
accepts it and the gate rejects it.

**HIR.** Each method becomes
`HirFunction { name: "C.m", params: [this: &C, …], … }` in
`HirProgram.functions`. The dot cannot appear in a user identifier, so
method names never collide with functions.

**Side table.** `typeck::check` additionally returns
`MemberResolutions = HashMap<Span, MemberRef>`, where
`MemberRef = Field { class, index } | Method { class, name }`. Keys are the span
of a bare member identifier and the span of the `Field` callee in `p.m(…)`.
`frontend::check` passes it to sema and keeps it in `CheckResult`.

## 3. Sema (ownership)

`analyze_with_span_tys(program, span_tys, members)` takes the side table.

**Registration.** Every method is inserted into `fun_sigs` / `fun_param_tys` as
`"C.m"` with `[this: Val, &C] ++ params`. `analyze_function` analyzes the body
in region `"fun C.m"`, with an extra leading `RegionParam` for `this`
(`ty: &C`, `kind: Val`, `borrow_from: REF_PARAM_BORROW_FROM`). `this` therefore
behaves exactly like a `p: &Point` parameter today:
- the borrow rules block escaping `this` with the same messages;
- `this` appears in the region's bindings, so LSP hover on `this` works.

**Method call** (`walk`, `Expr::Call`). When the `Field` callee span has a
`Method { class, name }` resolution:
- formals and parameter types come from `"C.m"`;
- the receiver is treated as an `&T` argument: the root name of the
  `name(.field)*` chain gets `note_borrow(root, …, TransferSink::CallArg)`. This
  reuses the parameter re-borrow rule (`this.m()`, `p.m()` for `p: &Point`) and
  reports use-after-move when the root was moved;
- explicit arguments use the existing path with the index shifted by one.

**Bare members inside a method:**
- `x` resolved to `Field` gives `note_use("this", span)`;
- `x = v` resolved to `Field` is a field assignment on `this`, the same path as
  `AssignTarget::Field` with receiver `this`; the target type comes from
  `span_tys`;
- `m(a)` resolved to `Method` is a call to `"C.m"` with an implicit
  `note_borrow("this", …, CallArg)`.

Move and borrow rules, branch joins, `hoist`, and `escape` are unchanged.

## 4. Memory, gate, codegen

- **`memory::plan`:** unchanged. A receiver has type `&C`, so `record_name`
  sees a record argument and marks the object `Dynamic` / `Program`, as for
  `sum(&p)` today (see Out of scope).
- **Gate:** no new rejections. Trailing closures stay rejected by the existing
  `has_trailing_closure` rule.
- **LLVM:**
  - `declare_function` already emits `bork.{name}`, so a method becomes
    `@bork.Point.sum`. LLVM struct types (`%bork.Point`) live in a separate
    namespace, so the names do not clash.
  - `CallAbi::of` is unchanged: `this` is the first explicit parameter, a
    `{ptr arena, ptr object}` handle; the hidden parent arena and the
    `OwningReturn` result sink work as for functions.
  - **Risk:** `Borrow(ObjectField(this, i))` for a class-typed field must
    produce the field object's handle. If `ObjectField` emission for a record
    field does not already yield a handle, add that case in
    `src/codegen/llvm/expr.rs`. The plan starts this layer with the
    `field_receiver` fixture.
- `main` must stay a top-level function; a method is never an entry point.

## 5. LSP

- **Diagnostics:** new parser, typeck, and sema errors reach the LSP through
  `frontend::check` with real spans. No server changes.
- **Hover:**
  - `this`: through sema bindings, e.g. "`this` in arena `fun Point.sum`,
    Type: `&Point`, Ownership: borrow".
  - A bare field (`x` in a method), a method name at a call site (`sum` in
    `p.sum(1)` or bare `sum()`), and a method name in its declaration: when
    `find_span_hit` finds nothing, a new `member_hover` looks up
    `MemberResolutions` (and method declaration spans) for a span containing
    the cursor.
    - Field: "field `x` of `Point` (via `this`), Type: `i32`".
    - Method: "`fun Point.sum(a: i32): i32`".
- **Plumbing:** `frontend::CheckResult` gains `members` plus the method
  signatures needed for formatting; `lsp::Analysis` stores them next to
  `report`. `hover_for_analysis` keeps its call shape for `bork_lsp.rs`.
  No new server capabilities.

## 6. Testing and docs

**Corpus**, new topic `methods/`:

- `build/methods/` (`// exit:` and `// stdout:` where useful):
  - `basic`: `p.sum()` with bare `x + y`;
  - `explicit_this`: a parameter shadows a field, `this.x = x`;
  - `mutate_field`: `x = x + 1` in a method is visible to the caller;
  - `method_calls_method`: bare `sum()`, `this.sum()`, a recursive method;
  - `borrowed_receiver`: `fun f(p: &Point) { p.m() }`;
  - `field_receiver`: `this.inner.m()` and `outer.inner.m()`;
  - `owning_return`: a method returning `String` and one returning a new
    object;
  - `if_val_receiver`: `if val n = r { n.m() }` on `Ref<T>`;
  - `same_name`: a method named like a top-level function, and the same
    method name in two classes.
- `check_fail/methods/` (`// diag:`):
  - `move_this`, `assign_this`, `this_outside_method`;
  - `ref_receiver`, `nullable_receiver`, `temporary_receiver`;
  - `unknown_method`, `method_field_clash`, `duplicate_method`;
  - `use_after_move_receiver`, `wrong_arg_count`.
- `check/methods/`: a trailing closure on a method call (frontend OK).
- `build_fail/methods/`: the same, rejected by the gate.

**Rust tests:**
- `tests/parser.rs`: interleaved fields and methods, `this` as an expression,
  `val this` rejected.
- `src/typeck/tests/methods.rs`: HIR shape (`Call "Point.sum"` with `Borrow`,
  `ObjectField(this, i)` for a bare field) and name-resolution order.
- `src/sema/tests/methods.rs`: re-borrowing `this` in a call, use-after-move of
  a receiver, the `this` binding in region `fun Point.sum`.
- `src/codegen_gate.rs`: a program with methods passes the gate.
- `src/lsp.rs`: the hovers and diagnostics from section 5.

**Docs:**
- `docs/language.md`: drop "no methods" from Classes; add "Methods and `this`"
  with the rules above and the out-of-scope list.
- `docs/walkthrough-classes-functions.md`: replace "Metod klasowych nie ma"
  with a pointer to the new section.
- `programs/README.md`: add `methods/` to the topics table.

**Order:** branch `feat/class-methods` from `main`, TDD, layer by layer:
parser → typeck → sema → codegen → LSP → docs.
