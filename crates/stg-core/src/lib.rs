//! # stg-core —— 弹幕 STG 引擎的确定性内核（断层线以下）
//!
//! 权威设计见仓库根的 `design_doc.md`（v0.3 总纲）与 `stg-world-design.md`（v1.0 世界层蓝图）。
//! 硬纪律摘要（完整见 `CLAUDE.md`）：
//!
//! - **I1 数值**：唯一标量是定点数 Q16.16（`i32`）；`f32/f64` 不得出现在本 crate。
//! - **I2 角度**：BAM `u16`（一圈 = 65536），三角函数一律查表。
//! - **I3 随机**：PRNG 状态是 `World` 字段，随快照回滚；不碰宿主 RNG。
//! - **I4 顺序**：一切遍历按池索引升序；禁 `HashMap` 等无序容器参与模拟。
//! - **I5 协程**：模拟协程执行状态位于可 memcpy 的扁平内存（自建字节码 VM）。
//! - **I6 时间**：逻辑固定 60 Hz，一切计时用整数帧，step 不接受 delta。
//! - **I7 布局**：`World` 内无指针/引用/堆容器；快照 = 整块字节复制。
//!
//! 断层线靠 `Cargo.toml` 依赖图在编译期焊死，而非自律（见本 crate 清单里的确定性防火墙）。
//!
//! ## Phase 1 脚手架现状
//!
//! 当前仅落地确定性契约的最底层基础设施 —— [`checksum`]（vendored FNV-1a 64，D11）。
//! 数学核 / 池框架 / World 本体 / step 相位随 **M0** 逐模块长肉，全程 TDD + 金向量回归。

// 让 #[derive(Checksum)] 生成的 `::stg_core::…` 绝对路径在本 crate 内解析（serde 同款）。
extern crate self as stg_core;

pub use stg_derive::define_pool;

/// 最大自机数（共场 co-op 超集，§7.5）。
pub const MAX_PLAYERS: usize = 2;

pub mod bullets;
pub mod checksum;
pub mod enemy;
pub mod events;
pub mod input;
pub mod math;
pub mod player;
pub mod rng;
pub mod shots;
pub mod step;
pub mod world;

pub use step::{World, step, step_with_director};

// ── Phase 1 M0+ 起逐步落地（占位，勿在 scaffold 阶段实现）──────────────────
//
//   pub mod pool;   // D2 `define_pool!` 宏与六个实体池（SoA + generation 句柄）
//   pub mod world;  // Part III 世界本体（WorldBody 字段、pub(crate) 相位函数）
//   pub mod step;   // P2 组装层宪法顺序（导演槽 + PhaseGuard）
