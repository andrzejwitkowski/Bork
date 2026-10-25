//! Exhaustiveness and unreachable-arm checking for `when` (#51): Maranget's usefulness algorithm
//! over a type-free pattern form. The caller lowers `HirPat`s to [`Pat`], so this module needs
//! no `TyCtx` and is tested on its own.
//!
//! shortcut: integers and strings are an infinite signature, so they always need a catch-all arm;
//! no range analysis. Upgrade when a ticket adds range patterns.

use std::rc::Rc;

use crate::ast::PatLit;

/// The complete set of constructors a column can hold.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Sig {
    Enum { name: String, variants: Vec<(String, usize)> },
    /// `Some(_)` / `None`, for `Ref<T>` and `T?`.
    Option,
    Bool,
    /// Integer and string literals: never complete.
    Infinite,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Ctor {
    Variant(usize),
    Some,
    None,
    Bool(bool),
    Lit(PatLit),
}

#[derive(Debug, Clone)]
pub(super) enum Pat {
    Wild,
    Ctor { ctor: Ctor, sig: Rc<Sig>, args: Vec<Pat> },
}

/// Every constructor of a finite signature with its arity, in a fixed order.
fn all_ctors(sig: &Sig) -> Option<Vec<(Ctor, usize)>> {
    match sig {
        Sig::Enum { variants, .. } => Some(variants.iter().enumerate().map(|(i, (_, n))| (Ctor::Variant(i), *n)).collect()),
        Sig::Option => Some(vec![(Ctor::Some, 1), (Ctor::None, 0)]),
        Sig::Bool => Some(vec![(Ctor::Bool(false), 0), (Ctor::Bool(true), 0)]),
        Sig::Infinite => None,
    }
}

fn arity(ctor: &Ctor, sig: &Sig) -> usize {
    match (ctor, sig) {
        (Ctor::Variant(i), Sig::Enum { variants, .. }) => variants[*i].1,
        (Ctor::Some, _) => 1,
        _ => 0,
    }
}

type Row = Vec<Pat>;

fn wilds(n: usize) -> Vec<Pat> {
    vec![Pat::Wild; n]
}

/// The rows that can match a value built with `ctor`, with its sub-patterns spliced in.
fn specialize(rows: &[Row], ctor: &Ctor, n: usize) -> Vec<Row> {
    rows.iter()
        .filter_map(|row| match &row[0] {
            Pat::Ctor { ctor: c, args, .. } if c == ctor => Some(args.iter().chain(&row[1..]).cloned().collect()),
            Pat::Ctor { .. } => None,
            Pat::Wild => Some(wilds(n).into_iter().chain(row[1..].iter().cloned()).collect()),
        })
        .collect()
}

/// The rows that match whatever constructor the first column holds.
fn default_rows(rows: &[Row]) -> Vec<Row> {
    rows.iter().filter(|row| matches!(row[0], Pat::Wild)).map(|row| row[1..].to_vec()).collect()
}

/// The signature of the first column, and its constructors that some row names.
fn head_sig(rows: &[Row]) -> Option<(Rc<Sig>, Vec<Ctor>)> {
    let mut found: Option<Rc<Sig>> = None;
    let mut seen = Vec::new();
    for row in rows {
        if let Pat::Ctor { ctor, sig, .. } = &row[0] {
            found.get_or_insert_with(|| sig.clone());
            if !seen.contains(ctor) {
                seen.push(ctor.clone());
            }
        }
    }
    found.map(|sig| (sig, seen))
}

/// Whether `q` matches some value that no row of `rows` matches.
pub(super) fn useful(rows: &[Row], q: &[Pat]) -> bool {
    let Some(head) = q.first() else { return rows.is_empty() };
    match head {
        Pat::Ctor { ctor, sig, args } => {
            let next: Vec<Pat> = args.iter().chain(&q[1..]).cloned().collect();
            useful(&specialize(rows, ctor, arity(ctor, sig)), &next)
        }
        Pat::Wild => {
            if let Some((sig, seen)) = head_sig(rows) {
                if let Some(all) = all_ctors(&sig).filter(|all| all.iter().all(|(c, _)| seen.contains(c))) {
                    return all.iter().any(|(c, n)| {
                        let next: Vec<Pat> = wilds(*n).into_iter().chain(q[1..].iter().cloned()).collect();
                        useful(&specialize(rows, c, *n), &next)
                    });
                }
            }
            useful(&default_rows(rows), &q[1..])
        }
    }
}

/// Cap on witnesses kept per step; the message shows three and says if there are more.
const MAX_WITNESSES: usize = 4;

/// Value vectors of length `n` that no row matches; empty when the rows are exhaustive.
pub(super) fn witnesses(rows: &[Row], n: usize) -> Vec<Vec<Pat>> {
    if n == 0 {
        return if rows.is_empty() { vec![vec![]] } else { vec![] };
    }
    let mut out = Vec::new();
    match head_sig(rows) {
        Some((sig, seen)) => match all_ctors(&sig) {
            Some(all) => {
                let complete = all.iter().all(|(c, _)| seen.contains(c));
                for (c, k) in &all {
                    if complete {
                        for mut w in witnesses(&specialize(rows, c, *k), k + n - 1) {
                            let rest = w.split_off(*k);
                            out.push(std::iter::once(Pat::Ctor { ctor: c.clone(), sig: sig.clone(), args: w }).chain(rest).collect());
                        }
                    } else if !seen.contains(c) {
                        for w in witnesses(&default_rows(rows), n - 1) {
                            out.push(std::iter::once(Pat::Ctor { ctor: c.clone(), sig: sig.clone(), args: wilds(*k) }).chain(w).collect());
                        }
                    }
                }
            }
            None => out.extend(wild_witnesses(rows, n)),
        },
        None => out.extend(wild_witnesses(rows, n)),
    }
    out.truncate(MAX_WITNESSES);
    out
}

fn wild_witnesses(rows: &[Row], n: usize) -> Vec<Vec<Pat>> {
    witnesses(&default_rows(rows), n - 1).into_iter().map(|w| std::iter::once(Pat::Wild).chain(w).collect()).collect()
}

/// A witness as the user would write it: `Shape.Rect(_, _)`, `Some(None)`, `_`.
pub(super) fn show(pat: &Pat) -> String {
    let Pat::Ctor { ctor, sig, args } = pat else { return "_".to_string() };
    let list = || args.iter().map(show).collect::<Vec<_>>().join(", ");
    match (ctor, &**sig) {
        (Ctor::Variant(i), Sig::Enum { name, variants }) if args.is_empty() => format!("{name}.{}", variants[*i].0),
        (Ctor::Variant(i), Sig::Enum { name, variants }) => format!("{name}.{}({})", variants[*i].0, list()),
        (Ctor::Some, _) => format!("Some({})", list()),
        (Ctor::None, _) => "None".to_string(),
        (Ctor::Bool(b), _) => b.to_string(),
        (Ctor::Lit(PatLit::Int(n)), _) => n.to_string(),
        (Ctor::Lit(PatLit::Bool(b)), _) => b.to_string(),
        (Ctor::Lit(PatLit::Str(s)), _) => format!("{s:?}"),
        (Ctor::Variant(_), _) => unreachable!("a variant pattern carries an enum signature"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e_sig() -> Rc<Sig> {
        Rc::new(Sig::Enum {
            name: "E".into(),
            variants: vec![("A".into(), 0), ("B".into(), 1), ("C".into(), 2)],
        })
    }

    fn bool_sig() -> Rc<Sig> {
        Rc::new(Sig::Bool)
    }

    fn opt_sig() -> Rc<Sig> {
        Rc::new(Sig::Option)
    }

    fn v(i: usize, args: Vec<Pat>) -> Pat {
        Pat::Ctor { ctor: Ctor::Variant(i), sig: e_sig(), args }
    }

    fn b(x: bool) -> Pat {
        Pat::Ctor { ctor: Ctor::Bool(x), sig: bool_sig(), args: vec![] }
    }

    fn some(p: Pat) -> Pat {
        Pat::Ctor { ctor: Ctor::Some, sig: opt_sig(), args: vec![p] }
    }

    fn none() -> Pat {
        Pat::Ctor { ctor: Ctor::None, sig: opt_sig(), args: vec![] }
    }

    fn int(n: i64) -> Pat {
        Pat::Ctor { ctor: Ctor::Lit(PatLit::Int(n)), sig: Rc::new(Sig::Infinite), args: vec![] }
    }

    fn rows(pats: Vec<Pat>) -> Vec<Row> {
        pats.into_iter().map(|p| vec![p]).collect()
    }

    fn missing(pats: Vec<Pat>) -> Vec<String> {
        witnesses(&rows(pats), 1).iter().map(|w| show(&w[0])).collect()
    }

    fn redundant(pats: Vec<Pat>) -> Vec<usize> {
        let all = rows(pats);
        (0..all.len()).filter(|&i| !useful(&all[..i], &all[i])).collect()
    }

    #[test]
    fn missing_variants_are_named() {
        assert_eq!(missing(vec![v(0, vec![])]), ["E.B(_)", "E.C(_, _)"]);
        assert!(missing(vec![v(0, vec![]), v(1, vec![Pat::Wild]), v(2, vec![Pat::Wild, Pat::Wild])]).is_empty());
        assert!(missing(vec![Pat::Wild]).is_empty());
    }

    #[test]
    fn nested_patterns_are_checked_column_by_column() {
        let covered = vec![v(1, vec![b(true)]), v(1, vec![b(false)]), v(0, vec![]), v(2, vec![Pat::Wild, Pat::Wild])];
        assert!(missing(covered).is_empty());
        assert_eq!(missing(vec![v(1, vec![b(true)]), v(0, vec![]), v(2, vec![Pat::Wild, Pat::Wild])]), ["E.B(false)"]);
    }

    #[test]
    fn option_and_bool() {
        assert_eq!(missing(vec![some(Pat::Wild)]), ["None"]);
        assert_eq!(missing(vec![none()]), ["Some(_)"]);
        assert_eq!(missing(vec![b(true)]), ["false"]);
        assert!(missing(vec![none(), some(Pat::Wild)]).is_empty());
    }

    #[test]
    fn integers_need_a_catch_all() {
        assert_eq!(missing(vec![int(1), int(2)]), ["_"]);
        assert!(missing(vec![int(1), Pat::Wild]).is_empty());
    }

    #[test]
    fn unreachable_arms() {
        assert_eq!(redundant(vec![int(1), int(1), Pat::Wild]), [1]);
        assert_eq!(redundant(vec![Pat::Wild, int(1), v(0, vec![])]), [1, 2]);
        assert_eq!(redundant(vec![v(1, vec![Pat::Wild]), v(1, vec![b(true)])]), [1]);
        assert!(redundant(vec![v(1, vec![b(true)]), v(1, vec![Pat::Wild])]).is_empty());
    }

    #[test]
    fn at_most_four_witnesses() {
        assert_eq!(witnesses(&rows(vec![]), 1).len(), 1);
        let wide = Rc::new(Sig::Enum { name: "W".into(), variants: (0..9).map(|i| (format!("V{i}"), 0)).collect() });
        let one = Pat::Ctor { ctor: Ctor::Variant(0), sig: wide, args: vec![] };
        assert_eq!(witnesses(&rows(vec![one]), 1).len(), MAX_WITNESSES);
    }

    // Property test: compare against brute force over `Option<E>`, a type small enough to
    // enumerate (E = A | B(bool) | C(bool, bool) has 7 values, so 8 in all).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn gen_bool(g: &mut Lcg) -> Pat {
        match g.next(3) {
            0 => Pat::Wild,
            1 => b(true),
            _ => b(false),
        }
    }

    fn gen_e(g: &mut Lcg) -> Pat {
        match g.next(4) {
            0 => Pat::Wild,
            1 => v(0, vec![]),
            2 => v(1, vec![gen_bool(g)]),
            _ => v(2, vec![gen_bool(g), gen_bool(g)]),
        }
    }

    fn gen_pat(g: &mut Lcg) -> Pat {
        match g.next(4) {
            0 => Pat::Wild,
            1 => none(),
            _ => some(gen_e(g)),
        }
    }

    fn all_values() -> Vec<Pat> {
        let bools = || [b(false), b(true)];
        let mut e = vec![v(0, vec![])];
        e.extend(bools().map(|x| v(1, vec![x])));
        for x in bools() {
            for y in bools() {
                e.push(v(2, vec![x.clone(), y]));
            }
        }
        std::iter::once(none()).chain(e.into_iter().map(some)).collect()
    }

    fn matches(p: &Pat, value: &Pat) -> bool {
        match (p, value) {
            (Pat::Wild, _) => true,
            (Pat::Ctor { ctor: c, args: a, .. }, Pat::Ctor { ctor: d, args: x, .. }) => {
                c == d && a.iter().zip(x).all(|(p, value)| matches(p, value))
            }
            _ => false,
        }
    }

    #[test]
    fn agrees_with_brute_force() {
        let values = all_values();
        assert_eq!(values.len(), 8);
        let mut g = Lcg(7);
        for _ in 0..2000 {
            let pats: Vec<Pat> = (0..1 + g.next(5)).map(|_| gen_pat(&mut g)).collect();
            let matrix = rows(pats.clone());
            let covered = values.iter().all(|value| pats.iter().any(|p| matches(p, value)));
            assert_eq!(witnesses(&matrix, 1).is_empty(), covered, "{pats:?}");
            for i in 0..pats.len() {
                let reaches = values.iter().any(|value| matches(&pats[i], value) && !pats[..i].iter().any(|p| matches(p, value)));
                assert_eq!(useful(&matrix[..i], &matrix[i]), reaches, "arm {i} of {pats:?}");
            }
        }
    }
}
