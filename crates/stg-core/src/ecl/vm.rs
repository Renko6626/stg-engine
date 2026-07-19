//! ecl/vm.rs —— 单任务解释核 + 相位 2 调度租户（T2：`SPAWN`/`KILL_SELF`/`KILL_CHILDREN`
//! 语义落地；T3 起 `SYS` 派发进 `ecl::syscall::dispatch`——`VmCtx` 相应扩出
//! `body: &mut WorldBody` + `tables: &WorldTables`，取代原先单独的 `diag` 字段）。
//!
//! 指令编码：1 头字（opcode 在低 8 位，余位留白供未来 mask）+ N 操作数字（N 由
//! `ops::ARITY` 钉死）。循环：预算门（任务 1024 + 全局）→ 取头字（pc 越界 Fault(1)）→
//! op 低 8 位 → `op_implemented`? → 按 `ARITY` 取操作数字（越界 Fault(1)）→ 语义分派 →
//! `WAIT` 写 `wait` 并 `Yield` / `END` → `End`。
//!
//! `run_tasks` 是相位 2（`PH_DIRECTOR`）导演槽的默认租户（P2 既定）：升序遍历
//! `World.tasks`，owner 门禁 → 次帧首跑门禁 → wait 门禁 → 全局预算门禁 → `exec`。
#![allow(dead_code)]

use crate::ecl::image::EclImage;
use crate::ecl::ops::{self, ARITY};
use crate::ecl::syscall;
use crate::ecl::task::{
    CALL_DEPTH, EVAL_DEPTH, LOCALS, OWNER_BULLET, OWNER_ENEMY, OWNER_STAGE, TASK_CAP, Task,
    TaskPool,
};
use crate::events::{EVT_TASK_FAULT, Event};
use crate::math::Angle;
use crate::math::Fx;
use crate::math::trig;
use crate::tables::WorldTables;
use crate::world::{POOL_TASK, WorldBody};

/// 单任务每帧指令预算（spec 拍板 2：双层，超限确定性杀）。
pub const TASK_BUDGET: u32 = 1024;

/// 全局每帧指令预算（跨任务共享层，按池索引升序消耗，I4：两机饿死同一批）。
pub(crate) const GLOBAL_BUDGET: u32 = 65536;

// ── Fault 码（五类既定 + 一类 T1 占位；见 brief）───────────────────────────
/// 未知 op / `op_implemented` 判否。
pub const FAULT_BAD_OP: u8 = 0;
/// pc 或跳转目标越界（含"取操作数字越出 code 边界"的截断指令）。
pub const FAULT_PC_OOB: u8 = 1;
/// 求值栈上溢或下溢。
pub const FAULT_STACK: u8 = 2;
/// 指令预算耗尽（任务 1024 或全局余额先到者）。
pub const FAULT_BUDGET: u8 = 3;
/// 除零（`DIV`/`MOD`/`DIVF`）。
pub const FAULT_DIV_ZERO: u8 = 4;
/// 调用深度超限（`CALL` 满 8 层再调用）或 `RET` 时调用栈已空。
pub const FAULT_CALL_DEPTH: u8 = 5;
/// **T3 起保留但不再产出**：T1/T2 占位期 `SYS` 未接线时曾走这里；T3 起 `SYS` 派发进
/// `ecl::syscall::dispatch`（坏 syscall 号复用 `FAULT_BAD_OP`，见该模块文档），本码不再由
/// 任何路径产出。保留常量值（不重排）——fault 码"编号即契约"，占位过的号不回收复用。
pub const FAULT_UNIMPLEMENTED: u8 = 6;

/// 单次 `exec` 调用的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exec {
    Yield,
    End,
    Fault(u8),
}

/// 单帧执行上下文：本任务将要解码的字节码 + 全局剩余预算（跨任务共享，按池序消耗，I4）+
/// `SPAWN`/`KILL_CHILDREN` 所需的任务池句柄 + 脚本镜像 + 本任务自身索引/当前帧号 +
/// 世界本体（`SYS` 派发读写世界状态/诊断计数器，M1 T3 起扩）+ 静态表（appearance 查表等）。
/// 调度层（`run_tasks`）构造；单元测试也可直接手搭（`tasks`/`ecl`/`body`/`tables` 用最小
/// 占位值——测试惯例：`World::new(seed)` 提供 `body`/`tasks`，`&TABLES_V0` 提供 `tables`）。
pub(crate) struct VmCtx<'a> {
    pub code: &'a [u32],
    pub budget: &'a mut u32,
    /// 任务池——`SPAWN` 分配新槽、`KILL_CHILDREN` 扫描直系子、`SYS_CREATE_BULLET` 的
    /// `task_script` 挂弹派任务同走此路（T3 起）。**不含当前正在执行的任务**
    /// 的最新状态（那份状态在调用方的局部 `task` 拷贝里，见 `run_tasks` 的 copy-out/
    /// copy-back）；池内自身槽仍是本轮开始前的旧值，但 `spawn`/`kill_children` 只触碰
    /// *其它* 槽（自身槽的 alive 位在本轮全程保持置位，`first_free` 天然跳过），无别名冲突。
    pub tasks: &'a mut TaskPool,
    /// 脚本镜像——`SPAWN`/`SYS_CREATE_BULLET` 靠它把 `script id` 解析成子任务的入口 `pc`。
    pub ecl: &'a EclImage,
    /// 世界本体（M1 T3 起扩）：`SYS` 派发读写弹/敌/道具/globals/boss_ui/rng/诊断计数器
    /// 的唯一入口，取代原先单独的 `diag: &mut DiagCounters` 字段（`ctx.body.diag` 等价物）。
    pub body: &'a mut WorldBody,
    /// 静态只读表（appearance 查表等；T3 起随 `&WorldTables` 穿线同款惯例）。
    pub tables: &'a WorldTables,
    /// 本任务在池中的索引（`SPAWN` 的 `parent` 戳 = 此值+1；`KILL_CHILDREN` 的扫描目标同）。
    pub self_index: u16,
    /// 当前世界帧号（`SPAWN`/`SYS_CREATE_BULLET` 子任务的 `born_frame` 戳）。
    pub frame: u32,
}

/// 从 `task.pc` 起解释执行，直到 `WAIT` 让出 / `END` 完成 / Fault。
pub(crate) fn exec(task: &mut Task, ctx: &mut VmCtx) -> Exec {
    let mut task_count: u32 = 0;
    loop {
        // 预算门：任务内部 1024 + 全局余额，谁先到算谁——**恰在第 1025 条指令处报错**
        // （前 1024 条已如常执行完毕，边界腿见 vm.rs 测试）。
        if task_count >= TASK_BUDGET || *ctx.budget == 0 {
            return Exec::Fault(FAULT_BUDGET);
        }
        task_count += 1;
        *ctx.budget -= 1;

        let pc = task.pc as usize;
        let Some(&head) = ctx.code.get(pc) else {
            return Exec::Fault(FAULT_PC_OOB);
        };
        let op = (head & 0xFF) as u8;
        if !ops::op_implemented(op) {
            return Exec::Fault(FAULT_BAD_OP);
        }
        let arity = ARITY[op as usize] as usize;
        let opnd_start = pc + 1;
        let opnd_end = opnd_start + arity;
        if opnd_end > ctx.code.len() {
            return Exec::Fault(FAULT_PC_OOB); // 截断指令：操作数字越出 code 边界
        }
        let next_pc = opnd_end as u32; // 顺序落点；分支类 op 会显式覆写 task.pc 并 continue

        macro_rules! pop {
            () => {{
                if task.sp == 0 {
                    return Exec::Fault(FAULT_STACK);
                }
                task.sp -= 1;
                task.stack[task.sp as usize]
            }};
        }
        macro_rules! push {
            ($v:expr) => {{
                if task.sp as usize >= EVAL_DEPTH {
                    return Exec::Fault(FAULT_STACK);
                }
                task.stack[task.sp as usize] = $v;
                task.sp += 1;
            }};
        }

        match op {
            ops::OP_END => return Exec::End,
            ops::OP_WAIT => {
                let frames = pop!();
                task.wait = frames as u16;
                task.pc = next_pc;
                return Exec::Yield;
            }
            ops::OP_JMP => {
                task.pc = ctx.code[opnd_start];
                continue;
            }
            ops::OP_JZ => {
                let target = ctx.code[opnd_start];
                let v = pop!();
                task.pc = if v == 0 { target } else { next_pc };
                continue;
            }
            ops::OP_CALL => {
                if task.csp as usize >= CALL_DEPTH {
                    return Exec::Fault(FAULT_CALL_DEPTH);
                }
                let target = ctx.code[opnd_start];
                task.calls[task.csp as usize] = next_pc;
                task.csp += 1;
                task.pc = target;
                continue;
            }
            ops::OP_RET => {
                if task.csp == 0 {
                    return Exec::Fault(FAULT_CALL_DEPTH);
                }
                task.csp -= 1;
                task.pc = task.calls[task.csp as usize];
                continue;
            }
            ops::OP_PUSHI => {
                let imm = ctx.code[opnd_start] as i32;
                push!(imm);
            }
            ops::OP_PUSHL => {
                let idx = ctx.code[opnd_start] as usize;
                if idx >= LOCALS {
                    return Exec::Fault(FAULT_BAD_OP);
                }
                push!(task.locals[idx]);
            }
            ops::OP_POPL => {
                let idx = ctx.code[opnd_start] as usize;
                if idx >= LOCALS {
                    return Exec::Fault(FAULT_BAD_OP);
                }
                let v = pop!();
                task.locals[idx] = v;
            }
            ops::OP_DUP => {
                if task.sp == 0 {
                    return Exec::Fault(FAULT_STACK);
                }
                let v = task.stack[task.sp as usize - 1];
                push!(v);
            }
            ops::OP_POP => {
                pop!();
            }
            ops::OP_ADD => {
                let b = pop!();
                let a = pop!();
                push!(a.wrapping_add(b));
            }
            ops::OP_SUB => {
                let b = pop!();
                let a = pop!();
                push!(a.wrapping_sub(b));
            }
            ops::OP_MUL => {
                let b = pop!();
                let a = pop!();
                push!(a.wrapping_mul(b));
            }
            ops::OP_DIV => {
                let b = pop!();
                let a = pop!();
                if b == 0 {
                    return Exec::Fault(FAULT_DIV_ZERO);
                }
                push!(a.wrapping_div(b));
            }
            ops::OP_MOD => {
                let b = pop!();
                let a = pop!();
                if b == 0 {
                    return Exec::Fault(FAULT_DIV_ZERO);
                }
                push!(a.wrapping_rem(b));
            }
            ops::OP_NEG => {
                let a = pop!();
                push!(a.wrapping_neg());
            }
            ops::OP_MULF => {
                // Q16.16 乘：i64 中间量 >>16 归一化回 Q16.16（CLAUDE.md 定点乘法规范）。
                let b = pop!();
                let a = pop!();
                let p = ((a as i64 * b as i64) >> 16) as i32;
                push!(p);
            }
            ops::OP_DIVF => {
                let b = pop!();
                let a = pop!();
                if b == 0 {
                    return Exec::Fault(FAULT_DIV_ZERO);
                }
                let q = ((a as i64) << 16) / b as i64;
                push!(q as i32);
            }
            ops::OP_SINB => {
                let a = pop!();
                let bam = (a as u32 & 0xFFFF) as u16;
                push!(trig::sin(Angle(bam)).raw());
            }
            ops::OP_COSB => {
                let a = pop!();
                let bam = (a as u32 & 0xFFFF) as u16;
                push!(trig::cos(Angle(bam)).raw());
            }
            ops::OP_EQ => {
                let b = pop!();
                let a = pop!();
                push!((a == b) as i32);
            }
            ops::OP_NE => {
                let b = pop!();
                let a = pop!();
                push!((a != b) as i32);
            }
            ops::OP_LT => {
                let b = pop!();
                let a = pop!();
                push!((a < b) as i32);
            }
            ops::OP_LE => {
                let b = pop!();
                let a = pop!();
                push!((a <= b) as i32);
            }
            ops::OP_GT => {
                let b = pop!();
                let a = pop!();
                push!((a > b) as i32);
            }
            ops::OP_GE => {
                let b = pop!();
                let a = pop!();
                push!((a >= b) as i32);
            }
            ops::OP_SPAWN => {
                // 操作数 = script id（立即数内联，不弹栈）+ argc（M1.9 T3：表层语言
                // `spawn f(args)` 传参地基）。**拍板顺序**：argc 上限门 → 脚本号在册门 →
                // 父栈够不够门——三门都过才真的弹栈+spawn（早失败早止损，任何一门不过
                // 都零副作用：不碰父栈、不占任务池槽）。
                //
                // **弹栈约定（与编译器 pin 死的 ABI）**：编译器把实参按**声明顺序正序压栈**
                // （arg0 先压…argN 后压，栈顶 = 最后一个实参），故这里必须**逆序**弹出——
                // 第一次 pop 拿到的是 argN（栈顶），落进 `args[argc-1]`；最后一次 pop 拿到
                // 的是 arg0，落进 `args[0]`——弹完 `args[0..argc)` 才是声明序，与
                // `SubBuilder::spawn`/表层 codegen 的压栈序严格配对（同 syscall "正序压栈、
                // 逆序弹出"惯例的同款镜像）。
                let script = ctx.code[opnd_start] as u16;
                let argc = ctx.code[opnd_start + 1] as usize;
                if argc > LOCALS {
                    return Exec::Fault(FAULT_STACK);
                }
                let Some(pc0) = ctx.ecl.entry(script) else {
                    // 坏脚本号：同 PUSHL/POPL 越界处置口径，复用 FAULT_BAD_OP。
                    return Exec::Fault(FAULT_BAD_OP);
                };
                if (task.sp as usize) < argc {
                    // 父栈不够 argc 个值：确定性拒绝，复用 FAULT_STACK（同求值栈上溢下溢口径）。
                    return Exec::Fault(FAULT_STACK);
                }
                let mut args = [0i32; LOCALS];
                for k in (0..argc).rev() {
                    args[k] = pop!();
                }
                let owner = (task.owner_kind, task.owner_index, task.owner_gen);
                let parent = ctx.self_index + 1;
                match ctx.tasks.spawn(script, pc0, owner, parent, ctx.frame) {
                    Some(idx) => {
                        // 子任务 locals 已被 TaskPool::spawn 全零初始化（复用槽写满纪律）——
                        // 只需覆写 [0..argc) 段，argc=0 时这是 no-op（金向量两段不变的地基）。
                        ctx.tasks.slots[idx as usize].locals[..argc].copy_from_slice(&args[..argc]);
                        push!(idx as i32)
                    }
                    None => {
                        push!(-1);
                        ctx.body.diag.pool_full[POOL_TASK] =
                            ctx.body.diag.pool_full[POOL_TASK].wrapping_add(1);
                    }
                }
            }
            ops::OP_KILL_SELF => return Exec::End, // “End-like”：调度层按 End 统一收尸
            ops::OP_KILL_CHILDREN => {
                // 升序扫描：parent == 自己池索引+1 者杀（只杀直系一层，不递归——detached
                // 语义下孙辈是"别的任务的直系子"，与本任务无关）。
                let target_parent = ctx.self_index + 1;
                for j in 0..TASK_CAP {
                    if ctx.tasks.is_alive(j) && ctx.tasks.slots[j].parent == target_parent {
                        ctx.tasks.kill(j);
                    }
                }
            }
            ops::OP_SYS => {
                let no = ctx.code[opnd_start] as u16;
                if let Err(code) = syscall::dispatch(no, task, ctx) {
                    return Exec::Fault(code);
                }
            }
            _ => return Exec::Fault(FAULT_BAD_OP), // 防御：op_implemented 与本 match 若失步，不 panic
        }
        task.pc = next_pc;
    }
}

/// 组一条 `EVT_TASK_FAULT` 事件（`a_index` = 任务池索引，`data = [fault_code, script]`，
/// 见 `events.rs` 文档）。
fn fault_event(task_index: u16, fault_code: u8, script: u16) -> Event {
    Event {
        kind: EVT_TASK_FAULT,
        a_index: task_index,
        a_gen: 0,
        x: Fx::ZERO,
        y: Fx::ZERO,
        data: [fault_code as i32, script as i32],
    }
}

/// 相位 2（`PH_DIRECTOR`）导演槽的默认租户（P2 既定）——组装层 `step_with_director` 在
/// 注入的导演闭包**之前**调用本函数（"二者共存"，spec 既定）。
///
/// 升序遍历 `tasks`（I4）：
///
/// 1. **owner 门禁**：`OWNER_STAGE` 恒过；`OWNER_ENEMY`/`OWNER_BULLET` 查对应池
///    `is_alive + generation` 是否仍与任务出生时一致，不符 → **静默杀**（不发 `EVT_TASK_FAULT`，
///    不计 `task_faults`——owner 死是常态非错误，不是 bug）。
/// 2. **次帧首跑门禁**：`born_frame == frame` → 跳过（出生当帧不跑）。
/// 3. **wait 门禁**：`wait > 0` → 递减 1、跳过（不消耗预算）。
/// 4. **全局预算门禁**（本刀设计决策，T2 无先例——见下）。
/// 5. `exec`：`Yield` 写回、`End`/`Fault` 杀（`Fault` 额外发事件 + 计数）。
///
/// **"没轮到"与"自己撞墙"的边界**：进 `exec` 前若全局预算已耗尽为 0，视同调度层门禁
/// （同 `wait`）——静默跳过、任务状态原封不动、次帧满血重跑，**不产生 `Fault`、不杀**：
/// 这是"这一帧没轮到它"，不是它的错。但凡任务真正进了 `exec`（预算 >0 起跑），期间不论撞上
/// 自己的 1024 上限、还是把共享池撞到 0，都按既有语义 `Exec::Fault(FAULT_BUDGET)`——响亮地杀 +
/// 事件 + 计数（"死循环是作者 bug"，两机确定性撞在同一批任务上，I4）。两态用"进 `exec` 前
/// `budget` 是否已经是 0"一刀切分，`exec` 不必上报"这次到底跑了几条"。
pub(crate) fn run_tasks(
    tasks: &mut TaskPool,
    body: &mut WorldBody,
    ecl: &EclImage,
    tables: &WorldTables,
) {
    let frame = body.frame;
    let mut budget = GLOBAL_BUDGET;

    for i in 0..TASK_CAP {
        if !tasks.is_alive(i) {
            continue;
        }
        let mut t = tasks.slots[i];

        if t.owner_kind != OWNER_STAGE {
            let alive = match t.owner_kind {
                OWNER_ENEMY => {
                    body.enemies.is_alive(t.owner_index as usize)
                        && body.enemies.generation[t.owner_index as usize] == t.owner_gen
                }
                OWNER_BULLET => {
                    body.bullets.is_alive(t.owner_index as usize)
                        && body.bullets.generation[t.owner_index as usize] == t.owner_gen
                }
                _ => false, // 未知 owner_kind：不应由合法路径产生，视同悬垂静默回收
            };
            if !alive {
                tasks.kill(i);
                continue;
            }
        }

        if t.born_frame == frame {
            continue;
        }

        if t.wait > 0 {
            t.wait -= 1;
            tasks.slots[i] = t;
            continue;
        }

        if budget == 0 {
            continue; // 本帧没轮到：不是它的错，次帧满血重跑（不 Fault，见函数文档）
        }

        let Some(pc0) = ecl.entry(t.script) else {
            // 脚本号已不在册（画面外情形——正常 spawn 路径已在创建时校验，这里是防御）：
            // 视同确定性坏行为，杀 + 事件 + 计数。
            tasks.kill(i);
            body.diag.task_faults = body.diag.task_faults.wrapping_add(1);
            body.push_event(fault_event(i as u16, FAULT_BAD_OP, t.script));
            continue;
        };
        let _ = pc0; // 入口只在 spawn 时戳一次 pc；此处仅确认脚本仍在册，不重置 pc（履历续跑）。

        let mut ctx = VmCtx {
            code: &ecl.code,
            budget: &mut budget,
            tasks: &mut *tasks,
            ecl,
            body: &mut *body,
            tables,
            self_index: i as u16,
            frame,
        };
        // 三分支都先把本轮执行到的最终状态写回槽位，再决定是否 `kill`——`kill` 只翻
        // alive 位，不清字节（同 xform.rs/其它池先例）；死后的槽仍应是"最后一次真实执行
        // 到的状态"（校验和忠实反映模拟发生了什么），而不是执行前的陈值。
        match exec(&mut t, &mut ctx) {
            Exec::Yield => tasks.slots[i] = t,
            Exec::End => {
                tasks.slots[i] = t;
                tasks.kill(i);
            }
            Exec::Fault(code) => {
                tasks.slots[i] = t;
                tasks.kill(i);
                body.diag.task_faults = body.diag.task_faults.wrapping_add(1);
                body.push_event(fault_event(i as u16, code, t.script));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::ops::*;

    /// 测试专用最小世界（`World::new` 提供 `body`/`tasks`；`&TABLES_V0` 提供静态表）——
    /// T3 起 `VmCtx` 扩出 `body`/`tables`，测试构造从"裸 `TaskPool`+`DiagCounters`"改走此路。
    fn test_world() -> Box<crate::step::World> {
        crate::step::World::new(1)
    }

    /// 通用 op 走格用：空任务池 + 空镜像（不涉及 `SPAWN`/`KILL_CHILDREN` 的测试用它即可）。
    fn run(code: &[u32]) -> (Exec, Task) {
        let mut task = Task::default();
        let mut budget = u32::MAX;
        let mut w = test_world();
        let ecl = EclImage::empty();
        let mut ctx = VmCtx {
            code,
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        (r, task)
    }

    #[test]
    fn pushi_and_end() {
        let (r, t) = run(&[OP_PUSHI as u32, 42, OP_END as u32]);
        assert_eq!(r, Exec::End);
        assert_eq!(t.sp, 1);
        assert_eq!(t.stack[0], 42);
    }

    /// WAIT 截断语义钉死（M1 终审 Minor）：取栈顶低 16 位——wait(-1)=65535 帧、
    /// wait(65536)=0 帧。作者契约入 ecl-ops.md，此测试防"改成饱和/报错"的无声语义变化。
    #[test]
    fn wait_truncates_to_low_16_bits() {
        let (r, t) = run(&[OP_PUSHI as u32, -1i32 as u32, OP_WAIT as u32, OP_END as u32]);
        assert_eq!(r, Exec::Yield);
        assert_eq!(t.wait, 65535, "wait(-1) 截断为 65535");
        let (r, t) = run(&[OP_PUSHI as u32, 65536u32, OP_WAIT as u32, OP_END as u32]);
        assert_eq!(r, Exec::Yield);
        assert_eq!(t.wait, 0, "wait(65536) 截断为 0");
    }

    /// wrapping 语义钉死（T1 复审 Important）：`i32::MIN / -1`（Rust 裸 `/` 在 release
    /// 也 panic 的经典向量）与 MOD/NEG 同族边界——必须回绕不 Fault 不 panic。此测试是
    /// "有人把 wrapping_div 改回 `/`" 的 CI 闸门（debug 跑测试即炸）。
    #[test]
    fn arith_wrapping_edge_min_over_neg_one() {
        let (r, t) = run(&[
            OP_PUSHI as u32,
            i32::MIN as u32,
            OP_PUSHI as u32,
            -1i32 as u32,
            OP_DIV as u32,
            OP_END as u32,
        ]);
        assert_eq!(r, Exec::End, "MIN/-1 不得 Fault");
        assert_eq!(t.stack[0], i32::MIN, "wrapping_div 回绕语义");
        let (r, t) = run(&[
            OP_PUSHI as u32,
            i32::MIN as u32,
            OP_PUSHI as u32,
            -1i32 as u32,
            OP_MOD as u32,
            OP_END as u32,
        ]);
        assert_eq!(r, Exec::End);
        assert_eq!(t.stack[0], 0, "wrapping_rem(MIN,-1) = 0");
        let (r, t) = run(&[
            OP_PUSHI as u32,
            i32::MIN as u32,
            OP_NEG as u32,
            OP_END as u32,
        ]);
        assert_eq!(r, Exec::End);
        assert_eq!(t.stack[0], i32::MIN, "wrapping_neg(MIN) = MIN");
    }

    #[test]
    fn integer_arith_family() {
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 3, OP_ADD as u32]);
        assert_eq!(t.stack[0], 10);
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 3, OP_SUB as u32]);
        assert_eq!(t.stack[0], 4);
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 3, OP_MUL as u32]);
        assert_eq!(t.stack[0], 21);
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 3, OP_DIV as u32]);
        assert_eq!(t.stack[0], 2);
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 3, OP_MOD as u32]);
        assert_eq!(t.stack[0], 1);
        let (_, t) = run(&[OP_PUSHI as u32, 7, OP_NEG as u32]);
        assert_eq!(t.stack[0], -7);
    }

    #[test]
    fn div_and_mod_and_divf_by_zero_fault() {
        let (r, _) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 0, OP_DIV as u32]);
        assert_eq!(r, Exec::Fault(FAULT_DIV_ZERO));
        let (r, _) = run(&[OP_PUSHI as u32, 7, OP_PUSHI as u32, 0, OP_MOD as u32]);
        assert_eq!(r, Exec::Fault(FAULT_DIV_ZERO));
        let (r, _) = run(&[OP_PUSHI as u32, 65536, OP_PUSHI as u32, 0, OP_DIVF as u32]);
        assert_eq!(r, Exec::Fault(FAULT_DIV_ZERO));
    }

    #[test]
    fn mulf_and_divf_q16_16_semantics() {
        // 2.0 * 3.0 = 6.0
        let (_, t) = run(&[
            OP_PUSHI as u32,
            Fx::from_int(2).raw() as u32,
            OP_PUSHI as u32,
            Fx::from_int(3).raw() as u32,
            OP_MULF as u32,
        ]);
        assert_eq!(t.stack[0], Fx::from_int(6).raw());
        // 6.0 / 3.0 = 2.0
        let (_, t) = run(&[
            OP_PUSHI as u32,
            Fx::from_int(6).raw() as u32,
            OP_PUSHI as u32,
            Fx::from_int(3).raw() as u32,
            OP_DIVF as u32,
        ]);
        assert_eq!(t.stack[0], Fx::from_int(2).raw());
    }

    #[test]
    fn sinb_cosb_use_math_trig_tables() {
        // sin(QUARTER=π/2) == 1.0
        let (_, t) = run(&[OP_PUSHI as u32, 16384, OP_SINB as u32]);
        assert_eq!(t.stack[0], Fx::ONE.raw());
        // cos(0) == 1.0
        let (_, t) = run(&[OP_PUSHI as u32, 0, OP_COSB as u32]);
        assert_eq!(t.stack[0], Fx::ONE.raw());
    }

    #[test]
    fn compare_ops_push_zero_or_one() {
        // 3 <op> 5
        let cases: [(u8, i32); 6] = [
            (OP_LT, 1),
            (OP_GT, 0),
            (OP_EQ, 0),
            (OP_NE, 1),
            (OP_LE, 1),
            (OP_GE, 0),
        ];
        for (op, expect) in cases {
            let (_, t) = run(&[OP_PUSHI as u32, 3, OP_PUSHI as u32, 5, op as u32]);
            assert_eq!(t.stack[0], expect, "op {op} 3 vs 5 应为 {expect}");
        }
    }

    #[test]
    fn dup_and_pop_semantics() {
        let (_, t) = run(&[OP_PUSHI as u32, 42, OP_DUP as u32]);
        assert_eq!(t.sp, 2);
        assert_eq!((t.stack[0], t.stack[1]), (42, 42));
        let (_, t) = run(&[OP_PUSHI as u32, 1, OP_PUSHI as u32, 2, OP_POP as u32]);
        assert_eq!(t.sp, 1);
        assert_eq!(t.stack[0], 1);
    }

    #[test]
    fn dup_and_pop_on_empty_stack_fault() {
        let (r, _) = run(&[OP_DUP as u32]);
        assert_eq!(r, Exec::Fault(FAULT_STACK));
        let (r, _) = run(&[OP_POP as u32]);
        assert_eq!(r, Exec::Fault(FAULT_STACK));
    }

    #[test]
    fn jmp_unconditional_skips_to_target() {
        // idx0 JMP->4；idx2 PUSHI 999（跳过）；idx4 PUSHI 5；idx6 END
        let code = [
            OP_JMP as u32,
            4,
            OP_PUSHI as u32,
            999,
            OP_PUSHI as u32,
            5,
            OP_END as u32,
        ];
        let (r, t) = run(&code);
        assert_eq!(r, Exec::End);
        assert_eq!(t.sp, 1);
        assert_eq!(t.stack[0], 5);
    }

    #[test]
    fn jz_branches_on_zero_and_falls_through_otherwise() {
        // idx0 PUSHI v；idx2 JZ->7；idx4 PUSHI 111；idx6 END；idx7 PUSHI 222；idx9 END
        fn jz_program(v: i32) -> [u32; 10] {
            [
                OP_PUSHI as u32,
                v as u32,
                OP_JZ as u32,
                7,
                OP_PUSHI as u32,
                111,
                OP_END as u32,
                OP_PUSHI as u32,
                222,
                OP_END as u32,
            ]
        }
        let (r, t) = run(&jz_program(0));
        assert_eq!(r, Exec::End);
        assert_eq!(t.stack[0], 222, "零值跳转到 target");
        let (r, t) = run(&jz_program(7));
        assert_eq!(r, Exec::End);
        assert_eq!(t.stack[0], 111, "非零落到 fallthrough");
    }

    #[test]
    fn wait_yields_with_precise_wait_value_and_pc() {
        // idx0 PUSHI 30；idx2 WAIT；idx3 PUSHI 99；idx5 END
        let code = [
            OP_PUSHI as u32,
            30,
            OP_WAIT as u32,
            OP_PUSHI as u32,
            99,
            OP_END as u32,
        ];
        let mut task = Task::default();
        let mut budget = u32::MAX;
        let mut w = test_world();
        let ecl = EclImage::empty();
        let mut ctx = VmCtx {
            code: &code,
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Yield);
        assert_eq!(task.wait, 30);
        assert_eq!(task.pc, 3, "pc 落在 WAIT 之后的下一条指令");
        assert_eq!(task.sp, 0, "WAIT 消费了栈顶帧数");
        // 续跑：resume 后应从 pc=3 继续执行完剩余指令
        let r2 = exec(&mut task, &mut ctx);
        assert_eq!(r2, Exec::End);
        assert_eq!(task.stack[0], 99);
    }

    #[test]
    fn call_ret_roundtrip_shares_locals_across_call() {
        // main: idx0 PUSHI 5；idx2 POPL 0；idx4 CALL 9；idx6 PUSHL 0；idx8 END
        // sub : idx9 PUSHL 0；idx11 PUSHI 1；idx13 ADD；idx14 POPL 0；idx16 RET
        let code = [
            OP_PUSHI as u32,
            5, // 0,1
            OP_POPL as u32,
            0, // 2,3
            OP_CALL as u32,
            9, // 4,5
            OP_PUSHL as u32,
            0,             // 6,7
            OP_END as u32, // 8
            OP_PUSHL as u32,
            0, // 9,10
            OP_PUSHI as u32,
            1,             // 11,12
            OP_ADD as u32, // 13
            OP_POPL as u32,
            0,             // 14,15
            OP_RET as u32, // 16
        ];
        let (r, t) = run(&code);
        assert_eq!(r, Exec::End);
        assert_eq!(t.sp, 1);
        assert_eq!(
            t.stack[0], 6,
            "sub 内写 locals[0]=6，main RET 后读到的是新值（locals 任务全局共享）"
        );
        assert_eq!(t.locals[0], 6);
    }

    #[test]
    fn call_depth_exceeded_at_9th_call() {
        // 自递归 CALL：前 8 层成功压栈，第 9 层触发 Fault(5)。
        let code = [OP_CALL as u32, 0];
        let (r, t) = run(&code);
        assert_eq!(r, Exec::Fault(FAULT_CALL_DEPTH));
        assert_eq!(t.csp as usize, CALL_DEPTH, "恰用满 8 层后第 9 层拒");
    }

    #[test]
    fn ret_on_empty_call_stack_faults() {
        let (r, _) = run(&[OP_RET as u32]);
        assert_eq!(r, Exec::Fault(FAULT_CALL_DEPTH));
    }

    fn n_pushi(n: usize, v: i32) -> Vec<u32> {
        let mut code = Vec::with_capacity(n * 2 + 1);
        for _ in 0..n {
            code.push(OP_PUSHI as u32);
            code.push(v as u32);
        }
        code.push(OP_END as u32);
        code
    }

    #[test]
    fn eval_stack_full_at_32_overflow_at_33rd_push() {
        let (r, t) = run(&n_pushi(32, 1));
        assert_eq!(r, Exec::End, "恰 32 push 灌满不溢出");
        assert_eq!(t.sp as usize, EVAL_DEPTH);

        let (r, _) = run(&n_pushi(33, 1));
        assert_eq!(r, Exec::Fault(FAULT_STACK), "第 33 push 溢出");
    }

    fn jmp_chain_then_end(n_jmps: usize) -> Vec<u32> {
        let mut code = Vec::with_capacity(n_jmps * 2 + 1);
        for i in 0..n_jmps {
            code.push(OP_JMP as u32);
            code.push(((i + 1) * 2) as u32);
        }
        code.push(OP_END as u32);
        code
    }

    #[test]
    fn budget_boundary_1024_ok_1025_faults() {
        // 1023 条 JMP + 1 条 END = 恰 1024 条指令，不该 Fault。
        let code_ok = jmp_chain_then_end(1023);
        let (r, _) = run(&code_ok);
        assert_eq!(r, Exec::End, "恰 1024 条整不 Fault");

        // 1024 条 JMP + 1 条 END = 1025 条指令，第 1025 条（END）恰好触发 Fault(3)。
        let code_fault = jmp_chain_then_end(1024);
        let (r, _) = run(&code_fault);
        assert_eq!(r, Exec::Fault(FAULT_BUDGET), "第 1025 条恰 Fault(3)");
    }

    #[test]
    fn global_budget_exhaustion_also_faults() {
        // 自跳转死循环：全局预算 5 应先于任务 1024 上限触发。
        let code = [OP_JMP as u32, 0];
        let mut task = Task::default();
        let mut budget = 5u32;
        let mut w = test_world();
        let ecl = EclImage::empty();
        let mut ctx = VmCtx {
            code: &code,
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Fault(FAULT_BUDGET));
        assert_eq!(budget, 0);
    }

    #[test]
    fn unknown_op_faults() {
        let (r, _) = run(&[255u32]);
        assert_eq!(r, Exec::Fault(FAULT_BAD_OP));
    }

    #[test]
    fn pc_out_of_bounds_on_empty_code_faults() {
        let (r, _) = run(&[]);
        assert_eq!(r, Exec::Fault(FAULT_PC_OOB));
    }

    #[test]
    fn truncated_operand_is_pc_oob_fault() {
        // PUSHI 元数 1，但 code 只有头字没有操作数字。
        let (r, _) = run(&[OP_PUSHI as u32]);
        assert_eq!(r, Exec::Fault(FAULT_PC_OOB));
    }

    #[test]
    fn pushl_popl_bad_index_faults() {
        let (r, _) = run(&[OP_PUSHL as u32, LOCALS as u32]);
        assert_eq!(r, Exec::Fault(FAULT_BAD_OP));
        let (r, _) = run(&[OP_PUSHI as u32, 1, OP_POPL as u32, LOCALS as u32]);
        assert_eq!(r, Exec::Fault(FAULT_BAD_OP));
    }

    #[test]
    fn pushl_reads_locals_and_popl_writes_them() {
        let mut task = Task::default();
        task.locals[3] = 77;
        let mut budget = u32::MAX;
        let mut w = test_world();
        let ecl = EclImage::empty();
        let mut ctx = VmCtx {
            code: &[OP_PUSHL as u32, 3],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Fault(FAULT_PC_OOB)); // 无 END：code 耗尽后下一次 fetch 越界
        assert_eq!(task.stack[0], 77);

        let mut task2 = Task::default();
        let mut budget2 = u32::MAX;
        let mut w2 = test_world();
        let ecl2 = EclImage::empty();
        let mut ctx2 = VmCtx {
            code: &[OP_PUSHI as u32, 55, OP_POPL as u32, 9],
            budget: &mut budget2,
            tasks: &mut w2.tasks,
            ecl: &ecl2,
            body: &mut w2.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r2 = exec(&mut task2, &mut ctx2);
        assert_eq!(r2, Exec::Fault(FAULT_PC_OOB));
        assert_eq!(task2.locals[9], 55);
    }

    /// fault 码编号冻结钉死（T1/T2 占位期用过的 6 号不回收复用，见 `FAULT_UNIMPLEMENTED` 文档）。
    #[test]
    fn fault_unimplemented_numbering_frozen() {
        assert_eq!(FAULT_UNIMPLEMENTED, 6);
    }

    /// `SYS`：T3 起真派发——坏 syscall 号（不在 v1 号表内）→ `Fault(FAULT_BAD_OP)`
    /// （`ecl::syscall::dispatch` 的默认臂，同 `OP_SPAWN` 坏脚本号处置口径）。
    #[test]
    fn sys_op_bad_syscall_number_faults() {
        let (r, _) = run(&[OP_SYS as u32, 9999]);
        assert_eq!(r, Exec::Fault(FAULT_BAD_OP));
    }

    /// `SYS_FRAME`（0 参读）经 `OP_SYS` 端到端派发：压回当前帧号。
    #[test]
    fn sys_op_dispatches_frame_read() {
        let mut task = Task::default();
        let mut budget = u32::MAX;
        let mut w = test_world();
        w.body.frame = 7;
        let ecl = EclImage::empty();
        let mut ctx = VmCtx {
            code: &[
                OP_SYS as u32,
                crate::ecl::syscall::SYS_FRAME as u32,
                OP_END as u32,
            ],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 7,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert_eq!(task.stack[0], 7, "SYS_FRAME 押回 ctx.frame");
    }

    #[test]
    fn kill_self_ends_like_end() {
        let (r, _) = run(&[OP_KILL_SELF as u32]);
        assert_eq!(
            r,
            Exec::End,
            "KILL_SELF 即刻 End 语义（调度层按 End 统一收尸）"
        );
    }

    /// `SPAWN` 成功路径（argc=0，M1.9 T3 金向量等价基线）：owner 从当前任务继承、parent
    /// 戳为自身索引+1、born_frame 戳为当前帧、pc 戳为 `ecl.entry(script)`——句柄（池索引）
    /// 压回求值栈；argc=0 时子任务 locals 必须原封不动（全零，`TaskPool::spawn` 零初始化，
    /// 无任何覆写）——这是"argc=0 与带参扩展前行为逐位等价"的判别式。
    #[test]
    fn spawn_op_inherits_owner_and_stamps_child_fields() {
        let mut w = test_world();
        let ecl = EclImage {
            code: vec![OP_END as u32],
            subs: vec![0],
            content_hash: 0,
        };
        let mut budget = u32::MAX;
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: 5,
            owner_gen: 2,
            ..Task::default()
        };
        let mut ctx = VmCtx {
            code: &[OP_SPAWN as u32, 0, 0, OP_END as u32],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 7,
            frame: 42,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert_eq!(task.sp, 1, "SPAWN 把子句柄压回求值栈");
        let child_idx = task.stack[0];
        assert!(child_idx >= 0);
        let c = &w.tasks.slots[child_idx as usize];
        assert_eq!(c.owner_kind, OWNER_ENEMY, "owner 继承自当前任务");
        assert_eq!(c.owner_index, 5);
        assert_eq!(c.owner_gen, 2);
        assert_eq!(c.parent, 7 + 1, "parent = 当前任务索引+1");
        assert_eq!(c.born_frame, 42, "born_frame 戳为当前帧（次帧首跑）");
        assert_eq!(c.pc, 0, "pc 戳为 ecl.entry(script)");
        assert_eq!(
            c.locals, [0; LOCALS],
            "argc=0：子任务 locals 全零，未被触碰"
        );
    }

    /// 带参 `SPAWN`：父栈按**声明顺序正序压栈**（11,22,33），`SPAWN` 逆序弹出 argc 个值
    /// 落进子任务 `locals[0..argc)`——弹完后 `locals` 顺序仍是**声明序**（不是弹出序），
    /// 判别式核心：若弹出顺序与落位顺序不镜像（例如直接顺序落位不反转），本测试会红。
    #[test]
    fn spawn_with_args_lands_in_child_locals_in_declaration_order() {
        let mut w = test_world();
        let ecl = EclImage {
            code: vec![OP_END as u32],
            subs: vec![0],
            content_hash: 0,
        };
        let mut budget = u32::MAX;
        let mut task = Task::default();
        let mut ctx = VmCtx {
            code: &[
                OP_PUSHI as u32,
                11,
                OP_PUSHI as u32,
                22,
                OP_PUSHI as u32,
                33,
                OP_SPAWN as u32,
                0,
                3, // script=0, argc=3
                OP_END as u32,
            ],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert_eq!(task.sp, 1, "argc 个实参已被弹栈消费，只剩子句柄");
        let child_idx = task.stack[0] as usize;
        let c = &w.tasks.slots[child_idx];
        assert_eq!(
            &c.locals[0..3],
            &[11, 22, 33],
            "locals[0..3] = 声明序（第一个压栈的实参落 locals[0]）"
        );
        assert_eq!(
            &c.locals[3..],
            &[0; LOCALS - 3],
            "argc 之外的 locals 仍全零"
        );
    }

    /// `argc > 64`（`LOCALS`）→ `Fault(FAULT_STACK)`——检查先于脚本号在册/父栈够不够门，
    /// 空求值栈 + 空镜像也照样在 argc 这一门就短路拒绝。
    #[test]
    fn spawn_argc_over_64_faults() {
        let (r, _) = run(&[OP_SPAWN as u32, 0, (LOCALS + 1) as u32]);
        assert_eq!(r, Exec::Fault(FAULT_STACK));
    }

    /// 父栈不够 argc 个值（栈下溢）→ `Fault(FAULT_STACK)`；脚本号本身在册（越过脚本号门后
    /// 才轮到"父栈够不够"门），零副作用（任务池未新增槽）。
    #[test]
    fn spawn_insufficient_parent_stack_faults() {
        let mut w = test_world();
        let ecl = EclImage {
            code: vec![OP_END as u32],
            subs: vec![0],
            content_hash: 0,
        };
        let mut budget = u32::MAX;
        let mut task = Task::default();
        let alive_before = w.tasks.iter_alive().count();
        let mut ctx = VmCtx {
            code: &[OP_SPAWN as u32, 0, 3], // argc=3，但父栈空
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Fault(FAULT_STACK));
        assert_eq!(
            w.tasks.iter_alive().count(),
            alive_before,
            "父栈不够门未过：不该新增任何任务池槽"
        );
    }

    /// 坏脚本号（`script >= subs.len()`）→ `Fault(FAULT_BAD_OP)`（同 PUSHL/POPL 越界口径）。
    #[test]
    fn spawn_bad_script_id_faults() {
        let mut w = test_world();
        let ecl = EclImage::empty(); // subs 空——任何脚本号都越界
        let mut budget = u32::MAX;
        let mut task = Task::default();
        let mut ctx = VmCtx {
            code: &[OP_SPAWN as u32, 0, 0],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Fault(FAULT_BAD_OP));
    }

    /// 任务池满 → 压 -1 + `pool_full[POOL_TASK]` 计数（P4-a：确定性降级不 panic）。
    #[test]
    fn spawn_pushes_neg1_and_counts_pool_full_when_task_pool_exhausted() {
        let mut w = test_world();
        for _ in 0..TASK_CAP {
            w.tasks
                .spawn(0, 0, (OWNER_STAGE, 0, 0), 0, 0)
                .expect("池未满前应成功");
        }
        let ecl = EclImage {
            code: vec![OP_END as u32],
            subs: vec![0],
            content_hash: 0,
        };
        let mut budget = u32::MAX;
        let mut task = Task::default();
        let mut ctx = VmCtx {
            code: &[OP_SPAWN as u32, 0, 0, OP_END as u32],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert_eq!(task.stack[0], -1, "池满压 -1");
        assert_eq!(w.body.diag.pool_full[POOL_TASK], 1);
    }

    /// `KILL_CHILDREN`：只杀直系子（parent == 自己索引+1），孙辈与无关任务不受影响
    /// （detached 语义，不递归）。
    #[test]
    fn kill_children_kills_only_direct_children() {
        let mut w = test_world();
        let self_idx = w.tasks.spawn(0, 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();
        let child = w
            .tasks
            .spawn(0, 0, (OWNER_STAGE, 0, 0), self_idx + 1, 0)
            .unwrap();
        let grandchild = w
            .tasks
            .spawn(0, 0, (OWNER_STAGE, 0, 0), child + 1, 0)
            .unwrap();
        let unrelated = w.tasks.spawn(0, 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();

        let ecl = EclImage::empty();
        let mut budget = u32::MAX;
        let mut task = w.tasks.slots[self_idx as usize];
        let mut ctx = VmCtx {
            code: &[OP_KILL_CHILDREN as u32, OP_END as u32],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: self_idx,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert!(!w.tasks.is_alive(child as usize), "直系子应被杀");
        assert!(
            w.tasks.is_alive(grandchild as usize),
            "孙不应被杀（不递归）"
        );
        assert!(w.tasks.is_alive(unrelated as usize), "无关任务不受影响");
    }

    /// C12⑤ 复审修复：`parent` 曾无代际戳——父任务死后，其存活的孤儿子任务的 `parent`
    /// 字段会继续悬挂指向那个已死槽号；槽一旦被复用（最低空位分配器天然会捡它），新
    /// 占用者调 `KILL_CHILDREN` 会因"槽号+1"数值巧合而误杀前任毫不相干的孤儿。
    /// `TaskPool::kill` 现在在父死的瞬间就清空孤儿的 `parent`（单元级钉法见
    /// `task::tests::kill_detaches_surviving_children_parent_pointer`），这里钉端到端场景。
    #[test]
    fn kill_children_does_not_kill_a_reused_slots_previous_orphans() {
        let mut w = test_world();
        let ecl = EclImage::empty();

        let parent_a = w.tasks.spawn(0, 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();
        let orphan = w
            .tasks
            .spawn(0, 0, (OWNER_STAGE, 0, 0), parent_a + 1, 0)
            .unwrap();
        w.tasks.kill(parent_a as usize); // A 死——orphan 存活，但按 detached 语义应彻底断亲。

        // 最低空位分配器：A 的槽此刻是最低空位，D 的 spawn 天然捡回它——这正是本条测试
        // 要钉的"槽复用"场景，不是巧合。
        let d = w.tasks.spawn(0, 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();
        assert_eq!(d, parent_a, "复用同一槽号，才是本条测试要钉的场景");

        let mut budget = u32::MAX;
        let mut task = w.tasks.slots[d as usize];
        let mut ctx = VmCtx {
            code: &[OP_KILL_CHILDREN as u32, OP_END as u32],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &ecl,
            body: &mut w.body,
            tables: &crate::tables::TABLES_V0,
            self_index: d,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::End);
        assert!(
            w.tasks.is_alive(orphan as usize),
            "D 从未 spawn 过 orphan，KILL_CHILDREN 不应因槽号复用误杀前任的孤儿"
        );
    }
}
