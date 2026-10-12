# Class Methods and `this` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Classes declare methods; `recv.m(args)` and bare `m(args)` call them; `this` is an implicit `&Self` borrow with Kotlin-style implicit member lookup; everything works through `bork`, `bork build`, and the LSP.

**Architecture:** Methods live in `ast::Class.methods`. Typeck resolves members, lowers every method to a plain `HirFunction` named `C.m` with a leading `this: &C` parameter, lowers calls to `Call(Ident "C.m", [Borrow(recv), …])`, and records a `MemberResolutions` side table (`Span → MemberRef`). Sema (AST-based) registers methods as functions and uses the side table to borrow receivers and map bare fields to `this`. Memory, gate, and LLVM see ordinary functions. The LSP hovers members from the same table.

**Tech Stack:** Rust, LALRPOP (`src/parser.lalrpop`, built by `build.rs`), inkwell/LLVM behind `--features codegen`, tower-lsp.

**Spec:** `docs/superpowers/specs/2026-10-12-class-methods-design.md`

## Global Constraints

- `this` is always an implicit borrow `&Self`; no `move this`, no `this = …`, no returning `this` as owned.
- Bare name lookup order: locals/params → fields/methods of the enclosing class → top-level functions/classes.
- Receivers: owned class value (implicit borrow), `&C`, or a `name(.field)*` chain. Rejected: `Ref<C>`, nullable, temporaries, `?.m()`.
- HIR / LLVM name of a method is `C.m`; LLVM symbol `bork.C.m`.
- **Invariant:** `report.roots[i]` (sema) corresponds to `hir.functions[i]` (typeck). Both list top-level functions first, then for each class in `program.classes` order, its methods in declaration order. `region_walk` indexes roots by HIR function position; breaking this panics.
- Error messages (exact substrings, used by fixtures):
  - `` `this` is only available inside a class method ``
  - `` cannot assign to `this` ``
  - `` cannot move `this`; it is a borrow ``
  - `` class `C` already has a field named `m` ``
  - `` duplicate method `m` in class `C` ``
  - `` class `C` has no method `m` ``
  - `` method call on managed Ref requires `if val` or `when` ``
  - `method call on a nullable receiver`
  - `` method receiver must be a named value; bind it to a `val` first ``
- Commit after every task; end commit messages with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Test commands: `cargo test` (no LLVM), `cargo test programs_corpus_check`, `cargo test --features codegen` (LLVM; needed for `programs/build*`).

## Review Focus

1. A local declared *after* a bare field use in the same method: `x` before `val x = 1` is the field, after it the local. Pinned in Task 4 (`bare_field_then_local_shadow`).
2. A method call inside `while`/`if` inside a method re-borrows `this` from an outer region; must not report "cannot lift borrow". Pinned in Task 3 (`method_call_in_loop_reborrows_this`).
3. Receiver also passed as a `&` argument (`p.merge(&p)`) — two shared borrows, must be accepted. Pinned in Task 3 (`receiver_and_borrow_arg_same_name`).
4. Nested method calls as arguments (`p.add(q.sum())`) — codegen evaluation order / eval stack. Pinned in Task 5 (`nested_calls.bork`).
5. Receiver reached through a `Ref` field inside a guard (`if val n = this.next { n.sum() }`). Pinned in Task 5 (`if_val_receiver.bork`).

---

## File Structure

| File | Change |
|---|---|
| `src/ast.rs` | `Class.methods`, `ClassMember`, `Class::from_members`, `Function.name_span` |
| `src/parser.lalrpop` | `this` keyword, class members, `move this` |
| `tests/parser.rs` | parser tests |
| `src/typeck/mod.rs` | `MemberRef`, `MemberResolutions`, `check_with_members`, method sig collection, method bodies |
| `src/typeck/env.rs` | `self_class`, `members`, `ClassInfo.methods`, `implicit_field`, `implicit_method` |
| `src/typeck/expr/mod.rs` | `this` errors in `check_ident`, implicit field read |
| `src/typeck/expr/call.rs` | `finish_call` extracted, method dispatch |
| `src/typeck/expr/method.rs` (new) | `check_method_call` |
| `src/typeck/stmt/assign.rs` | `this =` error, implicit field assignment |
| `src/typeck/tests/methods.rs` (new) | typeck tests |
| `src/sema/analyze.rs` | method registration, `analyze_with_members` |
| `src/sema/env.rs` | `Analyzer.members` |
| `src/sema/walk.rs` | receiver borrow, bare field → `this` |
| `src/sema/tests/methods.rs` (new) | sema tests |
| `src/frontend.rs` | thread `members` through, expose in `CheckResult` |
| `src/lsp.rs`, `src/bin/bork_lsp.rs` | member hover |
| `src/codegen_gate.rs` | gate test |
| `programs/{build,check,check_fail,build_fail}/methods/*.bork` | corpus |
| `docs/language.md`, `docs/walkthrough-classes-functions.md`, `programs/README.md` | docs |

---

### Task 1: Parser and AST

**Files:**
- Modify: `src/ast.rs:9-51`
- Modify: `src/parser.lalrpop` (match block lines 11-32, `ClassDecl`…`ClassFieldDecl` lines 73-104, `Function` lines 106-115, `Atom` ~line 445, `PresenceOperand` ~line 519)
- Test: `tests/parser.rs`

**Interfaces:**
- Produces: `ast::Class { name: String, fields: Vec<ClassField>, methods: Vec<Function> }`; `ast::ClassMember::{Field(ClassField), Method(Function)}`; `Class::from_members(name: String, members: Vec<ClassMember>) -> Class`; `ast::Function.name_span: Span`; `this` parses to `Expr::Ident { name: "this", span }`; `move this` parses to `Expr::Move { name: "this", span }`.

- [ ] **Step 1: Write failing parser tests** — append to `tests/parser.rs`:

```rust
#[test]
fn parses_class_with_interleaved_fields_and_methods() {
    let src = "class Point {\n    x: i32\n    fun sum(): i32 {\n        return x + this.y\n    }\n    y: i32\n}\nfun main() {}";
    let prog = parse(src).expect("class with methods parses");
    let class = &prog.classes[0];
    assert_eq!(class.fields.len(), 2);
    assert_eq!(class.fields[1].name.name, "y");
    assert_eq!(class.methods.len(), 1);
    assert_eq!(class.methods[0].name, "sum");
    let span = class.methods[0].name_span;
    assert_eq!(&src[span.start..span.end], "sum");
    assert_eq!(prog.functions.len(), 1, "methods are not top-level functions");
}

#[test]
fn this_parses_as_identifier_expression() {
    let src = "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}";
    let prog = parse(src).expect("parses");
    let Stmt::Return(Some(Expr::Field { receiver, .. })) = &prog.classes[0].methods[0].body.stmts[0] else {
        panic!("expected return this.x");
    };
    assert!(matches!(receiver.as_ref(), Expr::Ident { name, .. } if name == "this"));
}

#[test]
fn move_this_parses() {
    let src = "class P {\n    fun f() {\n        val q = move this\n    }\n}";
    assert!(parse(src).is_ok());
}

#[test]
fn this_cannot_be_declared() {
    assert!(parse("fun main() {\n    val this = 1\n}").is_err());
    assert!(parse("fun f(this: i32) {}").is_err());
    assert!(parse("class P { this: i32 }").is_err());
}
```

If `Stmt`/`Expr` are not already imported in `tests/parser.rs`, they come from `use bork::*;` (`bork::ast` is re-exported); if not, add `use bork::ast::{Expr, Stmt};`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test parser`
Expected: compile errors (`no field methods`, `no field name_span`).

- [ ] **Step 3: Update AST** — in `src/ast.rs` replace `struct Class` and `struct Function`, and add `ClassMember`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub fields: Vec<ClassField>,
    pub methods: Vec<Function>,
}

/// One entry of a class body, in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassMember {
    Field(ClassField),
    Method(Function),
}

impl Class {
    pub fn from_members(name: String, members: Vec<ClassMember>) -> Self {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        for member in members {
            match member {
                ClassMember::Field(field) => fields.push(field),
                ClassMember::Method(method) => methods.push(method),
            }
        }
        Self {
            name,
            fields,
            methods,
        }
    }
}
```

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub name_span: crate::span::Span,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Block,
}
```

- [ ] **Step 4: Update grammar** — in `src/parser.lalrpop`:

Add `ClassMember` to the `use crate::ast::{…}` list.

In the `match` block, after `"class" => "class",` add:
```
    "this" => "this",
```

Replace `ClassDecl`, `ClassBody`, `ClassFieldRest` with:
```
ClassDecl: Class = {
    "class" <name:Ident> "{" <members:ClassBody> "}" => Class::from_members(name, members),
};

ClassBody: Vec<ClassMember> = {
    => vec![],
    "NL" => vec![],
    <first:ClassMember> <rest:ClassMemberRest> => {
        let mut members = vec![first];
        members.extend(rest);
        members
    },
    "NL" <first:ClassMember> <rest:ClassMemberRest> => {
        let mut members = vec![first];
        members.extend(rest);
        members
    },
};

ClassMemberRest: Vec<ClassMember> = {
    => vec![],
    "NL" => vec![],
    "NL" <member:ClassMember> <rest:ClassMemberRest> => {
        let mut members = vec![member];
        members.extend(rest);
        members
    },
};

ClassMember: ClassMember = {
    <field:ClassFieldDecl> => ClassMember::Field(field),
    <method:Function> => ClassMember::Method(method),
};
```
(keep `ClassFieldDecl` as is).

Replace `Function`:
```
Function: Function = {
    "fun" <name:SpannedIdent> "(" <params:Comma<Param>?> ")" <ret:ReturnType> <body:Block> => {
        Function {
            name: name.name,
            name_span: name.span,
            params: params.unwrap_or_default(),
            return_type: ret,
            body,
        }
    },
};
```

In `Atom`, after the `"promote"` alternative add:
```
    "move" <start:@L> "this" <end:@R> => Expr::Move {
        name: "this".to_string(),
        span: Span::new(start, end),
    },
    <start:@L> "this" <end:@R> => Expr::Ident {
        name: "this".to_string(),
        span: Span::new(start, end),
    },
```

In `PresenceOperand`, after the `<id:SpannedIdent>` alternative add:
```
    <start:@L> "this" <end:@R> => Expr::Ident {
        name: "this".to_string(),
        span: Span::new(start, end),
    },
```

- [ ] **Step 5: Build and run parser tests**

Run: `cargo test --test parser`
Expected: PASS. If LALRPOP reports a conflict, it is between the two `ClassBody` leading-`"NL"` alternatives and `ClassMemberRest`; resolve by mirroring exactly the shape of the old `ClassBody`/`ClassFieldRest` rules (they were conflict-free).

- [ ] **Step 6: Run the whole suite** (keyword `this` must not break existing sources)

Run: `cargo test`
Expected: PASS (all existing tests).

- [ ] **Step 7: Commit**

```bash
git add src/ast.rs src/parser.lalrpop tests/parser.rs
git commit -m "feat(parser): parse class methods and this

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Typeck and sema — method declarations, bodies, and `this`

Methods are checked and lowered, and sema registers them, but nothing calls them yet (calls come in Task 3). Both layers ship together because `frontend::check` requires the roots/functions invariant.

**Files:**
- Modify: `src/typeck/mod.rs`, `src/typeck/env.rs`, `src/typeck/expr/mod.rs:207-218`, `src/typeck/stmt/assign.rs:10-24`
- Modify: `src/sema/analyze.rs`, `src/sema/env.rs:77-102`, `src/sema/mod.rs:14`
- Modify: `src/frontend.rs:42-44`
- Create: `src/typeck/tests/methods.rs`; register `mod methods;` in `src/typeck/tests/mod.rs`
- Create: `programs/check_fail/methods/{this_outside_method,assign_this,move_this,method_field_clash,duplicate_method}.bork`

**Interfaces:**
- Consumes: Task 1 AST.
- Produces:
  - `pub enum typeck::MemberRef { Field { class: String, name: String, index: usize, ty: Ty }, Method { class: String, name: String, signature: String } }` (derive `Debug, Clone, PartialEq`)
  - `pub type typeck::MemberResolutions = HashMap<Span, MemberRef>`
  - `pub fn typeck::check_with_members(&Program) -> (HirProgram, SpanTypes, MemberResolutions, Vec<Diagnostic>)`; `typeck::check` unchanged in shape.
  - `Env.self_class: Option<String>`, `Env.members: MemberResolutions`, `Env::implicit_field(&self, name) -> Option<(String, usize, Ty)>`, `Env::implicit_method(&self, name) -> Option<String>` (returns class name).
  - `ClassInfo.methods: HashMap<String, String>` (method name → display signature `fun C.m(a: i32): i32`).
  - Method signatures live in `fun_sigs` under `"C.m"` with params `[&C, …]`.
  - `pub fn sema::analyze_with_members(&Program, SpanTypes, MemberResolutions) -> (ArenaReport, Vec<SemaError>)`; `Analyzer.members: MemberResolutions`.
  - Sema region label for a method: `fun C.m`; first region param `this: &C`.

- [ ] **Step 1: Write failing typeck tests** — create `src/typeck/tests/methods.rs`:

```rust
use super::{diags_of, hir_of};
use crate::hir::{HirExprKind, HirStmt, TyKind};

#[test]
fn method_lowers_to_function_with_this_param() {
    let hir = hir_of(
        "class Point {\n    x: i32\n    y: i32\n    fun sum(): i32 {\n        return this.x + this.y\n    }\n}\nfun main(): i32 {\n    return 0\n}",
    );
    let names: Vec<_> = hir.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["main", "Point.sum"]);
    let method = &hir.functions[1];
    assert_eq!(method.params[0].name, "this");
    assert!(matches!(&method.params[0].ty.kind, TyKind::Ref(inner) if inner.record_name() == Some("Point")));
    let HirStmt::Return { value: Some(value) } = &method.body.stmts[0] else {
        panic!("expected return");
    };
    let HirExprKind::Binary { lhs, .. } = &value.kind else { panic!("expected +") };
    assert!(matches!(&lhs.kind, HirExprKind::ObjectField { field_index: 0, .. }));
}

#[test]
fn method_param_may_shadow_field() {
    hir_of("class P {\n    x: i32\n    fun set(x: i32) {\n        this.x = x\n    }\n}\nfun main() {}");
}

#[test]
fn this_outside_method_is_error() {
    let diags = diags_of("fun main(): i32 {\n    return this.x\n}");
    assert!(diags.iter().any(|d| d.message.contains("`this` is only available inside a class method")), "{diags:?}");
}

#[test]
fn assign_this_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun f(other: &P) {\n        this = other\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("cannot assign to `this`")), "{diags:?}");
}

#[test]
fn move_this_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun f() {\n        val q = move this\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("cannot move `this`; it is a borrow")), "{diags:?}");
}

#[test]
fn method_named_like_field_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun x(): i32 {\n        return 1\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("class `P` already has a field named `x`")), "{diags:?}");
}

#[test]
fn duplicate_method_is_error() {
    let diags = diags_of("class P {\n    fun f() {}\n    fun f() {}\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("duplicate method `f` in class `P`")), "{diags:?}");
}
```

Add `mod methods;` to the `mod` list at the top of `src/typeck/tests/mod.rs`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test typeck::tests::methods`
Expected: FAIL (`names` is `["main"]`; error messages missing).

- [ ] **Step 3: Extend typeck env** — in `src/typeck/env.rs`:

Add `use super::MemberResolutions;` (defined in Step 4).

Extend `ClassInfo`:
```rust
#[derive(Clone, Debug)]
pub(super) struct ClassInfo {
    pub(super) name: String,
    pub(super) fields: Vec<ClassFieldInfo>,
    /// Method name -> display signature (`fun C.m(a: i32): i32`). Signatures are in `fun_sigs` as `C.m`.
    pub(super) methods: HashMap<String, String>,
}
```

Extend `Env` with two fields and initialise them in `Env::new`:
```rust
    /// Class whose method body is being checked; `None` in top-level functions.
    pub(super) self_class: Option<String>,
    /// Member names resolved by typeck, for sema and LSP.
    pub(super) members: MemberResolutions,
```
```rust
            self_class: None,
            members: MemberResolutions::new(),
```

Add methods to `impl Env`:
```rust
    /// Field of the enclosing class that a bare `name` refers to. Locals and params shadow fields.
    pub(super) fn implicit_field(&self, name: &str) -> Option<(String, usize, Ty)> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.classes.get(self.self_class.as_deref()?)?;
        let index = class.fields.iter().position(|field| field.name == name)?;
        Some((class.name.clone(), index, class.fields[index].ty.clone()))
    }

    /// Class of the enclosing method when a bare call `name(..)` is one of its methods.
    pub(super) fn implicit_method(&self, name: &str) -> Option<String> {
        if self.binding(name).is_some() {
            return None;
        }
        let class = self.self_class.as_ref()?;
        self.fun_sigs
            .contains_key(&format!("{class}.{name}"))
            .then(|| class.clone())
    }
```

- [ ] **Step 4: Typeck module: types, sig collection, bodies** — in `src/typeck/mod.rs`:

Add after `pub type SpanTypes …`:
```rust
/// What a bare member name or a method-call callee resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberRef {
    /// Bare `name` inside a method, read or assigned through `this`.
    Field { class: String, name: String, index: usize, ty: Ty },
    /// `recv.m(..)` / bare `m(..)` call, or a method declaration name.
    Method { class: String, name: String, signature: String },
}

/// Member resolutions keyed by the span of a bare name, a `recv.m` callee, or a method name.
pub type MemberResolutions = HashMap<Span, MemberRef>;
```

Rename the body of `check` into `check_with_members` and make `check` a wrapper:
```rust
pub fn check(program: &Program) -> (HirProgram, SpanTypes, Vec<Diagnostic>) {
    let (hir, span_tys, _, diagnostics) = check_with_members(program);
    (hir, span_tys, diagnostics)
}

pub fn check_with_members(
    program: &Program,
) -> (HirProgram, SpanTypes, MemberResolutions, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    reject_builtin_redefines(program, &mut diagnostics);
    let mut classes = build_classes(program, &mut diagnostics);
    validate_owned_layouts(&classes, &mut diagnostics);
    let mut fun_sigs = collect_fun_sigs(program, &classes, &mut diagnostics);
    let mut members = MemberResolutions::new();
    collect_method_sigs(program, &mut classes, &mut fun_sigs, &mut members, &mut diagnostics);
    let mut span_tys = SpanTypes::new();
    let mut functions: Vec<HirFunction> = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            check_function(function, None, &fun_sigs, &classes, &mut diagnostics, &mut span_tys, &mut members)
        })
        .collect();
    // Same order as sema roots: top-level functions, then methods class by class.
    for class in &program.classes {
        for method in &class.methods {
            functions.push(check_function(
                method,
                Some(&class.name),
                &fun_sigs,
                &classes,
                &mut diagnostics,
                &mut span_tys,
                &mut members,
            ));
        }
    }
    // ... existing hir_classes construction unchanged ...
    (
        HirProgram { classes: hir_classes, functions },
        span_tys,
        members,
        diagnostics,
    )
}
```

In `build_classes`, add `methods: HashMap::new(),` to the `ClassInfo { … }` literal.

Add after `collect_fun_sigs`:
```rust
/// Registers each method as `C.m` with a leading `this: &C` parameter.
fn collect_method_sigs(
    program: &Program,
    classes: &mut ClassTable,
    fun_sigs: &mut HashMap<String, FunSig>,
    members: &mut MemberResolutions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for class in &program.classes {
        for method in &class.methods {
            if class.fields.iter().any(|field| field.name.name == method.name) {
                diagnostics.push(type_error(
                    format!("class `{}` already has a field named `{}`", class.name, method.name),
                    Some(method.name_span),
                ));
            }
            let qualified = format!("{}.{}", class.name, method.name);
            if fun_sigs.contains_key(&qualified) {
                diagnostics.push(type_error(
                    format!("duplicate method `{}` in class `{}`", method.name, class.name),
                    Some(method.name_span),
                ));
                continue;
            }
            let params: Vec<Ty> = method
                .params
                .iter()
                .map(|param| lower_type(&param.ty, classes, diagnostics))
                .collect();
            let return_ty = lower_type(&method.return_type, classes, diagnostics);
            let shown_params: Vec<String> = method
                .params
                .iter()
                .zip(&params)
                .map(|(param, ty)| format!("{}: {ty}", param.name.name))
                .collect();
            let signature = format!("fun {qualified}({}): {return_ty}", shown_params.join(", "));
            let this_ty = Ty::new(
                TyKind::Ref(Box::new(Ty::new(TyKind::Named(class.name.clone()), false))),
                false,
            );
            fun_sigs.insert(
                qualified,
                FunSig {
                    params: std::iter::once(this_ty).chain(params).collect(),
                    return_ty,
                },
            );
            if let Some(info) = classes.get_mut(&class.name) {
                info.methods.insert(method.name.clone(), signature.clone());
            }
            members.insert(
                method.name_span,
                MemberRef::Method {
                    class: class.name.clone(),
                    name: method.name.clone(),
                    signature,
                },
            );
        }
    }
}
```

Replace `check_function` with an owner-aware version:
```rust
fn check_function(
    function: &Function,
    owner: Option<&str>,
    fun_sigs: &HashMap<String, FunSig>,
    classes: &ClassTable,
    diagnostics: &mut Vec<Diagnostic>,
    span_tys: &mut SpanTypes,
    members: &mut MemberResolutions,
) -> HirFunction {
    let mut env = Env::new(fun_sigs, classes);
    env.self_class = owner.map(str::to_string);
    let name = match owner {
        Some(class) => format!("{class}.{}", function.name),
        None => function.name.clone(),
    };
    let signature = env
        .fun_sigs
        .get(&name)
        .cloned()
        .expect("signature was collected for every function");
    let mut param_tys = signature.params;
    let mut params = Vec::new();
    if owner.is_some() {
        let this_ty = param_tys.remove(0);
        env.bind("this".to_string(), crate::ast::BindingKind::Val, this_ty.clone(), true);
        params.push(HirParam {
            kind: crate::ast::BindingKind::Val,
            name: "this".to_string(),
            ty: this_ty,
        });
    }
    params.extend(bind_params(function, param_tys, &mut env));
    let return_ty = signature.return_ty;
    let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
    if return_ty != Ty::unit() && !stmt::block_always_returns(&body) {
        env.error(
            format!("function must return a value of type {return_ty} on all paths"),
            None,
        );
    }
    diagnostics.append(&mut env.diagnostics);
    span_tys.extend(env.span_tys);
    members.extend(env.members);
    HirFunction {
        name,
        params,
        return_ty,
        body,
    }
}
```

- [ ] **Step 5: `this` errors** — in `src/typeck/expr/mod.rs`, at the top of `check_ident` (before `let Some(binding) = …`):
```rust
    if name == "this" {
        if env.self_class.is_none() {
            env.error("`this` is only available inside a class method", Some(span));
            return HirExpr::spanned(
                HirExprKind::Ident {
                    name: name.to_string(),
                    use_kind: requested,
                },
                Ty::unknown(),
                span,
            );
        }
        if matches!(requested, UseKind::Move | UseKind::Promote) {
            env.error("cannot move `this`; it is a borrow", Some(span));
        }
    }
```

In `src/typeck/stmt/assign.rs`, at the start of the `AssignTarget::Name { name, name_span } =>` arm:
```rust
            if name == "this" {
                env.error("cannot assign to `this`", Some(*name_span));
                return HirStmt::Expr(expr::check(value, None, return_ty, env));
            }
```

- [ ] **Step 6: Sema registration** — in `src/sema/env.rs` add to `Analyzer`:
```rust
    /// Member resolutions from typeck (bare fields, method calls).
    pub(super) members: crate::typeck::MemberResolutions,
```
and `members: Default::default(),` in `Analyzer::new`.

Replace `src/sema/analyze.rs` entry points and `analyze_function`:
```rust
use super::env::{Analyzer, Ty};
use super::region::RegionParam;
use super::report::{ArenaNode, ArenaReport, SemaError};
use super::walk::open_ordinary;
use crate::ast::{BindingKind, Function, Program, Type};

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let (_, span_tys, members, _) = crate::typeck::check_with_members(program);
    analyze_with_members(program, span_tys, members)
}

/// Same as [`analyze`], but reuses `span_tys` from an earlier `typeck::check` (avoids duplicate work).
pub fn analyze_with_span_tys(
    program: &Program,
    span_tys: crate::typeck::SpanTypes,
) -> (ArenaReport, Vec<SemaError>) {
    analyze_with_members(program, span_tys, Default::default())
}

/// Same as [`analyze_with_span_tys`], plus typeck's member resolutions for methods.
pub fn analyze_with_members(
    program: &Program,
    span_tys: crate::typeck::SpanTypes,
    members: crate::typeck::MemberResolutions,
) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    az.span_tys = span_tys;
    az.members = members;
    az.classes = program
        .classes
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect();
    // MVP: keyed by bare function name (no local shadowing of callees yet).
    for f in &program.functions {
        az.fun_sigs.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.kind).collect(),
        );
        az.fun_param_tys.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.ty.clone()).collect(),
        );
    }
    for class in &program.classes {
        for method in &class.methods {
            let qualified = format!("{}.{}", class.name, method.name);
            az.fun_sigs.insert(
                qualified.clone(),
                std::iter::once(BindingKind::Val)
                    .chain(method.params.iter().map(|p| p.kind))
                    .collect(),
            );
            az.fun_param_tys.insert(
                qualified,
                std::iter::once(this_type(&class.name))
                    .chain(method.params.iter().map(|p| p.ty.clone()))
                    .collect(),
            );
        }
    }
    // Same order as typeck's HIR functions: top-level, then methods class by class.
    let mut roots: Vec<ArenaNode> = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f, None))
        .collect();
    for class in &program.classes {
        for method in &class.methods {
            roots.push(analyze_function(&mut az, method, Some(&class.name)));
        }
    }
    (ArenaReport { roots }, az.errors)
}

fn this_type(class: &str) -> Type {
    Type::Ref {
        inner: Box::new(Type::Named {
            name: class.to_string(),
            nullable: false,
        }),
        nullable: false,
    }
}

fn analyze_function(az: &mut Analyzer, func: &Function, owner: Option<&str>) -> ArenaNode {
    let this = owner.map(|class| RegionParam {
        name: "this".to_string(),
        ty: Ty::Known(this_type(class)),
        kind: BindingKind::Val,
        span: None,
        borrow_from: super::region::REF_PARAM_BORROW_FROM,
    });
    let params: Vec<RegionParam> = this
        .into_iter()
        .chain(func.params.iter().map(|p| RegionParam {
            name: p.name.name.clone(),
            ty: Ty::Known(p.ty.clone()),
            kind: p.kind,
            span: Some(p.name.span),
            borrow_from: super::region::REF_PARAM_BORROW_FROM,
        }))
        .collect();
    let label = match owner {
        Some(class) => format!("fun {class}.{}", func.name),
        None => format!("fun {}", func.name),
    };
    open_ordinary(az, &label, &func.body, &params, None)
}
```

In `src/sema/mod.rs:14` export the new function: `pub use analyze::{analyze, analyze_with_members, analyze_with_span_tys};`.

- [ ] **Step 7: Frontend** — in `src/frontend.rs` replace lines 42-44:
```rust
    let (hir, span_tys, members, mut type_diagnostics) = typeck::check_with_members(&program);
    let (mut report, ownership_errors) =
        crate::sema::analyze_with_members(&program, span_tys, members);
```
(Task 6 also stores `members` in `CheckResult`; clone it then.)

- [ ] **Step 8: Run typeck tests**

Run: `cargo test typeck::tests::methods`
Expected: PASS.

- [ ] **Step 9: Corpus fixtures** — create `programs/check_fail/methods/`:

`this_outside_method.bork`:
```bork
// diag: `this` is only available inside a class method
fun main(): i32 {
    return this.x
}
```
`assign_this.bork`:
```bork
// diag: cannot assign to `this`
class P {
    x: i32
    fun f(other: &P) {
        this = other
    }
}
fun main() {}
```
`move_this.bork`:
```bork
// diag: cannot move `this`; it is a borrow
class P {
    x: i32
    fun f() {
        val q = move this
    }
}
fun main() {}
```
`method_field_clash.bork`:
```bork
// diag: class `P` already has a field named `x`
class P {
    x: i32
    fun x(): i32 {
        return 1
    }
}
fun main() {}
```
`duplicate_method.bork`:
```bork
// diag: duplicate method `f` in class `P`
class P {
    fun f() {}
    fun f() {}
}
fun main() {}
```

- [ ] **Step 10: Full suite**

Run: `cargo test && cargo test programs_corpus_check`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/typeck src/sema src/frontend.rs programs/check_fail/methods
git commit -m "feat(typeck): check class methods with an implicit this borrow

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Method calls (`recv.m(args)` and bare `m(args)`)

**Files:**
- Modify: `src/typeck/expr/call.rs` (extract `finish_call`, dispatch)
- Create: `src/typeck/expr/method.rs`; add `mod method;` in `src/typeck/expr/mod.rs`
- Modify: `src/sema/walk.rs` (`Expr::Call` arm ~line 377)
- Create: `src/sema/tests/methods.rs`; add `mod methods;` in `src/sema/tests/mod.rs`
- Modify: `src/typeck/tests/methods.rs`
- Create: `programs/check_fail/methods/{ref_receiver,nullable_receiver,temporary_receiver,unknown_method,use_after_move_receiver,wrong_arg_count}.bork`

**Interfaces:**
- Consumes: Task 2 (`fun_sigs["C.m"]`, `Env.members`, `Env::implicit_method`, `MemberRef`, `Analyzer.members`).
- Produces:
  - `call::finish_call(name: String, span: Span, params: Vec<Ty>, call_return_ty: Ty, receiver: Option<HirExpr>, args: &[Expr], trailing: Option<&Closure>, return_ty: &Ty, env: &mut Env<'_>) -> HirExpr`
  - `method::check_method_call(receiver: &Expr, method: &str, safe: bool, span: Span, args: &[Expr], trailing: Option<&Closure>, return_ty: &Ty, env: &mut Env<'_>) -> HirExpr`
  - `MemberRef::Method` recorded under the `Expr::Field` callee span (`p.m`) or the bare callee ident span (`m`).
  - Sema: `walk_receiver(az, receiver, node)` in `walk.rs`.

- [ ] **Step 1: Failing typeck tests** — append to `src/typeck/tests/methods.rs`:

```rust
use crate::ast::UnaryOp;
use crate::hir::HirExpr;

fn call_in_return(hir: &crate::hir::HirProgram, function: &str) -> HirExpr {
    let f = hir.functions.iter().find(|f| f.name == function).expect("function");
    let HirStmt::Return { value: Some(value) } = f.body.stmts.last().unwrap() else {
        panic!("expected return");
    };
    value.clone()
}

#[test]
fn receiver_call_lowers_to_qualified_call_with_borrowed_receiver() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun add(k: i32): i32 {\n        return this.x + k\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.add(2)\n}",
    );
    let call = call_in_return(&hir, "main");
    let HirExprKind::Call { callee, args, .. } = &call.kind else { panic!("call") };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.add"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0].kind, HirExprKind::Unary { op: UnaryOp::Borrow, .. }));
    assert!(args[0].ty.is_ref());
}

#[test]
fn bare_and_this_calls_resolve_to_method() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun one(): i32 {\n        return 1\n    }\n    fun two(): i32 {\n        return one() + this.one()\n    }\n}\nfun main() {}",
    );
    let HirExprKind::Binary { lhs, rhs, .. } = &call_in_return(&hir, "P.two").kind else { panic!("+") };
    for side in [lhs, rhs] {
        let HirExprKind::Call { callee, args, .. } = &side.kind else { panic!("call") };
        assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.one"));
        assert_eq!(args.len(), 1, "only the receiver");
    }
}

#[test]
fn method_wins_over_top_level_function_inside_class() {
    let hir = hir_of(
        "fun one(): i32 {\n    return 9\n}\nclass P {\n    fun one(): i32 {\n        return 1\n    }\n    fun two(): i32 {\n        return one()\n    }\n}\nfun main(): i32 {\n    return one()\n}",
    );
    let HirExprKind::Call { callee, .. } = &call_in_return(&hir, "P.two").kind else { panic!() };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.one"));
    let HirExprKind::Call { callee, .. } = &call_in_return(&hir, "main").kind else { panic!() };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "one"));
}

#[test]
fn borrowed_and_field_chain_receivers_are_accepted() {
    hir_of(
        "class In {\n    n: i32\n    fun get(): i32 {\n        return this.n\n    }\n}\nclass Out {\n    inner: In\n    fun deep(): i32 {\n        return this.inner.get()\n    }\n}\nfun via(o: &Out): i32 {\n    return o.inner.get() + o.deep()\n}\nfun main(): i32 {\n    val o = Out(In(1))\n    return via(&o) + o.inner.get()\n}",
    );
}

#[test]
fn method_call_receiver_errors() {
    let cases = [
        ("class P {\n    fun f() {}\n}\nfun main() {\n    val r: Ref<P> = None\n    r.f()\n}", "method call on managed Ref requires `if val` or `when`"),
        ("class P {\n    fun f() {}\n}\nfun g(p: P?) {\n    p.f()\n}\nfun main() {}", "method call on a nullable receiver"),
        ("class P {\n    fun f() {}\n}\nfun main() {\n    P().f()\n}", "method receiver must be a named value; bind it to a `val` first"),
        ("class P {\n    fun f() {}\n}\nfun main() {\n    val p = P()\n    p.g()\n}", "class `P` has no method `g`"),
        ("class P {\n    fun f(a: i32) {}\n}\nfun main() {\n    val p = P()\n    p.f()\n}", "function `P.f` expects 1 arguments, got 0"),
    ];
    for (src, expected) in cases {
        let diags = diags_of(src);
        assert!(diags.iter().any(|d| d.message.contains(expected)), "{expected}: {diags:?}");
    }
}
```

- [ ] **Step 2: Failing sema tests** — create `src/sema/tests/methods.rs` and add `mod methods;` in `src/sema/tests/mod.rs`:

```rust
use super::super::*;
use crate::parse;

fn errors(src: &str) -> Vec<SemaError> {
    analyze(&parse(src).expect("parse")).1
}

#[test]
fn method_call_borrows_receiver_without_moving_it() {
    let errs = errors(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    val a = p.get()\n    val q = move p\n    return a\n}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn method_call_after_move_is_error() {
    let errs = errors(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    val q = move p\n    return p.get()\n}",
    );
    assert!(errs.iter().any(|e| e.message.contains("use of `p` after move")), "{errs:?}");
}

#[test]
fn method_call_in_loop_reborrows_this() {
    let errs = errors(
        "class P {\n    x: i32\n    fun one(): i32 {\n        return 1\n    }\n    fun sum(): i32 {\n        var t = 0\n        var i = 0\n        while (i < 3) {\n            if (i > 0) {\n                t = t + this.one() + one()\n            }\n            i = i + 1\n        }\n        return t\n    }\n}\nfun main() {}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn receiver_and_borrow_arg_same_name() {
    let errs = errors(
        "class P {\n    x: i32\n    fun merge(o: &P): i32 {\n        return this.x + o.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.merge(&p)\n}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn this_is_a_borrow_param_of_the_method_region() {
    let (report, errs) = analyze(&parse(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main() {}",
    ).unwrap());
    assert!(errs.is_empty(), "{errs:?}");
    let method = report.roots.iter().find(|r| r.label == "fun P.get").expect("method root");
    assert!(method.bindings.iter().any(|b| b.name == "this"));
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test methods`
Expected: FAIL (typeck: "only named functions can be called"; sema: lift/borrow assertions).

- [ ] **Step 4: Extract `finish_call`** — in `src/typeck/expr/call.rs`, move everything in `check_call` from `let (regular_params, trailing_signature) = …` to the end into a new function, and call it:

```rust
    let Some((name, span, params, call_return_ty)) = callee_signature(callee, env) else {
        // ... unchanged fallback ...
    };
    finish_call(name, span, params, call_return_ty, None, args, trailing, return_ty, env)
}

/// Checks explicit `args` against `params` (after the receiver, if any) and builds the call.
pub(super) fn finish_call(
    name: String,
    span: Span,
    params: Vec<Ty>,
    call_return_ty: Ty,
    receiver: Option<HirExpr>,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    let explicit = if receiver.is_some() { &params[1..] } else { &params[..] };
    let (regular_params, trailing_signature) =
        split_trailing_param(&name, span, explicit, trailing, env);
    // ... existing body verbatim from `let checked_args …` through the
    //     trailing-closure check and `callee_params` computation ...
    let checked_args: Vec<HirExpr> = receiver.into_iter().chain(checked_args).collect();
    // ... existing `callee_ty` and final `HirExpr::new(HirExprKind::Call { … })` ...
}
```

The `callee_params` print special case reads `checked_args[0]` — keep it **before** the line that prepends the receiver (print is never a method). `callee_ty` uses the full `params`.

- [ ] **Step 5: Method call checking** — create `src/typeck/expr/method.rs`:

```rust
//! Method calls: `recv.m(args)` and bare `m(args)` inside a method.

use crate::ast::{Closure, Expr, UnaryOp};
use crate::hir::{HirExpr, HirExprKind, Ty, TyKind, UseKind};
use crate::span::Span;

use super::super::env::Env;
use super::super::MemberRef;
use super::call::finish_call;
use super::check;

/// `receiver.method(args)`; the receiver is borrowed for the call.
pub(super) fn check_method_call(
    receiver: &Expr,
    method: &str,
    safe: bool,
    span: Span,
    args: &[Expr],
    trailing: Option<&Closure>,
    return_ty: &Ty,
    env: &mut Env<'_>,
) -> HirExpr {
    if !is_place(receiver) {
        env.error(
            "method receiver must be a named value; bind it to a `val` first",
            Some(span),
        );
        return poisoned(args, span, return_ty, env);
    }
    let checked = check(receiver, None, return_ty, env);
    if checked.ty.is_unknown() {
        return poisoned(args, span, return_ty, env);
    }
    if checked.ty.is_managed_ref() {
        env.error("method call on managed Ref requires `if val` or `when`", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    if checked.ty.is_nullable() || safe {
        env.error("method call on a nullable receiver", Some(span));
        return poisoned(args, span, return_ty, env);
    }
    let Some(class) = checked.ty.record_name().map(str::to_string) else {
        env.error("only named functions can be called", Some(span));
        return poisoned(args, span, return_ty, env);
    };
    let qualified = format!("{class}.{method}");
    let (Some(sig), Some(signature)) = (
        env.fun_sig(&qualified).cloned(),
        env.class(&class).and_then(|info| info.methods.get(method).cloned()),
    ) else {
        env.error(format!("class `{class}` has no method `{method}`"), Some(span));
        return poisoned(args, span, return_ty, env);
    };
    env.members.insert(
        span,
        MemberRef::Method {
            class,
            name: method.to_string(),
            signature,
        },
    );
    finish_call(
        qualified,
        span,
        sig.params,
        sig.return_ty,
        Some(borrow(checked)),
        args,
        trailing,
        return_ty,
        env,
    )
}

/// `name(.field)*` — something that has an address the method can borrow.
fn is_place(expr: &Expr) -> bool {
    match expr {
        Expr::Ident { .. } => true,
        Expr::Field {
            receiver,
            safe: false,
            ..
        } => is_place(receiver),
        _ => false,
    }
}

fn borrow(receiver: HirExpr) -> HirExpr {
    let ty = if receiver.ty.is_ref() {
        receiver.ty.clone()
    } else {
        Ty::new(TyKind::Ref(Box::new(receiver.ty.clone())), false)
    };
    let span = receiver.span;
    let receiver = match receiver.kind {
        HirExprKind::Ident { name, .. } => HirExpr {
            kind: HirExprKind::Ident {
                name,
                use_kind: UseKind::Borrow,
            },
            ..receiver
        },
        _ => receiver,
    };
    HirExpr {
        kind: HirExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(receiver),
        },
        ty,
        span,
    }
}

fn poisoned(args: &[Expr], span: Span, return_ty: &Ty, env: &mut Env<'_>) -> HirExpr {
    for arg in args {
        check(arg, None, return_ty, env);
    }
    HirExpr::spanned(HirExprKind::None, Ty::unknown(), span)
}
```

Register `mod method;` in `src/typeck/expr/mod.rs` next to `mod call;`.

- [ ] **Step 6: Dispatch in `check_call`** — at the top of `check_call` in `call.rs` (before the class-constructor check):

```rust
    if let Expr::Field {
        receiver,
        name,
        safe,
        span,
    } = callee
    {
        return super::method::check_method_call(
            receiver, name, *safe, *span, args, trailing, return_ty, env,
        );
    }
    if let Expr::Ident { name, span } = callee {
        if env.implicit_method(name).is_some() {
            let this = Expr::Ident {
                name: "this".to_string(),
                span: *span,
            };
            return super::method::check_method_call(
                &this, name, false, *span, args, trailing, return_ty, env,
            );
        }
    }
```

- [ ] **Step 7: Sema receiver borrow** — in `src/sema/walk.rs`:

Add `use crate::typeck::MemberRef;` to imports.

Replace the start of the `Expr::Call` arm (the `walk(az, callee, …)` call and the `let (formals, param_tys) = match …` block) with:

```rust
            let (formals, param_tys) = if let Some((receiver, qualified)) = method_call(az, callee) {
                walk_receiver(az, &receiver, node);
                (
                    az.fun_sigs.get(&qualified).map(|f| f[1..].to_vec()),
                    az.fun_param_tys.get(&qualified).map(|p| p[1..].to_vec()),
                )
            } else {
                walk(az, callee, node, None, record_borrow);
                match callee.as_ref() {
                    // ... existing Ident / class / fun_sigs lookup unchanged ...
                }
            };
```

Add helper functions near `note_borrow`:

```rust
/// Receiver expression and `C.m` name when typeck resolved `callee` to a method.
fn method_call(az: &Analyzer, callee: &Expr) -> Option<(Expr, String)> {
    let (receiver, span) = match callee {
        Expr::Field { receiver, span, .. } => ((**receiver).clone(), *span),
        Expr::Ident { span, .. } => (
            Expr::Ident {
                name: "this".to_string(),
                span: *span,
            },
            *span,
        ),
        _ => return None,
    };
    match az.members.get(&span)? {
        MemberRef::Method { class, name, .. } => Some((receiver, format!("{class}.{name}"))),
        MemberRef::Field { .. } => None,
    }
}

/// A method receiver is borrowed for the call, like an `&T` argument; a field chain borrows its root.
fn walk_receiver(az: &mut Analyzer, receiver: &Expr, node: &mut ArenaNode) {
    let mut root = receiver;
    while let Expr::Field { receiver, .. } = root {
        root = receiver;
    }
    let Expr::Ident { name, span } = root else {
        walk(az, root, node, None, true);
        return;
    };
    let name = if matches!(az.members.get(span), Some(MemberRef::Field { .. })) {
        "this"
    } else {
        name.as_str()
    };
    let sink = TransferSink::CallArg {
        formal: BindingKind::Val,
    };
    note_borrow(az, name, Some(*span), node, Some(&sink));
}
```

- [ ] **Step 8: Run tests**

Run: `cargo test methods`
Expected: PASS. If `method_call_in_loop_reborrows_this` fails with "cannot lift borrow `this`", check `note_borrow` (`walk.rs:763-772`): the re-borrow exemption requires `TransferSink::CallArg`, which `walk_receiver` passes; confirm the receiver path reaches `note_borrow` and not `note_use`.

- [ ] **Step 9: Corpus fixtures** — `programs/check_fail/methods/`:

`ref_receiver.bork`:
```bork
// diag: method call on managed Ref requires `if val` or `when`
class P {
    fun f() {}
}
fun main() {
    val r: Ref<P> = None
    r.f()
}
```
`nullable_receiver.bork`:
```bork
// diag: method call on a nullable receiver
class P {
    fun f() {}
}
fun g(p: P?) {
    p.f()
}
fun main() {}
```
`temporary_receiver.bork`:
```bork
// diag: method receiver must be a named value
class P {
    x: i32
    fun f(): i32 {
        return this.x
    }
}
fun main(): i32 {
    return P(1).f()
}
```
`unknown_method.bork`:
```bork
// diag: class `P` has no method `g`
class P {
    fun f() {}
}
fun main() {
    val p = P()
    p.g()
}
```
`use_after_move_receiver.bork`:
```bork
// diag: use of `p` after move
class P {
    x: i32
    fun get(): i32 {
        return this.x
    }
}
fun main(): i32 {
    val p = P(1)
    val q = move p
    return p.get()
}
```
`wrong_arg_count.bork`:
```bork
// diag: function `P.f` expects 1 arguments, got 0
class P {
    fun f(a: i32) {}
}
fun main() {
    val p = P()
    p.f()
}
```

- [ ] **Step 10: Full suite**

Run: `cargo test && cargo test programs_corpus_check`
Expected: PASS.

- [ ] **Step 11: Commit**

```bash
git add src/typeck src/sema programs/check_fail/methods
git commit -m "feat(typeck): lower method calls to borrowed-receiver calls

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Implicit `this` for fields

**Files:**
- Modify: `src/typeck/expr/mod.rs` (`Expr::Ident` arm, line 71)
- Modify: `src/typeck/expr/method.rs` (add `implicit_field`)
- Modify: `src/typeck/stmt/assign.rs` (`AssignTarget::Name` arm)
- Modify: `src/sema/walk.rs` (`Expr::Ident` arm line 306, `Stmt::Assign` arm line 169)
- Test: `src/typeck/tests/methods.rs`, `src/sema/tests/methods.rs`

**Interfaces:**
- Consumes: `Env::implicit_field`, `check_field` (`src/typeck/expr/field.rs`), `check_receiver_field` (`src/typeck/stmt/assign.rs`), `MemberRef::Field`.
- Produces: `MemberRef::Field` recorded under the bare identifier span (read and assignment); for assignment `span_tys[name_span] = field type` (written by `check_receiver_field`).

- [ ] **Step 1: Failing typeck tests** — append to `src/typeck/tests/methods.rs`:

```rust
#[test]
fn bare_field_reads_and_writes_go_through_this() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun bump(): i32 {\n        x = x + 1\n        return x\n    }\n}\nfun main() {}",
    );
    let method = hir.functions.iter().find(|f| f.name == "P.bump").unwrap();
    let HirStmt::Assign { target, .. } = &method.body.stmts[0] else { panic!("assign") };
    assert!(matches!(target, crate::hir::HirAssignTarget::Field { field_index: 0, .. }));
    let HirExprKind::ObjectField { receiver, .. } = &call_in_return(&hir, "P.bump").kind else {
        panic!("bare x reads the field");
    };
    assert!(matches!(&receiver.kind, HirExprKind::Ident { name, .. } if name == "this"));
}

#[test]
fn bare_field_then_local_shadow() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun f(): i32 {\n        val a = x\n        val x = 5\n        return a + x\n    }\n}\nfun main() {}",
    );
    let method = hir.functions.iter().find(|f| f.name == "P.f").unwrap();
    let HirStmt::VarDecl { value, .. } = &method.body.stmts[0] else { panic!("val a") };
    assert!(matches!(&value.kind, HirExprKind::ObjectField { .. }), "before the local: field");
    let HirExprKind::Binary { rhs, .. } = &call_in_return(&hir, "P.f").kind else { panic!("+") };
    assert!(matches!(&rhs.kind, HirExprKind::Ident { name, .. } if name == "x"), "after the local: local");
}

#[test]
fn bare_field_outside_method_is_unknown() {
    let diags = diags_of("class P {\n    x: i32\n}\nfun main(): i32 {\n    return x\n}");
    assert!(diags.iter().any(|d| d.message.contains("unknown binding `x`")), "{diags:?}");
}
```

`HirStmt::VarDecl` field names: check `src/hir/mod.rs` for the exact variant shape (`value` field) and adjust the pattern if it differs.

- [ ] **Step 2: Failing sema test** — append to `src/sema/tests/methods.rs`:

```rust
#[test]
fn bare_field_use_observes_this() {
    let (report, errs) = analyze(&parse(
        "class P {\n    name: String\n    fun f(): i32 {\n        name = \"b\"\n        return name.length\n    }\n}\nfun main() {}",
    ).unwrap());
    assert!(errs.is_empty(), "{errs:?}");
    let method = report.roots.iter().find(|r| r.label == "fun P.f").unwrap();
    assert!(method.observations.iter().any(|o| o.name == "this"), "{:?}", method.observations);
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test methods`
Expected: FAIL ("unknown binding `x`" inside the method).

- [ ] **Step 4: Typeck implicit field read** — add to `src/typeck/expr/method.rs`:

```rust
/// Bare `name` inside a method that names a field of the enclosing class: `this.name`.
pub(super) fn implicit_field(name: &str, span: Span, return_ty: &Ty, env: &mut Env<'_>) -> Option<HirExpr> {
    let (class, index, ty) = env.implicit_field(name)?;
    env.members.insert(
        span,
        MemberRef::Field {
            class,
            name: name.to_string(),
            index,
            ty,
        },
    );
    let this = Expr::Ident {
        name: "this".to_string(),
        span,
    };
    Some(super::field::check_field(&this, name, false, span, return_ty, env))
}
```

In `src/typeck/expr/mod.rs`, replace the `Expr::Ident` arm:
```rust
        Expr::Ident { name, span } => method::implicit_field(name, *span, return_ty, env)
            .unwrap_or_else(|| check_ident(name, *span, UseKind::Local, env)),
```

- [ ] **Step 5: Typeck implicit field assignment** — in `src/typeck/stmt/assign.rs`, in the `AssignTarget::Name` arm right after the `this` check from Task 2:

```rust
            if let Some((class, index, ty)) = env.implicit_field(name) {
                env.members.insert(
                    *name_span,
                    crate::typeck::MemberRef::Field {
                        class,
                        name: name.clone(),
                        index,
                        ty,
                    },
                );
                let this = Expr::Ident {
                    name: "this".to_string(),
                    span: *name_span,
                };
                return check_receiver_field(&this, name, *name_span, value, return_ty, env);
            }
```

- [ ] **Step 6: Sema bare fields** — in `src/sema/walk.rs`:

At the top of the `Expr::Ident { name, span } =>` arm in `walk`:
```rust
            if matches!(az.members.get(span), Some(MemberRef::Field { .. })) {
                // Bare field inside a method: a read through the `this` borrow.
                note_use(az, "this", Some(*span), node);
                return;
            }
```

At the top of the `Stmt::Assign { target, value } =>` arm in `walk_stmt`:
```rust
            if let AssignTarget::Name { name_span, .. } = target {
                if matches!(az.members.get(name_span), Some(MemberRef::Field { .. })) {
                    let target_ty = az.span_tys.get(name_span).cloned().map(ty_from_hir);
                    note_use(az, "this", Some(*name_span), node);
                    walk_typed(
                        az,
                        value,
                        node,
                        None,
                        true,
                        target_ty.and_then(|ty| ty.as_option()).as_ref(),
                    );
                    return;
                }
            }
```

- [ ] **Step 7: Run tests**

Run: `cargo test methods && cargo test && cargo test programs_corpus_check`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/typeck src/sema
git commit -m "feat(typeck): resolve bare field names through this

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Codegen — build corpus and gate

**Files:**
- Create: `programs/build/methods/*.bork` (below)
- Create: `programs/check/methods/trailing_closure.bork`, `programs/build_fail/methods/trailing_closure.bork`
- Modify: `src/codegen_gate.rs` (test module)
- Possibly modify: `src/codegen/llvm/expr.rs` / `src/codegen/llvm/managed_ref.rs` (only if a fixture fails)

**Interfaces:**
- Consumes: HIR from Tasks 2-4 (`HirFunction "C.m"`, `Call(Ident "C.m", [Borrow(recv), …])`).
- Produces: runnable binaries; LLVM symbol `@bork.C.m` (from existing `declare_function`, `src/codegen/llvm/emit_fn.rs:63`).

- [ ] **Step 1: Gate test** — add to `src/codegen_gate.rs` tests:

```rust
    #[test]
    fn methods_pass_codegen_gate() {
        let source = "class P {\n    x: i32\n    fun get(): i32 {\n        return x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.get()\n}";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        assert!(super::gate(result.hir.as_ref().unwrap()).is_empty());
    }
```

Run: `cargo test methods_pass_codegen_gate` — Expected: PASS (no new gate rules needed).

- [ ] **Step 2: Build fixtures** — create `programs/build/methods/`:

`basic.bork`:
```bork
// exit: 7
class Point {
    x: i32
    y: i32

    fun sum(): i32 {
        return x + y
    }
}

fun main(): i32 {
    val p = Point(3, 4)
    return p.sum()
}
```
`explicit_this.bork`:
```bork
// exit: 9
class Point {
    x: i32

    fun set(x: i32) {
        this.x = x
    }
}

fun main(): i32 {
    val p = Point(1)
    p.set(9)
    return p.x
}
```
`mutate_field.bork`:
```bork
// exit: 3
class Counter {
    n: i32

    fun bump() {
        n = n + 1
    }
}

fun main(): i32 {
    val c = Counter(0)
    c.bump()
    c.bump()
    c.bump()
    return c.n
}
```
`method_calls_method.bork`:
```bork
// exit: 30
class Point {
    x: i32
    y: i32

    fun sum(): i32 {
        return x + y
    }
    fun twice(): i32 {
        return sum() + this.sum()
    }
    fun fact(k: i32): i32 {
        if (k <= 1) {
            return 1
        }
        return k * fact(k - 1)
    }
}

fun main(): i32 {
    val p = Point(1, 2)
    return p.twice() + p.fact(3) + 18
}
```
`borrowed_receiver.bork`:
```bork
// exit: 5
class Point {
    x: i32

    fun get(): i32 {
        return x
    }
}

fun read(p: &Point): i32 {
    return p.get()
}

fun main(): i32 {
    val p = Point(5)
    return read(&p)
}
```
`field_receiver.bork`:
```bork
// exit: 14
class Inner {
    n: i32

    fun get(): i32 {
        return n
    }
}
class Outer {
    inner: Inner

    fun deep(): i32 {
        return this.inner.get() + inner.get()
    }
}

fun main(): i32 {
    val o = Outer(Inner(7))
    return o.deep() + o.inner.get() - 7
}
```
`owning_return.bork`:
```bork
// exit: 8
// stdout: hello\n
class Point {
    name: String
    x: i32

    fun label(): String {
        return concat(name, "lo")
    }
    fun moved(dx: i32): Point {
        return Point("p", x + dx)
    }
}

fun main(): i32 {
    val p = Point("hel", 3)
    println(p.label())
    val q = p.moved(5)
    return q.x
}
```
`if_val_receiver.bork`:
```bork
// exit: 4
class Node {
    v: i32
    next: Ref<Node>

    fun value(): i32 {
        return v
    }
    fun next_value(): i32 {
        if val n = this.next {
            return n.value()
        }
        return 0
    }
}

fun main(): i32 {
    val a = Node(1)
    val b = Node(3, a)
    return b.next_value() + b.value()
}
```
`same_name.bork`:
```bork
// exit: 12
fun size(): i32 {
    return 10
}
class A {
    fun size(): i32 {
        return 1
    }
    fun total(): i32 {
        return size()
    }
}
class B {
    fun size(): i32 {
        return 1
    }
}

fun main(): i32 {
    val a = A()
    val b = B()
    return a.total() + b.size() + size()
}
```
`nested_calls.bork`:
```bork
// exit: 10
class Point {
    x: i32

    fun get(): i32 {
        return x
    }
    fun add(k: i32): i32 {
        return x + k
    }
}

fun main(): i32 {
    val p = Point(3)
    val q = Point(4)
    return p.add(q.get()) + p.get()
}
```

Check `concat`'s signature in `src/builtins.rs` before writing `owning_return.bork`; if `concat` requires `move` for owned `String` arguments, write `concat(name, "lo")` accordingly so the fixture checks cleanly (the point of the fixture is `OwningReturn` ABI on a method, not `concat` semantics).

- [ ] **Step 3: Trailing-closure fixtures**

`programs/check/methods/trailing_closure.bork`:
```bork
class P {
    x: i32

    fun apply(k: i32, f: (i32) -> i32): i32 {
        return f(k)
    }
}

fun main(): i32 {
    val p = P(1)
    return p.apply(2) { v -> v + 1 }
}
```
`programs/build_fail/methods/trailing_closure.bork`: same source.

Check the existing `build_fail` fixture (`programs/build_fail/escape_inner_return_string.bork`) for whether a `// diag:` directive is required; if so add `// diag: trailing` to the build_fail copy.

- [ ] **Step 4: Run the corpus with LLVM**

Run: `cargo test --features codegen programs_corpus`
Expected: PASS. If a fixture fails:
- Use superpowers:systematic-debugging; dump IR with `cargo run --features codegen -- build <file>` plus the repo's IR dump option (see `src/main.rs` flags) before changing code.
- Most likely site: `field_receiver.bork` — `Borrow(ObjectField(…))` must yield the field object's `{arena, ptr}` handle. `Unary Borrow` returns its operand (`src/codegen/llvm/expr.rs:214`) and `emit_object_field` loads the field slot (`src/codegen/llvm/managed_ref.rs:230`); a record-typed field slot must hold a handle for this to work. Fix there, keep the fixture.
- Second most likely: `memory::plan` / `escape::check_function` rejecting `Borrow` over a non-`Ident` operand; fix by treating it like the `Ident` case for origins.

- [ ] **Step 5: Full suite**

Run: `cargo test && cargo test --features codegen`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add programs src/codegen_gate.rs src/codegen
git commit -m "test(codegen): build and run class method programs

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: LSP hover for members

**Files:**
- Modify: `src/frontend.rs` (`CheckResult.members`)
- Modify: `src/lsp.rs` (`Analysis.members`, `member_hover`, `hover`)
- Modify: `src/bin/bork_lsp.rs:1,105-109`
- Test: `src/lsp.rs` test module

**Interfaces:**
- Consumes: `typeck::{MemberRef, MemberResolutions}`; `CheckResult` from `frontend::check`.
- Produces: `CheckResult.members: MemberResolutions` (empty on parse error); `pub fn lsp::hover(analysis: &Analysis, source: &str, position: Position) -> Option<String>` (binding hover first, then member hover). `hover_for_analysis` keeps its signature.

- [ ] **Step 1: Failing LSP tests** — add to the `src/lsp.rs` test module:

```rust
    const METHOD_SRC: &str = "class Point {\n    x: i32\n    fun sum(k: i32): i32 {\n        return this.x + x + k\n    }\n}\nfun main(): i32 {\n    val p = Point(1)\n    return p.sum(2)\n}\n";

    fn hover_at(source: &str, needle: &str, nth: usize) -> String {
        let offset = source.match_indices(needle).nth(nth).expect("needle").0;
        let analysis = analyze_source(source);
        hover(&analysis, source, byte_offset_to_position(source, offset)).expect("hover")
    }

    #[test]
    fn hover_on_this_shows_borrow_of_class() {
        let h = hover_at(METHOD_SRC, "this", 0);
        assert!(h.contains("fun Point.sum"), "{h}");
        assert!(h.contains("&Point"), "{h}");
    }

    #[test]
    fn hover_on_bare_field_shows_field_via_this() {
        // `this` is used before bare `x` so its first observation carries the `this` span.
        let h = hover_at(METHOD_SRC, "x + k", 0);
        assert!(h.contains("field `x` of `Point`"), "{h}");
        assert!(h.contains("i32"), "{h}");
    }

    #[test]
    fn hover_on_method_call_and_declaration_shows_signature() {
        for nth in [0, 1] {
            let h = hover_at(METHOD_SRC, "sum", nth);
            assert!(h.contains("fun Point.sum(k: i32): i32"), "{h}");
        }
    }

    #[test]
    fn move_this_diagnostic_has_range() {
        let source = "class P {\n    x: i32\n    fun f() {\n        val q = move this\n    }\n}\nfun main() {}\n";
        let diagnostics = diagnostics_for_source(source);
        let d = diagnostics
            .iter()
            .find(|d| d.message.contains("cannot move `this`"))
            .expect("diagnostic");
        assert_eq!(d.range.start.line, 3);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib lsp::`
Expected: compile error (`hover` not found).

- [ ] **Step 3: Frontend** — in `src/frontend.rs`:

Add to `CheckResult`:
```rust
    /// Member names resolved by typeck (bare fields, method calls, method declarations).
    pub members: crate::typeck::MemberResolutions,
```
In the parse-error early return add `members: Default::default(),`. Change the typeck/sema call to:
```rust
    let (hir, span_tys, members, mut type_diagnostics) = typeck::check_with_members(&program);
    let (mut report, ownership_errors) =
        crate::sema::analyze_with_members(&program, span_tys, members.clone());
```
and add `members,` to the final `CheckResult { … }`.

- [ ] **Step 4: LSP** — in `src/lsp.rs`:

Add `members: crate::typeck::MemberResolutions,` to `Analysis`; set `members: result.members,` in `analyze_source` (move `result.report` and `result.members` out; `result.hir` is borrowed earlier for the gate — keep that borrow before the moves).

Add after `hover_for_analysis`:
```rust
/// Hover for a binding (via the arena report) or, failing that, a class member.
pub fn hover(analysis: &Analysis, source: &str, position: Position) -> Option<String> {
    analysis
        .report()
        .and_then(|report| hover_for_analysis(report, source, position))
        .or_else(|| member_hover(&analysis.members, source, position))
}

fn member_hover(
    members: &crate::typeck::MemberResolutions,
    source: &str,
    position: Position,
) -> Option<String> {
    use crate::typeck::MemberRef;
    let map = SourceMap::new(source);
    let word = map.word_at(position)?;
    let offset = map.position_to_offset(position)?;
    members
        .iter()
        .filter(|(span, member)| {
            span_contains(**span, offset)
                && match member {
                    MemberRef::Field { name, .. } | MemberRef::Method { name, .. } => *name == word,
                }
        })
        .min_by_key(|(span, _)| span.end - span.start)
        .map(|(_, member)| match member {
            MemberRef::Field { class, name, ty, .. } => {
                format!("field `{name}` of `{class}` (via `this`)\nType: `{ty}`")
            }
            MemberRef::Method { signature, .. } => format!("`{signature}`"),
        })
}
```

- [ ] **Step 5: Server** — in `src/bin/bork_lsp.rs` import `hover` instead of `hover_for_analysis` and replace the `let value = …` expression with:
```rust
        let value = hover(&doc.analysis, &doc.text, pos);
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib lsp:: && cargo build --bin bork-lsp`
Expected: PASS. (Binary name per `Cargo.toml`; if it is `bork_lsp`, use that.) If `hover_on_this_shows_borrow_of_class` fails because the `this` observation has no span, check `note_use`/`record_observation` in `src/sema/walk.rs:803-830`: uses of `this` are recorded with the use span; the param itself has `span: None`.

- [ ] **Step 7: Commit**

```bash
git add src/frontend.rs src/lsp.rs src/bin/bork_lsp.rs
git commit -m "feat(lsp): hover class members and this

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Documentation

**Files:**
- Modify: `docs/language.md:41-76`
- Modify: `docs/walkthrough-classes-functions.md` (sentence "Klasy w tym branchu mają tylko pola. Metod klasowych nie ma, …")
- Modify: `programs/README.md` (topics table)

- [ ] **Step 1: `docs/language.md`** — in the Classes bullet list change "fields in declaration order, no methods and no inheritance" to "fields in declaration order, methods, no inheritance". Append after the `Ref<T>` bullets (before "For the complete strong/weak…"):

````markdown
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
- Not supported: static/companion methods, visibility, inheritance, interfaces, extension functions, method references, `?.m()`.
````

- [ ] **Step 2: Walkthrough** — replace "Klasy w tym branchu mają tylko pola. Metod klasowych nie ma, więc operacje na klasie są funkcjami top-level, tak jak `sum`." with:

```markdown
Ten przykład używa funkcji top-level `sum`. Klasy mogą też mieć metody z `this`;
opisuje je [language.md](language.md#methods-and-this). Typeck zamienia metodę
`Point.sum` na zwykłą funkcję z parametrem `this: &Point`, więc dalsza część
pipeline'u wygląda tak samo jak dla `sum` poniżej.
```

- [ ] **Step 3: `programs/README.md`** — add a row to the Topics table after `classes/`:
```markdown
| [`methods/`](build/methods/) | Class methods, `this`, implicit members, receivers |
```

- [ ] **Step 4: Commit**

```bash
git add docs/language.md docs/walkthrough-classes-functions.md programs/README.md
git commit -m "docs: describe class methods and this

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Final verification

- [ ] `cargo test && cargo test --features codegen` — all green.
- [ ] `cargo clippy --all-targets --features codegen` — no new warnings in touched files.
- [ ] Manually: `cargo run --features codegen -- build programs/build/methods/basic.bork` produces a binary returning 7.
