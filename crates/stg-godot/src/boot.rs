//! 纯 Rust:源码文本 → EclImage → World::new_game(spec §12)。零 gdext 类型。

use stg_core::ecl::image::EclImage;
use stg_core::step::World;
use stg_core::tables::{TABLES_V0, WorldTables};

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

pub fn boot(ecl_source: &str, seed: u64, rank: i32) -> Result<Game, BootError> {
    let image = stg_ecl_compiler::lang::compile(ecl_source, "bridge.ecl").map_err(|errs| {
        let msg: Vec<String> = errs.iter().map(|e| e.render("bridge.ecl")).collect();
        BootError::Compile(msg.join("\n\n"))
    })?;
    let world =
        World::new_game(seed, rank, &image).map_err(|e| BootError::Start(format!("{e:?}")))?;
    Ok(Game {
        world,
        image,
        tables: &TABLES_V0,
    })
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
}
