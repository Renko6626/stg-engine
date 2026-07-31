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

/// 引擎确定性契约版本。**bump 纪律**：凡改 op 表/syscall 号语义、校验和算法、烘焙表内容、
/// 池 SoA 布局/字段序、step 相位序、**SaveBytes 载荷/叶型编码**（存档字节格式——头形变更
/// 走 `save::SAVE_FILE_VER`，载荷编码变更走本值，见 spec §5）——任何使旧回放/旧对端/旧档
/// 不可对拍的变更——必须 +1 并过评审。
/// 回放头（M3）/联机握手（M4）身份三元组之一（另两个：表 `content_hash`、镜像 `content_hash`）。
///
/// **1 → 2**（颜色轴刀 T7，2026-07-26）：op 清单新增 `OP_SET_SHAPE`(32)/`OP_SET_COLOR`(33)
/// （`set_sprite` 细化出的"只改形"/"只改色"两个部分设 op）。
///
/// **2 → 3**（ECL parity 刀 Task 2，2026-07-30）：syscall 号表新增 `SYS_CLEAR_BULLETS`(54)
/// （全场清弹 B19）——号表变更同 op 表冻结纪律，同款口径 bump。
///
/// **3 → 4**（敌人死亡效果刀 T3，2026-07-30）——**两条理由，缺一不可**：
/// ① syscall 号表新增 `SYS_DROP_CLEAR`(58)/`SYS_DROP_ADD`(59)/`SYS_DROP_ITEMS`(60)/
///   `SYS_DIE`(61)；
/// ② **敌人池字段布局变更**（本刀 T1）：`drop_table: u16` → `drop_count: [u8; 5]`，
///   `EnemyPool` 的 SoA 字段序/宽度变了 ⇒ SaveBytes 载荷编码随之变化（存档 wire format
///   变更）。T1 落地时未 bump（本刀内的中间状态），身份三元组直到本步才动，故理由记在
///   这里——只写 ① 会让人误以为旧档还能读。
/// **4 → 5**（shooter 刀 Task 1，2026-07-31）——**两条理由**：
/// ① syscall 号表将新增 62–76（`sh_*` 族 setter + `sh_fire`，本刀 T2/T3）；
/// ② **`TaskPool` 布局变更**（本刀 T1，**就是本次提交**）：新增并行数组
///   `shooters: [[ShooterSlot; 4]; 256]`（+45056 B），`SaveBytes` derive 自动把它写进载荷
///   ⇒ 存档 wire format 变化，旧档不可用新版解析。
///
/// **为什么在 T1 就 bump**：布局在本步就变了。上一刀（敌人死亡效果）把 bump 拖到第三步，
/// 结果中间几个 commit 处于"存档格式变了而身份三元组没变"的状态——这次提前。
/// ① 是对**本刀余下两步**的预告，落地时不再二次 bump。
///
/// **5 → 6**（小清洗刀，2026-07-31）：syscall 号表新增 `SYS_ATAN2`(77)/`SYS_DIST`(78)/
/// `SYS_NEAREST_ENEMY`(79)。**只有号表这一条理由**——本刀不动 `World` 布局（尺寸哨兵
/// 未变）、不动存档编码、不动任何既有 syscall 的语义，故旧档形状上仍可读；bump 是号表
/// 冻结纪律的机械要求（同 2→3 那次的单理由 bump 口径）。
pub const ENGINE_VER: u32 = 6;

pub use stg_derive::define_pool;

/// 最大自机数（共场 co-op 超集，§7.5）。
pub const MAX_PLAYERS: usize = 2;

pub mod boss;
pub mod bullets;
pub mod checksum;
pub mod consts;
pub mod ecl;
pub mod enemy;
pub mod events;
pub mod field;
pub mod input;
pub mod items;
pub mod math;
pub mod player;
pub mod reqs;
pub mod rng;
pub mod save;
pub mod shots;
pub mod spell;
pub mod step;
pub mod tables;
pub mod world;
pub mod xform;

pub use step::{World, step, step_with_director};

// ── Phase 1 M0+ 起逐步落地（占位，勿在 scaffold 阶段实现）──────────────────
//
//   pub mod pool;   // D2 `define_pool!` 宏与六个实体池（SoA + generation 句柄）
//   pub mod world;  // Part III 世界本体（WorldBody 字段、pub(crate) 相位函数）
//   pub mod step;   // P2 组装层宪法顺序（导演槽 + PhaseGuard）
