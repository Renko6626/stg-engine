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
    /// 表现锚点四读口（整局流程刀 spec §4；仅 5x 族 syscall 写，见 `WorldBody` 字段文档）。
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
}
