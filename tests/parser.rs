use bork::*;
use lalrpop_util::ParseError;

#[test]
fn empty_input_parses() {
    assert!(parse("").is_ok());
}

#[test]
fn rejects_struct_declarations() {
    assert!(parse("struct Point(val x: i32)\nfun main() {}").is_err());
}

#[test]
fn parses_mvp_sample() {
    let prog = parse(MVP_SAMPLE).expect("sample should parse");
    assert_eq!(prog.functions.len(), 2);
    assert_eq!(prog.functions[0].name, "action");
    assert_eq!(prog.functions[1].name, "main");
    assert_eq!(
        prog.functions[1].return_type,
        Type::Primitive {
            name: "unit".into(),
            nullable: false,
        }
    );
    assert_eq!(
        prog.functions[0].return_type,
        Type::Primitive {
            name: "i32".into(),
            nullable: false,
        }
    );
    assert!(matches!(prog.functions[0].params[2].ty, Type::Func { .. }));
    assert!(matches!(prog.functions[1].body.stmts[2], Stmt::For { .. }));
}

#[test]
fn index_assign_is_assign_and_bare_index_is_expr() {
    let prog = parse("fun main() {\n    a[i] = 1\n    a[i]\n}").expect("parse");
    let stmts = &prog.functions[0].body.stmts;
    assert!(matches!(
        &stmts[0],
        Stmt::Assign {
            target: AssignTarget::Index { name, .. },
            ..
        } if name == "a"
    ));
    assert!(matches!(stmts[1], Stmt::Expr(Expr::Index { .. })));
}

#[test]
fn parses_val_var_function_params() {
    let prog = parse("fun bump(buf: &[i32; 2]) { buf[0] = 1 }").expect("parse");
    assert!(matches!(
        prog.functions[0].params[0].ty,
        Type::Ref { .. }
    ));

    let prog = parse("fun f(val x: Int, var y: String): Int { return x }").expect("parse");
    assert_eq!(prog.functions[0].params[0].kind, BindingKind::Val);
    assert_eq!(prog.functions[0].params[1].kind, BindingKind::Var);
}

#[test]
fn parses_class_with_managed_ref_fields_and_field_assignment() {
    let prog = parse(
        r#"class Node {
    name: String
    next: Ref<Node>
}

fun link(a: Node, b: Node) {
    a.next = Some(b)
}"#,
    )
    .expect("class and managed reference syntax should parse");

    assert_eq!(prog.classes.len(), 1);
    assert_eq!(prog.classes[0].name, "Node");
    assert!(matches!(
        &prog.classes[0].fields[1].ty,
        Type::ManagedRef { inner }
            if matches!(inner.as_ref(), Type::Named { name, nullable: false } if name == "Node")
    ));
    assert!(matches!(
        &prog.functions[0].body.stmts[0],
        Stmt::Assign {
            target: AssignTarget::Field { name, .. },
            value: Expr::Some { .. },
        } if name == "next"
    ));
}

#[test]
fn parses_if_val_and_exhaustive_when() {
    let prog = parse(
        r#"fun use(ref: Ref<String>) {
    if val value = ref { value } else { None }
    when ref {
        Some(value) => { value }
        None => { None }
    }
}"#,
    )
    .expect("presence syntax should parse");

    assert!(matches!(
        &prog.functions[0].body.stmts[0],
        Stmt::Expr(Expr::IfVal { bindings, .. }) if bindings[0].name.name == "value"
    ));
    assert!(matches!(
        &prog.functions[0].body.stmts[1],
        Stmt::Expr(Expr::When { some_name, .. }) if some_name.name == "value"
    ));
}

#[test]
fn bare_param_defaults_to_val() {
    let prog = parse("fun f(x: Int): Int { return x }").expect("parse");
    assert_eq!(prog.functions[0].params[0].kind, BindingKind::Val);
}

#[test]
fn parses_move_expression_in_var_decl() {
    let prog = parse(
        r#"fun main() {
    var s: String = "hi"
    var x = move s
}"#,
    )
    .expect("parse");
    let Stmt::VarDecl { value, .. } = &prog.functions[0].body.stmts[1] else {
        panic!("expected second var decl");
    };
    assert!(matches!(value, Expr::Move { name, .. } if name == "s"));
}

#[test]
fn parses_move_expression_as_call_arg() {
    let prog = parse(
        r#"fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(move s)
}"#,
    )
    .expect("parse");
    let Stmt::Expr(Expr::Call { args, .. }) = &prog.functions[1].body.stmts[1] else {
        panic!("expected call");
    };
    assert!(matches!(&args[0], Expr::Move { name, .. } if name == "s"));
}

#[test]
fn function_param_names_have_spans() {
    let src = "fun add(x: Int, y: Int): Int { return x + y }";
    let prog = parse(src).expect("parse");
    let x = &prog.functions[0].params[0].name;
    assert_eq!(x.name, "x");
    assert!(x.span.end > x.span.start, "{:?}", x.span);
    assert_eq!(&src[x.span.start..x.span.end], "x");
}

#[test]
fn same_line_statements_require_a_separator() {
    assert!(parse("fun main(): Unit { val x = 1 var y = 2 }").is_err());
}

#[test]
fn parses_final_expression_after_declaration() {
    let prog = parse("fun main() { val x = 1\n x }")
        .expect("newline should separate declaration and final expression");

    assert!(matches!(
        prog.functions[0].body.stmts.as_slice(),
        [Stmt::VarDecl { .. }, Stmt::Expr(Expr::Ident { name, .. })] if name == "x"
    ));
}

#[test]
fn parses_return_after_declaration() {
    let prog = parse("fun main() { val x = 1\n return x }")
        .expect("newline should separate declaration and return");

    assert!(matches!(
        prog.functions[0].body.stmts.as_slice(),
        [Stmt::VarDecl { .. }, Stmt::Return(Some(Expr::Ident { name, .. }))] if name == "x"
    ));
}

#[test]
fn parses_else_on_following_line() {
    parse("fun main() { if (c) { a }\n else { b } }")
        .expect("else may follow the if block on the next line");
}

#[test]
fn parses_else_if_desugars_to_nested_if() {
    let prog = parse("fun main() { if (a) { 1 } else if (b) { 2 } else { 3 } }")
        .expect("else if should parse");
    let Stmt::Expr(Expr::If {
        else_block: Some(else_block),
        ..
    }) = &prog.functions[0].body.stmts[0]
    else {
        panic!("expected outer if");
    };
    assert!(matches!(
        else_block.stmts.as_slice(),
        [Stmt::Expr(Expr::If {
            else_block: Some(_),
            ..
        })]
    ));
}

#[test]
fn parses_else_if_on_following_line() {
    parse("fun main() { if (a) { 1 }\n else if (b) { 2 } }")
        .expect("else if may follow the if block on the next line");
}

#[test]
fn parses_class_decl_and_constructor() {
    let prog = parse("class Point {\n x: i32\n y: i32\n}\nfun main(): i32 { return Point(1, 2).x }")
        .expect("class and constructor should parse");
    assert_eq!(prog.classes.len(), 1);
    assert_eq!(prog.classes[0].name, "Point");
    assert_eq!(prog.classes[0].fields.len(), 2);
    assert!(matches!(
        &prog.functions[0].body.stmts[0],
        Stmt::Return(Some(Expr::Field {
            receiver,
            name,
            ..
        })) if name == "x" && matches!(receiver.as_ref(), Expr::Call { callee, args, .. } if matches!(callee.as_ref(), Expr::Ident { name, .. } if name == "Point") && args.len() == 2)
    ));
}

#[test]
fn parses_class_field_assign() {
    let prog = parse("class Point { x: i32 }\nfun main() { val p = Point(1)\n p.x = 2 }")
        .expect("field assign should parse");
    assert_eq!(prog.classes[0].fields[0].name.name, "x");
    assert!(matches!(
        &prog.functions[0].body.stmts[1],
        Stmt::Assign {
            target: AssignTarget::Field { name, .. },
            ..
        } if name == "x"
    ));
}

#[test]
fn standalone_comment_between_statements_is_part_of_the_separator() {
    let prog = parse("fun main() {\n val x = 1\n // explanation\n val y = 2\n}")
        .expect("a standalone comment should not create an extra separator");

    assert_eq!(prog.functions[0].body.stmts.len(), 2);
}

#[test]
fn standalone_comment_between_functions_is_part_of_the_separator() {
    let prog = parse("fun first() {}\n// next function\nfun second() {}")
        .expect("a standalone comment should separate functions");

    assert_eq!(prog.functions.len(), 2);
}

#[test]
fn standalone_comment_between_then_block_and_else_parses() {
    parse("fun main() { if (c) { a }\n // alternate\n else { b } }")
        .expect("comments may appear between a then block and else");
}

#[test]
fn parses_newlines_inside_parentheses() {
    parse(
        "fun apply(\n f: (Int,\n Int) -> Int,\n x: Int\n) {\n\
         val y = f(\n x,\n x\n)\n\
         if (\n y >\n x\n) { y } else { x }\n\
         for (\n i\n in\n 0..\n y\n) { i }\n}",
    )
    .expect("newlines inside parentheses should be insignificant");
}

#[test]
fn newlines_in_blocks_nested_inside_parentheses_still_end_statements() {
    let prog = parse("fun main() { f(if (c) {\n val x = 1\n x\n} else { 0 }) }")
        .expect("a block nested in call arguments keeps statement newlines");

    let Stmt::Expr(Expr::Call { args, .. }) = &prog.functions[0].body.stmts[0] else {
        panic!("body should contain a call");
    };
    let Expr::If { then_block, .. } = &args[0] else {
        panic!("call argument should be an if expression");
    };
    assert_eq!(then_block.stmts.len(), 2);
}

#[test]
fn multiline_parenthesized_syntax_error_preserves_parse_error_location() {
    let source = "fun main() {\n val x = (1 +\n)\n}";
    let expected_location = source
        .rfind(')')
        .expect("test source has a closing parenthesis");

    match parse(source).expect_err("incomplete addition should fail") {
        ParseError::InvalidToken { location } => assert_eq!(location, expected_location),
        ParseError::UnrecognizedToken {
            token: (start, _, _),
            ..
        } => assert_eq!(start, expected_location),
        other => panic!("expected a located lexer/parser error, got {other:?}"),
    }
}

#[test]
fn newline_before_call_parenthesis_ends_statement() {
    let prog = parse("fun main() { val x = f\n (y) }")
        .expect("parenthesized expression should start the next statement");

    assert!(matches!(
        prog.functions[0].body.stmts.as_slice(),
        [Stmt::VarDecl { value: Expr::Ident { name, .. }, .. }, Stmt::Expr(Expr::Ident { name: y, .. })]
            if name == "f" && y == "y"
    ));
}

#[test]
fn nullable_function_return_belongs_to_return_type() {
    let prog = parse("fun f(): () -> Int? { return 1 }")
        .expect("nullable function return type should parse");

    assert_eq!(
        prog.functions[0].return_type,
        Type::Func {
            params: vec![],
            ret: Box::new(Type::Primitive {
                name: "i32".into(),
                nullable: true,
            }),
            nullable: false,
        }
    );
}

#[test]
fn nullable_function_type_as_param() {
    let prog = parse("fun f(x: ((Int) -> Int)?): Int { return 1 }")
        .expect("nullable function type param should parse");

    assert_eq!(
        prog.functions[0].params[0].ty,
        Type::Func {
            params: vec![Type::Primitive {
                name: "i32".into(),
                nullable: false,
            }],
            ret: Box::new(Type::Primitive {
                name: "i32".into(),
                nullable: false,
            }),
            nullable: true,
        }
    );
}

#[test]
fn nullable_function_type_allows_space_before_question_mark() {
    let prog = parse("fun f(x: ((Int) -> Int) ?): Int { return 1 }")
        .expect("whitespace before function nullability should be insignificant");

    assert!(matches!(
        prog.functions[0].params[0].ty,
        Type::Func { nullable: true, .. }
    ));
}

#[test]
fn nullable_function_types_compose_in_lists_and_returns() {
    let prog = parse("fun f(x: (((Int) -> Int) ?, Int) -> ((Int) -> Int) ?): Int { return 1 }")
        .expect("nullable function types should compose recursively");

    let Type::Func { params, ret, .. } = &prog.functions[0].params[0].ty else {
        panic!("outer parameter type should be a function");
    };
    assert!(matches!(params[0], Type::Func { nullable: true, .. }));
    assert!(matches!(ret.as_ref(), Type::Func { nullable: true, .. }));
}

#[test]
fn nullable_parenthesized_named_type_is_a_parse_error_not_a_panic() {
    assert!(parse("fun f(x: (Int)?): Int { return 1 }").is_err());
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    let prog = parse("fun main() { 1 + 2 * 3 }").expect("expression should parse");

    assert!(matches!(
        &prog.functions[0].body.stmts[0],
        Stmt::Expr(Expr::Binary {
            op: BinOp::Add,
            lhs,
            rhs,
            ..
        }) if matches!(lhs.as_ref(), Expr::Int(1))
            && matches!(
                rhs.as_ref(),
                Expr::Binary {
                    op: BinOp::Mul,
                    lhs,
                    rhs,
                    ..
                } if matches!(lhs.as_ref(), Expr::Int(2))
                    && matches!(rhs.as_ref(), Expr::Int(3))
            )
    ));
}

#[test]
fn nested_bare_blocks_are_not_flattened() {
    let prog = parse("fun main() {{{{ }}}}").expect("nested blocks should parse");
    let mut stmts = prog.functions[0].body.stmts.as_slice();

    for _ in 0..3 {
        let [Stmt::Block(block)] = stmts else {
            panic!("each brace pair should produce one nested block");
        };
        stmts = &block.stmts;
    }
    assert!(stmts.is_empty());
}

#[test]
fn oversized_integer_literal_is_parse_error_not_panic() {
    let too_large = i64::MAX as u128 + 1;
    assert!(parse(&format!("fun main() {{ return {too_large} }}")).is_err());
}

#[test]
fn if_without_else_parses_in_value_position() {
    parse("fun f() { val x = if (c) { 1 } }").expect("if without else is a value expression");
}

#[test]
fn parses_process_user_sample() {
    let prog = parse(PROCESS_USER_SAMPLE).expect("processUser sample should parse");
    let f = &prog.functions[0];
    assert_eq!(f.name, "processUser");
    assert_eq!(
        f.params[0].ty,
        Type::Named {
            name: "String".into(),
            nullable: true,
        }
    );
    assert_eq!(
        f.params[1].ty,
        Type::Primitive {
            name: "i32".into(),
            nullable: false,
        }
    );
    assert_eq!(
        f.return_type,
        Type::Primitive {
            name: "i32".into(),
            nullable: false,
        }
    );

    assert!(matches!(
        &f.body.stmts[0],
        Stmt::VarDecl {
            ty: Some(Type::Named { name, nullable: false }),
            value: Expr::Binary {
                op: BinOp::Elvis,
                rhs,
                ..
            },
            ..
        } if name == "String" && matches!(rhs.as_ref(), Expr::Str(s) if s == "Guest")
    ));

    let Stmt::Block(inner) = &f.body.stmts[2] else {
        panic!("expected bare block");
    };
    assert!(matches!(
        &inner.stmts[0],
        Stmt::VarDecl {
            ty: Some(Type::Named { name, nullable: true }),
            value: Expr::Some { .. },
            ..
        } if name == "String"
    ));
    assert!(matches!(
        &inner.stmts[1],
        Stmt::VarDecl {
            value: Expr::None { .. },
            ..
        }
    ));
    assert!(matches!(
        &inner.stmts[2],
        Stmt::VarDecl {
            name,
            value: Expr::Binary {
                op: BinOp::Elvis,
                lhs,
                ..
            },
            ..
        } if name == "verifiedLength" && matches!(
            lhs.as_ref(),
            Expr::Field { name, safe: true, .. } if name == "length"
        )
    ));
    assert!(matches!(
        &inner.stmts[3],
        Stmt::Expr(Expr::If {
            else_block: None,
            cond,
            ..
        }) if matches!(
            cond.as_ref(),
            Expr::Binary {
                op: BinOp::Gt,
                lhs,
                ..
            } if matches!(
                lhs.as_ref(),
                Expr::Ident { name, .. } if name == "verifiedLength"
            )
        )
    ));
}

#[test]
fn parses_move_block_with_captures() {
    let prog = parse(
        r#"
fun main() {
    val a = 1
    val b = 2
    move (a, b) {
        return a
    }
}
"#,
    )
    .expect("move block should parse");
    assert!(matches!(
        &prog.functions[0].body.stmts[2],
        Stmt::MoveBlock {
            captures: Some(captures),
            ..
        } if captures.iter().map(|c| c.name.as_str()).eq(["a", "b"])
    ));
}

#[test]
fn parses_trailing_move_closure() {
    let prog = parse(
        r#"
fun action(block: (Int) -> Int): Int {
    return block(1)
}

fun main() {
    val acc = 0
    action() move (acc) { x ->
        acc + x
    }
}
"#,
    )
    .expect("trailing move should parse");
    let Stmt::Expr(Expr::Call {
        trailing: Some(c), ..
    }) = &prog.functions[1].body.stmts[1]
    else {
        panic!("expected call with trailing");
    };
    assert!(c.is_move);
    assert_eq!(
        c.captures
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["acc"]
    );
    assert_eq!(
        c.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        vec!["x"]
    );
}

#[test]
fn parses_trailing_move_without_capture_list() {
    let prog = parse(
        r#"
fun action(block: () -> Int): Int {
    return block()
}

fun main() {
    val acc = 0
    action() move {
        acc
    }
}
"#,
    )
    .expect("move bare trailing should parse");
    let Stmt::Expr(Expr::Call {
        trailing: Some(c), ..
    }) = &prog.functions[1].body.stmts[1]
    else {
        panic!("expected call");
    };
    assert!(c.is_move);
    assert!(c.captures.is_none());
}

#[test]
fn parses_move_with_explicit_empty_captures() {
    let prog = parse(
        r#"
fun main() {
    val s = "hi"
    move () {
        return 1
    }
}
"#,
    )
    .expect("explicit empty move captures should parse");
    assert!(matches!(
        &prog.functions[0].body.stmts[1],
        Stmt::MoveBlock {
            captures: Some(captures),
            ..
        } if captures.is_empty()
    ));
}

#[test]
fn parses_bool_literals() {
    let prog = parse("fun main() {\n    val b: bool = true\n    val c = false\n}").expect("parse");
    let Stmt::VarDecl { value, .. } = &prog.functions[0].body.stmts[0] else {
        panic!("expected val decl");
    };
    assert!(matches!(value, Expr::Bool(true)));
    let Stmt::VarDecl { value, .. } = &prog.functions[0].body.stmts[1] else {
        panic!("expected val decl");
    };
    assert!(matches!(value, Expr::Bool(false)));
}

#[test]
fn parses_logical_and_or_and_not() {
    let prog = parse(
        r#"
fun main(): i32 {
    if (a && b || !c) { return 1 }
    return 0
}
"#,
    )
    .expect("logical ops should parse");
    let Stmt::Expr(Expr::If { cond, .. }) = &prog.functions[0].body.stmts[0] else {
        panic!("expected if");
    };
    assert!(matches!(
        cond.as_ref(),
        Expr::Binary {
            op: BinOp::Or,
            lhs,
            ..
        } if matches!(lhs.as_ref(), Expr::Binary { op: BinOp::And, .. })
    ));
}

#[test]
fn parses_while_break_continue() {
    let prog = parse(
        r#"
fun main() {
    while (true) {
        break
        continue
    }
}
"#,
    )
    .expect("while/break/continue should parse");
    let Stmt::While { body, .. } = &prog.functions[0].body.stmts[0] else {
        panic!("expected while");
    };
    assert!(matches!(&body.stmts[0], Stmt::Break { .. }));
    assert!(matches!(&body.stmts[1], Stmt::Continue { .. }));
}

#[test]
fn parses_chained_if_val_headers() {
    for header in [
        "(a = ref)",
        "(a = ref, b = a.next)",
        "(a = ref, b = a.next, c = b.next, d = c.next, e = d.next,)",
        "(\n a = ref, // first\n\n b = fetch((a.next)),\n)",
        "(a = if (true) { ref } else { other }, b = a.next)",
        "(a = fetch() { -> ref }, b = a.next)",
    ] {
        let source = format!("fun main() {{ if val {header} {{ a }} else {{ 0 }} }}");
        assert!(parse(&source).is_ok(), "{source}");
    }
}

#[test]
fn rejects_malformed_chained_if_val_headers() {
    for header in [
        "()",
        "(a = ref b = ref)",
        "(a ref)",
        "(a =)",
        "(a = ref,,)",
        "(var a = ref)",
        "(a: T = ref)",
        "a = ref,",
    ] {
        assert!(
            parse(&format!("fun main() {{ if val {header} {{}} }}")).is_err(),
            "{header}"
        );
    }
}

#[test]
fn chained_header_spans_refer_to_original_multiline_source() {
    let source = "fun main() { if val (\n first = source, // comment\n second = first.next,\n) {}\n// after body\nelse {} }";
    let parsed = parse(source).expect("parse");
    let Stmt::Expr(Expr::IfVal {
        bindings,
        else_block,
        ..
    }) = &parsed.functions[0].body.stmts[0]
    else {
        panic!("if val")
    };
    assert!(else_block.is_some());
    for (binding, name, value) in [
        (&bindings[0], "first", "source"),
        (&bindings[1], "second", "first.next"),
    ] {
        assert_eq!(
            &source[binding.name.span.start..binding.name.span.end],
            name
        );
        assert_eq!(
            &source[binding.value_span.start..binding.value_span.end],
            value
        );
    }
}
