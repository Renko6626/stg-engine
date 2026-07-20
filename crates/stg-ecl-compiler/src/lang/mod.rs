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
mod entryck;
pub mod lex;
pub mod parse;
pub mod slots;
pub mod typeck;
pub(crate) mod xform_map;

pub use ast::{CompileError, Program};
pub use stg_core::ecl::image::EclImage;

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

/// 核心接口块钉死的全管线入口：`compile(src, file) -> Result<EclImage, Vec<CompileError>>`。
///
/// **T3 起管线全线贯通**：`parse → entryck::check → typeck::check` →
/// `slots::allocate → codegen::generate`，五趟任何一趟报错都会被收集进最终 `Err`；全部通过则
/// 产出可执行的 [`EclImage`]（`stg-core` VM 直接消费的镜像格式，`content_hash` 占位 0，同
/// `ImageBuilder::build` 既有惯例——文件加载/内容哈希整包归 C11 刀）。
///
/// **`CompileError.src_line` 在这里被回填**：`entryck::check`/`typeck::check`/
/// `slots::allocate`/`codegen::generate` 都没有原始源码文本（签名只收
/// `&Program`/`&TypedInfo`/`&SlotMap`），
/// 产出的错误 `src_line` 恒为空串（见各模块文档）——本函数是唯一持有 `src` 的地方，用
/// `src.lines()` 把这些错误的 `src_line` 补全，让最终交给用户的 `CompileError::render()`
/// 仍是完整契约格式。
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
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
    match codegen::generate(&program, &typed, &slot_map) {
        Ok(image) => Ok(image),
        Err(mut errors) => {
            attach_src_lines(&mut errors, src);
            Err(errors)
        }
    }
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
        assert_eq!(image.subs.len(), 1, "一个 sub = 一个入口");
        assert!(!image.code.is_empty(), "至少要有一条终结 op");
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
        assert_eq!(img1.code, img2.code, "两次编译的字节码必须逐字相同");
        assert_eq!(img1.subs, img2.subs, "两次编译的入口表必须逐字相同");
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
}
