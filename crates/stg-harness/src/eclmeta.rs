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

pub fn cmd_gen() -> ExitCode {
    match std::fs::write(meta_path(), render_meta_json()) {
        Ok(()) => {
            eprintln!("gen-ecl-meta: 写入 {}", meta_path().display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-ecl-meta 失败: {e}");
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
}
