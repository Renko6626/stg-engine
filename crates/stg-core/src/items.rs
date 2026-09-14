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
/// 停止碎片进位数（玩法刀 2026-09-14：5 → 4，gameplay-design §1）。
pub const PIECES_PER_BOMB: u8 = 4;

// cap 512 → 1024（F12 定案，2026-09-03）：**消弹转星星是 1:1**（`world/settle.rs` 趟一
// 逐颗调 `spawn_star_at`），而弹池 cap 是 8192 —— 任何一次大规模消弹都可能一帧内要走
// 比道具池宽得多的格子。实测 demo 局收卡那一帧（4954）场上 626 颗弹全转星星，**四个难度
// 档全部溢出**（Easy 3 / Normal 37 / Hard 67 / Lunatic 104 颗没生成）。1024 盖得住当前
// 内容的弹数峰值（rank 3 为 814）并留余量。
//
// ⚠ **1024 不是结构性的保证,只是把线挪远**：弹池是道具池的 8 倍,1:1 转换天生可能溢出。
// 溢出时的处置是 P4-a 确定性降级（该颗不生成 + 逐颗计数,消弹循环有界不短路,判别腿
// `star_pool_full_counts_every_missing_star`）——这条口径写在 `docs/ecl-ops.md` 的 54 号
// 与 `spawn_star_at` 的文档里,**是已知设计边界,不是待修的债**。
define_pool! {
    Item, cap = 1024,
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
