//! storm —— 恢复重演风暴闸（spec L2）：伪随机输入跑全程，沿途多点双源存档（内存快照 +
//! 磁盘字节往返），逐点重演到终点，校验和流逐位对拍。"恢复 == 从未离开"的 CI 性质。

use std::process::ExitCode;
use stg_core::World;
use stg_core::input::{
    BTN_BOMB, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame,
};

pub(crate) fn cmd_storm(rest: &[String]) -> ExitCode {
    let mut frames: u32 = 1200;
    let mut saves: usize = 8;
    let mut seed: u64 = 0x5701;
    let mut i = 0;
    while i < rest.len() {
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--frames", Some(v)) => {
                frames = v.parse().expect("--frames 要 u32");
                i += 2;
            }
            ("--saves", Some(v)) => {
                saves = v.parse().expect("--saves 要 usize");
                i += 2;
            }
            ("--seed", Some(v)) => {
                seed = v.parse().expect("--seed 要 u64");
                i += 2;
            }
            (a, _) => {
                eprintln!("storm: 未知参数 {a}");
                return ExitCode::from(2);
            }
        }
    }
    match run_storm(frames, saves, seed) {
        Ok(()) => {
            eprintln!("storm: {frames} 帧 × {saves} 点 × 双源，全部逐位一致");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("storm: 失败——{msg}");
            ExitCode::from(1)
        }
    }
}

/// 伪随机输入（宿主侧独立 Pcg32，I3）：方向 3 帧一换，射击常按，低速/bomb 低频。
fn gen_inputs(frames: u32, seed: u64) -> Vec<u32> {
    let mut rng = stg_core::rng::Pcg32::new(seed ^ 0x1257_0AB1, 0x9E37_79B9_7F4A_7C15);
    let dirs = [
        0,
        BTN_LEFT,
        BTN_RIGHT,
        BTN_UP,
        BTN_DOWN,
        BTN_LEFT | BTN_UP,
        BTN_RIGHT | BTN_DOWN,
    ];
    let mut out = Vec::with_capacity(frames as usize);
    let mut cur = 0u32;
    for f in 0..frames {
        if f % 3 == 0 {
            cur = dirs[rng.rand_range(dirs.len() as u32) as usize] | BTN_SHOT;
            if rng.rand_range(10) == 0 {
                cur |= BTN_SLOW;
            }
            if rng.rand_range(120) == 0 {
                cur |= BTN_BOMB;
            }
        }
        out.push(cur);
    }
    out
}

/// `World` 无 `Clone`（I7：世界层不给整块复制起花名，只给显式 `copy_into`）——本地小
/// helper：堆零构造一个新 World，再 `copy_into` 灌满，充当"深拷贝"给双源重演的内存腿用。
fn clone_world(src: &World) -> Box<World> {
    let mut dst = World::new(0);
    src.copy_into(&mut dst);
    dst
}

/// 存档点集计算（B14 抽出，独立可测）：均匀分布 `saves` 个存档帧号，**去重**（`frames` 小而
/// `saves` 大时可能算出重复帧号），去重后若为空集则报错——`--saves 0` 不再零存档点、零重演
/// 腿却照样"全部逐位一致"退出 0（正确性工具自身的假绿脚枪）。
fn save_points(frames: u32, saves: usize) -> Result<Vec<u32>, String> {
    let mut save_at: Vec<u32> = (1..=saves as u32)
        .map(|k| k * frames / (saves as u32 + 1))
        .collect();
    save_at.dedup();
    if save_at.is_empty() {
        return Err(format!(
            "saves={saves} 算出空存档点集(零重演腿)——storm 存在意义是逐点重演对拍,\
             saves 至少要 1 且 frames 足够大"
        ));
    }
    Ok(save_at)
}

pub(crate) fn run_storm(frames: u32, saves: usize, seed: u64) -> Result<(), String> {
    let (mut w, image, _boss) = crate::build_rainbow_world(seed);
    let inputs = gen_inputs(frames, seed);
    let save_at = save_points(frames, saves)?;

    // 主跑：逐帧校验和流 + 沿途双源存档
    let mut stream = Vec::with_capacity(frames as usize);
    let mut snaps: Vec<(u32, Box<World>, Vec<u8>)> = Vec::new();
    for f in 0..frames {
        let mut input = InputFrame::empty(f);
        input.actions[0].buttons = inputs[f as usize];
        stg_core::step(&mut w, &stg_core::tables::TABLES_V0, &image, &input);
        stream.push(w.checksum());
        if save_at.contains(&f) {
            let snap = clone_world(&w);
            let bytes = w.save_bytes(&image);
            // 规范自洽：load→save 字节全等 + checksum 等
            let loaded = World::load_bytes(&bytes, &stg_core::tables::TABLES_V0, &image)
                .map_err(|e| format!("帧 {f} 载档失败：{e:?}"))?;
            if loaded.checksum() != w.checksum() {
                return Err(format!("帧 {f}：load 后 checksum 与原世界不符"));
            }
            if loaded.save_bytes(&image) != bytes {
                return Err(format!("帧 {f}：save→load→save 字节不自洽"));
            }
            snaps.push((f, snap, bytes));
        }
    }

    // 逐点重演：内存源 + 磁盘源
    for (f0, snap, bytes) in &snaps {
        let disk = World::load_bytes(bytes, &stg_core::tables::TABLES_V0, &image).unwrap();
        for (label, src) in [("内存快照", clone_world(snap)), ("磁盘往返", disk)] {
            let mut rw = src;
            for f in (*f0 + 1)..frames {
                let mut input = InputFrame::empty(f);
                input.actions[0].buttons = inputs[f as usize];
                stg_core::step(&mut rw, &stg_core::tables::TABLES_V0, &image, &input);
                let expect = stream[f as usize];
                let got = rw.checksum();
                if got != expect {
                    return Err(format!(
                        "{label} 自帧 {f0} 重演，首分歧于帧 {f}：{got:016x} != {expect:016x}"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storm_short_gate() {
        super::run_storm(240, 3, 0xAB).expect("短风暴必须全逐位一致");
    }

    /// 正确性工具自身的假绿脚枪（B14）：`--saves 0` 曾零存档点、零重演腿，
    /// 照样打印"全部逐位一致"退出 0。守卫后必须报错。
    #[test]
    fn storm_rejects_zero_saves_instead_of_vacuous_pass() {
        let err = run_storm(/* frames */ 120, /* saves */ 0, /* seed */ 1)
            .expect_err("saves=0 必须报错,不得空转假绿");
        assert!(
            err.to_string().contains("saves"),
            "错误信息该点出是 saves 参数的问题,实际: {err}"
        );
    }

    /// 去重：`frames` 小而 `saves` 大时会算出重复帧号；去重后非空则正常跑通。
    #[test]
    fn storm_dedups_save_points_when_frames_small_saves_large() {
        let pts = super::save_points(5, 20).expect("frames=5 saves=20 去重后仍非空");
        let mut sorted = pts.clone();
        sorted.dedup();
        assert_eq!(pts, sorted, "save_points 返回值必须已去重");
        assert!(!pts.is_empty());
    }

    /// 变异检验：篡改重演输入一帧必须报分歧——证明对拍真在比，不是恒真（spec §6.4 判别式）。
    /// 内联微型版：跑 60 帧记流，快照于 30，重演时第 40 帧改一位输入，断言 Err 且信息含
    /// "首分歧"。
    #[test]
    fn storm_detects_divergence_by_construction() {
        const FRAMES: u32 = 60;
        const SAVE_AT: u32 = 30;
        const FLIP_AT: u32 = 40;

        let (mut w, image, _boss) = crate::build_rainbow_world(0xAB);
        let inputs = gen_inputs(FRAMES, 0xAB);

        let mut stream = Vec::with_capacity(FRAMES as usize);
        let mut snap: Option<Box<World>> = None;
        for f in 0..FRAMES {
            let mut input = InputFrame::empty(f);
            input.actions[0].buttons = inputs[f as usize];
            stg_core::step(&mut w, &stg_core::tables::TABLES_V0, &image, &input);
            stream.push(w.checksum());
            if f == SAVE_AT {
                snap = Some(clone_world(&w));
            }
        }

        // 从快照重演，但在 FLIP_AT 那帧故意翻转一位输入（与主跑记录不符）。
        let mut rw = snap.expect("SAVE_AT 必已落在 [0, FRAMES) 内");
        let mut divergence: Option<String> = None;
        for f in (SAVE_AT + 1)..FRAMES {
            let mut input = InputFrame::empty(f);
            input.actions[0].buttons = if f == FLIP_AT {
                inputs[f as usize] ^ BTN_LEFT // 篡改一位——与记录流分道扬镳
            } else {
                inputs[f as usize]
            };
            stg_core::step(&mut rw, &stg_core::tables::TABLES_V0, &image, &input);
            let expect = stream[f as usize];
            let got = rw.checksum();
            if got != expect {
                divergence = Some(format!("首分歧于帧 {f}：{got:016x} != {expect:016x}"));
                break;
            }
        }
        let msg = divergence.expect("篡改输入后必须产生分歧——对拍不能是恒真的");
        assert!(msg.contains("首分歧"), "分歧信息应含“首分歧”定位：{msg}");
    }
}
