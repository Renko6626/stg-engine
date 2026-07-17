//! 道具池（D7）——`define_pool!` 第五实例 + 类型常量 + 配置表 v0 + 掉落表。
//! 行为世界侧固定（账本公平性同级，D7）；**扩展性四步清单**（spec 2026-07-16）：
//! 新增类型 = ①加类型常量（编号只增不改）②`ITEM_CFG` 加行（数组类型强制表长==类型数）
//! ③`credit_item`（world/settle.rs）加 match 臂 ④按需加掉落表行。
//! 配置表 v0 = 引擎常量（player.rs 先例）；WorldTables 建成后整表搬家、结构体不动。

use crate::define_pool;
use crate::math::Fx;

// ── 类型编号（冻结；只增不改，钉死测试押运）─────────────────────────
pub const ITEM_POWER: u8 = 0;
pub const ITEM_POINT: u8 = 1;
pub const ITEM_LIFE_PIECE: u8 = 2;
pub const ITEM_BOMB_PIECE: u8 = 3;
pub const ITEM_TYPE_COUNT: usize = 4;

/// `magnet_to` 哨兵：未锁定 / 已拾取（等待 cleanup 回收）。0..MAX_PLAYERS = 锁定目标。
pub const MAGNET_NONE: u8 = 0xFF;
pub const MAGNET_PICKED: u8 = 0xFE;

// ── 入账常数（账本规则世界侧固定，D7/D9）────────────────────────────
pub const POWER_MAX: u16 = 128;
pub const PIECES_PER_LIFE: u8 = 5;
pub const PIECES_PER_BOMB: u8 = 5;

/// 全局重力（≈0.15 px/帧²；未锁定道具 vy += 至终速钉住）。
pub(crate) const ITEM_GRAVITY: Fx = Fx::from_raw(9_830);

/// 每类型配置（v0 引擎常量；金向量实测后调参）。
pub struct ItemTypeCfg {
    pub score: u32,
    pub eject_speed: Fx,
    pub terminal_vy: Fx,
    pub magnet_speed: Fx,
    pub pickup_radius: Fx,
    pub attract_radius: Fx,
}

const STD: ItemTypeCfg = ItemTypeCfg {
    score: 0, // 各行覆写
    eject_speed: Fx::from_int(3),
    terminal_vy: Fx::from_raw(144_179), // ≈2.2
    magnet_speed: Fx::from_int(8),
    pickup_radius: Fx::from_int(16),
    attract_radius: Fx::from_int(40),
};

/// 索引 = 类型编号；数组类型使"表长 == 类型数"成为编译期事实。
pub(crate) const ITEM_CFG: [ItemTypeCfg; ITEM_TYPE_COUNT] = [
    ItemTypeCfg { score: 10, ..STD },  // POWER
    ItemTypeCfg { score: 100, ..STD }, // POINT
    ItemTypeCfg { score: 50, ..STD },  // LIFE_PIECE
    ItemTypeCfg { score: 50, ..STD },  // BOMB_PIECE
];

/// 掉落表 v0：表 id → [(类型, 数量)]。表 0 = 空（enemy.drop_table 零默认 = 不掉）。
pub(crate) const DROP_TABLES: &[&[(u8, u8)]] = &[
    &[],
    &[(ITEM_POWER, 2), (ITEM_POINT, 1)], // 表 1：标准杂鱼
];

define_pool! {
    Item, cap = 512,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        item_type: u8, magnet_to: u8, timer: u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;
    use crate::math::Fx;

    /// 类型编号冻结（快照/回放契约；改动=有意识确认）。
    #[test]
    fn item_type_numbering_frozen() {
        assert_eq!(
            (ITEM_POWER, ITEM_POINT, ITEM_LIFE_PIECE, ITEM_BOMB_PIECE),
            (0, 1, 2, 3)
        );
        assert_eq!(ITEM_TYPE_COUNT, 4);
        assert_eq!((MAGNET_NONE, MAGNET_PICKED), (0xFF, 0xFE));
    }

    /// 配置表逐行健全：分值/物理参数为正、拾取半径 ≤ MAX_ENTITY_RADIUS（行 5 加法证明前提）。
    #[test]
    fn item_cfg_rows_sane() {
        for (t, cfg) in ITEM_CFG.iter().enumerate() {
            assert!(cfg.score > 0, "type {t}");
            assert!(cfg.eject_speed.raw() > 0 && cfg.terminal_vy.raw() > 0);
            assert!(cfg.magnet_speed.raw() > 0 && cfg.attract_radius.raw() > 0);
            assert!(
                cfg.pickup_radius.raw() > 0
                    && cfg.pickup_radius.raw() <= crate::world::MAX_ENTITY_RADIUS.raw(),
                "type {t} 拾取半径越出行 5 加法安全域"
            );
        }
        assert_eq!(ITEM_CFG[ITEM_POINT as usize].score, 100);
    }

    /// 掉落表：表 0 恒空（enemy.drop_table 零初始化默认 = 不掉）；表 1 = 标准杂鱼。
    #[test]
    fn drop_tables_shape() {
        assert!(DROP_TABLES[0].is_empty());
        assert_eq!(DROP_TABLES[1], &[(ITEM_POWER, 2), (ITEM_POINT, 1)]);
        assert!(
            DROP_TABLES
                .iter()
                .flat_map(|t| t.iter())
                .all(|&(ty, _)| (ty as usize) < ITEM_TYPE_COUNT),
            "掉落表条目类型必须合法——扩展四步第④步的脚下网"
        );
    }

    /// 池确定性 + 校验和敏感（与其余池同款纪律）。
    #[test]
    fn item_pool_deterministic_and_checksum_sensitive() {
        let mut w = crate::step::World::new(1);
        let base = w.body.items.checksum();
        let h = w
            .body
            .items
            .alloc(ItemInit {
                x: Fx::ZERO,
                y: Fx::ZERO,
                vx: Fx::ZERO,
                vy: Fx::ZERO,
                item_type: ITEM_POWER,
                magnet_to: MAGNET_NONE,
                timer: 0,
            })
            .unwrap();
        assert_ne!(w.body.items.checksum(), base);
        let i = w.body.items.get(h).unwrap();
        assert_eq!(w.body.items.magnet_to[i], MAGNET_NONE);
    }
}
