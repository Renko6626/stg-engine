//! syscall 测试入口与跨领域共用的辅助函数。

use super::*;
use crate::ecl::image::{EclImage, EntryInit, SubInit, SubKind, test_image};
use crate::ecl::shooter::{SH_ABS_OFFSET, SH_AIMED, SH_RING, SHOOTERS_PER_TASK, ShooterSlot};
use crate::ecl::task::{OWNER_ENEMY, OWNER_STAGE, Task};
use crate::step::World;
use crate::tables::TABLES_V0;

// 外观表行号（颜色轴刀 T4 起 ② 段清空、旧的 `APPEARANCE_*` 引擎常量退场——弹型名
// 归内容包的 `.ecl` const，引擎侧测试直接写行号）。id = 弹型 × color_stride(16) + 色号：
// 0 = 0 号形第 0 色，1 = 0 号形第 1 色，2 = 0 号形第 2 色。三者只用于"随便挑一个
// 合法格"，与具体形/色语义无关。
const ROW_A: i32 = 0;
const ROW_B: i32 = 1;
const ROW_C: i32 = 2;

mod bullet;
mod contract;
mod enemy;
mod laser;
mod query;
mod shooter;
mod stage_spell;

/// 派发测试助手：把 `args`（脚本**声明顺序**，正序）压栈，直连 `dispatch`（不经
/// `OP_SYS`/`exec`——聚焦 syscall 语义本身，`OP_SYS` 派发链路已由 `vm.rs` 测试覆盖）。
fn call(w: &mut World, ecl: &EclImage, task: &mut Task, no: u16, args: &[i32]) -> Result<(), u8> {
    call_with_tables(w, ecl, task, no, args, &TABLES_V0)
}

/// 绑定**指定表**的 `call`——空格判据一类"内建表提供不了靶子"的测试用它挂合成表
/// （当前图集 12 行全满 16 色，没有空格；数据如实反映美术，不为测试在内建表里造假）。
fn call_with_tables(
    w: &mut World,
    ecl: &EclImage,
    task: &mut Task,
    no: u16,
    args: &[i32],
    tables: &crate::tables::WorldTables,
) -> Result<(), u8> {
    call_full(w, ecl, task, no, args, tables, 0)
}

fn call_full(
    w: &mut World,
    ecl: &EclImage,
    task: &mut Task,
    no: u16,
    args: &[i32],
    tables: &crate::tables::WorldTables,
    self_index: u16,
) -> Result<(), u8> {
    for &a in args {
        task.stack[task.sp as usize] = a;
        task.sp += 1;
    }
    let mut budget = u32::MAX;
    let frame = w.body.frame;
    let mut ctx = VmCtx {
        code: &[],
        budget: &mut budget,
        tasks: &mut w.tasks,
        ecl,
        body: &mut w.body,
        tables,
        self_index,
        frame,
    };
    dispatch(no, task, &mut ctx)
}

/// 合成一张"某格是空格"的表（其余与内建表逐位相同）。
fn tables_with_hole(hole: usize) -> crate::tables::WorldTables {
    let mut t = crate::tables::build_tables_v0();
    let mut rows = t.appearances.to_vec();
    rows[hole].valid = false;
    t.appearances = rows.into_boxed_slice();
    t
}

fn fresh() -> (Box<World>, EclImage) {
    (World::new(1), EclImage::empty())
}

/// 单个 0 参 Async sub（raw=1）的镜像——`spell_begin` 的 `pattern:SubRef` 测试专用
/// （代码体本身无关紧要，只需能被 `run_tasks` 安全 wait）。
fn async_pattern_image() -> EclImage {
    test_image(
        vec![
            crate::ecl::ops::OP_PUSHI as u32,
            1,
            crate::ecl::ops::OP_WAIT as u32,
        ],
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(0, SubKind::Async, vec![]),
        ],
        vec![EntryInit::new("pattern", 1)],
        Some(0),
    )
}

/// 单个 **CallOnly**（非 Async）sub（raw=1）的镜像——task 号支路③专用（B25）：
/// 号在册（`sub_id`/`sub_meta` 均命中）但 `kind() != SubKind::Async`，用来钉死
/// "在册但不是零参 Async" 这条校验，而不是被支路②（号不在册）顺手拦下。
/// CallOnly sub 不需要 entry 表项（`MissingAsyncEntry` 只查 Async sub）。
fn wrong_kind_task_image() -> EclImage {
    test_image(
        vec![crate::ecl::ops::OP_RET as u32],
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(0, SubKind::CallOnly, vec![]),
        ],
        vec![],
        Some(0),
    )
}

/// 单个**带参** `Async` sub（raw=1，一个 `Fx` 参）的镜像——支路③另一半专用（终审 M-4）：
/// `kind() != Async || !param_types().is_empty()` 这个判据是**或**，`wrong_kind_task_image`
/// 只打了左半（kind 不对）；这个 helper 打右半（kind 对但带参）——号在册、`sub_meta`
/// 命中、`kind()==Async`，唯独 `param_types` 非空。若把 `!param_types().is_empty()`
/// 那半判据删掉，这里必须能把变异逮住（否则"带参 Async sub 不许当 task 挂载"这条
/// 约束就是无网状态）。先例：`image.rs:606` 的 `SubInit::new(0, SubKind::Async,
/// vec![EclValueType::Fx])`。
fn async_with_param_task_image() -> EclImage {
    test_image(
        vec![
            crate::ecl::ops::OP_PUSHI as u32,
            1,
            crate::ecl::ops::OP_WAIT as u32,
        ],
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(0, SubKind::Async, vec![crate::ecl::image::EclValueType::Fx]),
        ],
        vec![EntryInit::new("pattern", 1)],
        Some(0),
    )
}

// ── SYS_SPELL_BEGIN/END/TIMER（Task 2；符卡机构 spec 2026-07-24 §5）─────────

fn enemy_task(w: &mut World, hp: i32) -> (EnemyHandle, Task) {
    let boss = crate::world::test_support::spawn_enemy(w, 0, 80, hp);
    let task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: boss.index,
        owner_gen: boss.generation,
        ..Task::default()
    };
    (boss, task)
}

// ── 敌人死亡效果四 syscall（520-530；敌人死亡效果刀 T3）────────────────────

/// 造一只 owner 敌 + 指向它的任务（掉落计数**空**——要灌表 1 用 `load_drop_table_1`）。
fn enemy_owner_task(w: &mut World, hp: i32, score: u16) -> (EnemyHandle, Task) {
    let eh = crate::world::test_support::spawn_enemy(w, 0, 0, hp);
    w.body.enemies.score[eh.index as usize] = score;
    let task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    (eh, task)
}

/// `fresh()` + 把 0 号任务的 shooter 槽置成 `spawn` 会给的初值。
///
/// ⚠️ **必须显式做这一步**：`World::new` 走 `alloc_zeroed`（见 `step::World::new`），
/// 而 `ShooterSlot::default()` **非全零**（`n_angle/n_speed=1`、`task_script=0xFFFF`）——
/// 生产路径上是 `TaskPool::spawn` 负责重置，而本模块的 `call` 助手用的是手拼 `Task`
/// （从没经过 `spawn`）。不补这一步，测试比对的基线就是"全零槽"而非真实初值，
/// `sh_reset` 一类断言会以看不出所以然的方式红。坑档见 `TaskPool::new` 文档。
fn fresh_with_shooters() -> (Box<World>, EclImage) {
    let (mut w, ecl) = fresh();
    w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
    (w, ecl)
}
