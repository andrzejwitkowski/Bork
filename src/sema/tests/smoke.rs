use super::super::*;
use crate::ast::{Block, Stmt};
use crate::parse;

#[test]
fn collapses_nested_bare_braces() {
    let nested = Block {
        stmts: vec![Stmt::Block(Block {
            stmts: vec![Stmt::Block(Block {
                stmts: vec![Stmt::Return(None)],
            })],
        })],
    };
    let (b, n) = peel_blocks(&nested);
    assert_eq!(n, 2);
    assert!(matches!(b.stmts[0], Stmt::Return(None)));
}

#[test]
fn mvp_sample_analyzes_without_move_errors() {
    let prog = parse(crate::MVP_SAMPLE).expect("parse");
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    assert!(!report.roots.is_empty());
}
