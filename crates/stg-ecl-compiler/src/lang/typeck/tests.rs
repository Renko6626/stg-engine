use super::*;
use crate::lang::ast::Ty;
use crate::lang::parse as parse_program;
use stg_core::consts::EngineConst;
use stg_core::ecl::image::EclValueType;

/// 便于按源码片段快速构造 `Program`（复用 T1 前端；本趟测试不手搭 AST，除非要覆盖
/// AST 层面到不了的边界）。
fn prog(src: &str) -> Program {
    parse_program(src, "t.ecl").unwrap_or_else(|e| panic!("解析失败：{e:?}\n源码：\n{src}"))
}

fn ok(src: &str) -> TypedInfo {
    check(&prog(src), &[], None).unwrap_or_else(|e| panic!("判型失败：{e:?}\n源码：\n{src}"))
}

fn err(src: &str) -> Vec<CompileError> {
    check(&prog(src), &[], None).expect_err(&format!("期望判型失败，源码：\n{src}"))
}

fn check_with(src: &str, engine: &[EngineConst]) -> Result<TypedInfo, Vec<CompileError>> {
    check(&prog(src), engine, None)
}

// ── C14 Task 3：引擎常量预填注入 ─────────────────────────────────────────────

#[test]
fn engine_consts_are_injected_as_predeclared_constants() {
    let ti = check_with(
        "sub main() { var a: int = FOO; loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 7)],
    )
    .expect("注入的 FOO 应可用");
    assert!(
        ti.consts
            .iter()
            .any(|(n, t, v)| n == "FOO" && *t == Ty::Int && *v == 7),
        "注入常量应出现在折叠后的 const 表：{:?}",
        ti.consts
    );
}

#[test]
fn script_cannot_shadow_engine_const() {
    let errs = check_with(
        "const FOO: int = 5; sub main() { loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 1)],
    )
    .expect_err("脚本重声明引擎常量应失败");
    assert!(
        errs.iter().any(|e| e.msg.contains("与引擎常量重名")),
        "{errs:?}"
    );
}

#[test]
fn engine_const_type_participates_in_checking() {
    // FOO 是 int，用在需 fx 的位置应判型失败（证明注入常量带类型进了判型）
    let errs = check_with(
        "sub main() { var a: fx = FOO; loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 7)],
    )
    .expect_err("int 注入常量赋给 fx 应失败");
    assert!(!errs.is_empty(), "应有判型错误");
}

fn check_err(src: &str) -> Vec<CompileError> {
    err(src)
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
    let errors = err("sub main() { fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未消费")),
        "{errors:?}"
    );
}

#[test]
fn call_with_return_value_discarded_is_ok() {
    ok("sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
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

// ── `wait_spell` 保留字化（Task 3 复审修，C16 同类事故）──────────────────────
//
// `wait_spell()` 语句糖在 `lang::parse` 层靠"名字恰为 'wait_spell' 且紧跟 '('"这一
// 纯文本条件截胡展开，**先于任何 sub/const/var 名解析**——跟 C16 修复前的 `global()`
// 是同一类雷，但 `global` 那次是"改造成普通 builtin、走 sub 优先的调用解析顺序"，这次
// `wait_spell` 不是可调用符号（它是整条语句的糖，不是表达式位置的函数名），走同一条路
// 修不通：正确修法是把它保留字化——声明处直接拒绝同名 sub/const/var（详见
// `typeck::checker::RESERVED_SUGAR_NAMES`）。

#[test]
fn sub_named_wait_spell_is_rejected_as_reserved_sugar_name() {
    // 复现审员报告的确切场景：声明 `sub wait_spell() {}` 后在语句位置调用
    // `wait_spell();`——修复前这段脚本静默编译通过（parser 展开成 while，用户 sub 体
    // 编译进镜像但永远调不到，零诊断零 fault）；修复后必须在声明处报编译错。
    let errors = err("sub wait_spell() { } sub main() { wait_spell(); }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("wait_spell") && e.msg.contains("保留字")),
        "{errors:?}"
    );
}

#[test]
fn const_named_wait_spell_is_rejected_as_reserved_sugar_name() {
    let errors = err("const wait_spell: int = 1; sub main() { loop { wait(1); } }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("wait_spell") && e.msg.contains("保留字")),
        "{errors:?}"
    );
}

#[test]
fn var_named_wait_spell_is_rejected_as_reserved_sugar_name() {
    let errors = err("sub main() { var wait_spell: int = 1; }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("wait_spell") && e.msg.contains("保留字")),
        "{errors:?}"
    );
}

#[test]
fn wait_spell_call_as_a_plain_statement_still_type_checks() {
    // 对照：保留字化只拦"声明同名符号"，不影响 `wait_spell();` 本身作为语句糖正常使用
    // ——parser 早已把它展开成普通 `Stmt::While`，本趟看到的就是一个平平无奇的 while，
    // 糖展开路径分毫未动。
    ok("sub main() { wait_spell(); }");
}

// ── Task 4 复审修：`RESERVED_SUGAR_NAMES` 冲突文案按撞上的名字动态生成 ──────────
//
// `check_not_reserved_sugar_name` 原先硬编码"符卡等待语句糖 `wait_spell()` 专用"，
// `mark` 加入保留名单（Task 4）后，若脚本声明 `sub mark()`，会报出这句字面提到
// `wait_spell()` 却答非所问的话——用户撞的明明是 `mark`。文案须按实际撞上的名字动态
// 生成，两个保留名各自的报错都必须点名自己、不能串到另一个身上。

#[test]
fn sub_named_mark_is_rejected_with_name_specific_message() {
    let errors = err("sub mark() { } sub main() { loop { wait(1); } }");
    assert!(
        errors.iter().any(|e| e.msg.contains("mark")
            && e.msg.contains("保留字")
            && !e.msg.contains("wait_spell")),
        "mark 撞保留字的报错必须点名 mark、不能残留 wait_spell 字样：{errors:?}"
    );
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

#[test]
fn main_cannot_be_a_sync_call_target() {
    let errors = check_err("sub main() {} sub helper() { main(); }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("main 只能作为关卡根入口启动"))
    );
}

#[test]
fn main_cannot_be_spawned() {
    let errors = check_err("sub main() { spawn main(); }");
    assert!(
        errors
            .iter()
            .any(|e| e.msg.contains("spawn 目标 'main' 必须声明为 async"))
    );
}

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
         sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, trail); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("无参 async sub")),
        "{errors:?}"
    );
}

#[test]
fn fire_task_ref_on_plain_sub_is_an_error() {
    let errors = err("sub on_hit() { } \
         sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, on_hit); }");
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
    ok("sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
}

#[test]
fn fire_xf_known_xformdef_is_ok_and_recorded() {
    let ti = ok(
        "xformdef RING { turn(90deg); } sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, RING, none); }",
    );
    let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
    assert_eq!(main.xform_refs, vec!["RING".to_string()]);
}

#[test]
fn fire_xf_unknown_xformdef_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, NOPE, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未知的 xformdef")),
        "{errors:?}"
    );
}

#[test]
fn fire_task_known_sub_is_ok() {
    ok(
        "async sub bullet_task() { } sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, bullet_task); }",
    );
}

#[test]
fn fire_task_unknown_sub_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, nope); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("未知的 sub")),
        "{errors:?}"
    );
}

#[test]
fn fire_xf_non_identifier_expr_is_an_error() {
    let errors = err("sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, 1, none); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("标识符")),
        "{errors:?}"
    );
}

// ── 符卡（spell_begin/spell_end/spell_timer；spec 2026-07-24 §5）──────────────

/// `spell_begin` 第三位 `pattern` 是 `SubRef`（同 `fire` 的 `task` 参同款：sub 名或
/// `none`，编译期解析，不收求值表达式）——接受已声明的无参 async sub。
#[test]
fn spell_begin_third_param_accepts_sub_name() {
    ok("async sub p() { loop { wait(1); } } \
        sub main() { spell_begin(0, 1, p, 60, 100, 0, 0); }");
}

#[test]
fn spell_begin_third_param_accepts_none() {
    ok("sub main() { spell_begin(0, 1, none, 60, 100, 0, 0); }");
}

/// `pattern` 位不收求值表达式——语法上必须是裸标识符/`none`（同 `fire` 的 xf/task 位）。
#[test]
fn spell_begin_third_param_rejects_expression() {
    let errors = err("sub main() { spell_begin(0, 1, 1 + 1, 60, 100, 0, 0); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("标识符")),
        "{errors:?}"
    );
}

/// `spell_timer()` 返回 `int`，可用在任何需要 `int` 的位置。
#[test]
fn spell_timer_usable_where_int_expected() {
    ok("sub main() { var t: int = spell_timer(); while spell_timer() >= 0 { wait(1); } }");
}

/// `spell_end()` 无返回值：只能作独立语句，出现在表达式位置是错误（同其余 `ret:None`
/// 内建一致的诊断路径）。
#[test]
fn spell_end_is_statement_only_and_has_no_return_value() {
    ok("sub main() { spell_end(); }");
    let errors = err("sub main() { var x: int = spell_end(); }");
    assert!(
        errors.iter().any(|e| e.msg.contains("表达式的值")),
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

// ── 通道 B `emit_req`：RawVal 六位三型任意、id 位仍钉 Int ───────────────────

#[test]
fn emit_req_rawval_accepts_all_three_types_id_stays_int() {
    // RawVal 六位三型任意（fx/int/angle/表达式混填皆良型）
    ok("sub main() { emit_req(64, 1.5fx, -3, 90deg, 2 + 3, 0fx, 0); loop { wait(1); } }");
    // id 位仍是 Val(Int)：传 fx 必须报参数类型错（本断言在 emit_req 未注册前会因
    // "未定义函数"类错误而**不含此文案**——红），实现后转为精确命中
    let errs = err("sub main() { emit_req(1.0fx, 0, 0, 0, 0, 0, 0); loop { wait(1); } }");
    assert!(
        errs.iter().any(|e| e.msg.contains("第 1 个参数期待")),
        "id 位传 fx 应报参数类型错：{errs:?}"
    );
}

// ── 颜色轴（T3）：表派生常量 `BULLET_COLOR_STRIDE` ────────────────────────────
//
// 名字是引擎级词汇（结构），值来自绑定的那张表（内容）；不对称拍板：绑定表时注入，
// 未绑定表（`None`）时不注入——脚本引用它应得到"未知标识符"，而不是一个撒谎的
// 默认值（本节走完整 `compile`/`compile_with_options` 管线，不是裸 `typeck::check`，
// 故用 `crate::lang::` 完整路径，不复用本文件顶部只喂 `typeck::check` 的 `ok`/`err`
// 辅助函数）。

/// 绑定表时注入表派生常量 `BULLET_COLOR_STRIDE`（值来自表，不是引擎硬编码）。
#[test]
fn bound_table_injects_color_stride_const() {
    let src = "sub main() { var w: int = BULLET_COLOR_STRIDE; _ = w; }";
    let img = crate::lang::compile(src, "t.ecl").expect("绑定内建表应能引用 stride 常量");
    let _ = img;
}

/// 未绑定表（table = None）时不注入——脚本引用它应报"未定义的变量"，
/// 而不是悄悄拿到某个默认值。
#[test]
fn unbound_table_does_not_inject_color_stride() {
    let src = "sub main() { var w: int = BULLET_COLOR_STRIDE; _ = w; }";
    let errs = crate::lang::compile_with_options(
        src,
        "t.ecl",
        crate::lang::CompileOptions {
            debug_info: crate::lang::DebugInfo::None,
        },
        stg_core::consts::ENGINE_CONSTS,
        None,
    )
    .expect_err("未绑定表不得注入 stride 常量");
    // 复审 M-4：只断言"有错误"抓不住"把'未绑表'改成别的硬错误"这类语义变更——
    // 补断言错误内容确实是"未定义标识符"（`BULLET_COLOR_STRIDE` 没被注入进常量表，
    // 落到与任何未声明变量同样的判型路径），而不是碰巧因为别的理由报错。
    assert!(
        errs.iter()
            .any(|e| e.msg.contains("未定义") && e.msg.contains("BULLET_COLOR_STRIDE")),
        "应报'未定义'且点名 BULLET_COLOR_STRIDE：{errs:?}"
    );
}

// ── A10：常量 `wait(n)` 的越界值编译期挡一道 ─────────────────────────────────

/// 字面量与 `const` 折出的越界 `wait` 都报编译错。
///
/// **判别力**：三条腿分别钉住三种错法——
/// ① `65536` 是**恰好截断成 0** 的那个值（漏挡 ⇒ `loop { wait(65536) }` 变死循环）；
/// ② `-1` 走的是另一侧（截断成 65535，"等 18 分钟"）；
/// ③ `const` 折叠腿证明闸挂在**常量求值之后**，不是只认字面量 token。
/// 若把判据写成 `v > 65535`（漏掉负数侧）②转红；写成只查 `Expr::IntLit` 则③转红。
#[test]
fn const_wait_out_of_range_is_a_compile_error() {
    for src in [
        "sub main() { wait(65536); }",
        "sub main() { wait(-1); }",
        "const W: int = 131072;\nsub main() { wait(W); }",
    ] {
        let es = err(src);
        assert!(
            es.iter()
                .any(|e| e.msg.contains("wait") && e.msg.contains("越界")),
            "期望 wait 越界编译错，实得 {es:?}\n源码：\n{src}"
        );
    }
}

/// 边界两端合法 + **运行期表达式一律放行**（A10 明写的分工：截断语义只在编译期常量上被挡，
/// 运行期不加检查——那会落进断层线以下）。
///
/// **判别力**：若把闸误做成"对所有 wait 都查"，第三条（`$self_age` 是运行期值）转红；
/// 若把范围写成 `1..=65535`（顺手把 `wait(0)` 也禁了），第一条转红——`wait(0)` 是刻意
/// 裁定的合法 no-op。
#[test]
fn wait_boundaries_are_legal_and_runtime_expressions_pass_through() {
    ok("sub main() { wait(0); }");
    ok("sub main() { wait(65535); }");
    ok("async sub s() { wait($self_age); }\nsub main() { wait(1); }");
}
