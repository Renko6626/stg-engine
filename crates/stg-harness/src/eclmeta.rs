//! ecl-meta.json 生成器(编辑体验刀 spec §2)——单一真相源 = builtins::all()。
//! JSON 手写 format!(表序即输出序,确定性字节;无 serde 依赖)。

use std::process::ExitCode;
use stg_ecl_compiler::lang::ast::Ty;
use stg_ecl_compiler::lang::builtins::{Builtin, ParamKind, all};

pub fn meta_path() -> std::path::PathBuf {
    // harness 的 CARGO_MANIFEST_DIR = crates/stg-harness
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../editors/vscode/stg-ecl/ecl-meta.json")
}

fn ty_str(t: Ty) -> &'static str {
    match t {
        Ty::Int => "int",
        Ty::Fx => "fx",
        Ty::Angle => "angle",
    }
}

fn kind_str(k: ParamKind) -> String {
    match k {
        ParamKind::Val(t) => ty_str(t).to_string(),
        ParamKind::XformRef => "xform|none".to_string(),
        ParamKind::SubRef => "sub|none".to_string(),
        ParamKind::RawVal => "int|fx|angle".to_string(),
    }
}

/// `fire(appearance: int, x: fx, ...) -> int` 式签名渲染。
pub fn signature(b: &Builtin) -> String {
    let params: Vec<String> = b
        .params
        .iter()
        .zip(b.param_names.iter())
        .map(|(k, n)| format!("{n}: {}", kind_str(*k)))
        .collect();
    let ret = match b.ret {
        Some(t) => format!(" -> {}", ty_str(t)),
        None => String::new(),
    };
    format!("{}({}){ret}", b.name, params.join(", "))
}

/// JSON 字符串转义：`\`/`"` 两个结构字符 + 全部 `0x00..=0x1F` 控制字符（JSON 规范禁止
/// 字面出现在字符串里）。`\n`/`\r`/`\t` 用短转义，其余控制字符落 `\u{:04x}` 通用形式
/// （修复审裁 Important①：doc/name 里出现换行等控制字符时曾产出非法 JSON）。
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn render_meta_json() -> String {
    let mut out = String::from("{\n  \"version\": 1,\n  \"builtins\": [\n");
    let n = all().len();
    for (i, b) in all().iter().enumerate() {
        let params: Vec<String> = b
            .params
            .iter()
            .zip(b.param_names.iter())
            .map(|(k, name)| {
                format!(
                    "{{\"name\": \"{}\", \"ty\": \"{}\"}}",
                    esc(name),
                    kind_str(*k)
                )
            })
            .collect();
        out.push_str(&format!(
            "    {{\"name\": \"{}\", \"signature\": \"{}\", \"ret\": {}, \"doc\": \"{}\", \"params\": [{}]}}{}\n",
            esc(b.name),
            esc(&signature(b)),
            match b.ret { Some(t) => format!("\"{}\"", ty_str(t)), None => "null".to_string() },
            esc(b.doc),
            params.join(", "),
            if i + 1 == n { "" } else { "," }
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

/// 文档生成段(ecl-lang.md 的 builtin 表)——渲染格式与 `render_meta_json` 同源
/// (`all()`/`signature()`),但产物是给人读的 Markdown 列表而非机器读的 JSON。
pub fn render_doc_segment() -> String {
    let mut out = String::new();
    for b in all() {
        out.push_str(&format!("- `{}` — {}\n", signature(b), b.doc));
    }
    out
}

const DOC_BEGIN: &str = "<!-- gen:builtins:begin -->";
const DOC_END: &str = "<!-- gen:builtins:end -->";

pub fn doc_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ecl-lang.md")
}

/// 幂等替换 ecl-lang.md 生成段;找不到标记 = 错。
///
/// 注(本刀实测修正)：简报给的原始 `format!("{}{}\n{}{}", prefix, "\n", body, suffix)`
/// 在 `"\n"` 实参之外，模板字面量里又硬编码了一个 `\n`——两个换行叠加，产出
/// begin 标记后**两个**空行，与本刀 `committed_doc_segment_matches_generated`
/// 期望的"`\n` + `render_doc_segment()`"（单个换行分隔符）对不上，首次
/// `gen-ecl-meta` 后自身防漂移测试即红（已用真实生成跑过一遍实测确认，非猜测）。
/// 改为单一换行分隔符，语义仍是"幂等替换"，未改变函数签名/调用方式。
pub fn splice_doc() -> Result<(), String> {
    let p = doc_path();
    let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let (Some(b), Some(e)) = (s.find(DOC_BEGIN), s.find(DOC_END)) else {
        return Err("ecl-lang.md 缺生成段标记".into());
    };
    let new = format!(
        "{}\n{}{}",
        &s[..b + DOC_BEGIN.len()],
        render_doc_segment(),
        &s[e..]
    );
    std::fs::write(&p, new).map_err(|e| e.to_string())
}

pub fn cmd_gen() -> ExitCode {
    match std::fs::write(meta_path(), render_meta_json()) {
        Ok(()) => {
            eprintln!("gen-ecl-meta: 写入 {}", meta_path().display());
        }
        Err(e) => {
            eprintln!("gen-ecl-meta 失败: {e}");
            return ExitCode::FAILURE;
        }
    }
    match splice_doc() {
        Ok(()) => {
            eprintln!("gen-ecl-meta: 刷新 {} 生成段", doc_path().display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-ecl-meta: splice_doc 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `esc()` 判别式:反斜杠/引号/三个具名短转义(`\n`/`\r`/`\t`)/一个通用 `\u{:04x}`
    /// 控制字符(0x01,挑一个不属于三条短转义快捷道的值,判别值互异),手算期望输出逐字节比对。
    #[test]
    fn esc_escapes_backslash_quote_and_control_chars_as_valid_json() {
        let input = "a\\b\"c\nd\re\tf\u{01}g";
        let expected = "a\\\\b\\\"c\\nd\\re\\tf\\u0001g";
        assert_eq!(esc(input), expected);
    }

    /// 防漂移(verify-tables 同款):现生成 == commit 字节。
    #[test]
    fn committed_meta_matches_generated() {
        let generated = render_meta_json();
        let committed = std::fs::read_to_string(meta_path()).expect("ecl-meta.json 应已 commit");
        assert_eq!(
            generated, committed,
            "跑 `cargo run -p stg-harness -- gen-ecl-meta` 再 commit"
        );
    }

    /// 文档生成段防漂移:commit 的段内容 == 现渲染。
    #[test]
    fn committed_doc_segment_matches_generated() {
        let s = std::fs::read_to_string(doc_path()).unwrap();
        let b = s.find(DOC_BEGIN).expect("缺 begin 标记") + DOC_BEGIN.len();
        let e = s.find(DOC_END).expect("缺 end 标记");
        assert_eq!(
            s[b..e].trim_end(),
            format!("\n{}", render_doc_segment()).trim_end(),
            "跑 `cargo run -p stg-harness -- gen-ecl-meta` 再 commit(会同步刷新 ecl-lang.md 生成段)"
        );
    }

    /// ecl-lang.md 的每个 ```ecl 围栏例子必须能编译(文档即规格,例子腐烂即红)。
    #[test]
    fn every_ecl_fenced_example_in_doc_compiles() {
        let s = std::fs::read_to_string(doc_path()).unwrap();
        let mut n = 0;
        let mut rest = s.as_str();
        while let Some(start) = rest.find("```ecl\n") {
            let body = &rest[start + 7..];
            let end = body.find("```").expect("未闭合的 ecl 围栏");
            let src = &body[..end];
            if let Err(errors) = stg_ecl_compiler::lang::compile(src, "doc.ecl") {
                let msg: Vec<String> = errors.iter().map(|e| e.render("doc.ecl")).collect();
                panic!("文档例 #{n} 编译失败:\n{src}\n---\n{}", msg.join("\n"));
            }
            n += 1;
            rest = &body[end + 3..];
        }
        assert!(n >= 3, "文档至少应有 3 个可编译示例,实得 {n}");
    }
}
