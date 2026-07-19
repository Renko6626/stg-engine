use super::*;
use crate::lang::lex::Lexer;

fn parse_src(src: &str) -> (Program, Vec<CompileError>) {
    let (tokens, mut errors) = Lexer::new(src).lex();
    let (program, parse_errors) = Parser::new(tokens, src).parse_program();
    errors.extend(parse_errors);
    (program, errors)
}

fn must_parse(src: &str) -> Program {
    let (program, errors) = parse_src(src);
    assert!(errors.is_empty(), "意外解析错误：{errors:?}\n源码：\n{src}");
    program
}

fn single_stmt(stmt_src: &str) -> Stmt {
    let src = format!("sub main() {{ {stmt_src} }}");
    let p = must_parse(&src);
    let mut body = p.subs.into_iter().next().expect("应有 main sub").body;
    assert_eq!(body.len(), 1, "期望恰一条语句，源码：{stmt_src}");
    body.remove(0)
}

/// 递归把 `Expr` 树里所有 `Span` 归零，供结构对比时忽略精确位置（位置正确性另有
/// 专门的错误路径测试覆盖，这里只关心 AST 形状）。
fn zero(e: &Expr) -> Expr {
    match e {
        Expr::IntLit(v) => Expr::IntLit(*v),
        Expr::FxLit(v) => Expr::FxLit(*v),
        Expr::AngleLit(v) => Expr::AngleLit(*v),
        Expr::Var(n, _) => Expr::Var(n.clone(), Span::default()),
        Expr::EngineVar(ev, _) => Expr::EngineVar(*ev, Span::default()),
        Expr::GlobalRead { slot, .. } => Expr::GlobalRead {
            slot: Box::new(zero(slot)),
            span: Span::default(),
        },
        Expr::Call { name, args, .. } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(zero).collect(),
            span: Span::default(),
        },
        Expr::Binary { op, l, r, .. } => Expr::Binary {
            op: *op,
            l: Box::new(zero(l)),
            r: Box::new(zero(r)),
            span: Span::default(),
        },
        Expr::Unary { op, e, .. } => Expr::Unary {
            op: *op,
            e: Box::new(zero(e)),
            span: Span::default(),
        },
        Expr::Cast { e, to, .. } => Expr::Cast {
            e: Box::new(zero(e)),
            to: *to,
            span: Span::default(),
        },
    }
}

/// 把 `_ = <expr>;` 包成一条语句解析出来，取其（span 归零后的）表达式树——专供表达式
/// 形状类测试复用，避免每条测试都手写 `sub main(){}` 包装 + 目标语句解构。
fn expr_of(expr_src: &str) -> Expr {
    let s = single_stmt(&format!("_ = {expr_src};"));
    match s {
        Stmt::ExprStmt {
            expr, discarded, ..
        } => {
            assert!(discarded);
            zero(&expr)
        }
        other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
    }
}

fn var(name: &str) -> Expr {
    Expr::Var(name.to_string(), Span::default())
}

// ── 顶层声明形状 ─────────────────────────────────────────────────────

#[test]
fn top_level_const_def_shape() {
    let p = must_parse("const RANK_SLOT: int = 0;");
    assert_eq!(p.consts.len(), 1);
    assert_eq!(p.consts[0].name, "RANK_SLOT");
    assert_eq!(p.consts[0].ty, Ty::Int);
    assert_eq!(zero(&p.consts[0].value), Expr::IntLit(0));
    assert!(p.subs.is_empty());
    assert!(p.xformdefs.is_empty());
}

#[test]
fn top_level_sub_def_no_params_shape() {
    let p = must_parse("sub foo() { }");
    assert_eq!(p.subs.len(), 1);
    let s = &p.subs[0];
    assert_eq!(s.name, "foo");
    assert!(!s.is_async);
    assert!(s.params.is_empty());
    assert!(s.body.is_empty());
}

#[test]
fn top_level_async_sub_with_param_shape() {
    let p = must_parse("async sub timer_ui(spell: int) { }");
    assert_eq!(p.subs.len(), 1);
    let s = &p.subs[0];
    assert_eq!(s.name, "timer_ui");
    assert!(s.is_async);
    assert_eq!(s.params, vec![("spell".to_string(), Ty::Int)]);
}

#[test]
fn top_level_xformdef_with_wait_prefix_syntax_shape() {
    // 选定语法：`@N` 前缀 = 该条 op 之前的等待帧数（Global Constraints 推荐写法）。
    let p = must_parse("xformdef RING { turn(90deg); @8 set_ang_vel(128); }");
    assert_eq!(p.xformdefs.len(), 1);
    let xf = &p.xformdefs[0];
    assert_eq!(xf.name, "RING");
    assert_eq!(xf.slots.len(), 2);
    assert_eq!(xf.slots[0].wait, 0);
    assert_eq!(xf.slots[0].op_name, "turn");
    assert_eq!(
        xf.slots[0].args.iter().map(zero).collect::<Vec<_>>(),
        vec![Expr::AngleLit(16384)]
    );
    assert_eq!(xf.slots[1].wait, 8);
    assert_eq!(xf.slots[1].op_name, "set_ang_vel");
    assert_eq!(
        xf.slots[1].args.iter().map(zero).collect::<Vec<_>>(),
        vec![Expr::IntLit(128)]
    );
}

/// 负例：顶层出现语句层面的 token（既非 const/xformdef/sub/async）→ 报错，且恢复后
/// 仍能继续解析后面合法的顶层声明。
#[test]
fn top_level_unknown_token_is_an_error_and_recovers() {
    let (p, errors) = parse_src("42;\nsub main() { }");
    assert!(!errors.is_empty());
    assert_eq!(p.subs.len(), 1);
    assert_eq!(p.subs[0].name, "main");
}

/// 回归（Critical）：`recover()` 的契约是"扫到 `}`/`Eof` 不消费，留给外层花括号循环判断
/// 退出"——这个假设对 `parse_block`/`parse_xformdef` 成立，但顶层 `parse_program` 没有外层
/// 花括号循环接住它。孤立的 `}`（脚本作者手滑多打一个收尾括号）落进顶层错误分支后，
/// `recover()` 原地不消费、`while` 条件不变 → 死循环 + `errors` 无界增长。
///
/// 用独立线程 + 超时守护而非直接调用：这条回归一旦复发，直接调用会把整个测试套件挂死；
/// 超时守护让它退化成一条普通的失败测试。
#[test]
fn stray_top_level_rbrace_does_not_hang_parser() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(parse_src("}"));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok((_, errors)) => assert!(!errors.is_empty(), "孤立 `}}` 应至少报一条错误"),
        Err(_) => panic!("解析孤立 `}}` 超过 5 秒未返回——顶层 recover() 死循环复发"),
    }
}

/// 回归（Critical）的姊妹用例：参数名写成字面量导致 `recover()` 吃穿 sub 体、一路吃到
/// sub 自己的收尾 `}` 才停手——同样在顶层触发死循环（不需要真的多打一个 `}`，坏参数名
/// 这种更自然的笔误就能触发）。
#[test]
fn malformed_param_name_leaking_rbrace_does_not_hang_parser() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(parse_src("sub f(1: int) { }"));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok((_, errors)) => assert!(!errors.is_empty(), "坏参数名应至少报一条错误"),
        Err(_) => panic!("解析坏参数名超过 5 秒未返回——顶层 recover() 死循环复发"),
    }
}

// ── 语句形状 ─────────────────────────────────────────────────────────

#[test]
fn var_stmt_shape() {
    let s = single_stmt("var x: fx = 1.5fx;");
    match s {
        Stmt::Var { name, ty, init, .. } => {
            assert_eq!(name, "x");
            assert_eq!(ty, Ty::Fx);
            assert_eq!(zero(&init), Expr::FxLit(98304));
        }
        other => panic!("期望 Stmt::Var，得到 {other:?}"),
    }
}

#[test]
fn assign_stmt_shape() {
    let s = single_stmt("x = x + 1;");
    match s {
        Stmt::Assign { name, value, .. } => {
            assert_eq!(name, "x");
            assert_eq!(
                zero(&value),
                Expr::Binary {
                    op: BinOp::Add,
                    l: Box::new(var("x")),
                    r: Box::new(Expr::IntLit(1)),
                    span: Span::default(),
                }
            );
        }
        other => panic!("期望 Stmt::Assign，得到 {other:?}"),
    }
}

#[test]
fn discard_prefix_sets_discarded_flag() {
    let s = single_stmt("_ = fire(1, 2, 3);");
    match s {
        Stmt::ExprStmt {
            discarded, expr, ..
        } => {
            assert!(discarded);
            assert_eq!(
                zero(&expr),
                Expr::Call {
                    name: "fire".to_string(),
                    args: vec![Expr::IntLit(1), Expr::IntLit(2), Expr::IntLit(3)],
                    span: Span::default(),
                }
            );
        }
        other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
    }
}

#[test]
fn expr_stmt_without_discard_prefix_shape() {
    let s = single_stmt("move_to(90, -100fx, 100fx, 1);");
    match s {
        Stmt::ExprStmt {
            discarded, expr, ..
        } => {
            assert!(!discarded);
            let expected = Expr::Call {
                name: "move_to".to_string(),
                args: vec![
                    Expr::IntLit(90),
                    Expr::Unary {
                        op: UnOp::Neg,
                        e: Box::new(Expr::FxLit(6_553_600)),
                        span: Span::default(),
                    },
                    Expr::FxLit(6_553_600),
                    Expr::IntLit(1),
                ],
                span: Span::default(),
            };
            assert_eq!(zero(&expr), expected);
        }
        other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
    }
}

#[test]
fn if_else_shape() {
    let s = single_stmt("if a > 0 { b = 1; } else { b = 2; }");
    match s {
        Stmt::If {
            cond,
            then_b,
            else_b,
            ..
        } => {
            assert_eq!(
                zero(&cond),
                Expr::Binary {
                    op: BinOp::Gt,
                    l: Box::new(var("a")),
                    r: Box::new(Expr::IntLit(0)),
                    span: Span::default(),
                }
            );
            assert_eq!(then_b.len(), 1);
            assert!(matches!(then_b[0], Stmt::Assign { .. }));
            let else_b = else_b.expect("应有 else 分支");
            assert_eq!(else_b.len(), 1);
            assert!(matches!(else_b[0], Stmt::Assign { .. }));
        }
        other => panic!("期望 Stmt::If，得到 {other:?}"),
    }
}

#[test]
fn if_without_else_has_none_else_branch() {
    let s = single_stmt("if a { b = 1; }");
    match s {
        Stmt::If { else_b, .. } => assert!(else_b.is_none()),
        other => panic!("期望 Stmt::If，得到 {other:?}"),
    }
}

/// `else if` 链降低为"单语句块套一条嵌套 `Stmt::If`"，不是独立 AST 变体。
#[test]
fn else_if_chain_desugars_to_nested_if() {
    let s = single_stmt("if a { } else if b { } else { }");
    match s {
        Stmt::If { else_b, .. } => {
            let outer_else = else_b.expect("应有 else 分支");
            assert_eq!(outer_else.len(), 1);
            match &outer_else[0] {
                Stmt::If {
                    else_b: inner_else, ..
                } => assert!(inner_else.is_some()),
                other => panic!("期望嵌套 Stmt::If，得到 {other:?}"),
            }
        }
        other => panic!("期望 Stmt::If，得到 {other:?}"),
    }
}

#[test]
fn while_stmt_shape() {
    let s = single_stmt("while t > 0 { t = t - 1; }");
    match s {
        Stmt::While { cond, body, .. } => {
            assert_eq!(
                zero(&cond),
                Expr::Binary {
                    op: BinOp::Gt,
                    l: Box::new(var("t")),
                    r: Box::new(Expr::IntLit(0)),
                    span: Span::default(),
                }
            );
            assert_eq!(body.len(), 1);
        }
        other => panic!("期望 Stmt::While，得到 {other:?}"),
    }
}

#[test]
fn loop_stmt_shape() {
    let s = single_stmt("loop { wait(1); }");
    match s {
        Stmt::Loop { body, .. } => {
            assert_eq!(body.len(), 1);
            assert!(matches!(body[0], Stmt::Wait { .. }));
        }
        other => panic!("期望 Stmt::Loop，得到 {other:?}"),
    }
}

#[test]
fn for_stmt_shape() {
    let s = single_stmt("for i in 0..5 { }");
    match s {
        Stmt::For {
            var: name,
            from,
            to,
            body,
            ..
        } => {
            assert_eq!(name, "i");
            assert_eq!(zero(&from), Expr::IntLit(0));
            assert_eq!(zero(&to), Expr::IntLit(5));
            assert!(body.is_empty());
        }
        other => panic!("期望 Stmt::For，得到 {other:?}"),
    }
}

#[test]
fn wait_stmt_shape() {
    let s = single_stmt("wait(90);");
    match s {
        Stmt::Wait { frames, .. } => assert_eq!(zero(&frames), Expr::IntLit(90)),
        other => panic!("期望 Stmt::Wait，得到 {other:?}"),
    }
}

#[test]
fn spawn_stmt_shape_no_args() {
    let s = single_stmt("spawn patrol();");
    match s {
        Stmt::Spawn { name, args, .. } => {
            assert_eq!(name, "patrol");
            assert!(args.is_empty());
        }
        other => panic!("期望 Stmt::Spawn，得到 {other:?}"),
    }
}

#[test]
fn spawn_stmt_shape_with_args() {
    let s = single_stmt("spawn timer_ui(1);");
    match s {
        Stmt::Spawn { name, args, .. } => {
            assert_eq!(name, "timer_ui");
            assert_eq!(
                args.iter().map(zero).collect::<Vec<_>>(),
                vec![Expr::IntLit(1)]
            );
        }
        other => panic!("期望 Stmt::Spawn，得到 {other:?}"),
    }
}

#[test]
fn return_break_continue_shapes() {
    assert!(matches!(single_stmt("return;"), Stmt::Return { .. }));
    assert!(matches!(single_stmt("break;"), Stmt::Break { .. }));
    assert!(matches!(single_stmt("continue;"), Stmt::Continue { .. }));
}

// ── 优先级 / 括号 / cast / $ 变量 / global ──────────────────────────

#[test]
fn precedence_mul_binds_tighter_than_add() {
    let e = expr_of("1+2*3");
    assert_eq!(
        e,
        Expr::Binary {
            op: BinOp::Add,
            l: Box::new(Expr::IntLit(1)),
            r: Box::new(Expr::Binary {
                op: BinOp::Mul,
                l: Box::new(Expr::IntLit(2)),
                r: Box::new(Expr::IntLit(3)),
                span: Span::default(),
            }),
            span: Span::default(),
        }
    );
}

#[test]
fn precedence_arithmetic_binds_tighter_than_comparison() {
    let e = expr_of("1 + 2 > 3");
    assert_eq!(
        e,
        Expr::Binary {
            op: BinOp::Gt,
            l: Box::new(Expr::Binary {
                op: BinOp::Add,
                l: Box::new(Expr::IntLit(1)),
                r: Box::new(Expr::IntLit(2)),
                span: Span::default(),
            }),
            r: Box::new(Expr::IntLit(3)),
            span: Span::default(),
        }
    );
}

#[test]
fn precedence_and_binds_looser_than_comparison() {
    // a > 0 && b < 1  =>  (a>0) && (b<1)
    let e = expr_of("a > 0 && b < 1");
    assert_eq!(
        e,
        Expr::Binary {
            op: BinOp::And,
            l: Box::new(Expr::Binary {
                op: BinOp::Gt,
                l: Box::new(var("a")),
                r: Box::new(Expr::IntLit(0)),
                span: Span::default(),
            }),
            r: Box::new(Expr::Binary {
                op: BinOp::Lt,
                l: Box::new(var("b")),
                r: Box::new(Expr::IntLit(1)),
                span: Span::default(),
            }),
            span: Span::default(),
        }
    );
}

#[test]
fn precedence_or_is_lowest() {
    // a && b || c  =>  (a&&b) || c
    let e = expr_of("a && b || c");
    assert_eq!(
        e,
        Expr::Binary {
            op: BinOp::Or,
            l: Box::new(Expr::Binary {
                op: BinOp::And,
                l: Box::new(var("a")),
                r: Box::new(var("b")),
                span: Span::default(),
            }),
            r: Box::new(var("c")),
            span: Span::default(),
        }
    );
}

#[test]
fn parens_override_precedence() {
    let e = expr_of("(1+2)*3");
    assert_eq!(
        e,
        Expr::Binary {
            op: BinOp::Mul,
            l: Box::new(Expr::Binary {
                op: BinOp::Add,
                l: Box::new(Expr::IntLit(1)),
                r: Box::new(Expr::IntLit(2)),
                span: Span::default(),
            }),
            r: Box::new(Expr::IntLit(3)),
            span: Span::default(),
        }
    );
}

#[test]
fn cast_postfix_shape() {
    let e = expr_of("x as fx");
    assert_eq!(
        e,
        Expr::Cast {
            e: Box::new(var("x")),
            to: Ty::Fx,
            span: Span::default(),
        }
    );
}

#[test]
fn cast_postfix_chains_left_to_right() {
    let e = expr_of("x as fx as int");
    assert_eq!(
        e,
        Expr::Cast {
            e: Box::new(Expr::Cast {
                e: Box::new(var("x")),
                to: Ty::Fx,
                span: Span::default(),
            }),
            to: Ty::Int,
            span: Span::default(),
        }
    );
}

#[test]
fn engine_var_shape() {
    assert_eq!(
        expr_of("$self_hp"),
        Expr::EngineVar(EngVar::SelfHp, Span::default())
    );
}

#[test]
fn unknown_engine_var_is_an_error() {
    let (_, errors) = parse_src("sub main() { _ = $bogus; }");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].msg.contains("bogus"), "{}", errors[0].msg);
}

#[test]
fn global_read_is_a_dedicated_ast_node() {
    let e = expr_of("global(RANK_SLOT)");
    assert_eq!(
        e,
        Expr::GlobalRead {
            slot: Box::new(var("RANK_SLOT")),
            span: Span::default(),
        }
    );
}

#[test]
fn set_global_is_a_generic_call_not_global_read() {
    let e = expr_of("set_global(0, 1)");
    assert_eq!(
        e,
        Expr::Call {
            name: "set_global".to_string(),
            args: vec![Expr::IntLit(0), Expr::IntLit(1)],
            span: Span::default(),
        }
    );
}

// ── 错误路径：行列 + 源行摘录 + 恢复 ────────────────────────────────

#[test]
fn missing_semicolon_error_message_is_pinned() {
    let (_, errors) = parse_src("sub main() { var x: int = 5 }");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 1);
    assert_eq!(errors[0].msg, "期待 ';'，实际是 '}'");
}

#[test]
fn missing_closing_brace_error_message_is_pinned() {
    let (_, errors) = parse_src("sub main() { if a { ");
    assert!(!errors.is_empty());
    assert!(
        errors.iter().any(|e| e.msg == "期待 '}'，实际是 文件末尾"),
        "{errors:?}"
    );
}

#[test]
fn bad_token_at_expression_start_is_an_error() {
    let (_, errors) = parse_src("sub main() { var x: int = @; }");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].msg, "期待表达式，遇到 '@'");
}

/// 恢复策略核心判别：两处**独立**语句各自的错误都要报出来，不能第一个错误的恢复
/// 把第二个错误所在的整条语句都吞掉。
#[test]
fn error_recovery_reports_two_independent_errors_same_file() {
    let src = "sub main() {\n    var x: int = ;\n    var y: int = 1 + ;\n}";
    let (_, errors) = parse_src(src);
    assert_eq!(errors.len(), 2, "两处独立错误都应报出：{errors:?}");
    assert_eq!(errors[0].line, 2);
    assert_eq!(errors[1].line, 3);
}

/// 端到端：真实解析产出的错误经 `render()` 精确匹配契约格式（行/列/源行/caret 对齐）。
#[test]
fn end_to_end_error_render_matches_pinned_format() {
    let src = "sub main() {\n  wait(1;\n}\n";
    let (_, errors) = parse_src(src);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].line, 2);
    assert_eq!(errors[0].col, 9);
    assert_eq!(errors[0].msg, "期待 ')'，实际是 ';'");
    assert_eq!(errors[0].src_line, "  wait(1;");

    let rendered = errors[0].render("boss.ecl");
    let expected_line3 = format!("  {}^", " ".repeat(8));
    let expected = format!(
        "boss.ecl:2:9: 期待 ')'，实际是 ';'\n  {}\n{}",
        "  wait(1;", expected_line3
    );
    assert_eq!(rendered, expected);
}
