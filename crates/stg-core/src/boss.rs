//! boss 公告板（A2）——**脚本写、UI 读、世界自身不读**的类型化数据槽。
//!
//! 原 `StageState` 里的"boss 阶段"概念在 A2 拆解后只剩这块公告板：当前符卡 id、血条比例、
//! 倒计时、剩余阶段数。写入方是 ECL 的 `boss_set` syscall（M1；世界 API `WorldBody::boss_set`
//! 整槽写入），读取方是断层线以上的 UI/表现层——**超时/换卡/血条刷新全是 boss 主控任务的事**，
//! 世界逻辑不解释这里的任何字段（`enemy` 句柄有效性也不校验：世界不读它，读方按"悬垂视同
//! 已失效"处置，P4-b 哲学）。零初始化合法（`active = 0` = 无 boss）。

use crate::enemy::EnemyHandle;
use crate::math::Fx;

/// boss 槽数（D10 预算"boss_ui×2"：双 boss 同屏上限）。
pub const MAX_BOSSES: usize = 2;

/// 单 boss 公告板槽。字段语义归脚本/UI 约定，世界只存不读。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct BossUiSlot {
    /// 哪个敌人是 boss（悬垂由读方处置）。
    pub enemy: EnemyHandle,
    /// 血条比例（Fx）。非符卡段脚本经 `boss_set` 刷；**符卡 active 期间由符卡机构自动喂
    /// 逐卡血条 `(hp−threshold)/(hp_start−threshold)`（spell.rs settle_spells，2026-07-24）**。
    pub hp_ratio: Fx,
    /// 当前符卡 id。
    pub spell_id: u16,
    /// 倒计时帧数（脚本负责递减）。
    pub timer_frames: u16,
    /// 剩余阶段数（血条下的星星）。
    pub phase_left: u8,
    /// 0 = 无 boss（零初始化合法）；非零语义归脚本。
    pub active: u8,
}

impl Default for BossUiSlot {
    fn default() -> Self {
        BossUiSlot {
            enemy: EnemyHandle::NULL,
            hp_ratio: Fx::ZERO,
            spell_id: 0,
            timer_frames: 0,
            phase_left: 0,
            active: 0,
        }
    }
}
