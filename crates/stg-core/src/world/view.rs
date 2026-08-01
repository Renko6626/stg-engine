//! 通道 A 读侧零拷贝视图（A9）——step 后表现层/headless 经 `world.view()` 只读五池 SoA。
//! 借 `&WorldBody`：视图活着期间无法 step（要 `&mut`），借用检查器天然保证时序安全。
//! 只出原始定点/整数切片，绝不转 float（float 留 Godot 桥，守 I1）。

use super::WorldBody;
use crate::bullets::BulletPool;
use crate::enemy::EnemyPool;
use crate::field::FieldPool;
use crate::items::ItemPool;
use crate::player::PlayerState;
use crate::shots::ShotPool;

/// 只读世界视图。`Copy`（仅一个借用）；方法取 `self` 以还 `'w` 生命。
#[derive(Clone, Copy)]
pub struct WorldView<'w> {
    pub(crate) body: &'w WorldBody,
}

impl<'w> WorldView<'w> {
    pub fn bullets(self) -> &'w BulletPool {
        &self.body.bullets
    }
    pub fn shots(self) -> &'w ShotPool {
        &self.body.shots
    }
    pub fn enemies(self) -> &'w EnemyPool {
        &self.body.enemies
    }
    pub fn items(self) -> &'w ItemPool {
        &self.body.items
    }
    pub fn fields(self) -> &'w FieldPool {
        &self.body.fields
    }
    pub fn players(self) -> &'w [PlayerState] {
        self.body.players()
    }
    /// 符卡计器槽只读切片（spec 2026-07-24；每 boss 一个，参考 `crate::boss::MAX_BOSSES`）。
    pub fn spells(self) -> &'w [crate::spell::SpellSlot] {
        &self.body.spells
    }
    /// 表现锚点四读口（整局流程刀 spec §4；仅 5xx 族 syscall 写，见 `WorldBody` 字段文档）。
    pub fn bgm_id(self) -> u16 {
        self.body.bgm_id
    }
    pub fn bg_id(self) -> u16 {
        self.body.bg_id
    }
    pub fn bg_phase(self) -> u16 {
        self.body.bg_phase
    }
    pub fn bg_phase_frame(self) -> u32 {
        self.body.bg_phase_frame
    }
    /// boss 公告板只读切片（A2/D6）——`WorldBody::boss_ui` 收 `pub(crate)` 后跨 crate
    /// 消费者（godot 桥/harness viewer）的唯一读口；写口仍是 `WorldBody::boss_set`。
    pub fn boss_ui(self) -> &'w [crate::boss::BossUiSlot] {
        &self.body.boss_ui
    }
    /// 全局变量竞技场只读切片（A2/D6）。刻意绕开 `get_var`——那条口子取 `&mut self` 是为了让
    /// 坏槽读也入 `contract_viol` 计数（两机必须一样错），纯观测不该顺带触发那份写路径副作用。
    pub fn globals(self) -> &'w [i32] {
        &self.body.globals
    }
    /// 诊断计数器只读快照（P4/D6）。`Copy`，按值返回（八池 `pool_full` + 五个 `u32` 计数，
    /// 见 `DiagCounters`）。
    pub fn diag(self) -> crate::world::DiagCounters {
        self.body.diag
    }
    /// 上一次写 API 调用的状态码（P4-b/D6）；`STATUS_OK`/`STATUS_BAD_ARGS`/… 见 `world.rs`
    /// 顶部 `STATUS_*` 常量表。
    pub fn last_status(self) -> u16 {
        self.body.last_status
    }
}

#[cfg(test)]
mod tests {
    use crate::boss::BossUiSlot;
    use crate::math::Fx;
    use crate::step::World;
    use crate::world::STATUS_BAD_ARGS;

    /// `boss_ui()` 读口：`boss_set` 喂一槽非零值，经读口原样读回（判别式：全零世界读口也
    /// 会"绿"，必须喂非默认值才反证读口真的接的是这块内存而非默认构造）。
    #[test]
    fn boss_ui_accessor_reflects_boss_set() {
        let mut w = World::new(1);
        w.body.boss_set(
            0,
            BossUiSlot {
                active: 1,
                hp_ratio: Fx::from_raw(12_345),
                spell_id: 7,
                timer_frames: 900,
                ..Default::default()
            },
        );
        let slot = w.body.view().boss_ui()[0];
        assert_eq!(slot.active, 1);
        assert_eq!(slot.hp_ratio.raw(), 12_345);
        assert_eq!(slot.spell_id, 7);
        assert_eq!(slot.timer_frames, 900);
    }

    /// `globals()` 读口：`set_var` 后经切片读回，且不触碰 `get_var` 的 `&mut self`/计数副作用
    /// 路径（本测试全程不需要 `w` 是 `mut` 之外的任何 `&mut` 借用来读，`view()` 只借 `&self`）。
    #[test]
    fn globals_accessor_reflects_set_var() {
        let mut w = World::new(1);
        w.body.set_var(20, 42);
        assert_eq!(w.body.view().globals()[20], 42);
    }

    /// `diag()`/`last_status()` 读口：制造一次 `contract_viol`（`set_var` 越界槽）后两个读口
    /// 同步可见——判别 P4-b 违约路径确实把计数/状态码写进了 `WorldBody`，读口只是原样转发。
    #[test]
    fn diag_and_last_status_accessors_reflect_contract_violation() {
        let mut w = World::new(1);
        assert_eq!(w.body.view().diag().contract_viol, 0, "开局零违约");
        w.body.set_var(crate::world::GLOBALS_CAP as u16, 0); // 恰过界
        assert_eq!(w.body.view().diag().contract_viol, 1);
        assert_eq!(w.body.view().last_status(), STATUS_BAD_ARGS);
    }
}
