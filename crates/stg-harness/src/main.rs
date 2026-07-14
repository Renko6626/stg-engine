//! stg-harness —— 金向量对拍 + 烘焙表工具的命令行入口。
//!
//! 子命令：
//!   golden [--out FILE]   跑金向量，逐帧输出校验和（CI 跨平台对拍的数据源）
//!   bake-tables           用 f64 生成 sin/cos/easing 烘焙表原始字节（M0 落地）
//!   verify-tables         断言现生成的表字节 == 已 commit 的字节（CI 防漂移）
//!
//! 本 crate 在断层线【以上】，可用浮点；stg-core 只消费 commit 的表字节。

use std::process::ExitCode;

mod tables;

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

/// 金向量 —— 真实 step 演化的纯弹幕场景，逐帧 World 校验和（CI 跨平台对拍的数据源）。
///
/// 导演每帧从中心铺一圈 12 发（基角随帧旋转 + rng 抖动 → 压 sincos + PCG32），弹积分，
/// 越界/寿命尽经 cleanup 回收（压掩码分配器 churn）。600 帧逐帧 World checksum 三平台逐点对拍。
fn cmd_golden(rest: &[String]) -> ExitCode {
    use stg_core::bullets::BulletInit;
    use stg_core::math::{Angle, Fx, polar_to_vec};
    use stg_core::step::{World, step_with_director};

    const FRAMES: u32 = 600; // 10 秒 @ 60Hz
    const SEED: u64 = 0x5147_4f4c_4445_4e00; // "GOLDEN"
    let mut world = World::new(SEED);
    let mut lines = String::new();

    for frame in 0..FRAMES {
        step_with_director(&mut world, |b| {
            let base = (frame.wrapping_mul(797) & 0xFFFF) as u16; // 基角随帧旋转
            let n: u16 = 12;
            let astep = (65536u32 / n as u32) as u16; // 每发角步（避 65536 溢 u16）
            for k in 0..n {
                let spread = b.rng.rand_range(512) as u16; // 抖动，消耗 RNG
                let a = Angle(
                    base.wrapping_add(k.wrapping_mul(astep))
                        .wrapping_add(spread),
                );
                let (vx, vy) = polar_to_vec(Fx::from_int(3), a); // 压 sincos
                let bi = BulletInit {
                    x: Fx::ZERO,
                    y: Fx::from_int(100),
                    vx,
                    vy,
                    speed: Fx::from_int(3),
                    angle: a,
                    ang_vel: 0,
                    accel: Fx::ZERO,
                    ax: Fx::ZERO,
                    ay: Fx::ZERO,
                    sprite: 0,
                    radius: Fx::from_int(2),
                    delay: 0,
                    life: 200,
                    flags: 0,
                    grazed_by: 0,
                    transform_head: 0xFFFF,
                    xform_wait: 0,
                    xform_next: 0,
                };
                b.create_bullet(bi); // 池满时 P4-a 静默降级（计数入校验和）
            }
        });
        lines.push_str(&format!("{frame} {:016x}\n", world.checksum()));
    }

    match parse_out(rest) {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, lines) {
                eprintln!("error: 写入 {path} 失败: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("golden: {FRAMES} 帧真实 step 演化校验和已写入 {path}");
        }
        None => print!("{lines}"),
    }
    ExitCode::SUCCESS
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
