//! stg-harness —— 金向量对拍 + 烘焙表工具的命令行入口。
//!
//! 子命令：
//!   golden [--out FILE]   跑金向量，逐帧输出校验和（CI 跨平台对拍的数据源）
//!   bake-tables           用 f64 生成 sin/cos/easing 烘焙表原始字节（M0 落地）
//!   verify-tables         断言现生成的表字节 == 已 commit 的字节（CI 防漂移）
//!
//! 本 crate 在断层线【以上】，可用浮点；stg-core 只消费 commit 的表字节。

use std::process::ExitCode;

use stg_core::checksum::Fnv1a64;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("golden") => cmd_golden(&args[2..]),
        Some("bake-tables") => cmd_bake_tables(),
        Some("verify-tables") => cmd_verify_tables(),
        _ => {
            eprintln!("usage: stg-harness <golden [--out FILE] | bake-tables | verify-tables>");
            ExitCode::FAILURE
        }
    }
}

/// 从 `rest` 中解析 `--out FILE`。
fn parse_out(rest: &[String]) -> Option<String> {
    rest.iter()
        .position(|a| a == "--out")
        .and_then(|i| rest.get(i + 1).cloned())
}

/// 金向量 —— 逐帧校验和。
///
/// **Phase 1 脚手架版**：世界本体尚未落地，此处用一段确定性整数序列喂入 vendored
/// FNV-1a，端到端验证"三平台构建 → 产出校验和工件 → 逐字节对拍"这条 DoD 流水线可用。
/// **M0 起**替换为真实的整数世界模拟：`world = step(world, ecl, input)` 逐帧推进，
/// 对全部实体池做字段级校验和。届时这条流水线才真正证明跨平台 bit 级一致。
fn cmd_golden(rest: &[String]) -> ExitCode {
    const FRAMES: u32 = 600; // 10 秒 @ 60Hz

    let mut lines = String::new();
    let mut acc = Fnv1a64::new();
    for frame in 0..FRAMES {
        // 占位的确定性演化：真实版本这里是 step()。
        acc.write_u32(frame);
        acc.write_u32(frame.wrapping_mul(2_654_435_761)); // Knuth 乘法散列，纯整数
        lines.push_str(&format!("{frame} {:016x}\n", acc.finish()));
    }

    match parse_out(rest) {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, lines) {
                eprintln!("error: 写入 {path} 失败: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("golden: {FRAMES} 帧校验和已写入 {path}");
        }
        None => print!("{lines}"),
    }
    ExitCode::SUCCESS
}

/// 烘焙 sin/cos/easing 表 —— M0 落地（用 f64 生成 → commit 原始字节到 stg-core）。
fn cmd_bake_tables() -> ExitCode {
    eprintln!("bake-tables: 尚无表可烘焙（数学核烘焙表于 M0 落地）");
    ExitCode::SUCCESS
}

/// 断言现生成的表字节 == 已 commit 的字节（§2.1 CI 防漂移）—— M0 落地。
fn cmd_verify_tables() -> ExitCode {
    eprintln!("verify-tables: 尚无 commit 的烘焙表（数学核于 M0 落地）—— 空验证通过");
    ExitCode::SUCCESS
}
