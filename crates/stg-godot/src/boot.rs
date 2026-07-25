//! 纯 Rust:源码文本 → EclImage → World::new_game(spec §12)。零 gdext 类型。

use stg_core::ecl::image::EclImage;
use stg_core::step::World;
use stg_core::tables::{TABLES_V0, WorldTables};

/// `#[derive(Debug)]` 借 `World` 手写占位 Debug(`finish_non_exhaustive`,见 step.rs)与
/// `EclImage`/`WorldTables` 自身 derive——只为满足测试 `Result::expect`/`{:?}` 打印场景
/// (`boot_at` 失败测试用 `panic!("...{other:?}")`),不逐字段展开真实状态。
#[derive(Debug)]
pub struct Game {
    pub world: Box<World>,
    pub image: EclImage,
    pub tables: &'static WorldTables, // v1 恒 &TABLES_V0(follow-ups A3)
}

#[derive(Debug)]
pub enum BootError {
    /// 编译失败(带行列的渲染消息)。
    Compile(String),
    /// new_game 失败(TaskStartError 转述)。
    Start(String),
}

/// 多单元中段开机完全体(整局流程刀 Task 7 桥面;T1 `compile_units` + T6 `new_game_at`
/// 的桥层组装)。错误逐条经 `CompileError::render(file)` 渲染各自所属文件名后拼接。
pub fn boot_at(
    units: &[(String, String)],
    seed: u64,
    rank: i32,
    start: i32,
    loadout: stg_core::player::Loadout,
) -> Result<Game, BootError> {
    let image = stg_ecl_compiler::lang::compile_units(units).map_err(|errs| {
        let msg: Vec<String> = errs.iter().map(|(file, e)| e.render(file)).collect();
        BootError::Compile(msg.join("\n\n"))
    })?;
    let world = World::new_game_at(seed, rank, start, loadout, &image)
        .map_err(|e| BootError::Start(format!("{e:?}")))?;
    Ok(Game {
        world,
        image,
        tables: &TABLES_V0,
    })
}

/// 单文件从头开机(既有调用方零改)—— 委托 `boot_at` 的单元素/start=0/默认装备特例。
pub fn boot(ecl_source: &str, seed: u64, rank: i32) -> Result<Game, BootError> {
    boot_at(
        &[("bridge.ecl".to_string(), ecl_source.to_string())],
        seed,
        rank,
        0,
        stg_core::player::Loadout::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_ok_and_frame_zero() {
        let g = boot("sub main() { loop { wait(60); } }", 7, 2).expect("boot");
        assert_eq!(g.world.frame(), 0);
        assert_ne!(g.world.checksum(), 0);
    }

    #[test]
    fn boot_compile_error_carries_location() {
        let e = boot("sub main() { 这不是合法脚本 }", 7, 2);
        let Err(BootError::Compile(msg)) = e else {
            panic!("应为编译错误")
        };
        assert!(!msg.is_empty());
    }

    #[test]
    fn boot_at_multi_unit_midstart() {
        let units = vec![
            (
                "main.ecl".to_string(),
                "sub main() { stage1(); mark(2); loop { wait(60); } }\n".to_string(),
            ),
            (
                "s1.ecl".to_string(),
                "sub stage1() { bgm(1); wait(120); }\n".to_string(),
            ),
        ];
        let g = boot_at(
            &units,
            7,
            2,
            2,
            stg_core::player::Loadout {
                power: 400,
                ..Default::default()
            },
        )
        .expect("多单元中段 boot");
        assert_eq!(g.world.body.view().players()[0].power, 400);
    }

    #[test]
    fn boot_at_compile_error_names_offending_file() {
        let units = vec![
            ("main.ecl".to_string(), "sub main() { }\n".to_string()),
            (
                "bad.ecl".to_string(),
                "sub b() {\n  wait(1)\n}\n".to_string(),
            ),
        ];
        match boot_at(&units, 7, 2, 0, Default::default()) {
            Err(BootError::Compile(msg)) => assert!(msg.contains("bad.ecl"), "{msg}"),
            other => panic!("期望 Compile 错:{other:?}"),
        }
    }
}
