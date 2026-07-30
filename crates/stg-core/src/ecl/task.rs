//! Task 池（M1 地基）—— 手写特例池（`xform.rs` 先例：存活位图即分配器，最低空位，无逐槽
//! generation——任务句柄 = (index, birth_frame) 由调度层管；对外只 spawn/kill/iter）。
//!
//! 容量全为编译期常量（spec 拍板 3）：求值栈 32 字 / locals 64 字 / 调用栈 8 帧 / 池 cap 256——
//! `Task` = **448 B**、`TaskPool` = **159 776 B ≈ 156 KiB**（其中 shooter 并行数组占
//! 45 056 B，见下方 `shooters` 字段；World = 1 129 944 B ≈ 1.08 MB，任务池占其 **14%**，
//! 逐帧全量进校验和）。全部编译期常量，金向量实测不够再调。
//!
//! （数字随 shooter 刀 2026-07-31 订正：旧值 "≈460 B / ≈118 KB / World ~1.04MB" 是加
//! `shooters` 之前的估算口径，与 `bench-baseline.md` 的内存账/`step.rs` 的
//! `world_size_sentinel` 对齐后取实测值。`docs/superpowers/specs/2026-07-18-m1-ecl-vm-design.md`
//! 里那串旧数字**不动**——那是冻结的设计记录。）
//!
//! **为何手写而非 `define_pool!`**：`define_pool!` 把每个字段展开成独立 SoA 数组
//! （`[T; CAP]` per field），适合"细粒度字段各自成阵列"的场景；`Task` 本身已是一块含定长
//! 栈/locals 的扁平内存（I5：协程完整状态 ip+栈+局部位于可 memcpy 内存），拆成 SoA 对它
//! 无意义，故 AoS（`slots: [Task; TASK_CAP]`）手写特例，仿 `xform.rs` 段池。

pub const TASK_CAP: usize = 256;
pub const EVAL_DEPTH: usize = 32;
pub const CALL_DEPTH: usize = 8;
pub const LOCALS: usize = 64;

/// owner 三态：0=关卡（恒有效）/1=敌/2=弹；index+gen 仅 kind≠0 时有意义。
pub const OWNER_STAGE: u8 = 0;
pub const OWNER_ENEMY: u8 = 1;
pub const OWNER_BULLET: u8 = 2;
use crate::ecl::image::SubId;
use crate::ecl::shooter::{SHOOTERS_PER_TASK, ShooterSlot};

/// 一个 ECL 任务（协程）的完整可 memcpy 状态（I5：模拟协程完整状态位于可 memcpy 的扁平内存）。
#[repr(C)]
#[derive(Clone, Copy, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct Task {
    /// `EclImage.subs` 入口索引（T2 起穿线：`pc` 由 spawn 调用方解析 `ecl.entry(script)`
    /// 一次性戳入，运行期不重解——脚本号只用于 owner 门禁之外的一处校验：调度层每帧确认
    /// `ecl.entry(script)` 仍在册，越界即 Fault）。
    pub script: SubId,
    /// 程序计数器：`VmCtx.code` 的字（word）索引——**全局绝对索引**（不是相对脚本入口的偏移），
    /// spawn 时由调用方戳为 `ecl.entry(script)` 的值。
    pub pc: u32,
    /// 剩余等待帧数；>0 时调度层门禁跳过本任务（次帧递减，T2 接线）。
    pub wait: u16,
    /// 出生帧号：`== 当前帧` 则本帧跳过（次帧首跑，T2 接线）。
    pub born_frame: u32,
    pub owner_kind: u8,
    pub owner_index: u16,
    pub owner_gen: u16,
    /// 任务池索引 + 1；0 = 无父（仅 `kill_children` 遍历用，T2）。
    pub parent: u16,
    /// 绑定符卡槽号+1（0=不绑）：`spell_begin` spawn 的模式任务随卡生死（spec §2.1）；
    /// `spawn` 派生子任务继承本值，`fire` 挂弹任务不继承。POD，checksum/save derive 自动盖。
    /// 字段位置刻意卡在 `parent`（u16）后、`sp`（u8）前——`repr(C)` 下恰好落进
    /// `parent`→`stack` 对齐间隙，不改变 `size_of::<Task>()`（尺寸哨兵因此不红）。
    pub spell_bound: u8,
    /// 绑定槽当刻的代际戳（ABA 修复，复审 Task 2）：`spell_bound` 生效时捕获
    /// `WorldBody::spells[slot].epoch` 的当刻值，随任务一起 memcpy；`spawn` 派生子任务与
    /// `spell_bound` 一起继承（整棵模式树共享同一代际身份）。相位 2 调度门禁除了看
    /// `spell_bound`/槽 `active` 还须比较本值——槽被同帧复用给新卡时代际戳换新，旧卡残留
    /// 任务的 `spell_epoch` 因而与新槽不匹配，被门禁杀掉，不会与新卡并发（见
    /// `ecl::vm::run_tasks`）。`spell_bound==0`（未绑）时本值恒为 0，不参与任何判据。
    pub spell_epoch: u16,
    /// 求值栈栈顶（0..=EVAL_DEPTH）。
    pub sp: u8,
    /// 调用栈栈顶（0..=CALL_DEPTH）。
    pub csp: u8,
    pub stack: [i32; EVAL_DEPTH],
    pub calls: [u32; CALL_DEPTH],
    pub locals: [i32; LOCALS],
}

impl Default for Task {
    /// 全零任务——与池 `alloc_zeroed`（`step::World::new`）天然一致：`owner_kind=0`
    /// (`OWNER_STAGE`)、`pc/wait/born_frame=0`、栈/调用栈/locals 全零。
    fn default() -> Self {
        Task {
            script: SubId::default(),
            pc: 0,
            wait: 0,
            born_frame: 0,
            owner_kind: OWNER_STAGE,
            owner_index: 0,
            owner_gen: 0,
            parent: 0,
            spell_bound: 0,
            spell_epoch: 0,
            sp: 0,
            csp: 0,
            stack: [0; EVAL_DEPTH],
            calls: [0; CALL_DEPTH],
            locals: [0; LOCALS],
        }
    }
}

/// 任务池本体（AoS；I7 无堆容器，inline 数组住 `World`，全零 = 合法空池）。
#[repr(C)]
#[derive(crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct TaskPool {
    pub(crate) slots: [Task; TASK_CAP],
    /// 每任务 4 个发射器槽（shooter 刀 2026-07-31）。**并行数组而非塞进 `Task`**——
    /// `Task` 的 `repr(C)` 布局是精算过的（`spell_bound` 卡在对齐间隙里以免改 `size_of`），
    /// 不该为这个塞 176 B 进去。
    /// `spawn` 负责重置（复用槽写满纪律，撑"校验和哈希全槽不掩码"）。
    pub(crate) shooters: [[ShooterSlot; SHOOTERS_PER_TASK]; TASK_CAP],
    pub(crate) alive: [u64; TASK_CAP / 64],
}

impl TaskPool {
    /// 空池构造（供独立于 `World` 的单测使用；`World::new` 走 `alloc_zeroed`，不经此路）。
    ///
    /// ⚠️ **本构造与 `alloc_zeroed` 造出的池校验和不同**——写死在这里免得后人踩。
    ///
    /// `Task` 那条"全零天然一致"（见 `Task::default()` 文档）**不覆盖 `shooters`**：
    /// `ShooterSlot::default()` 非全零（`n_angle/n_speed=1`、`task_script=SH_NO_TASK=0xFFFF`），
    /// 而校验和**哈希全槽、不看存活位**（P6），故那 45056 B 的差异**直接进校验和**——
    /// `TaskPool::new().checksum() != <alloc_zeroed 的池>.checksum()`。
    ///
    /// **为什么无害**：生产路径**只有** `alloc_zeroed` 一条（`step::World::new`；本函数只被
    /// 单测调用），所有生产世界彼此位等价，确定性不受影响；且未 `spawn` 的槽的 shooter
    /// 永不被读（`spawn` 一律重置），两条路径下**活槽**内容恒等。
    ///
    /// **什么情况下会咬人**：**混用两个构造器再比校验和**——譬如写一条测试拿
    /// `TaskPool::new()`（或 `TaskPool::default()`）搭出来的东西去和 `World::new()` 的
    /// `tasks` 对哈希，会撞上一个看不出所以然的不等。要比就两边用同一个构造器。
    pub(crate) fn new() -> Self {
        TaskPool {
            slots: [Task::default(); TASK_CAP],
            shooters: [[ShooterSlot::default(); SHOOTERS_PER_TASK]; TASK_CAP],
            alive: [0; TASK_CAP / 64],
        }
    }

    /// 分配任务（最低空位，I4 确定性分配）：写满全部字段（复用槽写满纪律，撑"哈希全槽不掩码"）。
    /// `owner = (kind, index, gen)`；`parent` = 调用方传入的父任务池索引+1（0=无父）；
    /// `pc` 由调用方解析 `ecl.entry(script)` 后传入（T2 起——本池不知道 `EclImage` 存在，
    /// P1：`task.rs` 是纯池，脚本→入口的解析权在调用方，`vm.rs::exec` 的 `OP_SPAWN` 与
    /// `step::World::spawn_task` 是目前的两个调用方）。
    pub(crate) fn spawn(
        &mut self,
        script: SubId,
        pc: u32,
        owner: (u8, u16, u16),
        parent: u16,
        frame: u32,
    ) -> Option<u16> {
        for w in 0..self.alive.len() {
            if self.alive[w] != u64::MAX {
                let bit = (!self.alive[w]).trailing_zeros() as usize;
                let idx = w * 64 + bit;
                if idx >= TASK_CAP {
                    return None; // 末字幽灵位防御（TASK_CAP 恰 64 倍数时不可达，同 xform.rs 先例）
                }
                self.alive[w] |= 1 << bit;
                self.slots[idx] = Task {
                    script,
                    pc,
                    wait: 0,
                    born_frame: frame,
                    owner_kind: owner.0,
                    owner_index: owner.1,
                    owner_gen: owner.2,
                    parent,
                    // 新任务默认不绑符卡槽——`spell_bound` 由调用方按三处 spawn 各自明确
                    // 的语义事后覆写（OP_SPAWN 继承父值 / spell_begin 显式设 slot+1 /
                    // fire 挂弹任务保持 0，spec §2.1）；本池本身不知道符卡是什么（P1）。
                    spell_bound: 0,
                    // 同 `spell_bound`：默认 0，由调用方按同一三处事后覆写（ABA 修复，
                    // 复审 Task 2，`spell_bound`/`spell_epoch` 恒同批覆写，从不单独设一个）。
                    spell_epoch: 0,
                    sp: 0,
                    csp: 0,
                    stack: [0; EVAL_DEPTH],
                    calls: [0; CALL_DEPTH],
                    locals: [0; LOCALS],
                };
                // 复用槽写满：新任务的发射器一律回默认值。漏这步会让上一个任务的发射器
                // 参数泄漏给新任务——而且因为校验和哈希全槽,泄漏值还会进校验和。
                self.shooters[idx] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
                return Some(idx as u16);
            }
        }
        None
    }

    /// 按索引释放（清 alive 位 + 顺手清空存活子任务的 `parent` 引用）；越界属引擎 bug
    /// （P4-c debug 断言，release no-op）。
    ///
    /// **C12⑤ 复审修复"`KILL_CHILDREN` 无代际戳"**：本池无逐槽 generation（模块文档
    /// "手写特例"），`parent` 只是"槽号+1"，父死后若不处理，孤儿的 `parent` 会继续悬挂
    /// 指向那个已死槽号——槽一旦被最低空位分配器复用，新占用者调 `KILL_CHILDREN` 会因
    /// 槽号数值巧合而误杀前任毫不相干的孤儿（`ecl::vm::tests::
    /// kill_children_does_not_kill_a_reused_slots_previous_orphans` 端到端复现过）。
    /// 用"父死的瞬间断开亲子关系"代替代际戳：`kill` 时把所有存活的直系子（`parent == i+1`）
    /// 的 `parent` 清零，使其成为永久无父的独立任务——语义上与既有的"孙辈不递归杀、
    /// 视为与本任务无关的独立任务"（`OP_KILL_CHILDREN` 文档）完全一致，只是把这层
    /// "detached"提前到父死那一刻兑现，不再等到槽复用才暴露风险。
    pub(crate) fn kill(&mut self, i: usize) {
        debug_assert!(i < TASK_CAP, "kill 越界（引擎 bug）");
        if i < TASK_CAP {
            let orphaned_parent = (i + 1) as u16;
            for j in 0..TASK_CAP {
                if self.is_alive(j) && self.slots[j].parent == orphaned_parent {
                    self.slots[j].parent = 0;
                }
            }
            self.alive[i / 64] &= !(1 << (i % 64));
        }
    }

    pub(crate) fn is_alive(&self, i: usize) -> bool {
        i < TASK_CAP && (self.alive[i / 64] >> (i % 64)) & 1 != 0
    }

    /// 升序 alive 索引迭代（I4）。
    pub fn iter_alive(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.alive.len()).flat_map(move |w| {
            let mut bits = self.alive[w];
            core::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(w * 64 + b)
            })
        })
    }

    /// 快照拷贝（`copy_into` 家族，M0-15 同款：安全逐字段/整块 `copy_from_slice`）。
    pub(crate) fn copy_into(&self, dst: &mut TaskPool) {
        dst.slots.copy_from_slice(&self.slots);
        dst.shooters.copy_from_slice(&self.shooters);
        dst.alive.copy_from_slice(&self.alive);
    }
}

impl Default for TaskPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    #[test]
    fn spawn_is_lowest_free_and_writes_all_fields() {
        let mut p = TaskPool::new();
        let a = p
            .spawn(SubId::default(), 99, (OWNER_ENEMY, 7, 1), 0, 10)
            .unwrap();
        let b = p
            .spawn(SubId::default(), 0, (OWNER_BULLET, 9, 2), a + 1, 11)
            .unwrap();
        assert_eq!((a, b), (0, 1), "最低空位升序");
        assert!(p.is_alive(a as usize));
        assert!(p.is_alive(b as usize));
        let t = &p.slots[a as usize];
        assert_eq!(t.script, SubId::default());
        assert_eq!(t.pc, 99, "pc 由调用方传入戳死（T2 起不再硬编码 0）");
        assert_eq!(t.wait, 0);
        assert_eq!(t.born_frame, 10);
        assert_eq!(t.owner_kind, OWNER_ENEMY);
        assert_eq!(t.owner_index, 7);
        assert_eq!(t.owner_gen, 1);
        assert_eq!(t.parent, 0);
        assert_eq!(t.sp, 0);
        assert_eq!(t.csp, 0);
        let t2 = &p.slots[b as usize];
        assert_eq!(t2.parent, a + 1);
        assert_eq!(t2.born_frame, 11);
    }

    /// 「复用槽写满」纪律：`spawn` 必须把新任务的 4 个 shooter 写成默认值。
    /// 判别腿是**先脏后建**——直接查一个刚 new 出来的池只能证明"全零构造对"，
    /// 证不了"复用时会重置"。
    #[test]
    fn spawn_resets_all_shooters_of_the_reused_slot() {
        use crate::ecl::shooter::ShooterSlot;
        use crate::math::Fx;

        let mut p = TaskPool::new();
        let h = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        // 弄脏这个槽的全部 4 个 shooter
        for s in p.shooters[h as usize].iter_mut() {
            s.n_angle = 99;
            s.flags = 0xFF;
            s.dist = Fx::from_int(7);
        }
        p.kill(h as usize);
        // 复用同一个槽（最低空位分配 ⇒ 必然是它）
        let h2 = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        assert_eq!(h2, h, "最低空位分配应复用同一槽");
        for (k, s) in p.shooters[h2 as usize].iter().enumerate() {
            assert_eq!(*s, ShooterSlot::default(), "槽 {k} 未被重置为默认值");
        }
    }

    #[test]
    fn kill_frees_slot_for_reuse() {
        let mut p = TaskPool::new();
        let a = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        p.kill(a as usize);
        assert!(!p.is_alive(a as usize));
        let b = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        assert_eq!(b, a, "还槽后复用最低位");
    }

    /// C12⑤ 复审修复"`KILL_CHILDREN` 无代际戳"：`kill()` 必须顺手清空存活子任务的
    /// `parent` 引用——本池无逐槽 generation（模块文档"手写特例"），故用"父死的瞬间
    /// 断开亲子关系"代替代际戳，关掉"槽复用后新占用者继承前任孤儿"的窗口（端到端复现见
    /// `ecl::vm::tests::kill_children_does_not_kill_a_reused_slots_previous_orphans`）。
    #[test]
    fn kill_detaches_surviving_children_parent_pointer() {
        let mut p = TaskPool::new();
        let parent = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        let child = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), parent + 1, 0)
            .unwrap();
        p.kill(parent as usize);
        assert!(
            p.is_alive(child as usize),
            "子任务本身不受父死牵连（detached 语义，不递归杀）"
        );
        assert_eq!(
            p.slots[child as usize].parent, 0,
            "父死后子任务的 parent 引用应被立即清空，不能悬挂指向已死槽号"
        );
    }

    #[test]
    fn iter_alive_ascending_matches_alive_bits() {
        let mut p = TaskPool::new();
        let a = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        let b = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        p.kill(a as usize);
        let c = p
            .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
            .unwrap();
        assert_eq!(c, a, "复用最低位");
        let alive: Vec<usize> = p.iter_alive().collect();
        assert_eq!(alive, vec![a as usize, b as usize], "升序遍历（I4）");
    }

    #[test]
    fn pool_full_returns_none() {
        let mut p = TaskPool::new();
        for k in 0..TASK_CAP {
            assert!(
                p.spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
                    .is_some(),
                "第 {k} 个应成功"
            );
        }
        assert!(
            p.spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
                .is_none(),
            "256 个耗尽"
        );
    }

    #[test]
    fn copy_into_full_roundtrip_and_checksum_matches() {
        let mut p = TaskPool::new();
        let a = p
            .spawn(SubId::default(), 0, (OWNER_ENEMY, 3, 1), 0, 5)
            .unwrap();
        p.slots[a as usize].locals[10] = 77;
        let mut dst = TaskPool::new();
        p.copy_into(&mut dst);
        assert_eq!(dst.checksum(), p.checksum());
        assert_eq!(dst.slots[a as usize].locals[10], 77);
        assert!(dst.is_alive(a as usize));
    }

    /// P6 判别腿：任意槽字节（哪怕从未 spawn 过的槽）入校验和——"只哈希占用槽"的变异体在此维度
    /// 无从分辨（xform.rs `checksum_sensitive_to_any_slot_byte_and_occupancy` 同款判别）。
    #[test]
    fn checksum_sensitive_to_unoccupied_slot_byte() {
        let p0 = TaskPool::new();
        let base = p0.checksum();
        let mut p1 = TaskPool::new();
        p1.slots[5].locals[0] = 42; // 槽 5 从未 spawn
        assert_ne!(p1.checksum(), base, "未占用槽的字节也必须入哈希");
    }
}
