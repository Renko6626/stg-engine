//! 碰撞命中缓冲 `Hit` 与世界大事记 `Event`（A5）。两者皆**纯输出**：帧内私有、
//! checksum-skip、begin 清空、重演确定性再生。Hit 由相位 6 收集、相位 7 三趟消费；
//! Event 由相位 7/相位 3 死亡结算产出，相位 8 ECL 挂钩 + 表现层只读消费。

use crate::math::Fx;

pub(crate) const HITS_CAP: usize = 8192;
pub(crate) const EVENTS_CAP: usize = 512;
/// `vanished` 缓冲上限（表现契约 v2，2026-09-07）：bomb 峰值实测约 814 颗（自机能力刀），
/// 1024 行够用；超限确定性丢弃 + `diag.vanished_overflow`（进校验和，P4-a）。
pub(crate) const VANISHED_CAP: usize = 1024;

// ── 碰撞矩阵行号（D8）─────────────────────────────────────────────
pub(crate) const ROW_BULLET_PLAYER_HIT: u8 = 1;
pub(crate) const ROW_BULLET_PLAYER_GRAZE: u8 = 2;
pub(crate) const ROW_BODY_PLAYER_HIT: u8 = 3;
pub(crate) const ROW_SHOT_ENEMY: u8 = 4;
pub(crate) const ROW_ITEM_PLAYER: u8 = 5; // 道具 × 自机拾取圈（graze_radius 兼拾取圈，D7）
pub(crate) const ROW_FIELD_BULLET: u8 = 6; // 作用区 × 敌弹 → 消弹
pub(crate) const ROW_FIELD_ENEMY: u8 = 7; // 作用区 × 敌人 hurtbox → 扣血

// ── 事件种类 ──────────────────────────────────────────────────────
pub const EVT_ENEMY_DIED: u8 = 1;
pub const EVT_PLAYER_DIED: u8 = 2;
/// 作用区本帧消弹的聚合事实（每 field 每帧至多一条，`data[0]` = 本帧本 field 消了几颗）。
///
/// **聚合而非逐弹**：弹池 cap 8192 而 events cap 512，逐弹发在全屏消弹下必爆（溢出 16×）。
pub const EVT_FIELD_CLEARED: u8 = 3;
/// 一颗道具被拾取（行 5 结算）：`a_index/a_gen` = 道具句柄位，`data = [item_type, player]`。
pub const EVT_ITEM_PICKED: u8 = 4;
/// ECL 任务确定性报错被杀（M1 T2，相位 2 ECL 调度租户产出）：`a_index` = 任务池索引
/// （`a_gen` 恒 0——任务池无逐槽 generation，见 `ecl::task` 文档）、
/// `data = [fault_code, script]`（fault 码见 `ecl::vm::FAULT_*`）。owner 死亡导致的静默回收
/// **不**发本事件（owner 死是常态非错误，见 `ecl::vm::run_tasks` 文档）。
pub const EVT_TASK_FAULT: u8 = 5;
/// 符卡宣言（`spell::spell_begin_internal` 成功产出，spec 2026-07-24 §4）：`a_index/a_gen`
/// = boss 句柄，`data = [spell_id, bonus0]`。RL episode 边界正典信号之一。
pub const EVT_SPELL_DECLARED: u8 = 6;
/// 符卡收卡（HP 路径资格在 / 耐久卡活到超时）：`data = [spell_id, 实付 bonus]`。
pub const EVT_SPELL_CAPTURED: u8 = 7;
/// 符卡失败（HP 路径资格失 / 普通卡超时 / 耐久卡超时且资格失）：`data = [spell_id, reason]`
/// （reason：1=资格失 2=超时）。
pub const EVT_SPELL_FAILED: u8 = 8;
/// 自机弹命中敌人（相位 7 趟二，伤害真正结算的那一刻）：`a_index/a_gen` = 被打的敌，
/// **`x`/`y` = 自机弹当帧位置**（命中点在弹上、不在敌心——表现层要在这里冒火花），
/// `data = [damage, 0]`（`data[1]` 预留）。
///
/// **逐命中发、不聚合**：自机弹同屏受池 cap 1024 限，现实命中数 <50/帧，`EVENTS_CAP=512`
/// 绰绰有余（对照 `EVT_FIELD_CLEARED` 那条——它必须聚合，因为弹池 8192 远超 events 512）。
/// 擦弹另说：满屏擦弹频率高一个量级，真要发事件须单独评估聚合口径。
pub const EVT_SHOT_HIT_ENEMY: u8 = 9;

// ── `vanished`：本帧离开池的敌弹（表现契约 v2 spec §3.4）────────────────────
/// 弹 `life` 归零。
pub const VANISH_LIFE: u8 = 1;
/// 弹被作用区清除（`BULLET_CLEARED`：bomb / deathbomb / 自机中弹清屏 / `clear_bullets`）。
pub const VANISH_CLEARED: u8 = 2;

/// 一行 `vanished`（12 B）：本帧在相位 9 被回收的一颗**敌弹**的最后位置与外观。
///
/// **只记弹、越界不记**：自机弹消失已有 `EVT_SHOT_HIT_ENEMY`、道具已有 `EVT_ITEM_PICKED`；
/// 越界弹在屏外没有淡出可画，也免得白占行。第四条纯输出缓冲，与 `hits`/`frame_events`/
/// `reqs` 同族：帧内私有、checksum-skip、`begin` 清空、回滚重演确定性再生——
/// **必须在两次 step 之间取走**。消费者：表现层的消弹淡出（`render-contract.md` §3.6）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vanished {
    pub x: Fx,
    pub y: Fx,
    pub sprite: u16,
    /// `VANISH_LIFE` / `VANISH_CLEARED`。
    pub reason: u8,
    pub _pad: u8,
}

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
