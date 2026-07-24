//! 纯 Rust:L1 存读的桥侧管道。换弹夹式载入 = 天然原子(失败不动旧世界)。

use crate::boot::Game;
use stg_core::step::World;

pub fn save(game: &Game) -> Vec<u8> {
    game.world.save_bytes(&game.image)
}

pub fn load_into(game: &mut Game, bytes: &[u8]) -> Result<(), String> {
    match World::load_bytes(bytes, game.tables, &game.image) {
        Ok(w) => {
            game.world = w;
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
        let c0 = g.world.checksum();
        load_into(&mut g, &bytes).expect("load");
        assert_eq!(g.world.checksum(), c0);
    }

    #[test]
    fn load_bad_bytes_keeps_world_intact() {
        let mut g = crate::boot::boot("sub main() { loop { wait(60); } }", 7, 2).unwrap();
        let c0 = g.world.checksum();
        let mut bad = save(&g);
        bad[0] ^= 0xFF; // 毁 magic
        assert!(load_into(&mut g, &bad).is_err());
        assert_eq!(g.world.checksum(), c0, "失败不动世界");
    }
}
