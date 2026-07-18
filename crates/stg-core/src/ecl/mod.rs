//! ECL 任务协程层地基（M1 Task 1）—— 断层线以下（`stg-core` 内）。字节码栈机 VM + Task 池。
//!
//! **T1 版现状**：`task` 池 + `vm` 解释核落地，**零 step 集成**——`World.tasks` 字段已加入
//! 快照/校验和，但没有任何相位驱动它（协程调度 + `EclImage` 穿线是 T2 的事）；金向量因而
//! 逐位不变。`ops` 承载编号即契约的 opcode 表 + 元数表。
//!
//! 依赖方向（P1）：`ECL → world`；world 不 import 本模块的任何类型知识——`World.tasks`
//! 字段物理住组装层 `stg_core::step`，但 world 侧代码从不引用 `ecl::*`。

pub mod ops;
pub mod task;
pub(crate) mod vm;
