use super::*;
use crate::lang::ast::Ty;
use crate::lang::parse as parse_program;

/// 便于按源码片段快速构造 `Program`（复用 T1 前端；本趟测试不手搭 AST，除非要覆盖
/// AST 层面到不了的边界）。
fn prog(src: &str) -> Program {
    parse_program(src, "t.ecl").unwrap_or_else(|e| panic!("解析失败：{e:?}\n源码：\n{src}"))
}

fn ok(src: &str) -> TypedInfo {
    check(&prog(src)).unwrap_or_else(|e| panic!("判型失败：{e:?}\n源码：\n{src}"))
}

fn err(src: &str) -> Vec<CompileError> {
    check(&prog(src)).expect_err(&format!("期望判型失败，源码：\n{src}"))
}

fn main_body(ti: &TypedInfo) -> &[TypedStmt] {
    &ti.subs
        .iter()
        .find(|s| s.name == "main")
        .expect("应有 main")
        .body
}

/// 抽出 `main` 里唯一一条 `ExprStmtDiscard`/`Var` 的求值表达式，供矩阵测试断言型与
/// intent（每条测试源码只放一条相关语句，简化定位）。
fn only_var_init(ti: &TypedInfo) -> &TypedExpr {
    match &main_body(ti)[0] {
        TypedStmt::Var { init, .. } => init,
        other => panic!("期望 Var 语句，得到 {other:?}"),
    }
}

// ── 类型矩阵：`+`/`-` ────────────────────────────────────────────────────────

#[test]
fn add_int_int_is_legal_int_addi() {
    let ti = ok("sub main() { var x: int = 1 + 2; }");
    let e = only_var_init(&ti);
    assert_eq!(e.ty, Ty::Int);
    assert!(matches!(
        e.kind,
        TypedExprKind::Binary {
            intent: BinIntent::AddI,
            ..
        }
    ));
}

#[test]
fn add_fx_fx_is_legal_fx_addi() {
    let ti = ok("sub main() { var x: fx = 1.0fx + 2.0fx; }");
    let e = only_var_init(&ti);
    assert_eq!(e.ty, Ty::Fx);
    assert!(matches!(
        e.kind,
        TypedExprKind::Binary {
            intent: BinIntent::AddI,
            ..
        }
    ));
}

#[test]
fn add_angle_angle_is_legal_angle_addi() {
    let ti = ok("sub main() { var x: angle = 10deg + 20deg; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Angle);
}

#[test]
fn sub_angle_angle_is_legal() {
    let ti = ok("sub main() { var x: angle = 90deg - 10deg; }");
    let e = only_var_init(&ti);
    assert_eq!(e.ty, Ty::Angle);
    assert!(matches!(
        e.kind,
        TypedExprKind::Binary {
            intent: BinIntent::SubI,
            ..
        }
    ));
}

#[test]
fn add_fx_int_is_illegal_no_implicit_conversion() {
    let errors = err("sub main() { var x: fx = 1.0fx + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 类型矩阵：`*` ────────────────────────────────────────────────────────────

#[test]
fn mul_int_int_is_legal_muli() {
    let ti = ok("sub main() { var x: int = 3 * 4; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Int);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::MulI,
            ..
        }
    ));
}

#[test]
fn mul_fx_fx_is_legal_mulf() {
    let ti = ok("sub main() { var x: fx = 1.0fx * 2.0fx; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Fx);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::MulF,
            ..
        }
    ));
}

#[test]
fn mul_fx_int_is_legal_fx_muli() {
    let ti = ok("sub main() { var x: fx = 1.0fx * 3; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Fx);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::MulI,
            ..
        }
    ));
}

#[test]
fn mul_int_fx_is_legal_fx_muli_symmetric() {
    let ti = ok("sub main() { var x: fx = 3 * 1.0fx; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Fx);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::MulI,
            ..
        }
    ));
}

#[test]
fn mul_angle_int_is_illegal() {
    let errors = err("sub main() { var x: angle = 90deg * 2; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

/// 矩阵非法格的**真判别**腿（T5 变异①存活的教训）：上面两条 angle-乘测试的声明型
/// 恰好也不匹配，"非法格放行"的变异会被 var 声明二次错误掩护而存活。此处声明为
/// `int`——变异放行后表达式恰好判 Int、整句零错误，err() 助手立刻炸——矩阵格本身
/// 被钉死，二次错误无从掩护。
#[test]
fn mul_angle_angle_illegal_cell_pinned_without_secondary_mask() {
    let errors = err("sub main() { var x: int = 90deg * 45deg; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 类型矩阵：`/`（asymmetric fx//int）───────────────────────────────────────

#[test]
fn div_int_int_is_legal_divi() {
    let ti = ok("sub main() { var x: int = 10 / 3; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Int);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::DivI,
            ..
        }
    ));
}

#[test]
fn div_fx_fx_is_legal_divf() {
    let ti = ok("sub main() { var x: fx = 1.0fx / 2.0fx; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Fx);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::DivF,
            ..
        }
    ));
}

#[test]
fn div_fx_int_is_legal_divi() {
    let ti = ok("sub main() { var x: fx = 1.0fx / 2; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Fx);
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Binary {
            intent: BinIntent::DivI,
            ..
        }
    ));
}

/// 判别式核心：`int/fx` 与 `fx/int` **不对称**——`fx/int` 合法，`int/fx` 非法（除数
/// 是 Q16.16 时纯整数除会错位 65536 倍），矩阵里唯一的方向性陷阱。
#[test]
fn div_int_fx_is_illegal_asymmetric() {
    let errors = err("sub main() { var x: fx = 1 / 2.0fx; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 类型矩阵：`%`（仅 int）────────────────────────────────────────────────────

#[test]
fn mod_int_int_is_legal() {
    let ti = ok("sub main() { var x: int = 7 % 3; }");
    assert_eq!(only_var_init(&ti).ty, Ty::Int);
}

#[test]
fn mod_fx_fx_is_illegal() {
    let errors = err("sub main() { var x: fx = 1.0fx % 2.0fx; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 类型矩阵：比较（同型合法，异型非法）───────────────────────────────────────

#[test]
fn comparison_same_type_yields_int_for_all_three_types() {
    for (src, _) in [
        ("sub main() { var x: int = (1 > 2); }", "int"),
        ("sub main() { var x: int = (1.0fx > 2.0fx); }", "fx"),
        ("sub main() { var x: int = (10deg > 20deg); }", "angle"),
    ] {
        let ti = ok(src);
        assert_eq!(only_var_init(&ti).ty, Ty::Int, "src={src}");
    }
}

#[test]
fn comparison_mismatched_types_is_illegal() {
    let errors = err("sub main() { var x: int = (1 > 2.0fx); }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 类型矩阵：`&&`/`||`（拍板：仅 int）────────────────────────────────────────

#[test]
fn logic_and_or_int_int_is_legal() {
    let ti = ok(
        "sub main() { var a: int = 1; var b: int = 0; var x: int = a && b; var y: int = a || b; }",
    );
    // 第 3、4 条语句才是 And/Or；直接检查末条 Or。
    match &main_body(&ti)[3] {
        TypedStmt::Var { init, .. } => {
            assert_eq!(init.ty, Ty::Int);
            assert!(matches!(
                init.kind,
                TypedExprKind::Binary {
                    intent: BinIntent::LogicOr,
                    ..
                }
            ));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn logic_and_fx_fx_is_illegal() {
    let errors = err("sub main() { var x: int = 1.0fx && 2.0fx; }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 一元运算 ─────────────────────────────────────────────────────────────────

#[test]
fn neg_is_legal_for_int_fx_angle() {
    for src in [
        "sub main() { var x: int = -1; }",
        "sub main() { var x: fx = -1.0fx; }",
        "sub main() { var x: angle = -90deg; }",
    ] {
        ok(src);
    }
}

#[test]
fn not_is_legal_for_int_only() {
    ok("sub main() { var x: int = !1; }");
    let errors = err("sub main() { var x: int = !(1.0fx); }");
    assert!(errors.iter().any(|e| e.msg.contains("int")), "{errors:?}");
}

// ── cast 规则 ───────────────────────────────────────────────────────────────

#[test]
fn cast_int_to_fx_is_legal_with_int_to_fx_intent() {
    let ti = ok("sub main() { var x: fx = 1 as fx; }");
    let e = only_var_init(&ti);
    assert_eq!(e.ty, Ty::Fx);
    assert!(matches!(
        e.kind,
        TypedExprKind::Cast {
            intent: CastIntent::IntToFx,
            ..
        }
    ));
}

#[test]
fn cast_fx_to_int_is_legal_with_fx_to_int_intent() {
    let ti = ok("sub main() { var x: int = 1.0fx as int; }");
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Cast {
            intent: CastIntent::FxToInt,
            ..
        }
    ));
}

#[test]
fn cast_int_angle_bitcast_both_directions() {
    let ti = ok("sub main() { var x: angle = 1 as angle; }");
    assert!(matches!(
        only_var_init(&ti).kind,
        TypedExprKind::Cast {
            intent: CastIntent::Bitcast,
            ..
        }
    ));
    let ti2 = ok("sub main() { var x: int = 90deg as int; }");
    assert!(matches!(
        only_var_init(&ti2).kind,
        TypedExprKind::Cast {
            intent: CastIntent::Bitcast,
            ..
        }
    ));
}

#[test]
fn cast_fx_to_angle_is_not_whitelisted() {
    err("sub main() { var x: angle = 1.0fx as angle; }");
}

#[test]
fn cast_angle_to_fx_is_not_whitelisted() {
    err("sub main() { var x: fx = 90deg as fx; }");
}

// ── 值消费检查 ──────────────────────────────────────────────────────────────

#[test]
fn call_with_return_value_not_discarded_is_an_error() {
    let errors = err("sub main() { fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未消费")),
        "{errors:?}"
    );
}

#[test]
fn call_with_return_value_discarded_is_ok() {
    ok("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
}

#[test]
fn void_builtin_statement_without_discard_is_ok() {
    ok("sub main() { pulse_signal(0); }");
}

#[test]
fn void_builtin_discarded_is_an_error_nothing_to_discard() {
    let errors = err("sub main() { _ = pulse_signal(0); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("无值可丢弃")),
        "{errors:?}"
    );
}

#[test]
fn bare_expr_statement_not_discarded_is_an_error() {
    let errors = err("sub main() { var x: int = 1; x + 1; }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未消费")),
        "{errors:?}"
    );
}

#[test]
fn bare_expr_statement_discarded_is_ok() {
    ok("sub main() { var x: int = 1; _ = x + 1; }");
}

// ── sub 调用：只能作独立语句 ───────────────────────────────────────────────

#[test]
fn sub_call_as_bare_statement_is_ok() {
    ok("sub helper() { } sub main() { helper(); }");
}

#[test]
fn sub_call_discarded_is_an_error() {
    let errors = err("sub helper() { } sub main() { _ = helper(); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("无值可丢弃")),
        "{errors:?}"
    );
}

#[test]
fn sub_call_in_expression_position_is_an_error() {
    let errors = err("sub helper() { } sub main() { var x: int = helper() + 1; }");
    assert!(
        errors.iter().any(|e| e.msg.contains("表达式的值")),
        "{errors:?}"
    );
}

// ── `$` 引擎变量判型 ────────────────────────────────────────────────────────

#[test]
fn engine_vars_type_per_plan_table() {
    let cases: &[(&str, Ty)] = &[
        ("$frame", Ty::Int),
        ("$player_x", Ty::Fx),
        ("$player_y", Ty::Fx),
        ("$self_x", Ty::Fx),
        ("$self_y", Ty::Fx),
        ("$self_hp", Ty::Int),
        ("$self_hp_max", Ty::Int),
        ("$self_age", Ty::Int),
    ];
    for (var, ty) in cases {
        let src = match ty {
            Ty::Int => format!("sub main() {{ var x: int = {var}; }}"),
            Ty::Fx => format!("sub main() {{ var x: fx = {var}; }}"),
            Ty::Angle => format!("sub main() {{ var x: angle = {var}; }}"),
        };
        let ti = ok(&src);
        assert_eq!(only_var_init(&ti).ty, *ty, "{var}");
    }
}

// ── `global`/`set_global` ──────────────────────────────────────────────────

#[test]
fn global_read_types_int_and_requires_int_slot() {
    let ti = ok("const S: int = 5; sub main() { var x: int = global(S); }");
    assert_eq!(only_var_init(&ti).ty, Ty::Int);
    let errors = err("sub main() { var x: int = global(1.0fx); }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

#[test]
fn sub_named_global_shadows_the_builtin_when_called_as_a_statement() {
    // C16 复审修复：`global` 曾是 parser 特判出的专属 AST 节点，永远绕过 `check_call` 的
    // "先查 subs、再查 builtins" 解析顺序——用户声明的同名 sub 编译进镜像但永远调不到。
    // 现在 `global` 是普通 builtin 表项，走跟 `set_global` 完全一样的调用解析路径：sub 优先。
    ok("sub global(n: int) { set_global(16, n); } sub main() { global(5); }");
}

#[test]
fn sub_named_global_shadows_the_builtin_and_rejects_expression_position() {
    // 同一颗雷的判别式对照：sub 调用无返回值，不能用作表达式的值——这条报错本身就是
    // "global(5) 被解析成了对用户 sub 的调用，而不是内建 GlobalRead"的实锤证据。
    let errors =
        err("sub global(n: int) { set_global(16, n); } sub main() { var x: int = global(5); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("不能用作表达式的值")),
        "{errors:?}"
    );
}

#[test]
fn set_global_is_a_void_call_checked_like_a_builtin() {
    ok("sub main() { set_global(1, 2); }");
    let errors = err("sub main() { set_global(1, 2.0fx); }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

// ── 字面量折叠交互：`90deg + 10deg` 判型但不做常量折叠 ─────────────────────────

#[test]
fn angle_literal_addition_type_checks_but_stays_a_binary_node_not_folded() {
    let ti = ok("sub main() { var x: angle = 90deg + 10deg; }");
    let e = only_var_init(&ti);
    assert_eq!(e.ty, Ty::Angle);
    assert!(
        matches!(e.kind, TypedExprKind::Binary { .. }),
        "T2 判型层不折叠字面量二元运算，应保持 Binary 节点：{:?}",
        e.kind
    );
}

// ── 未定义名字 ──────────────────────────────────────────────────────────────

#[test]
fn undefined_variable_is_an_error() {
    let errors = err("sub main() { var x: int = y; }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未定义")),
        "{errors:?}"
    );
}

#[test]
fn undefined_function_call_is_an_error() {
    let errors = err("sub main() { _ = no_such_fn(1); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未定义")),
        "{errors:?}"
    );
}

#[test]
fn undefined_spawn_target_is_an_error() {
    let errors = err("sub main() { spawn no_such_sub(); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未定义")),
        "{errors:?}"
    );
}

// ── 变量遮蔽（拒绝，含跨嵌套块 / for 归纳变量 / 局部遮蔽常量允许）──────────────

#[test]
fn duplicate_var_name_in_same_sub_is_rejected() {
    err("sub main() { var x: int = 1; var x: int = 2; }");
}

#[test]
fn duplicate_var_name_across_if_branches_is_still_rejected() {
    // pin：不做作用域分析，哪怕两个声明在互斥分支里也拒绝（简单、确定性槽分配）。
    err("sub main() { if 1 { var x: int = 1; } else { var x: int = 2; } }");
}

#[test]
fn for_loop_var_participates_in_duplicate_check() {
    err("sub main() { var i: int = 0; for i in 0..5 { } }");
}

#[test]
fn local_var_shadowing_a_const_name_is_allowed() {
    ok("const N: int = 1; sub main() { var N: int = 2; _ = N; }");
}

// ── 块作用域 / definite-assignment（复审 Critical 修复）────────────────────────
// locals 的"扁平命名空间"（不支持遮蔽，见上一节）只管**重名检测**——不等于"跨分支/
// 循环体声明的变量随处可读"。一个变量只在声明它的块＋其嵌套块内可见；if 无 else、
// while/for/loop 的循环体都可能一次都不执行，块外读取必须拒绝，否则读到的是该
// locals 槽此前遗留的值（VM locals 是任务级持久内存，不会在进块时清零）。

#[test]
fn var_declared_only_in_if_branch_is_invisible_in_else_branch() {
    let errors = err("sub main() { if 1 { var x: int = 1; } else { var y: int = x + 1; } }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn var_declared_in_if_without_else_is_invisible_after_the_if() {
    let errors = err("sub main() { if 1 { var x: int = 1; } var y: int = x + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn var_declared_in_while_body_is_invisible_after_the_loop() {
    let errors = err("sub main() { while 1 { var x: int = 1; } var y: int = x + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn var_declared_in_for_body_is_invisible_after_the_loop() {
    let errors = err("sub main() { for i in 0..5 { var x: int = 1; } var y: int = x + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn for_induction_var_is_invisible_after_the_loop() {
    let errors = err("sub main() { for i in 0..5 { } var y: int = i + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains('i')), "{errors:?}");
}

#[test]
fn var_declared_in_loop_body_is_invisible_after_the_loop() {
    // `loop {}` 保证至少执行一次，但 break 可能发生在声明之前——保守拒绝。
    let errors = err("sub main() { loop { var x: int = 1; break; } var y: int = x + 1; }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn assigning_to_a_var_out_of_scope_is_rejected_not_silently_typechecked() {
    let errors = err("sub main() { if 1 { var x: int = 1; } x = 2; }");
    assert!(errors.iter().any(|e| e.msg.contains('x')), "{errors:?}");
}

#[test]
fn var_read_within_same_branch_it_was_declared_in_is_ok() {
    ok("sub main() { if 1 { var x: int = 1; _ = x as fx; } }");
}

#[test]
fn var_declared_outside_is_visible_inside_nested_if_body() {
    ok("sub main() { var x: int = 1; if 1 { var y: int = x + 1; _ = y as fx; } }");
}

#[test]
fn var_declared_outside_can_be_reassigned_inside_if_and_read_after() {
    ok("sub main() { var x: int = 1; if 1 { x = 2; } var y: int = x + 1; _ = y as fx; }");
}

// ── spawn 参数校验 ──────────────────────────────────────────────────────────

#[test]
fn spawn_arity_mismatch_is_an_error() {
    let errors = err("async sub helper(a: int) { } sub main() { spawn helper(); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("参数个数")),
        "{errors:?}"
    );
}

#[test]
fn spawn_arg_type_mismatch_is_an_error() {
    let errors = err("async sub helper(a: int) { } sub main() { spawn helper(1.0fx); }");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

#[test]
fn spawn_arity_and_type_match_is_ok_and_recorded_but_not_a_sync_call() {
    let ti = ok("async sub helper(a: int) { } sub main() { spawn helper(1); }");
    let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
    assert!(
        main.sync_calls.is_empty(),
        "spawn 不是同步调用边：{:?}",
        main.sync_calls
    );
}

// ── async/同步途径强制分离（T2 复审 Critical 修复的判别腿）──────────────────────

/// 复审复现场景：同一 sub 既被 spawn 又被同步 CALL——修复前无声通过并给出错位槽，
/// 修复后是编译错误（此测试即当年 Critical 的墓碑）。
#[test]
fn sub_both_spawned_and_called_is_now_a_compile_error() {
    let errors = err("async sub helper(p: int) { var v: int = p; } \
         sub caller() { var pad: int = 0; helper(1); } \
         sub main() { spawn helper(5); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("只能被 spawn")),
        "{errors:?}"
    );
}

#[test]
fn spawn_on_plain_sub_is_an_error() {
    let errors = err("sub helper() { } sub main() { spawn helper(); }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("必须声明为 async sub")),
        "{errors:?}"
    );
}

#[test]
fn sync_call_on_async_sub_is_an_error() {
    let errors = err("async sub helper() { } sub main() { helper(); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("只能被 spawn")),
        "{errors:?}"
    );
}

/// 分离规则第四腿（M1.9 终审 Critical 墓碑）：fire 派生不带实参——带参 async sub
/// 当 fire task 引用时参数恒读零（T2 实参错位的孪生路径），语言层必须拒绝。
#[test]
fn fire_task_ref_with_params_is_an_error() {
    let errors = err("async sub trail(spd: fx) { wait(1); } \
         sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, trail); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("无参 async sub")),
        "{errors:?}"
    );
}

#[test]
fn fire_task_ref_on_plain_sub_is_an_error() {
    let errors = err("sub on_hit() { } \
         sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, on_hit); }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("必须声明为 async sub")),
        "{errors:?}"
    );
}

// ── sync_calls 记录（供 slots.rs 建图）────────────────────────────────────────

#[test]
fn sync_call_records_target_name_deduped() {
    let ti = ok("sub helper() { } sub main() { helper(); helper(); }");
    let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
    assert_eq!(main.sync_calls, vec!["helper".to_string()]);
}

// ── fire 的 xf/task 标识符参数解析 ─────────────────────────────────────────

#[test]
fn fire_xf_none_and_task_none_is_ok() {
    ok("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
}

#[test]
fn fire_xf_known_xformdef_is_ok_and_recorded() {
    let ti = ok(
        "xformdef RING { turn(90deg); } sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, RING, none); }",
    );
    let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
    assert_eq!(main.xform_refs, vec!["RING".to_string()]);
}

#[test]
fn fire_xf_unknown_xformdef_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, NOPE, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未知的 xformdef")),
        "{errors:?}"
    );
}

#[test]
fn fire_task_known_sub_is_ok() {
    ok(
        "async sub bullet_task() { } sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, bullet_task); }",
    );
}

#[test]
fn fire_task_unknown_sub_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, nope); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未知的 sub")),
        "{errors:?}"
    );
}

#[test]
fn fire_xf_non_identifier_expr_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, 1, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("标识符")),
        "{errors:?}"
    );
}

// ── const 折叠 ──────────────────────────────────────────────────────────────

#[test]
fn const_literal_folds_directly() {
    let ti = ok("const N: int = 5;");
    assert_eq!(ti.consts, vec![("N".to_string(), Ty::Int, 5)]);
}

#[test]
fn const_arithmetic_combo_folds() {
    let ti = ok("const N: int = 2 + 3 * 4;");
    assert_eq!(ti.consts, vec![("N".to_string(), Ty::Int, 14)]);
}

#[test]
fn const_referencing_earlier_const_folds() {
    let ti = ok("const A: int = 10; const B: int = A + 1;");
    assert_eq!(
        ti.consts,
        vec![
            ("A".to_string(), Ty::Int, 10),
            ("B".to_string(), Ty::Int, 11)
        ]
    );
}

#[test]
fn const_type_mismatch_is_an_error() {
    let errors = err("const N: fx = 1 + 2;");
    assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
}

#[test]
fn const_referencing_undeclared_name_is_an_error() {
    let errors = err("const N: int = M + 1;");
    assert!(
        errors.iter().any(|e| e.msg.contains("不是已声明的常量")),
        "{errors:?}"
    );
}

#[test]
fn const_negative_angle_folds_with_wrapping() {
    let ti = ok("const A: angle = -90deg;");
    // -16384 の wrapping_neg 在 u16 域回绕 = 65536-16384 = 49152；本折叠走 i32
    // wrapping_neg（-16384i32 as raw），消费点低 16 位截断时同样得 49152——这里直接钉
    // i32 raw 值本身（-16384），符合"回绕天然发生在消费点"的设计（见模块文档 cast 段）。
    assert_eq!(ti.consts, vec![("A".to_string(), Ty::Angle, -16384)]);
}

// ── break/continue 越界检查（有益增项，超出 checklist 字面范围）───────────────

#[test]
fn break_outside_loop_is_an_error() {
    err("sub main() { break; }");
}

#[test]
fn continue_outside_loop_is_an_error() {
    err("sub main() { continue; }");
}

#[test]
fn break_continue_inside_loop_are_ok() {
    ok("sub main() { loop { break; } while 1 { continue; } for i in 0..5 { break; } }");
}

// ── 重复定义 ────────────────────────────────────────────────────────────────

#[test]
fn duplicate_sub_name_is_an_error() {
    err("sub main() { } sub main() { }");
}

#[test]
fn duplicate_const_name_is_an_error() {
    err("const N: int = 1; const N: int = 2;");
}
