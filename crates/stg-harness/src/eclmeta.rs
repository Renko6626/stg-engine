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

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
