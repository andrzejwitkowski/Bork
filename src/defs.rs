//! Definition table: a stable `DefId` for every class, function, method, constant and builtin.
//!
//! Built once per program. It also owns the same-name and builtin-clash rules, so the checkers
//! (`typeck`, `constants`) ask the table instead of each re-deriving them. `Named(String)` in
//! types is unchanged for now (#32).

use std::collections::{HashMap, HashSet};

use crate::ast::{Function, Program, Type};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Class,
    Fn,
    Method,
    Const,
    Intrinsic,
}

/// A declaration that clashes with another one. Each variant is reported at the site that
/// handles that declaration, so the diagnostic order matches the checking order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clash {
    /// A function named like a builtin.
    RedefinesBuiltin,
    ConstBuiltin,
    ConstFn,
    ConstClass,
    /// A class named like a primitive or `String`.
    ClassBuiltinType,
    ClassDuplicate,
    /// A class named like a function (including `print`/`println`/`concat`).
    ClassFn,
    MethodFieldName,
    MethodDuplicate,
}

#[derive(Debug, Clone)]
pub struct Def {
    pub id: DefId,
    pub kind: DefKind,
    pub name: String,
    /// The class of a method.
    pub owner: Option<DefId>,
    clashes: Vec<Clash>,
    /// Number of type parameters (classes only).
    pub arity: usize,
    /// Where this definition's clash diagnostics point (the AST has no class name span).
    span: Option<Span>,
}

#[derive(Debug, Default)]
pub struct DefTable {
    defs: Vec<Def>,
    classes: HashMap<String, DefId>,
    fns: HashMap<String, DefId>,
    consts: HashMap<String, DefId>,
    methods: HashMap<(DefId, String), DefId>,
    intrinsics: HashMap<String, DefId>,
    // Program order, parallel to `Program::{constants, functions, classes}` and `Class::methods`.
    const_ids: Vec<DefId>,
    fn_ids: Vec<DefId>,
    class_ids: Vec<DefId>,
    method_ids: Vec<Vec<DefId>>,
}

impl DefTable {
    pub fn build(program: &Program) -> DefTable {
        let mut table = DefTable::default();
        for name in INTRINSICS {
            let id = table.add(DefKind::Intrinsic, name, None, None);
            table.intrinsics.insert(name.to_string(), id);
        }

        for function in &program.functions {
            let id = table.add(DefKind::Fn, &function.name, None, builtin_redefine_span(function));
            if crate::builtins::is_intrinsic(&function.name) {
                table.defs[id.0 as usize].clashes.push(Clash::RedefinesBuiltin);
            } else {
                table.fns.insert(function.name.clone(), id);
            }
            table.fn_ids.push(id);
        }

        // Names a class must not reuse: user functions and the builtins that have a signature.
        let fn_names: HashSet<String> = program
            .functions
            .iter()
            .filter(|f| !crate::builtins::is_intrinsic(&f.name))
            .map(|f| f.name.clone())
            .chain(crate::builtins::signature_names().map(String::from))
            .collect();
        for class in &program.classes {
            let span = class.fields.first().map(|field| field.name.span);
            let id = table.add(DefKind::Class, &class.name, None, span);
            let mut clashes = Vec::new();
            if class.name == "String"
                || !matches!(Type::from_ident(&class.name, false), Type::Named { .. })
            {
                clashes.push(Clash::ClassBuiltinType);
            }
            if table.classes.contains_key(&class.name) {
                clashes.push(Clash::ClassDuplicate);
            } else {
                table.classes.insert(class.name.clone(), id);
            }
            if fn_names.contains(&class.name) {
                clashes.push(Clash::ClassFn);
            }
            table.defs[id.0 as usize].clashes = clashes;
            table.defs[id.0 as usize].arity = class.generics.len();
            table.class_ids.push(id);
        }

        let mut qualified = HashSet::new();
        for class in &program.classes {
            let owner = table.classes[&class.name];
            let mut ids = Vec::new();
            for method in &class.methods {
                let id = table.add(DefKind::Method, &method.name, Some(owner), Some(method.name_span));
                let mut clashes = Vec::new();
                if class.fields.iter().any(|field| field.name.name == method.name) {
                    clashes.push(Clash::MethodFieldName);
                }
                if !qualified.insert((class.name.clone(), method.name.clone())) {
                    clashes.push(Clash::MethodDuplicate);
                } else {
                    table.methods.insert((owner, method.name.clone()), id);
                }
                table.defs[id.0 as usize].clashes = clashes;
                ids.push(id);
            }
            table.method_ids.push(ids);
        }

        for decl in &program.constants {
            let name = &decl.name.name;
            let id = table.add(DefKind::Const, name, None, Some(decl.name.span));
            let clash = if crate::builtins::is_intrinsic(name) {
                Some(Clash::ConstBuiltin)
            } else if program.functions.iter().any(|f| f.name == *name) {
                Some(Clash::ConstFn)
            } else if program.classes.iter().any(|c| c.name == *name) {
                Some(Clash::ConstClass)
            } else {
                None
            };
            table.defs[id.0 as usize].clashes.extend(clash);
            table.consts.entry(name.clone()).or_insert(id);
            table.const_ids.push(id);
        }
        table
    }

    fn add(&mut self, kind: DefKind, name: &str, owner: Option<DefId>, span: Option<Span>) -> DefId {
        let id = DefId(self.defs.len() as u32);
        self.defs.push(Def { id, kind, name: name.to_string(), owner, clashes: Vec::new(), arity: 0, span });
        id
    }

    pub fn get(&self, id: DefId) -> &Def {
        &self.defs[id.0 as usize]
    }

    /// The first class declared with this name.
    pub fn lookup_class(&self, name: &str) -> Option<DefId> {
        self.classes.get(name).copied()
    }

    /// Every class by name (the first declaration of each name), in no particular order.
    pub fn classes(&self) -> impl Iterator<Item = (&str, DefId)> {
        self.classes.iter().map(|(name, id)| (name.as_str(), *id))
    }

    /// A user function (never a builtin; see [`DefTable::lookup_intrinsic`]).
    pub fn lookup_fn(&self, name: &str) -> Option<DefId> {
        self.fns.get(name).copied()
    }

    pub fn lookup_const(&self, name: &str) -> Option<DefId> {
        self.consts.get(name).copied()
    }

    pub fn lookup_method(&self, class: DefId, name: &str) -> Option<DefId> {
        self.methods.get(&(class, name.to_string())).copied()
    }

    pub fn lookup_intrinsic(&self, name: &str) -> Option<DefId> {
        self.intrinsics.get(name).copied()
    }

    pub fn const_id(&self, index: usize) -> DefId {
        self.const_ids[index]
    }

    pub fn fn_id(&self, index: usize) -> DefId {
        self.fn_ids[index]
    }

    pub fn class_id(&self, index: usize) -> DefId {
        self.class_ids[index]
    }

    pub fn method_id(&self, class_index: usize, method_index: usize) -> DefId {
        self.method_ids[class_index][method_index]
    }

    /// The diagnostics for the clashes of `id` selected by `pick`, in detection order.
    pub fn diagnostics(&self, id: DefId, pick: impl Fn(Clash) -> bool) -> Vec<Diagnostic> {
        let def = self.get(id);
        def.clashes
            .iter()
            .copied()
            .filter(|&clash| pick(clash))
            .map(|clash| Diagnostic {
                phase: Phase::Type,
                severity: Severity::Error,
                message: self.message(def, clash),
                span: def.span,
            })
            .collect()
    }

    fn message(&self, def: &Def, clash: Clash) -> String {
        let name = &def.name;
        match clash {
            Clash::RedefinesBuiltin => format!("cannot redefine builtin function `{name}`"),
            Clash::ConstBuiltin => format!("constant `{name}` conflicts with a builtin function"),
            Clash::ConstFn => format!("constant `{name}` conflicts with a function"),
            Clash::ConstClass => format!("constant `{name}` conflicts with a class"),
            Clash::ClassBuiltinType => format!("class `{name}` conflicts with a builtin type name"),
            Clash::ClassDuplicate => format!("duplicate class `{name}`"),
            Clash::ClassFn => format!("class `{name}` conflicts with another type or function"),
            Clash::MethodFieldName | Clash::MethodDuplicate => {
                let class = def.owner.map(|owner| self.get(owner).name.as_str()).unwrap_or("?");
                if clash == Clash::MethodFieldName {
                    format!("class `{class}` already has a field named `{name}`")
                } else {
                    format!("duplicate method `{name}` in class `{class}`")
                }
            }
        }
    }
}

/// Builtin functions, registered as `Intrinsic` definitions (mirrors `builtins::resolve`).
const INTRINSICS: [&str; 6] = ["print", "println", "concat", "stringFromUtf8", "strongCount", "weakCount"];

/// A builtin redefinition is reported at the first parameter (the AST keeps no span for the keyword).
fn builtin_redefine_span(function: &Function) -> Option<Span> {
    function.params.first().map(|param| param.name.span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn table(source: &str) -> (Program, DefTable) {
        let program = parse(source).expect("parses");
        let table = DefTable::build(&program);
        (program, table)
    }

    #[test]
    fn every_declaration_gets_a_distinct_def_with_the_right_kind() {
        let (_, t) = table(
            "const N: i32 = 1\nclass C {\n x: i32\n fun m() {}\n}\nfun f() {}\nfun main() {}\n",
        );
        let c = t.lookup_class("C").unwrap();
        let m = t.lookup_method(c, "m").unwrap();
        let ids = [c, m, t.lookup_fn("f").unwrap(), t.lookup_fn("main").unwrap(), t.lookup_const("N").unwrap()];
        let kinds = [DefKind::Class, DefKind::Method, DefKind::Fn, DefKind::Fn, DefKind::Const];
        for (id, kind) in ids.iter().zip(kinds) {
            assert_eq!(t.get(*id).kind, kind);
        }
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(t.get(m).owner, Some(c));
    }

    #[test]
    fn lookups_miss_and_builtins_are_intrinsics() {
        let (_, t) = table("fun main() {}\n");
        assert!(t.lookup_class("Nope").is_none());
        assert!(t.lookup_fn("print").is_none());
        let print = t.lookup_intrinsic("print").unwrap();
        assert_eq!(t.get(print).kind, DefKind::Intrinsic);
        assert!(crate::builtins::is_intrinsic("weakCount") && t.lookup_intrinsic("weakCount").is_some());
    }

    #[test]
    fn intrinsic_list_matches_builtins() {
        for name in INTRINSICS {
            assert!(crate::builtins::is_intrinsic(name), "{name}");
        }
        assert!(!crate::builtins::is_intrinsic("notABuiltin"));
    }

    #[test]
    fn duplicates_get_their_own_ids_and_clash() {
        let (p, t) = table("class C {\n a: i32\n}\nclass C {\n b: i32\n}\nfun main() {}\n");
        assert_ne!(t.class_id(0), t.class_id(1));
        assert_eq!(t.lookup_class("C"), Some(t.class_id(0)));
        assert!(t.diagnostics(t.class_id(0), |_| true).is_empty());
        let dup = t.diagnostics(t.class_id(1), |c| c == Clash::ClassDuplicate);
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].span, p.classes[1].fields.first().map(|f| f.name.span));
    }
}
