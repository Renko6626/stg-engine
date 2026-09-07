//! timeline —— World 之上的时间线层（时间机制内核刀 2026-09-07，spec
//! `docs/superpowers/specs/2026-09-07-timeline-observe-jump-rewind-design.md` §3）。
//! 快照环 + 遡行兑现 + 影子世界 + 输入日志/回放。World 对它无知（P1/P5）。
//! 本文件先只落常量；`Timeline` 本体随第二段提交进来。

/// 遡行落点深度：落点 = 被弹帧 − 本值（钳到环里最老一帧）。策划案 2.3 暂定 0.5–1 s，先取 0.5 s。
pub const REWIND_DEPTH: u32 = 30;
