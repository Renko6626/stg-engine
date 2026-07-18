//! 道具池（D7）——`define_pool!` 第五实例 + 类型常量 + 池入账常数。
//! 行为世界侧固定（账本公平性同级，D7）；**扩展性四步清单**（spec 2026-07-16）：
//! 新增类型 = ①加类型常量（编号只增不改）②`tables::TABLES_V0.item_cfg` 加行
//! （数组类型强制表长==类型数）③`credit_item`（world/settle.rs）加 match 臂
//! ④按需加掉落表行。
//!
//! **M0-17 T2 起**：`ItemTypeCfg` 结构体定义 + `ITEM_CFG`/`DROP_TABLES`/`ITEM_GRAVITY`
//! 三件内容已搬去 `crate::tables`（`WorldTables` 全家入驻，见 `tables.rs` 模块文档）。
//! 本文件只留类型编号（冻结）/池定义/哨兵/账本规则常数（`POWER_MAX` 等——世界规则不是
//! 内容数据，不迁）。

use crate::define_pool;
use crate::math::Fx;

// ── 类型编号（冻结；只增不改，钉死测试押运）─────────────────────────
pub const ITEM_POWER: u8 = 0;
pub const ITEM_POINT: u8 = 1;
pub const ITEM_LIFE_PIECE: u8 = 2;
pub const ITEM_BOMB_PIECE: u8 = 3;
/// 小星星（M0-15）：**只由消弹转化产生**（D9 趟一，不进掉落表），出生即磁吸，少量分数。
pub const ITEM_STAR: u8 = 4;
pub const ITEM_TYPE_COUNT: usize = 5;

/// `magnet_to` 哨兵：未锁定 / 已拾取（等待 cleanup 回收）。0..MAX_PLAYERS = 锁定目标。
pub const MAGNET_NONE: u8 = 0xFF;
pub const MAGNET_PICKED: u8 = 0xFE;

// ── 入账常数（账本规则世界侧固定，D7/D9）────────────────────────────
/// 火力上限（M0-16 定标：**1 单位 = 0.01 火力**，显示域 0.00-4.00，满 = 4.00）。
/// 一颗 `ITEM_POWER` = +1 单位（+0.01）；整数档位（0..=4，换弹幕形态用）见
/// `PlayerState::power_tier`。世界侧只存整数单位，除以 100 是表现层的事（I1）。
pub const POWER_MAX: u16 = 400;
pub const PIECES_PER_LIFE: u8 = 5;
pub const PIECES_PER_BOMB: u8 = 5;

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
            (
                ITEM_POWER,
                ITEM_POINT,
                ITEM_LIFE_PIECE,
                ITEM_BOMB_PIECE,
                ITEM_STAR
            ),
            (0, 1, 2, 3, 4)
        );
        assert_eq!(ITEM_TYPE_COUNT, 5);
        assert_eq!((MAGNET_NONE, MAGNET_PICKED), (0xFF, 0xFE));
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
