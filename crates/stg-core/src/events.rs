//! 碰撞命中缓冲 `Hit` 与世界大事记 `Event`（A5）。两者皆**纯输出**：帧内私有、
//! checksum-skip、begin 清空、重演确定性再生。Hit 由相位 6 收集、相位 7 三趟消费；
//! Event 由相位 7/相位 3 死亡结算产出，相位 8 ECL 挂钩 + 表现层只读消费。

use crate::math::Fx;

pub(crate) const HITS_CAP: usize = 8192;
pub(crate) const EVENTS_CAP: usize = 512;

// ── 碰撞矩阵行号（D8）─────────────────────────────────────────────
pub(crate) const ROW_BULLET_PLAYER_HIT: u8 = 1;
pub(crate) const ROW_BULLET_PLAYER_GRAZE: u8 = 2;
pub(crate) const ROW_BODY_PLAYER_HIT: u8 = 3;
pub(crate) const ROW_SHOT_ENEMY: u8 = 4;

// ── 事件种类 ──────────────────────────────────────────────────────
pub const EVT_ENEMY_DIED: u8 = 1;
pub const EVT_PLAYER_DIED: u8 = 2;

/// 一条碰撞命中（6 B）：矩阵行 + 主动/被动池索引。收集序天然按收集循环嵌套，无需排序。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Hit {
    pub row: u8,
    pub active: u16,
    pub passive: u16,
}

/// 一条世界大事记（A5）：`a_index/a_gen` = 相关实体句柄（玩家死亡时 a_index=自机号、a_gen=0）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Event {
    pub kind: u8,
    pub a_index: u16,
    pub a_gen: u16,
    pub x: Fx,
    pub y: Fx,
    pub data: [i32; 2],
}
