//! ECL 表层语言前端总入口（M1.9 T1）——`.ecl` 源码手感的编译器前端：手写 lexer，接一层
//! 递归下降与 Pratt 结合的 parser，产出 AST。**没有类型检查、没有 codegen**（分别是 T2/T3
//! 的刀，见 `docs/superpowers/plans/2026-07-18-m19-ecl-language.md`）。
//!
//! 编译器全程无 IO、无时钟、只用 `Vec`/`String`（不用 `HashMap` 等无序容器参与任何影响输出
//! 顺序的路径）——同源码两次编译必须逐字节产出同一份 AST（本刀先钉住 `Program` 的确定性，
//! T3 上升到 `EclImage` 层面钉全管线确定性）。

pub mod ast;
pub mod builtins;
pub mod codegen;
pub mod debug;
mod entryck;
pub mod lex;
pub mod parse;
pub mod slots;
pub mod typeck;
pub(crate) mod xform_map;

pub use ast::{CompileError, Program};
pub use debug::{DebugParamMeta, DebugSubMeta, EclDebugSymbols, PcSourceSpan};
pub use stg_core::ecl::image::EclImage;

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
pub fn compile_with_options(
    src: &str,
    file: &str,
    options: CompileOptions,
) -> Result<CompiledEcl, Vec<CompileError>> {
    let program = parse(src, file)?;
    if let Err(mut errors) = entryck::check(&program) {
        attach_src_lines(&mut errors, src);
        return Err(errors);
    }
    let typed = match typeck::check(&program) {
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
    let image = match codegen::generate(&program, &typed, &slot_map) {
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

/// 便利包装等价于 `compile_with_options(src, file, CompileOptions { debug_info: DebugInfo::None }).map(|ce| ce.image)`。
///
/// 见 [`compile_with_options`] 的完整文档。
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(src, file, CompileOptions { debug_info: DebugInfo::None })
        .map(|ce| ce.image)
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

    /// `compile()` 失败路径断言助手——`EclImage`（`compile` 的 `Ok` 类型）不 derive
    /// `Debug`（**有意**：stg-core 唯一触碰面钉死在 T3 Commit A，见 plan Self-Review
    /// "stg-core 唯一触碰 = T3-A"，Commit B 不得为了测试方便回头给 `EclImage` 加 derive），
    /// 故不能直接 `.unwrap_err()`（它要求 `Ok` 类型也 `Debug`）——手写 `match` 绕开。
    fn expect_compile_err(src: &str, file: &str) -> Vec<CompileError> {
        match compile(src, file) {
            Err(errors) => errors,
            Ok(_) => panic!("期望编译失败，源码：\n{src}"),
        }
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

    /// 编译器确定性（全管线级别，plan 明文钉死）：同源码两次 `compile` 必须产出逐字节相同
    /// 的 `code`/`subs`（`EclImage` 未 derive `PartialEq`，逐字段比较即可）。
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
        )
        .unwrap();
        let full = compile_with_options(
            src,
            "stage.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
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
        )
        .unwrap();
        let debug = out.debug.unwrap();
        let helper = debug.symbol("helper").unwrap();
        assert_eq!(helper.kind(), SubKind::CallOnly);
        assert_eq!(debug.param_name(helper, 0), Some("value"));
        assert_eq!(debug.source_at(helper.pc_start()).unwrap().file(), "stage.ecl");
    }
}
