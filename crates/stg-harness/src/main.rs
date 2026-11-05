//! stg-harness —— 金向量对拍 + 烘焙表工具的命令行入口。
//!
//! 子命令：
//!   golden [--out FILE]   跑金向量，逐帧输出校验和（CI 跨平台对拍的数据源）
//!   bake-tables           用 f64 生成 sin/cos/easing 烘焙表原始字节（M0 落地）
//!   verify-tables         断言现生成的表字节 == 已 commit 的字节（CI 防漂移）
//!   serve [--port 8611] [--seed 1]  起 WebSocket 查看器
//!   check <file.ecl|目录>  只编译不跑，渲染诊断（人/agent/CI 共用的最短反馈环；目录 =
//!                         多文件编译单元，走 compile_units）
//!   run <file.ecl|目录>   跑任意 .ecl 并输出可断言的事实（计数/峰值/末帧 + --at 单帧弹表）；
//!                         **有 task fault 即退非零码**（写完 .ecl 之后的观测出口）
//!   gen-ecl-meta          生成 editors/vscode/stg-ecl/ecl-meta.json + 刷新 docs/ecl-lang/7-reference.md
//!                       生成段（单一真相源=builtins::all()，两个 sink 同一次生成）
//!   rl-bench [--envs N] [--steps S] [--threads T] [--warmup W] [--cap C] [--workload default|dense] [--mark M] [--rank R] [--density K] [--profile]  stg-rl VecEnv 吞吐 / 单线程开销分解（spec §7）
//!                         （--threads T 夹到 min(T, envs)，缺省自动 1,2,4,…）
//!
//! 本 crate 在断层线【以上】，可用浮点；stg-core 只消费 commit 的表字节。

use std::process::ExitCode;

mod bench;
mod eclmeta;
mod golden;
mod replay;
mod rlbench;
mod run;
mod scenes;
mod storm;
mod tables;
mod viewer;
mod xformbench;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("golden") => golden::cmd_golden(&args[2..]),
        Some("bench") => bench::cmd_bench(&args[2..]),
        Some("bake-tables") => cmd_bake_tables(),
        Some("verify-tables") => cmd_verify_tables(),
        Some("serve") => viewer::cmd_serve(&args[2..]),
        Some("dump") => viewer::cmd_dump(&args[2..]),
        Some("storm") => storm::cmd_storm(&args[2..]),
        Some("check") => cmd_check(&args[2..]),
        Some("run") => run::cmd_run(&args[2..]),
        Some("replay") => replay::cmd_replay(&args[2..]),
        Some("rl-bench") => rlbench::cmd_rl_bench(&args[2..]),
        Some("gen-ecl-meta") => eclmeta::cmd_gen(),
        _ => {
            eprintln!(
                "usage: stg-harness <golden [--out FILE] | bench [--xform-expr] [--frames N] | bake-tables | verify-tables | serve [--port 8611] [--seed 1] [--ecl PATH] [--rank R] | dump --out FILE [--frames N] [--seed S] [--ecl PATH] | storm [--frames N] [--saves K] [--seed S] | check <file.ecl|目录> | run <file.ecl|目录> [--frames N] [--seed S] [--rank R] [--at F] | replay <log.stgr> --ecl <file.ecl|目录> [--every N] | rl-bench [--envs N] [--steps S] [--threads T (夹到 min(T,envs))] [--warmup W] [--cap C] [--workload default|dense] [--mark M] [--rank R] [--density K(每帧发弹数,dense)] [--profile] | gen-ecl-meta>"
            );
            ExitCode::FAILURE
        }
    }
}

/// 烘焙 sin/cos/easing 表 —— 用 f64 生成原始字节并写入 stg-core 源目录（§2.1）。
fn cmd_bake_tables() -> ExitCode {
    match tables::bake_all() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bake-tables 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 断言现生成的表字节 == 已 commit 的字节（§2.1 CI 防漂移）。
fn cmd_verify_tables() -> ExitCode {
    match tables::verify_all() {
        Ok(()) => {
            eprintln!("verify-tables: 全部表与 commit 字节一致 ✔");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("verify-tables 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 目录 → 全部 *.ecl 按文件名字节序;单文件 → 单元素。整局流程刀 spec §1 收集约定。
/// `check` 与 `run`/`serve --ecl`/`dump --ecl` 共用同一条路径处理（run 刀 2026-08-01）。
pub(crate) fn collect_units(path: &str) -> std::io::Result<Vec<(String, String)>> {
    let meta = std::fs::metadata(path)?;
    if meta.is_dir() {
        let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "ecl"))
            .collect();
        names.sort(); // 路径字节序 = 文件名字节序(同目录)
        names
            .into_iter()
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                std::fs::read_to_string(&p).map(|s| (name, s))
            })
            .collect()
    } else {
        std::fs::read_to_string(path).map(|s| vec![(path.to_string(), s)])
    }
}

/// 只编译不跑(编辑体验刀 spec §4):人 / 合作者 agent / CI 共用的最短诊断环。
/// 支持单文件与目录(整局流程刀 spec §1:目录 = 多文件编译单元,走 `compile_units`)。
fn cmd_check(rest: &[String]) -> ExitCode {
    let Some(path) = rest.first() else {
        eprintln!("usage: stg-harness check <file.ecl|目录>");
        return ExitCode::from(2);
    };
    let units = match collect_units(path) {
        Ok(u) if !u.is_empty() => u,
        Ok(_) => {
            eprintln!("error: {path} 目录下没有 .ecl 文件");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: 读 {path} 失败: {e}");
            return ExitCode::from(2);
        }
    };
    match stg_ecl_compiler::lang::compile_units(&units) {
        Ok(_) => {
            println!("OK");
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for (file, e) in &errors {
                eprintln!("{}", e.render(file));
            }
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests;
