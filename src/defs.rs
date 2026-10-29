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
    Enum,
    Variant,
    Fn,
    Method,
    Const,
    Intrinsic,
    Trait,
    /// A trait method signature; its owner is the trait.
    TraitMethod,
    /// An `impl Trait for Type` block.
    Impl,
    /// A method of an impl; its owner is the impl.
    ImplMethod,
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
    ConstEnum,
    /// A class named like a primitive or `String`.
    ClassBuiltinType,
    ClassDuplicate,
    /// A class named like a function (including `print`/`println`/`concat`).
    ClassFn,
    /// An enum named like a class, another enum, a primitive, `String` or a function.
    EnumConflict,
    VariantDuplicate,
    MethodFieldName,
    MethodDuplicate,
    /// A trait named like another trait, a class, an enum, a primitive, `String` or a function.
    TraitConflict,
    TraitMethodDuplicate,
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
    enums: HashMap<String, DefId>,
    /// The enum that is `T?`: named `Option`, one parameter, `Some(value)` and a payload-free `None`.
    option: Option<DefId>,
    variants: HashMap<(DefId, String), DefId>,
    fns: HashMap<String, DefId>,
    consts: HashMap<String, DefId>,
    methods: HashMap<(DefId, String), DefId>,
    intrinsics: HashMap<String, DefId>,
    traits: HashMap<String, DefId>,
    trait_items: HashMap<(DefId, String), DefId>,
    // Program order, parallel to `Program::{constants, functions, classes}` and `Class::methods`.
    const_ids: Vec<DefId>,
    fn_ids: Vec<DefId>,
    class_ids: Vec<DefId>,
    enum_ids: Vec<DefId>,
    variant_ids: Vec<Vec<DefId>>,
    method_ids: Vec<Vec<DefId>>,
    trait_ids: Vec<DefId>,
    trait_method_ids: Vec<Vec<DefId>>,
    impl_ids: Vec<DefId>,
    impl_method_ids: Vec<Vec<DefId>>,
}

fn is_option_shaped(e: &crate::ast::Enum) -> bool {
    let arity = |name: &str| e.variants.iter().find(|v| v.name.name == name).map(|v| v.fields.len());
    e.generics.len() == 1 && e.variants.len() == 2 && arity("Some") == Some(1) && arity("None") == Some(0)
}

impl DefTable {
    pub fn build(program: &Program) -> DefTable {
        let mut table = DefTable::default();
        table.add_intrinsics();
        table.add_functions(program);
        // Names a class or enum must not reuse: user functions and the builtins that have a signature.
        let fn_names: HashSet<String> = program
            .functions
            .iter()
            .filter(|f| !crate::builtins::is_intrinsic(&f.name))
            .map(|f| f.name.clone())
            .chain(crate::builtins::signature_names().map(String::from))
            .collect();
        table.add_classes(program, &fn_names);
        table.add_enums(program, &fn_names);
        table.add_methods(program);
        table.add_traits(program, &fn_names);
        table.add_impls(program);
        table.add_constants(program);
        table
    }

    fn add_intrinsics(&mut self) {
        for name in INTRINSICS {
            let id = self.add(DefKind::Intrinsic, name, None, None);
            self.intrinsics.insert(name.to_string(), id);
        }
    }

    fn add_functions(&mut self, program: &Program) {
        for function in &program.functions {
            let id = self.add(DefKind::Fn, &function.name, None, builtin_redefine_span(function));
            if crate::builtins::is_intrinsic(&function.name) {
                self.defs[id.0 as usize].clashes.push(Clash::RedefinesBuiltin);
            } else {
                self.fns.insert(function.name.clone(), id);
            }
            self.fn_ids.push(id);
        }
    }

    fn add_classes(&mut self, program: &Program, fn_names: &HashSet<String>) {
        for class in &program.classes {
            let span = class.fields.first().map(|field| field.name.span);
            let id = self.add(DefKind::Class, &class.name, None, span);
            let mut clashes = Vec::new();
            if is_builtin_type_name(&class.name) {
                clashes.push(Clash::ClassBuiltinType);
            }
            if self.classes.contains_key(&class.name) {
                clashes.push(Clash::ClassDuplicate);
            } else {
                self.classes.insert(class.name.clone(), id);
            }
            if fn_names.contains(&class.name) {
                clashes.push(Clash::ClassFn);
            }
            self.defs[id.0 as usize].clashes = clashes;
            self.defs[id.0 as usize].arity = class.generics.len();
            self.class_ids.push(id);
        }
    }

    fn add_enums(&mut self, program: &Program, fn_names: &HashSet<String>) {
        for e in &program.enums {
            let name = &e.name.name;
            let id = self.add(DefKind::Enum, name, None, Some(e.name.span));
            let conflict = self.type_name_taken(name) || fn_names.contains(name);
            if conflict {
                self.defs[id.0 as usize].clashes.push(Clash::EnumConflict);
            } else {
                self.enums.insert(name.clone(), id);
                if name == "Option" && is_option_shaped(e) {
                    self.option = Some(id);
                }
            }
            self.defs[id.0 as usize].arity = e.generics.len();
            self.enum_ids.push(id);
            let variants = self.add_variants(id, e, conflict);
            self.variant_ids.push(variants);
        }
    }

    /// A rejected enum keeps its variants out of the table, so one mistake is one error.
    fn add_variants(&mut self, owner: DefId, e: &crate::ast::Enum, rejected: bool) -> Vec<DefId> {
        let mut ids = Vec::new();
        for variant in &e.variants {
            let id = self.add(DefKind::Variant, &variant.name.name, Some(owner), Some(variant.name.span));
            if !rejected && self.variants.insert((owner, variant.name.name.clone()), id).is_some() {
                self.defs[id.0 as usize].clashes.push(Clash::VariantDuplicate);
            }
            ids.push(id);
        }
        ids
    }

    fn add_methods(&mut self, program: &Program) {
        let mut qualified = HashSet::new();
        for class in &program.classes {
            let owner = self.classes[&class.name];
            let mut ids = Vec::new();
            for method in &class.methods {
                let id = self.add(DefKind::Method, &method.name, Some(owner), Some(method.name_span));
                let mut clashes = Vec::new();
                if class.fields.iter().any(|field| field.name.name == method.name) {
                    clashes.push(Clash::MethodFieldName);
                }
                if !qualified.insert((class.name.clone(), method.name.clone())) {
                    clashes.push(Clash::MethodDuplicate);
                } else {
                    self.methods.insert((owner, method.name.clone()), id);
                }
                self.defs[id.0 as usize].clashes = clashes;
                ids.push(id);
            }
            self.method_ids.push(ids);
        }
    }

    fn add_traits(&mut self, program: &Program, fn_names: &HashSet<String>) {
        for t in &program.traits {
            let name = &t.name.name;
            let id = self.add(DefKind::Trait, name, None, Some(t.name.span));
            let conflict = self.type_name_taken(name) || fn_names.contains(name);
            if conflict {
                self.defs[id.0 as usize].clashes.push(Clash::TraitConflict);
            } else {
                self.traits.insert(name.clone(), id);
            }
            self.defs[id.0 as usize].arity = t.generics.len();
            self.trait_ids.push(id);
            let mut ids = Vec::new();
            for method in &t.methods {
                let mid = self.add(DefKind::TraitMethod, &method.name.name, Some(id), Some(method.name.span));
                // A rejected trait keeps its items out of the table, so one mistake is one error.
                if !conflict && self.trait_items.insert((id, method.name.name.clone()), mid).is_some() {
                    self.defs[mid.0 as usize].clashes.push(Clash::TraitMethodDuplicate);
                }
                ids.push(mid);
            }
            self.trait_method_ids.push(ids);
        }
    }

    fn add_impls(&mut self, program: &Program) {
        for i in &program.impls {
            let id = self.add(DefKind::Impl, &i.trait_name.name, None, Some(i.trait_name.span));
            self.defs[id.0 as usize].arity = i.generics.len();
            self.impl_ids.push(id);
            let ids = i
                .methods
                .iter()
                .map(|m| self.add(DefKind::ImplMethod, &m.name, Some(id), Some(m.name_span)))
                .collect();
            self.impl_method_ids.push(ids);
        }
    }

    fn add_constants(&mut self, program: &Program) {
        for decl in &program.constants {
            let name = &decl.name.name;
            let id = self.add(DefKind::Const, name, None, Some(decl.name.span));
            let clash = if crate::builtins::is_intrinsic(name) {
                Some(Clash::ConstBuiltin)
            } else if program.functions.iter().any(|f| f.name == *name) {
                Some(Clash::ConstFn)
            } else if program.classes.iter().any(|c| c.name == *name) {
                Some(Clash::ConstClass)
            } else if program.enums.iter().any(|e| e.name.name == *name) {
                Some(Clash::ConstEnum)
            } else {
                None
            };
            self.defs[id.0 as usize].clashes.extend(clash);
            self.consts.entry(name.clone()).or_insert(id);
            self.const_ids.push(id);
        }
    }

    fn add(&mut self, kind: DefKind, name: &str, owner: Option<DefId>, span: Option<Span>) -> DefId {
        let id = DefId(self.defs.len() as u32);
        self.defs.push(Def { id, kind, name: name.to_string(), owner, clashes: Vec::new(), arity: 0, span });
        id
    }

    pub fn get(&self, id: DefId) -> &Def {
        &self.defs[id.0 as usize]
    }

    /// The enum `T?` stands for. A program that defines its own `Option` of another shape has none.
    pub fn lookup_option(&self) -> Option<DefId> {
        self.option
    }

    pub fn lookup_enum(&self, name: &str) -> Option<DefId> {
        self.enums.get(name).copied()
    }

    pub fn lookup_variant(&self, owner: DefId, name: &str) -> Option<DefId> {
        self.variants.get(&(owner, name.to_string())).copied()
    }

    pub fn enum_id(&self, index: usize) -> DefId {
        self.enum_ids[index]
    }

    pub fn variant_id(&self, enum_index: usize, variant_index: usize) -> DefId {
        self.variant_ids[enum_index][variant_index]
    }

    /// The first class declared with this name.
    pub fn lookup_class(&self, name: &str) -> Option<DefId> {
        self.classes.get(name).copied()
    }

    /// Every class by name (the first declaration of each name), in no particular order.
    pub fn classes(&self) -> impl Iterator<Item = (&str, DefId)> {
        self.classes.iter().map(|(name, id)| (name.as_str(), *id))
    }

    /// Every enum by name (the first declaration of each name), in no particular order.
    pub fn enums(&self) -> impl Iterator<Item = (&str, DefId)> {
        self.enums.iter().map(|(name, id)| (name.as_str(), *id))
    }

    /// A user function (never a builtin; see [`DefTable::lookup_intrinsic`]).
    pub fn lookup_fn(&self, name: &str) -> Option<DefId> {
        self.fns.get(name).copied()
    }

    pub fn lookup_const(&self, name: &str) -> Option<DefId> {
        self.consts.get(name).copied()
    }

    /// Whether `name` is a builtin type or already names a class, enum or trait. Enums, traits and
    /// type parameters all use this one rule for the shared type namespace.
    pub fn type_name_taken(&self, name: &str) -> bool {
        is_builtin_type_name(name)
            || self.classes.contains_key(name)
            || self.enums.contains_key(name)
            || self.traits.contains_key(name)
    }

    pub fn lookup_trait(&self, name: &str) -> Option<DefId> {
        self.traits.get(name).copied()
    }

    pub fn lookup_trait_item(&self, t: DefId, name: &str) -> Option<DefId> {
        self.trait_items.get(&(t, name.to_string())).copied()
    }

    pub fn trait_id(&self, index: usize) -> DefId {
        self.trait_ids[index]
    }

    pub fn trait_method_id(&self, trait_index: usize, method_index: usize) -> DefId {
        self.trait_method_ids[trait_index][method_index]
    }

    pub fn impl_id(&self, index: usize) -> DefId {
        self.impl_ids[index]
    }

    pub fn impl_method_id(&self, impl_index: usize, method_index: usize) -> DefId {
        self.impl_method_ids[impl_index][method_index]
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
            Clash::ConstEnum => format!("constant `{name}` conflicts with an enum"),
            Clash::ClassBuiltinType => format!("class `{name}` conflicts with a builtin type name"),
            Clash::ClassDuplicate => format!("duplicate class `{name}`"),
            Clash::ClassFn => format!("class `{name}` conflicts with another type or function"),
            Clash::EnumConflict => format!("enum `{name}` conflicts with another type or function"),
            Clash::VariantDuplicate => {
                let owner = def.owner.map(|owner| self.get(owner).name.as_str()).unwrap_or("?");
                format!("duplicate variant `{name}` in enum `{owner}`")
            }
            Clash::TraitConflict => format!("trait `{name}` conflicts with another type, trait or function"),
            Clash::TraitMethodDuplicate => {
                let owner = def.owner.map(|owner| self.get(owner).name.as_str()).unwrap_or("?");
                format!("duplicate method `{name}` in trait `{owner}`")
            }
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

/// `String` or a primitive type name.
fn is_builtin_type_name(name: &str) -> bool {
    name == "String" || !matches!(Type::from_ident(name, false), Type::Named { .. })
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
