//! replay —— 重放一份 `InputLog`（`.stgr`）并打出校验和（时间机制内核刀，2026-09-07）。
//!
//! `stg-harness replay <log.stgr> --ecl <file.ecl|目录> [--every N]`：
//! 编译脚本 → 头校验（引擎版 / 词表 / 表 / 镜像 / 整体 FNV）→ `Timeline::replay` 从头重走
//! → 每 N 帧打一行校验和 + 末帧 + cuts 表。**任何不符退非零码**——它是 storm 闸的时间线版：
//! 桥的 `replay_bytes()` 倒出来的文件在这里必须能原样重现。
//!
//! 与 `run` 的分工：`run` 观测脚本干了什么（输入恒空）；`replay` 观测**玩家 + 脚本**一起干了
//! 什么（输入来自 log），包括跳躍与遡行的落地。

use std::process::ExitCode;

use stg_core::timeline::{InputLog, ReplayError, Timeline};

pub(crate) fn cmd_replay(rest: &[String]) -> ExitCode {
    ExitCode::from(replay_cli(rest))
}

pub(crate) fn replay_cli(rest: &[String]) -> u8 {
    let Some(file) = rest.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: stg-harness replay <log.stgr> --ecl <file.ecl|目录> [--every N]");
        return 2;
    };
    let mut ecl: Option<String> = None;
    let mut every: u32 = 60;
    let mut i = 1;
    while i < rest.len() {
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--ecl", Some(v)) => {
                ecl = Some(v.clone());
                i += 2;
            }
            ("--every", Some(v)) => {
                every = match v.parse::<u32>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        eprintln!("replay: --every 要正整数，收到 {v}");
                        return 2;
                    }
                };
                i += 2;
            }
            (other, _) => {
                eprintln!("replay: 未知参数 {other}");
                return 2;
            }
        }
    }
    let Some(ecl) = ecl else {
        eprintln!("replay: 缺 --ecl（log 不含脚本源码，只记镜像哈希）");
        return 2;
    };
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("replay: 读 {file} 失败: {e}");
            return 1;
        }
    };
    let (_, image, _) = match crate::run::build_ecl_world(&ecl, 0, 0) {
        Ok(x) => x,
        Err(msg) => {
            eprintln!("{msg}");
            return 1;
        }
    };
    let tables_hash = stg_core::tables::TABLES_V0.content_hash;
    let log = match InputLog::from_bytes(&bytes, tables_hash, image.content_hash()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("replay: 头校验失败: {}", render_err(&e));
            return 1;
        }
    };
    println!(
        "replay: boot={:?} frames={} cuts={}",
        log.boot,
        log.frames.len(),
        log.cuts.len()
    );
    match replay_stream(&log, image, every) {
        Ok(rows) => {
            for (f, sum) in rows {
                println!("{f:>6}  {sum:016x}");
            }
            for c in &log.cuts {
                println!(
                    "cut: 第 {} 帧请求 → 落点 {}（自机 {}）",
                    c.at, c.to, c.player
                );
            }
            0
        }
        Err(e) => {
            eprintln!("replay: 重放失败: {}", render_err(&e));
            1
        }
    }
}

/// 逐帧重放并按 `every` 采样校验和（末帧必含）——`Timeline::replay_with` 的观察闭包，
/// 与 `Timeline::replay` 是同一条循环。同帧多次观察（落地前/后）以最后一次为准。
pub(crate) fn replay_stream(
    log: &InputLog,
    image: stg_core::ecl::image::EclImage,
    every: u32,
) -> Result<Vec<(u32, u64)>, ReplayError> {
    let total = log.frames.len() as u32;
    let mut rows: Vec<(u32, u64)> = Vec::new();
    let t = Timeline::replay_with(log, image, |w| {
        let f = w.frame();
        if f % every == 0 || f == total {
            match rows.last_mut() {
                Some(last) if last.0 == f => last.1 = w.checksum(),
                _ => rows.push((f, w.checksum())),
            }
        }
    })?;
    debug_assert_eq!(rows.last().map(|r| r.1), Some(t.world().checksum()));
    Ok(rows)
}

fn render_err(e: &ReplayError) -> String {
    match e {
        ReplayError::BootNotReplayable => "log 出身是存档（Boot::Snapshot），不可从头重放".into(),
        ReplayError::EngineVerMismatch { file, engine } => {
            format!("引擎版不符：文件 {file}，本引擎 {engine}")
        }
        ReplayError::VocabMismatch { .. } => "输入词表指纹不符（动作位语义变了）".into(),
        ReplayError::ImageMismatch { .. } => "镜像哈希不符：--ecl 给的不是录制时那份脚本".into(),
        ReplayError::TablesMismatch { .. } => "表哈希不符".into(),
        ReplayError::HashMismatch { .. } => "文件整体 FNV 不符（损坏/被改）".into(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::input::{
        BTN_DOWN, BTN_JUMP, BTN_LEFT, BTN_REWIND, BTN_RIGHT, BTN_UP, InputFrame,
    };
    use stg_core::player::{LIFE_ALIVE, LIFE_DEATHWINDOW, Loadout};
    use stg_core::timeline::Boot;

    const DEMO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../godot/ecl/demo");

    /// 真 ECL 整局（demo 目录）跑进 `Timeline`：随机走位 + 周期跳躍 + 进决死窗口就遡行，
    /// 录 log → `Timeline::replay` 末态逐位同 + 字节往返全等 + 采样流两条路径对拍。
    /// 这是 storm 闸的时间线版：任务池随快照往返、遡行落在 ECL 演出中途，都在这一条里。
    #[test]
    fn demo_run_replays_bitwise_through_bytes() {
        let (_, image, _) = crate::run::build_ecl_world(DEMO, 5, 2).expect("demo 编译");
        let ld = Loadout::default();
        let mut live = Timeline::new_game_at(5, 2, 0, ld, image.clone()).unwrap();
        let mut rng = stg_core::rng::Pcg32::new(5, 11);
        let mut rewinds = 0;
        let mut jumps = 0;
        for _ in 0..900 {
            let st = live.world().view().players()[0].life_state;
            let mut b = match rng.rand_range(4) {
                0 => BTN_LEFT,
                1 => BTN_RIGHT,
                2 => BTN_UP,
                _ => BTN_DOWN,
            };
            if st == LIFE_DEATHWINDOW {
                b = BTN_REWIND;
            } else if st == LIFE_ALIVE && live.frame() % 120 == 60 {
                b = BTN_JUMP;
                jumps += 1;
            }
            let mut f = InputFrame::empty(live.frame());
            f.actions[0].buttons = b;
            if live.advance(&f).rewound.is_some() {
                rewinds += 1;
            }
        }
        assert!(jumps >= 3, "demo 局里必须跳过几次（实测 {jumps}）");
        eprintln!(
            "demo replay gate: frames={} jumps={jumps} rewinds={rewinds}",
            live.frame()
        );

        let bytes = live.log_bytes();
        let log = InputLog::from_bytes(
            &bytes,
            stg_core::tables::TABLES_V0.content_hash,
            image.content_hash(),
        )
        .unwrap();
        assert_eq!(&log, live.log());
        assert_eq!(log.cuts.len(), rewinds.min(log.cuts.len()));
        let rep = Timeline::replay(&log, image.clone()).unwrap();
        assert_eq!(
            rep.world().checksum(),
            live.world().checksum(),
            "末态逐位同"
        );
        let rows = replay_stream(&log, image, 100).unwrap();
        assert_eq!(rows.last().map(|r| r.1), Some(live.world().checksum()));
        assert!(matches!(
            log.boot,
            Boot::NewGameAt {
                seed: 5,
                rank: 2,
                ..
            }
        ));
    }

    /// CLI 判别：缺 --ecl / 坏文件各退非零，正常路径退 0。
    #[test]
    fn replay_cli_exit_codes() {
        let (_, image, _) = crate::run::build_ecl_world(DEMO, 1, 2).unwrap();
        let mut t = Timeline::new_game_at(1, 2, 0, Loadout::default(), image).unwrap();
        for _ in 0..30 {
            t.advance(&InputFrame::empty(t.frame()));
        }
        let dir = std::env::temp_dir().join(format!("stg-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.stgr");
        std::fs::write(&path, t.log_bytes()).unwrap();
        let p = path.to_string_lossy().to_string();
        assert_eq!(replay_cli(std::slice::from_ref(&p)), 2, "缺 --ecl");
        assert_eq!(
            replay_cli(&[p.clone(), "--ecl".into(), DEMO.into()]),
            0,
            "正常路径"
        );
        // 注意：log 头里的 image_hash 是 `EclImage.content_hash` = 表 coherence 哈希，**不是脚本
        // 身份**——换一份脚本、同一张表，头照样过（存档头同款局限）。脚本身份哈希是 follow-up。
        let mut bad = t.log_bytes();
        bad[20] ^= 1;
        let bp = dir.join("bad.stgr");
        std::fs::write(&bp, bad).unwrap();
        assert_eq!(
            replay_cli(&[
                bp.to_string_lossy().to_string(),
                "--ecl".into(),
                DEMO.into()
            ]),
            1,
            "FNV 不符"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
