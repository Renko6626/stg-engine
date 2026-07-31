//! ECL 表层语言前端总入口（M1.9 T1）——`.ecl` 源码手感的编译器前端：手写 lexer，接一层
//! 递归下降与 Pratt 结合的 parser，产出 AST。**没有类型检查、没有 codegen**（分别是 T2/T3
//! 的刀，见 `docs/superpowers/plans/2026-07-18-m19-ecl-language.md`）。
//!
//! 编译器全程无 IO、无时钟、只用 `Vec`/`String`（不用 `HashMap` 等无序容器参与任何影响输出
//! 顺序的路径）——同源码两次编译必须逐字节产出同一份 AST（本刀先钉住 `Program` 的确定性，
//! T3 上升到 `EclImage` 层面钉全管线确定性）。

pub mod ast;
pub(crate) mod atlas;
pub mod builtins;
pub mod codegen;
mod const_eval;
pub mod debug;
mod entryck;
pub mod lex;
pub mod parse;
pub mod slots;
mod type_rules;
pub mod typeck;
mod units;
pub(crate) mod xform_map;

pub use ast::{CompileError, Program};
pub use debug::{DebugParamMeta, DebugSubMeta, EclDebugSymbols, PcSourceSpan};
pub use stg_core::ecl::image::EclImage;
pub use units::compile_units;

/// 调试信息产出级别（Task 4 侧载开关）。`None` 侧载不存在（默认，`compile` 便利包装使用）；
/// `Full` 侧载包含完整 sub/参数/PC 区间/源码定位信息，EclImage 本身保持不变。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugInfo {
    None,
    Full,
}

/// 编译选项。当前只有 `debug_info` 一个字段，未来可扩展（如优化级别、目标平台等）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub debug_info: DebugInfo,
}

/// 编译产出：运行时镜像 + 可选的调试符号侧载。
///
/// `.image` 在 `DebugInfo::None` 与 `Full` 模式下**逐字节相同**（确定性契约：
/// 调试信息不参与运行时行为）。`.debug` 为 `Some` 当且仅当 `debug_info == Full`。
///
/// `derive(Debug)`：`EclImage`/`EclDebugSymbols` 均已 derive `Debug`（stg-core 侧，非本刀
/// 引入），这里补上纯粹是为了让 `compile_with_options` 的失败路径断言能用
/// `.expect_err(...)`（颜色轴 T3 的"未绑定表不注入"测试），不涉及 `stg-core` 唯一触碰面。
#[derive(Debug)]
pub struct CompiledEcl {
    pub image: EclImage,
    pub debug: Option<EclDebugSymbols>,
}

/// **本刀（T1）的契约入口**：源码 → AST，无类型检查/无 codegen。
///
/// `file` 目前不参与任何计算——`CompileError` 本身不存 `file`（见 `ast::CompileError` 文档的
/// 契约出入说明：同一次调用产出的错误共享一个 `file`，展示时由调用方自己传给
/// [`CompileError::render`]）。保留这个参数只是为了和核心接口块的签名对齐，`parse` 自己不需要
/// 用到它。
pub fn parse(src: &str, _file: &str) -> Result<Program, Vec<CompileError>> {
    let (tokens, mut errors) = lex::Lexer::new(src).lex();
    let (program, parse_errors) = parse::Parser::new(tokens, src).parse_program();
    errors.extend(parse_errors);
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors)
    }
}

/// 编译选项总入口：`parse → entryck::check → typeck::check` →
/// `slots::allocate → codegen::generate → (if Full) build_debug_symbols`。
///
/// 管线始终产生同一份 `EclImage`（与 `debug_info` 无关）；仅在 `DebugInfo::Full` 时
/// 额外构建调试符号侧载（见 [`CompiledEcl.debug`]）。`None` 模式等价于 [`compile`]。
///
/// **`CompileError.src_line` 在这里被回填**：各下游模块的 `CompileError` 没有原始源码
/// 文本（签名只收 `&Program`/`&TypedInfo`/`&SlotMap`），本函数用 `src.lines()` 补全。
///
/// `engine_consts`（C14 Task 3）透传给 `typeck::check`，作为"第 1 行前预声明"的常量注入
/// 判型命名空间；脚本不得重声明同名 const。[`compile`] 默认注入 `stg_core::consts::ENGINE_CONSTS`，
/// 调用方也可传 `&[]` 隔离（如本模块的调试侧载测试，测的是调试信息而非常量注入）。
///
/// `table`（颜色轴 T3，取代此前的裸 `content_hash: u64` 参）——`Some(t)` 时
/// `t.content_hash` 盖入产出 `EclImage.content_hash`，且额外注入表派生脚本常量
/// `BULLET_COLOR_STRIDE`（`int`，值 = `t.color_stride`——名字是引擎级词汇，值来自绑定的
/// 那张表，mod 表换成别的 stride 时脚本 `i % BULLET_COLOR_STRIDE` 自动正确）；`None` 时
/// `content_hash` 取 `0`（"未绑定任何表"）且**不注入** `BULLET_COLOR_STRIDE`——脚本引用它
/// 会得到"未知标识符"错误，而不是一个撒谎的默认值，这条不对称是有意的。`table` 本体也
/// 原样存进判型阶段的 `Checker`（见 `typeck::check`），供后续依赖表内容的判据使用。
/// 语义焊点在 [`compile_for_table`]。
pub fn compile_with_options(
    src: &str,
    file: &str,
    options: CompileOptions,
    engine_consts: &[stg_core::consts::EngineConst],
    table: Option<&stg_core::tables::WorldTables>,
) -> Result<CompiledEcl, Vec<CompileError>> {
    let content_hash = table.map_or(0, |t| t.content_hash);
    // 表派生常量：名字是引擎级词汇（结构），值来自绑定的那张表（内容）。
    // 未绑定表时不注入——脚本引用它会得到"未知标识符"，而不是一个撒谎的默认值。
    let mut consts: Vec<stg_core::consts::EngineConst> = engine_consts
        .iter()
        .map(|c| stg_core::consts::EngineConst::new(c.name, c.ty, c.value))
        .collect();
    if let Some(t) = table {
        consts.push(stg_core::consts::EngineConst::new(
            "BULLET_COLOR_STRIDE",
            stg_core::ecl::image::EclValueType::Int,
            i32::from(t.color_stride),
        ));
    }

    let program = parse(src, file)?;
    if let Err(mut errors) = entryck::check(&program) {
        attach_src_lines(&mut errors, src);
        return Err(errors);
    }
    let typed = match typeck::check(&program, &consts, table) {
        Ok(t) => t,
        Err(mut errors) => {
            attach_src_lines(&mut errors, src);
            return Err(errors);
        }
    };
    let slot_map = match slots::allocate(&program, &typed) {
        Ok(sm) => sm,
        Err(mut errors) => {
            attach_src_lines(&mut errors, src);
            return Err(errors);
        }
    };
    let image = match codegen::generate(&program, &typed, &slot_map, content_hash, table) {
        Ok(image) => image,
        Err(mut errors) => {
            attach_src_lines(&mut errors, src);
            return Err(errors);
        }
    };

    let debug = if options.debug_info == DebugInfo::Full {
        Some(debug::build_debug_symbols(&program, &typed, &image, file))
    } else {
        None
    };

    Ok(CompiledEcl { image, debug })
}

/// 为指定表编译：注入引擎常量（①②）+ 表派生常量 `BULLET_COLOR_STRIDE` + 盖
/// `table.content_hash` 进 `EclImage`（coherence 焊点）。①② 常量仍来自
/// `consts::ENGINE_CONSTS`；乙案将来从表读符号。
pub fn compile_for_table(
    src: &str,
    file: &str,
    table: &stg_core::tables::WorldTables,
) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(
        src,
        file,
        CompileOptions {
            debug_info: DebugInfo::None,
        },
        stg_core::consts::ENGINE_CONSTS,
        Some(table),
    )
    .map(|ce| ce.image)
}

/// 便利包装：绑定内建默认表 `TABLES_V0`。签名不变（既有调用方零改）。
///
/// 等价于 `compile_for_table(src, file, &stg_core::tables::TABLES_V0)`——内部默认注入引擎
/// 常量注册表（C14 Task 3），脚本自动获得引擎命名常量（如 `GVAR_RANK`）而无需任何调用方
/// 改动，并把 `TABLES_V0.content_hash`（LIVE）盖入产出的 `EclImage`（C1 coherence 焊点）。
///
/// 见 [`compile_with_options`] / [`compile_for_table`] 的完整文档。
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
    compile_for_table(src, file, &stg_core::tables::TABLES_V0)
}

/// 回填 `typeck`/`slots` 产出的 [`CompileError`] 的 `src_line`（它们构造时没有源码文本，
/// 见 `compile` 文档）——按 1-based `line` 索引 `src.lines()`，越界同 `CompileError::at`
/// 的兜底纪律退化成空串，不 panic。
fn attach_src_lines(errors: &mut [CompileError], src: &str) {
    let lines: Vec<&str> = src.lines().collect();
    for e in errors.iter_mut() {
        e.src_line = lines
            .get(e.line.saturating_sub(1) as usize)
            .copied()
            .unwrap_or("")
            .to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::ecl::image::SubKind;
    use stg_core::ecl::ops::OP_PUSHI;

    /// `compile()` 失败路径断言助手（手写 `match`，报错文案带上源码）。
    fn expect_compile_err(src: &str, file: &str) -> Vec<CompileError> {
        match compile(src, file) {
            Err(errors) => errors,
            Ok(_) => panic!("期望编译失败，源码：\n{src}"),
        }
    }

    /// 同上，只取错误正文（形/色三判据的断言口径：按措辞关键词判别哪一条判据开了火）。
    /// 内建图集(真美术)没有空格——空格类端到端测试挂在合成表上（见 atlas.rs 同款注释）。
    /// `HOLED_SHAPE`/`HOLED_COLOR` = 被挖空那一格的两个坐标。
    const HOLED_SHAPE: i32 = 3 * 16;
    const HOLED_COLOR: i32 = 7;
    fn holed_table() -> stg_core::tables::WorldTables {
        let mut t = stg_core::tables::build_tables_v0();
        let mut rows = t.appearances.to_vec();
        rows[(HOLED_SHAPE + HOLED_COLOR) as usize].valid = false;
        t.appearances = rows.into_boxed_slice();
        t
    }

    fn compile_err_msgs_for(src: &str, t: &stg_core::tables::WorldTables) -> Vec<String> {
        compile_for_table(src, "t.ecl", t)
            .expect_err("应当编译失败")
            .into_iter()
            .map(|e| e.msg)
            .collect()
    }

    fn compile_err_msgs(src: &str) -> Vec<String> {
        expect_compile_err(src, "t.ecl")
            .into_iter()
            .map(|e| e.msg)
            .collect()
    }

    /// 按 `ecl::ops::ARITY` 逐指令走一遍字节流，返回**出现过的 opcode 序列**。
    ///
    /// 不能用 `code().contains(&(OP_X as u32))` 裸扫字——操作数与 opcode 同住一个 `u32`
    /// 流，`SYS_CREATE_BULLET == 20 == OP_ADD` 就是现成的假阳性（`OP_SYS 20` 的操作数字
    /// 会被误当成一条 `OP_ADD`）。只对**单 sub** 源码可靠（多 sub 时字节流仍是线性拼接，
    /// 但本文件用到它的测试都只有一个 `main`）。
    fn opcodes_of(code: &[u32]) -> Vec<u8> {
        use stg_core::ecl::ops::ARITY;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < code.len() {
            let op = code[i] as u8;
            out.push(op);
            i += 1 + ARITY[op as usize] as usize;
        }
        out
    }

    /// `parse`：贯通 lex→parse，成功路径产出预期形状的 `Program`。
    #[test]
    fn parse_entry_point_wires_lexer_and_parser_together() {
        let program = parse("sub main() { wait(1); }", "smoke.ecl").expect("应解析成功");
        assert_eq!(program.subs.len(), 1);
        assert_eq!(program.subs[0].name, "main");
    }

    /// `parse`：失败路径把词法错误也并入最终 `Vec<CompileError>`（不仅仅是语法错误）。
    #[test]
    fn parse_entry_point_surfaces_lexer_errors_too() {
        let errors = parse("sub main() { _ = 1 ~ 2; }", "smoke.ecl").unwrap_err();
        assert!(errors.iter().any(|e| e.msg.contains('~')), "{errors:?}");
    }

    /// `compile` T3 起产出真正可执行的 `EclImage`（不再是 T1/T2 阶段的 `Program` 桩）——
    /// 空 `main` 仍应有恰一个入口、非空 `code`（至少一条终结 op）。
    #[test]
    fn compile_produces_executable_image_with_one_entry() {
        let image = compile("sub main() { }", "smoke.ecl").expect("应编译成功");
        assert_eq!(image.sub_count(), 1, "一个 sub = 一个入口");
        assert!(!image.code().is_empty(), "至少要有一条终结 op");
    }

    /// C14（引擎常量注入）端到端覆盖，编译器 crate 自己的一份：脚本引用注入的引擎常量
    /// （`REQ_BGM`）经 `TypedExprKind::ConstRef` 求值路径（见 `codegen.rs`）折叠为
    /// 字面量 `PUSHI <value>`——`EclImage` 运行时不认识"常量名"，只认识落地的字面值。
    /// 走真实 `compile()` 入口（默认注入 `stg_core::consts::ENGINE_CONSTS`，不是像调试侧载
    /// 测试那样传 `&[]` 隔离），押运"注入表确实喂到 codegen"这条完整链路（此前只有
    /// `stg-harness` 的 rainbow 金向量间接覆盖，本测试补上编译器 crate 内的直接断言）。
    ///
    /// 样本从 `APPEARANCE_STAR` 换成 `REQ_BGM`：颜色轴刀 T4 清空了 ② 段（弹型名归内容
    /// 包），注入表里只剩 ① 结构常量。
    #[test]
    fn injected_engine_const_folds_to_bytecode_literal() {
        let src = "sub main() { var a: int = REQ_BGM; loop { wait(1); } }";
        let image = compile(src, "smoke.ecl").expect("应编译成功");
        let expected = stg_core::consts::REQ_BGM as u32;
        assert!(
            image
                .code()
                .windows(2)
                .any(|pair| pair == [OP_PUSHI as u32, expected]),
            "REQ_BGM(={expected}) 应折叠为 PUSHI 字面量，code={:?}",
            image.code()
        );
    }

    /// 难度档具名化刀：脚本用 `RANK_HARD` 写难度分支应编译通过，且常量折叠成**正确取值**
    /// （`PUSHI 2`，= `stg_core::consts::RANK_HARD`）。判别力靠"取值"而非"能编译"——把
    /// 五个常量写成任意值都能编译过，只有比对折叠字面量才抓得住值错。
    #[test]
    fn rank_named_consts_compile_and_fold_to_correct_values() {
        let src = "sub main() { if global(GVAR_RANK) >= RANK_HARD { set_global(20, 1); } \
                   loop { wait(1); } }";
        let image = compile(src, "rank.ecl").expect("RANK_HARD 应是注入的引擎常量");
        let hard = stg_core::consts::RANK_HARD as u32;
        assert!(
            image
                .code()
                .windows(2)
                .any(|pair| pair == [OP_PUSHI as u32, hard]),
            "RANK_HARD(={hard}) 应折叠为 PUSHI 字面量，code={:?}",
            image.code()
        );
        // 五个档位名全部可用，且取值互异并等于引擎侧
        for (name, want) in [
            ("RANK_EASY", stg_core::consts::RANK_EASY),
            ("RANK_NORMAL", stg_core::consts::RANK_NORMAL),
            ("RANK_HARD", stg_core::consts::RANK_HARD),
            ("RANK_LUNATIC", stg_core::consts::RANK_LUNATIC),
            ("RANK_EXTRA", stg_core::consts::RANK_EXTRA),
        ] {
            let s = format!("sub main() {{ set_global(20, {name}); loop {{ wait(1); }} }}");
            let img = compile(&s, "rank.ecl").unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(
                img.code()
                    .windows(2)
                    .any(|p| p == [OP_PUSHI as u32, want as u32]),
                "{name} 应折叠为 PUSHI {want}"
            );
        }
    }

    /// 编译器确定性（全管线级别，plan 明文钉死）：同源码两次 `compile` 必须产出逐字节相同
    /// 的镜像（`EclImage` derive 了 `PartialEq`，整体比较即可）。
    #[test]
    fn compiling_same_source_twice_yields_identical_image() {
        let src = "const R: int = 1;\n\
                   sub helper(a: int) { var x: int = a + R; set_global(20, x); }\n\
                   sub main() { helper(5); loop { wait(1); } }";
        let img1 = compile(src, "a.ecl").expect("应编译成功");
        let img2 = compile(src, "a.ecl").expect("应编译成功");
        assert_eq!(img1, img2, "两次编译的镜像必须逐字段相同");
    }

    /// 编译器确定性的最小切片（T1 范围）：同源码两次 `parse` 必须产出逐字段相等的
    /// `Program`（`Expr`/`Stmt`/`Span` 全部 derive `PartialEq`，直接整体比较）。全管线级别
    /// （`EclImage` 逐字节相同）的确定性测试留给 T3。
    #[test]
    fn parsing_same_source_twice_yields_identical_program() {
        let src = "const R: int = 1;\nsub main() { var x: fx = 1.5fx; loop { wait(1); } }";
        let p1 = parse(src, "a.ecl").expect("应解析成功");
        let p2 = parse(src, "a.ecl").expect("应解析成功");
        assert_eq!(p1, p2);
    }

    /// `compile` T2 起贯通 `parse → typeck::check → slots::allocate`：类型层面合法的源码
    /// 应该编译通过（不再只看语法层面）。
    #[test]
    fn compile_runs_typeck_and_slots_on_top_of_parse() {
        let src = "sub main() { var x: fx = 1.0fx + 2.0fx; }";
        assert!(compile(src, "smoke.ecl").is_ok());
    }

    /// `compile` 把 `typeck::check` 的类型错误也纳入最终 `Err`（不仅仅是语法错误）——
    /// 判别式源码语法完全合法（parse 会成功），但 `fx + int` 违反类型矩阵。
    #[test]
    fn compile_surfaces_typeck_errors() {
        let errors = expect_compile_err("sub main() { var x: fx = 1.0fx + 1; }", "smoke.ecl");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    /// `compile` 把 `slots::allocate` 的容量错误也纳入最终 `Err`（递归环——语法/类型层面
    /// 都合法，只有槽分配趟才能发现）。
    #[test]
    fn compile_surfaces_slots_errors() {
        let errors = expect_compile_err("sub a() { a(); } sub main() { a(); }", "smoke.ecl");
        assert!(errors.iter().any(|e| e.msg.contains("递归")), "{errors:?}");
    }

    /// `typeck`/`slots` 产出的 `CompileError` 本身没有 `src_line`（见两模块文档）——
    /// `compile` 总入口必须用它持有的 `src` 回填，最终 `render()` 才是完整契约格式。
    #[test]
    fn compile_backfills_src_line_for_typeck_and_slots_errors() {
        let src = "sub main() { var x: fx = 1.0fx + 1; }";
        let errors = expect_compile_err(src, "smoke.ecl");
        let e = errors.first().expect("应有至少一条错误");
        assert_eq!(e.src_line, src, "单行源码，回填后应等于整行原文");
        let rendered = e.render("smoke.ecl");
        assert!(
            rendered.contains(src),
            "render() 应带上真实源行：{rendered}"
        );
    }

    #[test]
    fn compile_requires_exact_zero_arg_plain_main() {
        let cases = [
            ("async sub worker() {}", "缺少唯一根入口 'sub main()'"),
            ("async sub main() {}", "main 不能声明为 async"),
            ("sub main(x: int) {}", "main 必须是零参数"),
            ("sub main() {} sub main() {}", "sub 名称 'main' 重复"),
        ];
        for (src, needle) in cases {
            let errors = expect_compile_err(src, "root.ecl");
            assert!(errors.iter().any(|e| e.msg.contains(needle)), "{errors:?}");
        }
    }

    #[test]
    fn compile_rejects_calling_main() {
        let errors = expect_compile_err("sub main() {} sub helper() { main(); }", "root.ecl");
        assert!(
            errors
                .iter()
                .any(|e| e.msg.contains("main 只能作为关卡根入口启动"))
        );
    }

    // ── Task 4：调试符号侧载 ─────────────────────────────────────────────

    /// `DebugInfo::None` 与 `Full` 产出逐字节相同的 `EclImage`；`None` 无侧载，
    /// `Full` 有侧载。
    #[test]
    fn debug_mode_does_not_change_runtime_image() {
        let src = "sub main() { helper(1); } sub helper(x: int) {}";
        let none = compile_with_options(
            src,
            "stage.ecl",
            CompileOptions {
                debug_info: DebugInfo::None,
            },
            &[],
            None,
        )
        .unwrap();
        let full = compile_with_options(
            src,
            "stage.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
            None,
        )
        .unwrap();
        assert_eq!(none.image, full.image);
        assert!(none.debug.is_none());
        assert!(full.debug.is_some());
    }

    /// `DebugInfo::Full` 侧载保留 `CallOnly` sub 的名称、参数名、源码定位。
    #[test]
    fn full_debug_symbols_keep_call_only_and_parameter_names() {
        let src = "sub main() { helper(1); } sub helper(value: int) {}";
        let out = compile_with_options(
            src,
            "stage.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
            None,
        )
        .unwrap();
        let debug = out.debug.unwrap();
        let helper = debug.symbol("helper").unwrap();
        assert_eq!(helper.kind(), SubKind::CallOnly);
        assert_eq!(debug.param_name(helper, 0), Some("value"));
        assert_eq!(
            debug.source_at(helper.pc_start()).unwrap().file(),
            "stage.ecl"
        );
    }

    // ── C1（Task 4）：content_hash 透传 + compile_for_table ─────────────────

    /// `compile()` 绑定内建默认表 `TABLES_V0`（LIVE hash）——编译产出的 `EclImage.content_hash`
    /// 必须等于 `TABLES_V0.content_hash`，而不是历史上恒为 0 的桩值。这是 C11（运行时表校验）
    /// 依赖的 coherence 焊点：镜像自己记着"我是为哪张表编的"。
    #[test]
    fn compiled_image_carries_target_table_content_hash() {
        let img = compile("sub main() { }", "t.ecl").expect("minimal main compiles");
        assert_ne!(img.content_hash(), 0, "绑定 TABLES_V0（LIVE hash）");
        assert_eq!(img.content_hash(), stg_core::tables::TABLES_V0.content_hash);
    }

    // ── Task 4：`mark` 语句 + EclImage 标记表 ─────────────────────────────

    #[test]
    fn mark_compiles_in_main_top_level() {
        let img = compile(
            "const M: int = 3;\nsub main() { wait(1); mark(M); wait(1); }",
            "m.ecl",
        )
        .expect("合法 mark");
        assert!(img.resolve_mark(3).is_some());
    }

    #[test]
    fn mark_with_block_compiles() {
        compile("sub main() { mark(1) { bgm(2); } wait(1); }", "m.ecl").expect("带补偿块");
    }

    #[test]
    fn mark_outside_main_rejected() {
        let e = expect_compile_err("sub main() { s(); }\nsub s() { mark(1); }", "m.ecl");
        assert!(
            e.iter()
                .any(|e| e.msg.contains("mark") && e.msg.contains("main"))
        );
    }

    #[test]
    fn mark_nested_in_block_rejected() {
        let e = expect_compile_err("sub main() { if (1) { mark(1); } }", "m.ecl");
        assert!(e.iter().any(|e| e.msg.contains("顶层")));
    }

    #[test]
    fn mark_id_zero_or_dup_or_nonconst_rejected() {
        assert!(
            expect_compile_err("sub main() { mark(0); }", "m.ecl")
                .iter()
                .any(|e| e.msg.contains("正整数"))
        );
        assert!(
            expect_compile_err("sub main() { mark(1); wait(1); mark(1); }", "m.ecl")
                .iter()
                .any(|e| e.msg.contains("重复"))
        );
        assert!(
            expect_compile_err("sub main() { var x: int = 1; mark(x); }", "m.ecl")
                .iter()
                .any(|e| e.msg.contains("常量"))
        );
    }

    /// Task 4 复审修：mark 编号必须判为 `Ty::Int`（spec §2.1"编译期常量 int"）。
    /// `1.5fx` 折叠出的原始值 `98304`（Q16.16 raw）满足 `> 0`，仅看原始值会误放行；
    /// 必须连 `const_eval::evaluate` 返回的 `Ty` 一并判断。
    #[test]
    fn mark_id_must_be_int_typed() {
        let e = expect_compile_err("sub main() { mark(1.5fx); }", "m.ecl");
        assert!(e.iter().any(|e| e.msg.contains("int")), "{e:?}");
    }

    #[test]
    fn var_before_mark_rejected() {
        let e = expect_compile_err("sub main() { var x: int = 1; mark(2); }", "m.ecl");
        assert!(
            e.iter()
                .any(|e| e.msg.contains("var") && e.msg.contains("mark"))
        );
    }

    /// 字节码形状断言（简报 Step 3）：`resolve_mark` 落点前一条指令是 `OP_JMP` 且目标 > 落点
    /// ——正常流一跳跨过垫片，不落入补偿块。端到端（真正跑 `World`/`step` 观察正常流不执行
    /// 补偿块副作用）版本归 Task 6（`new_game_at` 消费 `resolve_mark` 之后才有意义）。
    #[test]
    fn normal_flow_skips_landing_pad() {
        use stg_core::ecl::ops::OP_JMP;
        let img =
            compile("sub main() { mark(9) { bgm(9); } wait(1); }", "m.ecl").expect("合法 mark");
        let ip = img.resolve_mark(9).expect("mark(9) 应已注册落点") as usize;
        // `JMP` 是 2 字指令（操作码 + 目标操作数）：落点前紧邻的两个字必须恰是这条 JMP。
        assert!(ip >= 2, "落点前必须还有至少两个字（JMP 操作码 + 操作数）");
        assert_eq!(
            img.code()[ip - 2],
            OP_JMP as u32,
            "落点前一条指令必须是 OP_JMP（正常流跨过垫片）"
        );
        let jmp_target = img.code()[ip - 1] as usize; // JMP 的操作数字（已回填为绝对目标）
        assert!(
            jmp_target > ip,
            "JMP 目标必须严格晚于落点（正常流跳过整段垫片，不落进补偿块）"
        );
    }

    // ── Task 5：mark 自动补偿（最近声明注入）── 字节码形状断言 ──────────────
    //
    // 端到端版本（真跑 `World`/`step`，经 `new_game_at(start=id)` 中段启动观察补偿
    // 生效）依赖 Task 6 的 `new_game_at`，本刀尚不存在——按简报"依赖顺序说明"，退而
    // 直接比对 `image.code()` 在 `resolve_mark` 落点处的指令形状：注入形态恒为
    // `push_i(v); sys(SYS_*)` 直发（`OP_PUSHI v, OP_SYS no` 两字一组），顺序固定
    // bgm→bg→bg_phase，且严格排在作者块之前。

    use stg_core::ecl::ops::OP_SYS;
    use stg_core::ecl::syscall::{SYS_BG, SYS_BG_PHASE, SYS_BGM};

    /// ① 最近声明注入三类齐全 + ⑤ 跨 sub（声明住被同步调用的 sub 里）取得到——单个
    /// 测试两条覆盖同时钉住：`mark(7)` 前最近一条同步调用链是 `main → stage1`，
    /// `stage1` 顶层依次声明 bgm/bg/bg_phase 三个常量，三类应全部注入且顺序固定。
    #[test]
    fn mark_injects_nearest_anchor_declarations_across_sub_call() {
        const SRC: &str = r#"
sub stage1() { bgm(11); bg(21); bg_phase(1); wait(1); }
sub main() {
    stage1();
    mark(7);
    loop { wait(60); }
}
"#;
        let img = compile(SRC, "comp.ecl").expect("编译");
        let ip = img.resolve_mark(7).expect("mark(7) 应已注册落点") as usize;
        let code = img.code();
        assert_eq!(
            &code[ip..ip + 12],
            &[
                OP_PUSHI as u32,
                11,
                OP_SYS as u32,
                SYS_BGM as u32,
                OP_PUSHI as u32,
                21,
                OP_SYS as u32,
                SYS_BG as u32,
                OP_PUSHI as u32,
                1,
                OP_SYS as u32,
                SYS_BG_PHASE as u32,
            ],
            "三类锚点应按 bgm→bg→bg_phase 顺序各注入一组 push_i/sys：{code:?}"
        );
    }

    /// ② 作者块顶层手写 bgm 则只注入 bg/bg_phase——逐类独立抑制，手写的那一类不再
    /// 自动补，未手写的两类（这里 bg_phase 从未声明过，恒 None；bg 未被手写）照常。
    #[test]
    fn mark_manual_override_suppresses_injection_per_kind() {
        const SRC: &str = r#"
sub stage1() { bgm(11); bg(21); wait(1); }
sub main() {
    stage1();
    mark(3) { bgm(99); }
    loop { wait(60); }
}
"#;
        let img = compile(SRC, "comp.ecl").expect("编译");
        let ip = img.resolve_mark(3).expect("mark(3) 应已注册落点") as usize;
        let code = img.code();
        // 只注入 bg（bgm 被作者块手写抑制；bg_phase 从未声明，本就是 None），随后紧跟
        // 作者块自己写的 `bgm(99)`。
        assert_eq!(
            &code[ip..ip + 8],
            &[
                OP_PUSHI as u32,
                21,
                OP_SYS as u32,
                SYS_BG as u32,
                OP_PUSHI as u32,
                99,
                OP_SYS as u32,
                SYS_BGM as u32,
            ],
            "bgm 类应被作者块手写抑制（不注入 11），bg 类仍注入，紧接作者块自己的 \
             bgm(99)：{code:?}"
        );
    }

    /// ③ `bg_phase` 声明早于最近一条 `bg` 声明 → 不注入 phase（旧背景的段号）；
    /// `bg` 本身仍正常注入。
    #[test]
    fn stale_bg_phase_older_than_bg_not_injected() {
        const SRC: &str = r#"
sub main() {
    bg_phase(5);
    bg(30);
    mark(1);
    loop { wait(60); }
}
"#;
        let img = compile(SRC, "comp.ecl").expect("编译");
        let ip = img.resolve_mark(1).expect("mark(1) 应已注册落点") as usize;
        let code = img.code();
        assert_eq!(
            &code[ip..ip + 4],
            &[OP_PUSHI as u32, 30, OP_SYS as u32, SYS_BG as u32],
            "只应注入 bg（30）；bg_phase(5) 早于 bg(30)，视为旧背景段号，不注入：{code:?}"
        );
    }

    /// ④ 变量参声明被跳过——`bgm(x)` 的 `x` 是局部变量（`LocalRef`），不是常量，
    /// 扫描不捕获；`mark` 处不应有任何注入，落点直接是作者块之后的第一条真实指令。
    #[test]
    fn mark_skips_declaration_with_variable_arg() {
        const SRC: &str = r#"
sub stage1() { var x: int = 5; bgm(x); wait(1); }
sub main() {
    stage1();
    mark(4);
    loop { wait(60); }
}
"#;
        let img = compile(SRC, "comp.ecl").expect("编译");
        let ip = img.resolve_mark(4).expect("mark(4) 应已注册落点") as usize;
        let code = img.code();
        // mark(4) 无补偿块，若无注入，落点直接是 main 里下一条语句 `loop { wait(60); }`
        // 的第一条指令：`wait(60)` 降低为 `push_i(60); OP_WAIT`。
        assert_eq!(
            &code[ip..ip + 3],
            &[OP_PUSHI as u32, 60, stg_core::ecl::ops::OP_WAIT as u32],
            "bgm(x) 的 x 是变量参，不应被捕获，落点不应有任何注入指令：{code:?}"
        );
    }

    /// ⑥ if 块内的声明取不到——扫描"不下潜"if/while/for/loop 块体，`if` 分支里的
    /// `bgm(77)` 不参与"最近声明"，`mark` 处应无任何注入。
    #[test]
    fn mark_skips_declaration_inside_if_block() {
        const SRC: &str = r#"
sub main() {
    if 1 == 1 { bgm(77); }
    mark(2);
    loop { wait(60); }
}
"#;
        let img = compile(SRC, "comp.ecl").expect("编译");
        let ip = img.resolve_mark(2).expect("mark(2) 应已注册落点") as usize;
        let code = img.code();
        assert_eq!(
            &code[ip..ip + 3],
            &[OP_PUSHI as u32, 60, stg_core::ecl::ops::OP_WAIT as u32],
            "if 块内的 bgm(77) 不该参与顶层线性扫描，落点不应有任何注入指令：{code:?}"
        );
    }

    // ── 颜色轴 T4：形/色两参糖 + 编译期三判据 ──────────────────────────────
    //
    // 三条判据的**施加顺序**是契约的一部分（色号 → 弹型 → 空格），理由见
    // `swapped_shape_and_color_is_caught_by_stride_check` 的文档：折叠之后的 id 无法
    // 区分"作者写反了"和"作者就要那一格"。

    /// 判据①：色号越界。
    #[test]
    fn color_out_of_range_is_compile_error() {
        let msgs =
            compile_err_msgs("sub main() { _ = fire(0, 99, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(msgs.iter().any(|m| m.contains("色号")), "实际: {msgs:?}");
    }

    /// 判据②：弹型不是 stride 的倍数。
    #[test]
    fn shape_not_on_stride_boundary_is_compile_error() {
        let msgs =
            compile_err_msgs("sub main() { _ = fire(5, 0, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(msgs.iter().any(|m| m.contains("弹型")), "实际: {msgs:?}");
    }

    /// 判据③：图集空格——本刀最有价值的一道闸（隐形弹）。
    #[test]
    fn blank_atlas_cell_is_compile_error() {
        // 判别前提：同一格在内建表里合法（编得过），只有合成表里是空格
        compile(
            "sub main() { _ = fire(48, 7, 0fx, 0fx, 0fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("内建表里这一格是合法的");
        let msgs = compile_err_msgs_for(
            "sub main() { _ = fire(48, 7, 0fx, 0fx, 0fx, 0deg, none, none); }",
            &holed_table(),
        );
        assert!(msgs.iter().any(|m| m.contains("空格")), "实际: {msgs:?}");
    }

    /// **形/色写反**：`fire(COLOR_BLUE, BULLET_AMULET, …)` = `fire(8, 112, …)`，折叠后
    /// `8 + 112 = 120` 恰是第 7 形的第 8 色——一个**完全合法**的格。折叠后查 id 的实现
    /// 会放行它（静默发出错误弹型，半径也跟着错）；只有"先分别校验两参、且色号先于
    /// 弹型"才抓得住。本测试专门钉死实现顺序，别把它优化掉。
    #[test]
    fn swapped_shape_and_color_is_caught_by_stride_check() {
        // 前提自检：折叠值确实落在一个合法格上，否则本测试没有判别力。
        let folded = &stg_core::tables::TABLES_V0.appearances[(8 + 112) as usize];
        assert!(folded.valid, "前提：8+112=120 必须是合法格");

        let msgs =
            compile_err_msgs("sub main() { _ = fire(8, 112, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(
            msgs.iter().any(|m| m.contains("色号")),
            "写反必须被色号越界抓住（120 折叠后是合法格，查 id 抓不到）；实际: {msgs:?}"
        );
    }

    /// `batch` 与 `fire` 同一套判据（别只改一边）。
    #[test]
    fn batch_shares_the_same_shape_color_checks() {
        let msgs = compile_err_msgs_for(
            "sub main() { _ = batch(48, 7, 0fx, 0fx, 1, 0deg, 0deg, 1, 0fx, 0fx); }",
            &holed_table(),
        );
        assert!(msgs.iter().any(|m| m.contains("空格")), "实际: {msgs:?}");
    }

    /// DoD 1（前腿）：**常量对折叠出的字节码 ≡ 整数字面量对折叠出的字节码**——`const`
    /// 引用路径（`ConstRef`）与字面量路径（`IntLit`）必须走同一个折叠分支，产出逐字段
    /// 相同的 `EclImage`（两者绑定同一张 `TABLES_V0`，`content_hash` 也相同）。
    #[test]
    fn const_pair_folds_like_integer_literal_pair() {
        let via_consts = compile(
            "const BULLET_BALL_S: int = 16;\n\
             const COLOR_CHARTREUSE: int = 3;\n\
             sub main() { _ = fire(BULLET_BALL_S, COLOR_CHARTREUSE, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("常量对应编译成功");
        let via_literals = compile(
            "sub main() { _ = fire(16, 3, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("字面量对应编译成功");
        assert_eq!(
            via_consts, via_literals,
            "const 折叠路径与字面量路径必须产出同一份镜像"
        );
    }

    /// DoD 1（后腿）：**常量对零运行期开销**——两参都是编译期常量时字节码里不该有
    /// `OP_ADD`（折成单个 `PUSHI`）；色参换成运行期变量才发一条加法。
    #[test]
    fn const_shape_color_pair_emits_no_add_but_variable_color_does() {
        use stg_core::ecl::ops::OP_ADD;

        let folded = compile(
            "sub main() { _ = fire(16, 3, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("常量对应编译成功");
        assert!(
            !opcodes_of(folded.code()).contains(&OP_ADD),
            "常量对必须折成单个字面量，不发加法：{:?}",
            folded.code()
        );
        // 折叠出的字面量确实是 16+3=19（不是"恰好没发加法但也没折对"）。
        assert!(
            folded
                .code()
                .windows(2)
                .any(|pair| pair == [OP_PUSHI as u32, 19]),
            "应折成 `PUSHI 19`：{:?}",
            folded.code()
        );

        let dynamic = compile(
            "sub main() { var c: int = 3; _ = fire(16, c, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("变量色应编译成功（判据只对编译期常量对施加）");
        assert!(
            opcodes_of(dynamic.code()).contains(&OP_ADD),
            "变量色必须发一条运行期加法：{:?}",
            dynamic.code()
        );
    }

    /// `batch` 也走折叠（不只是 `fire`）——`builtins::fold_start` 是两处共用的
    /// 单一权威，但只有真跑一遍才钉得死"codegen 那一侧没把 `batch` 漏掉"：漏掉 =
    /// 给 9 参 syscall 压 10 个值，整条参数序列错位，且没有任何显眼信号。
    #[test]
    fn batch_const_pair_folds_too() {
        use stg_core::ecl::ops::OP_ADD;
        let img = compile(
            "sub main() { _ = batch(16, 3, 0fx, 0fx, 1, 0deg, 0deg, 1, 1fx, 0fx); }",
            "t.ecl",
        )
        .expect("常量对应编译成功");
        assert!(
            !opcodes_of(img.code()).contains(&OP_ADD),
            "batch 的常量对同样应折成单个字面量：{:?}",
            img.code()
        );
        assert!(
            img.code()
                .windows(2)
                .any(|pair| pair == [OP_PUSHI as u32, 19]),
            "应折成 `PUSHI 19`：{:?}",
            img.code()
        );
    }

    /// **押运测试**（shooter 刀 T2）：颜色轴糖的折叠**起始下标**从硬编码的 0 改成按内建查
    /// （`fire`/`batch` 折前两参，`sh_sprite` 折第 2、3 参因为第 1 参是 `id`）——这条把改动
    /// 前的产物**逐字节**钉死，是动既有代码的安全网。数组是改动前跑出来的真实字节流，
    /// 常量对与变量色两条支路都在里面（一个 sub 里连着两次调用）。
    ///
    /// 红了先看 `codegen::gen_builtin_call` 的折叠判据，别急着更新期望值：
    /// `fire`/`batch` 的字节流变了 = 金向量必然漂移。
    #[test]
    fn fire_and_batch_bytecode_is_byte_for_byte_unchanged() {
        let fire = compile(
            "sub main() { _ = fire(16, 3, 1fx, 2fx, 3fx, 0deg, none, none); \
             var c: int = 3; _ = fire(16, c, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("应编译成功");
        assert_eq!(
            fire.code(),
            &[
                10, 19, 10, 65536, 10, 131072, 10, 196608, 10, 0, 10, 0, 10, 0, 10, 4294967295, 60,
                20, 14, 10, 3, 12, 0, 10, 16, 11, 0, 20, 10, 65536, 10, 131072, 10, 196608, 10, 0,
                10, 0, 10, 0, 10, 4294967295, 60, 20, 14, 0
            ],
            "fire 的产物必须与折叠下标改动前逐字节相同"
        );

        let batch = compile(
            "sub main() { _ = batch(16, 3, 0fx, 0fx, 1, 0deg, 0deg, 1, 1fx, 0fx); \
             var c: int = 3; _ = batch(16, c, 0fx, 0fx, 1, 0deg, 0deg, 1, 1fx, 0fx); }",
            "t.ecl",
        )
        .expect("应编译成功");
        assert_eq!(
            batch.code(),
            &[
                10, 19, 10, 0, 10, 0, 10, 1, 10, 0, 10, 0, 10, 1, 10, 65536, 10, 0, 60, 21, 14, 10,
                3, 12, 0, 10, 16, 11, 0, 20, 10, 0, 10, 0, 10, 1, 10, 0, 10, 0, 10, 1, 10, 65536,
                10, 0, 60, 21, 14, 0
            ],
            "batch 的产物必须与折叠下标改动前逐字节相同"
        );
    }

    /// `sh_sprite` 的折叠发生在**第 2、3 参**（第 1 参是 `id`）——常量对折成单个字面量，
    /// 变量色发一条运行期加法，与 `fire`/`batch` 同构，只是起点不同。
    #[test]
    fn sh_sprite_folds_at_index_one_not_zero() {
        use stg_core::ecl::ops::OP_ADD;
        let folded = compile("sub main() { sh_sprite(0, 16, 3); }", "t.ecl").expect("应编译成功");
        assert!(
            !opcodes_of(folded.code()).contains(&OP_ADD),
            "常量对应折成单个字面量：{:?}",
            folded.code()
        );
        assert!(
            folded.code().windows(2).any(|p| p == [OP_PUSHI as u32, 19]),
            "应折成 `PUSHI 19`：{:?}",
            folded.code()
        );
        // 判别腿：`id` 那一位**没有**被卷进折叠——0(id) 与 16(shape) 若被误折成
        // `PUSHI 16` 再加上 3，栈上只剩两个值，syscall 参数序列整条错位。
        assert!(
            folded.code().windows(2).any(|p| p == [OP_PUSHI as u32, 0]),
            "id 位必须独立压栈：{:?}",
            folded.code()
        );

        let dynamic = compile(
            "sub main() { var c: int = 3; sh_sprite(0, 16, c); }",
            "t.ecl",
        )
        .expect("应编译成功");
        assert!(
            opcodes_of(dynamic.code()).contains(&OP_ADD),
            "变量色必须发一条运行期加法：{:?}",
            dynamic.code()
        );
    }

    /// 变量色**跳过判据**的真正理由是"不是编译期常量"，不是"值恰好合法"——故拿一个
    /// **越界**的变量色（99 ≥ stride 16）来判别：必须照样编译通过，把拦截让给运行期
    /// syscall 的 `valid` 判据（先验后建）。若判据实现哪天顺手把 `LocalRef` 的初值也
    /// 折出来判，本测试立刻红。
    #[test]
    fn out_of_range_variable_color_is_left_to_runtime() {
        compile(
            "sub main() { var c: int = 99; _ = fire(16, c, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .expect("变量色越界不该在编译期报错——运行期 syscall 的 valid 判据兜底");
    }

    // ── 复审必修一：xformdef `set_sprite` 走同一套图集判据 ───────────────────
    //
    // 三判据当初只挂在 fire/batch 上，`set_sprite(shape, color)` 漏网——它写的是同一个
    // 数域，照样能造出"有判定但看不见的弹"，也照样会被写反。判据本体收进 `lang::atlas`
    // 后两条路共用；下面三条是 xformdef 侧的判别腿（措辞与 fire/batch 侧一致）。

    /// 空格：`set_sprite(144, 12)` = 第 9 形第 12 色（掩码 0x0FFF）——本刀最有价值的闸。
    #[test]
    fn set_sprite_blank_atlas_cell_is_compile_error() {
        let msgs = compile_err_msgs_for(
            "xformdef X { set_sprite(48, 7); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
            &holed_table(),
        );
        assert!(msgs.iter().any(|m| m.contains("空格")), "实际: {msgs:?}");
    }

    /// 色号越界：`set_sprite(0, 99)`。
    #[test]
    fn set_sprite_color_out_of_range_is_compile_error() {
        let msgs = compile_err_msgs(
            "xformdef X { set_sprite(0, 99); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
        );
        assert!(msgs.iter().any(|m| m.contains("色号")), "实际: {msgs:?}");
        assert!(
            msgs.iter().any(|m| m.contains("set_sprite")),
            "报错应点名 set_sprite（而不是某个 fire）：{msgs:?}"
        );
    }

    /// **写反**：`set_sprite(COLOR_BLUE, BULLET_AMULET)` = `set_sprite(8, 112)`。两参同型，
    /// 比 `fire` 更容易写反；折叠值 8+112=120 是合法格，只有"色号先查"抓得住。
    /// 报错必须落在 xformdef 那一行（第 1 行），不是 `fire` 那行。
    #[test]
    fn set_sprite_swapped_shape_and_color_is_caught() {
        let errs = expect_compile_err(
            "xformdef X { set_sprite(8, 112); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
            "t.ecl",
        );
        let e = errs.first().expect("应有至少一条错误");
        assert!(
            e.msg.contains("色号"),
            "写反必须被色号越界抓住（120 折叠后是合法格）；实际: {}",
            e.msg
        );
        assert_eq!(e.line, 1, "报错应落在 xformdef 槽那一行：{errs:?}");
        assert!(e.col >= 1);
    }

    /// 合法的 `set_sprite` 照常通过（防上面三条把整条路堵死的假绿）。
    #[test]
    fn set_sprite_valid_cell_compiles() {
        compile(
            "xformdef X { set_sprite(16, 3); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
            "t.ecl",
        )
        .expect("16+3 是合法格，应编译通过");
    }

    // ── 颜色轴刀 T7：`set_shape`/`set_color` 部分设——单轴判据 + 无表报错 ─────────
    //
    // xformdef 只在被某个 sub 的 `fire(...)` 引用后才会走 codegen staging（`gen_xformdef_staging`
    // 按 `sub.xform_refs` 遍历，未被引用的 xformdef 是死代码，slots 趟只查过 op 名合法性，
    // 不下潜到参数）——下面三条都跟 `set_sprite_*` 系列一样，在 `main` 里补一条 `fire`
    // 引用把 X 从死代码里捞出来，判据才真正跑得到。

    /// xformdef 里的单轴 op：坏值编译期拒、空格落点放行。
    #[test]
    fn partial_sprite_ops_reject_bad_values_only() {
        let bad = compile_err_msgs(
            "xformdef X { set_color(99); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
        );
        assert!(bad.iter().any(|m| m.contains("色号")), "实际: {bad:?}");

        let bad2 = compile_err_msgs(
            "xformdef X { set_shape(5); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
        );
        assert!(bad2.iter().any(|m| m.contains("弹型")), "实际: {bad2:?}");

        // 会落到空格的写法必须**编译通过**（裁定：允许，由作者负责）
        compile(
            "xformdef X { set_color(12); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
            "t.ecl",
        )
        .expect("部分设落到空格是允许的，不得报编译错误");
    }

    /// 编译器把绑定表的 stride 写进 args[1]——没有表就无从得知，必须报错而不是猜。
    #[test]
    fn partial_sprite_ops_require_a_bound_table() {
        let errs = compile_with_options(
            "xformdef X { set_color(3); }\n\
             sub main() { _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, X, none); }",
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::None,
            },
            stg_core::consts::ENGINE_CONSTS,
            None,
        )
        .expect_err("未绑定表时 set_color 无法确定色轴宽度，必须报错");
        // 复审 M-4：只断言"有错误"抓不住"把'未绑表'改成别的硬错误"这类语义变更——
        // 补断言措辞确实点名"色轴宽度"（codegen.rs 的 OpWithStride 分支报错原文），
        // 而不是碰巧因为别的理由报错。
        assert!(
            errs.iter().any(|e| e.msg.contains("色轴宽度")),
            "应点名'色轴宽度'：{errs:?}"
        );
    }

    /// 四条动词在表层语言里可用、类型正确（angle 参数收 angle 型、speed 收 fx 型）。
    #[test]
    fn motion_verbs_compile_from_source() {
        let src = "sub main() {\n\
            move_vel(20, 90deg, 4.0fx, 3);\n\
            move_vel_xy(12, 3.0fx, -7.0fx, 0);\n\
            move_angle(30, 45deg, 2);\n\
            move_speed(15, 2.5fx, 1);\n\
        }";
        let image = compile(src, "motion.ecl").expect("四条动词应能编译");
        assert!(!image.code().is_empty());
    }

    /// 【白名单完整性的判别式】"保住一轴"用例：只插 vy、vx 一字不动。
    /// 这是 §4.2 那个用例的正面证据，也是**白名单只加 $self_speed/$self_angle
    /// 而漏掉两个笛卡尔变量时唯一会红的测试**。
    #[test]
    fn keeping_one_cartesian_axis_compiles() {
        let src = "sub main() {\n\
            move_vel_xy(30, $self_vx, 4.0fx, 2);\n\
            move_angle(60, $self_angle + 15deg, 3);\n\
            move_speed(30, $self_speed * 2.0fx, 2);\n\
            move_vel_xy(0, 1.0fx, $self_vy, 0);\n\
        }";
        compile(src, "self_vel.ecl").expect("四个 $self_* 都要在白名单里");
    }

    // ── 终审必修 I-1：一张非 16 宽的表串起全部四个 stride 消费者 ───────────────
    //
    // 现有测试全在 `TABLES_V0`（stride=16）上跑：`bound_table_injects_color_stride_const`
    // （`typeck/tests.rs`）只断言"能编译"、从不断言值；两条
    // `*_threads_bound_table_stride_into_args1_end_to_end`（`codegen.rs`）虽名为"穿线判别"，
    // 但表是 16 宽，硬编码 16 与读表不可区分；此前唯一的 mod 形态表测试
    // （`mod_shaped_table_drives_checks_by_its_own_stride`，已并入本模块）只走 `fire`，
    // 不碰 `codegen.rs` `OpWithStride` 分支的穿线点。终审在隔离 worktree 里把
    // `lang/mod.rs` 注入常量的值、`lang/codegen.rs` `OpWithStride` 写 `args[1]` 两处分别
    // 换成字面量 `16`，`cargo test --workspace` 两次都绿——这个模块补的就是这份判别力：
    // 一张 **8 宽**表（与 16 处处可辨，不是它的因数变体）串起"引擎里读 `color_stride` 的
    // 地方"已知的全部四个消费点，任何一点悄悄换成字面量 `16` 都应该让本模块至少一条测试
    // 变红；将来再长出第五个消费点、忘了接线，也该在这里补第五条，而不是另开一个模块。
    mod mod_table_drives_every_stride_consumer {
        use super::*;

        /// 7 形 × 8 色的 mod 形态表：全表满色（`valid = true` 全铺），`color_stride = 8`。
        fn mod_table() -> stg_core::tables::WorldTables {
            let mut t = stg_core::tables::build_tables_v0();
            t.color_stride = 8;
            let rows: Vec<stg_core::tables::AppearanceCfg> = (0..7 * 8)
                .map(|i| stg_core::tables::AppearanceCfg {
                    radius: stg_core::math::Fx::from_int(4),
                    sprite: i as u16, // identity
                    valid: true,
                })
                .collect();
            t.appearances = rows.into_boxed_slice();
            t.content_hash = 0; // 未参与本模块任何测试
            t
        }

        /// 消费者①（终审变异点 `lang/mod.rs:113`）：`compile_with_options` 注入的
        /// `BULLET_COLOR_STRIDE` 常量值必须来自**这张表自己的** stride（8），不是硬编码
        /// 的 16。若该行被换成字面量 16，脚本引用 `BULLET_COLOR_STRIDE` 折出来的字节码会
        /// 是 `PUSHI 16`，本断言找不到 `PUSHI 8` 而红。
        #[test]
        fn injected_constant_reads_stride_from_the_table_not_16() {
            let t = mod_table();
            let img = compile_for_table(
                "sub main() { var w: int = BULLET_COLOR_STRIDE; _ = w; }",
                "t.ecl",
                &t,
            )
            .expect("绑定 8 宽表应能编译");
            assert!(
                img.code()
                    .windows(2)
                    .any(|pair| pair == [OP_PUSHI as u32, 8]),
                "BULLET_COLOR_STRIDE 应折成该表自己的 stride=8，不是硬编码 16：{:?}",
                img.code()
            );
        }

        /// 消费者②：`fire` 的形/色三判据（`lang::atlas::check_shape_color`）必须按表自己
        /// 的 stride 走。色号 8 在 8 色表越界（在内建 16 色表合法）；弹型 8（=1×8）在 8
        /// 色表合法（若误按 16 走会被判"不是 16 的倍数"而报错）——两个方向都验，防
        /// "只挡该挡的"这种单向假绿。
        #[test]
        fn fire_checks_go_by_the_tables_own_stride_not_16() {
            let t = mod_table();
            let e = compile_for_table(
                "sub main() { _ = fire(0, 8, 0fx, 0fx, 0fx, 0deg, none, none); }",
                "t.ecl",
                &t,
            )
            .expect_err("8 色表里色号 8 必须越界");
            assert!(e.iter().any(|x| x.msg.contains("色号")), "实际: {e:?}");

            compile_for_table(
                "sub main() { _ = fire(8, 1, 0fx, 0fx, 0fx, 0deg, none, none); }",
                "t.ecl",
                &t,
            )
            .expect("8 色表里弹型 8 是第 1 形，必须合法");
        }

        /// 消费者③：`batch` 与 `fire` 共用同一套判据（`builtins::fold_start`），但
        /// 此前从没有一条 mod 表测试盯过 `batch` 这一侧——补上，同一对判别输入。
        #[test]
        fn batch_checks_go_by_the_tables_own_stride_not_16() {
            let t = mod_table();
            let e = compile_for_table(
                "sub main() { _ = batch(0, 8, 0fx, 0fx, 1, 0deg, 0deg, 1, 0fx, 0fx); }",
                "t.ecl",
                &t,
            )
            .expect_err("8 色表里色号 8 必须越界");
            assert!(e.iter().any(|x| x.msg.contains("色号")), "实际: {e:?}");

            compile_for_table(
                "sub main() { _ = batch(8, 1, 0fx, 0fx, 1, 0deg, 0deg, 1, 0fx, 0fx); }",
                "t.ecl",
                &t,
            )
            .expect("8 色表里弹型 8 是第 1 形，必须合法");
        }

        /// 消费者④（终审变异点 `lang/codegen.rs:316`）：`OpWithStride`（`set_shape`/
        /// `set_color`）写进 `args[1]` 的值必须是绑定表自己的 stride——跑真 `World`：
        /// `fire(8, 2, …)` 折叠出初始 sprite = 8+2=10（第 1 形第 2 色，8 色表下合法）；
        /// `set_color(5)` 只应改色 → `(10 - 10%8) + 5 = 13`。若 `args[1]` 被写死成 16，
        /// `10 % 16 == 10`，运行期会算成 `(10 - 10) + 5 = 5`（两者可辨：13 ≠ 5）。
        #[test]
        fn opwithstride_threads_the_tables_own_stride_into_args1_end_to_end() {
            let t = mod_table();
            let src = "xformdef X { set_color(5); }\n\
                       sub main() {\n\
                         _ = fire(8, 2, 0fx, 0fx, 0fx, 0deg, X, none);\n\
                         wait(1000);\n\
                       }";
            let img = compile_for_table(src, "t.ecl", &t).expect("应编译成功");
            let mut w = stg_core::step::World::new(1);
            w.start_main(&img).expect("main 应能派生");
            for f in 0..3 {
                stg_core::step::step(&mut w, &t, &img, &stg_core::input::InputFrame::empty(f));
            }
            let view = w.body.view();
            let p = view.bullets();
            let i = p.iter_alive().next().expect("应有一颗弹");
            assert_eq!(
                p.sprite()[i],
                13,
                "set_color(5) 应按表自己的 stride=8 只改色 → (10-10%8)+5=13\
                 （stride 若被写死成 16 会是 5，不会是 13）"
            );
            assert_eq!(view.diag().task_faults, 0);
        }
    }
}
