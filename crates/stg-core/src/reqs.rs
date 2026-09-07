//! 通道 B 渲染请求（design_doc §6.2/§6.3，"核出请求，壳做演出"）。**半冻结跨语言契约**：
//! repr(C) 布局（2+2+24 = 28 B）+ id 分区 + 引擎 id args 约定表——M2 Godot 分发器按此
//! 路由/解码，改动过评审 + 视情 bump `engine_ver`。
//!
//! ## id 命名空间分区
//! - `0`：保留无效值（零结构体防呆；分发器忽略）。
//! - `1..=63`：引擎保留（`crate::consts` structural 段注册，Rust/脚本双侧单源）。
//! - `64..`：脚本 / mod 自由段（作者自配 `.ecl` `const`，与自家分发器 handler 自成契约）。
//!
//! ## 引擎 id args 约定表（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）
//! | id | args[0] | args[1] | args[2] | args[3] | args[4..] |
//! |---|---|---|---|---|---|
//! | `REQ_ENEMY_DEATH` | x (fx raw) | y (fx raw) | sprite (int) | score (int) | 0 |
//! | `REQ_SPELL_DECLARE` | spell_id (int) | bonus0 (int) | time_limit (int) | survival_flag (int) | 0 |
//! | `REQ_SPELL_RESULT` | spell_id (int) | captured (int) | 实付 bonus (int) | reason (int) | 0 |
//! | `REQ_STAGE_CLEAR` | **退役**（壳子刀 2026-09-07：流程改走 `EVT_STAGE_CLEARED` 事件，号保留作兼容） | — | — | — | — |
//! | `REQ_BGM` | id (int，= 写入的 `bgm_id`) | 0 | 0 | 0 | 0 |
//! | `REQ_BG` | id (int，= 写入的 `bg_id`) | 0 | 0 | 0 | 0 |
//! | `REQ_BG_PHASE` | phase (int，= 写入的 `bg_phase`) | 0 | 0 | 0 | 0 |
//! | `REQ_FX_AT` | x (fx raw) | y (fx raw) | kind (int) | param (int) | 0 |
//! | `REQ_FX_ATTACHED` | enemy index (int) | enemy gen (int) | kind (int) | param (int) | 0 |
//!
//! ## 类别（表现契约 v2，2026-09-07；壳侧分发器按此处置，见 `render-contract.md` §4）
//! - **即发即忘**：`REQ_ENEMY_DEATH` / `REQ_FX_AT` / `REQ_FX_ATTACHED`——播完即忘，回滚
//!   误播接受为鬼影。
//! - **须确认**：`REQ_SPELL_DECLARE` / `REQ_SPELL_RESULT` / `REQ_STAGE_CLEAR`——M3 后只在越过
//!   回滚地平线后播。
//! - **电平镜像**：`REQ_BGM` / `REQ_BG` / `REQ_BG_PHASE`——边沿通知，真值在 `anchors()`
//!   四字段，宿主开机/读档后先对表。

/// 一条渲染请求（§6.2，28 B）。id 语义世界不解释；`(frame, seq)` 全局唯一。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RenderReq {
    /// 请求名（编译期驻留；`0` = 保留无效值）。
    pub id: u16,
    /// 帧内自增序号（= 入缓冲索引）。
    pub seq: u16,
    /// 按 id 约定解释的裸载荷（模块文档约定表）。
    pub args: [i32; 6],
}

/// 每帧请求上限（D10 预算既有行：256 × 28 B = 7 KB）。
pub(crate) const REQS_CAP: usize = 256;
