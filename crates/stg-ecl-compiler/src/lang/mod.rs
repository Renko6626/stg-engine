//! ECL 表层语言前端总入口（M1.9 T1）——`.ecl` 源码手感的编译器前端：手写 lexer，接一层
//! 递归下降与 Pratt 结合的 parser，产出 AST。**没有类型检查、没有 codegen**（分别是 T2/T3
//! 的刀，见 `docs/superpowers/plans/2026-07-18-m19-ecl-language.md`）。
//!
//! 编译器全程无 IO、无时钟、只用 `Vec`/`String`（不用 `HashMap` 等无序容器参与任何影响输出
//! 顺序的路径）——同源码两次编译必须逐字节产出同一份 AST（本刀先钉住 `Program` 的确定性，
//! T3 上升到 `EclImage` 层面钉全管线确定性）。

pub mod ast;
pub mod lex;
pub mod parse;

pub use ast::{CompileError, Program};

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
/// **T1 阶段桩实现（有意的契约偏离，计划 Task 1 文件描述原文即"`compile` 入口暂 stub 到
/// parse"）**：类型趟（T2）、槽分配趟（T2）、codegen（T3）都还不存在，此刻不可能产出
/// `EclImage`。桩签名先退化成 `Result<Program, Vec<CompileError>>`（= 直接转发
/// [`parse`]），T3 落地时会把返回类型改成 `Result<EclImage, Vec<CompileError>>` 并接入完整
/// 管线——**调用方在 T3 之前不应该依赖本函数产出可执行镜像**，详见
/// `.superpowers/sdd/task-1-report.md` 的「契约偏离」记录。
pub fn compile(src: &str, file: &str) -> Result<Program, Vec<CompileError>> {
    parse(src, file)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// `compile` T1 桩：行为上等价 `parse`（成功/失败路径均直接转发）。
    #[test]
    fn compile_stub_forwards_to_parse() {
        let via_compile = compile("sub main() { }", "smoke.ecl");
        let via_parse = parse("sub main() { }", "smoke.ecl");
        assert_eq!(via_compile, via_parse);
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
}
