//! 纯 Rust:L1 存读的桥侧管道。换弹夹式载入 = 天然原子(失败不动旧世界)。
//! 时间机制内核刀:读档后 timeline 整条重建(环清空、log 重开、出身记 `Boot::Snapshot`——
//! 这样的 log 只能落盘不能从头重放,练习模式再把快照嵌进去,follow-ups)。

use crate::boot::Game;
use stg_core::step::World;
use stg_core::timeline::{Boot, Timeline};

pub fn save(game: &Game) -> Vec<u8> {
    game.world().save_bytes(game.image())
}

pub fn load_into(game: &mut Game, bytes: &[u8]) -> Result<(), String> {
    match World::load_bytes(bytes, game.tables, game.image()) {
        Ok(w) => {
            let boot = Boot::Snapshot {
                world_checksum: w.checksum(),
            };
            game.timeline = Timeline::from_world(w, game.image().clone(), boot);
            Ok(())
        }
        Err(e) => Err(format!("{e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip_checksum() {
        let mut g = crate::boot::boot("sub main() { loop { wait(60); } }", 7, 2).unwrap();
        let bytes = save(&g);
        let c0 = g.world().checksum();
        load_into(&mut g, &bytes).expect("load");
        assert_eq!(g.world().checksum(), c0);
        assert!(
            matches!(g.timeline.log().boot, Boot::Snapshot { world_checksum } if world_checksum == c0),
            "读档后 log 出身记为快照"
        );
        assert_eq!(
            g.timeline.ring().oldest(),
            Some(g.world().frame()),
            "环首帧 = 载入帧"
        );
    }

    #[test]
    fn load_bad_bytes_keeps_world_intact() {
        let mut g = crate::boot::boot("sub main() { loop { wait(60); } }", 7, 2).unwrap();
        let c0 = g.world().checksum();
        let mut bad = save(&g);
        bad[0] ^= 0xFF; // 毁 magic
        assert!(load_into(&mut g, &bad).is_err());
        assert_eq!(g.world().checksum(), c0, "失败不动世界");
    }
}
