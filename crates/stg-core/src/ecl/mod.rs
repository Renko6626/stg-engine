//! ECL 任务协程层（M1）—— 断层线以下（`stg-core` 内）。字节码栈机 VM + Task 池 + 只读镜像。
//!
//! **T3 版现状**：`vm::run_tasks` 是相位 2（`PH_DIRECTOR`）导演槽的默认租户——升序遍历
//! `World.tasks`，owner 门禁 → 次帧首跑门禁 → wait 门禁 → 全局预算门禁 → 解释执行；
//! `EclImage`（本模块 `image` 子模块）随 `&EclImage` 参数穿线（与 `&WorldTables` 同款，
//! 不进 `World`）。`OP_SPAWN`/`OP_KILL_SELF`/`OP_KILL_CHILDREN` 语义落地；`OP_SYS` 起 T3
//! 派发进 `syscall::dispatch`（号表 v1，`VmCtx` 相应扩出 `body: &mut WorldBody` +
//! `tables: &WorldTables`）。`ops` 承载编号即契约的 opcode 表 + 元数表。
//!
//! 依赖方向（P1）：`ECL → world`；world 不 import 本模块的任何类型知识——`World.tasks`
//! 字段物理住组装层 `stg_core::step`，但 world 侧代码从不引用 `ecl::*`。

pub mod image;
pub mod ops;
pub(crate) mod syscall;
pub mod task;
pub(crate) mod vm;

pub use image::EclImage;
