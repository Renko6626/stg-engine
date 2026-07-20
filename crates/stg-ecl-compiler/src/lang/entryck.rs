use super::ast::{CompileError, Program, Span};

fn err(span: Span, msg: impl Into<String>) -> CompileError {
    CompileError {
        line: span.line,
        col: span.col,
        msg: msg.into(),
        src_line: String::new(),
    }
}

pub(super) fn check(prog: &Program) -> Result<(), Vec<CompileError>> {
    let mains: Vec<_> = prog.subs.iter().filter(|sub| sub.name == "main").collect();
    if mains.is_empty() {
        let span = prog
            .subs
            .first()
            .map_or(Span { line: 1, col: 1 }, |sub| sub.span);
        return Err(vec![err(span, "缺少唯一根入口 'sub main()'")]);
    }
    if mains.len() != 1 {
        return Err(vec![err(mains[1].span, "sub 名称 'main' 重复")]);
    }
    let main = mains[0];
    let mut errors = Vec::new();
    if main.is_async {
        errors.push(err(main.span, "main 不能声明为 async；请写 'sub main()'"));
    }
    if !main.params.is_empty() {
        errors.push(err(main.span, "main 必须是零参数根入口"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
