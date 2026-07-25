//! # stg-ecl-compiler —— ECL 字节码编译器（design_doc.md §4.5）
//!
//! 面向手感的表层语言（或 M1 先用的 Rust builder / 宏 DSL）→ 编译器 →
//! **字节码 + 常量表 = `EclImage`**（只读，内容哈希写入回放头与联机握手）。
//!
//! 只在**离线 / 加载期**运行，绝不进任何热路径（design_doc.md §1.1）。
//!
//! **脚本作者请写 `.ecl`**（[`lang::compile`]，见 `docs/ecl-lang.md`）——本文件下方的
//! `ImageBuilder`/`SubBuilder` 是 M1.9 起表层语言编译器的 **codegen 后端**，不再是作者
//! 面向的接口（M1 阶段临时形态已升级：彩虹风铃卡等符卡自 M1.9 T4 起改用 `.ecl` 源码，
//! `crates/stg-harness/scenes/rainbow.ecl` 是范例）。
//!
//! ## 分期
//!
//! - **M1**：先用 Rust builder / 宏 DSL 直接拼字节码，把 VM 语义与 syscall 表跑通
//!   （T3——**临时形态**，糖层语义即表层语言编译器的后端，非丢弃件）；
//! - **M1.9**：表层语言 `.ecl`（`lang` 模块）落地，builder DSL 降级为其 codegen 后端。
//!
//! ## 依赖方向
//!
//! 本 crate 依赖 `stg-core` 只为共享字节码/opcode/syscall 号常量类型定义（编译器在断层线
//! 【以上】运行，可用堆/`Vec`；产出物 `EclImage` 供 `stg-core` 的 VM 只读消费）。单向：
//! `stg-core` 的依赖图里完全没有本 crate（连 dev-dependency 也没有——曾试过反向 dev 依赖
//! 好让 `stg-core` 自己的单测摸生成码跑 VM，但 Cargo 会把"被测 crate 自身"与"作为下游
//! 普通依赖"编译成两份不互认类型的 `stg-core`，此路不通；本 crate 自己的单测改用
//! `stg-core` 暴露的公开面（`globals`/`diag`/`iter_alive().count()`）验证生成码经真实
//! VM 执行，见测试模块）。
//!
//! ## Builder 用法速览
//!
//! ```
//! use stg_core::math::{Angle, Fx};
//! use stg_core::ecl::image::SubKind;
//! use stg_ecl_compiler::{ImageBuilder, SubBuilder};
//!
//! let mut ib = ImageBuilder::new();
//! let mut main = SubBuilder::new();
//! main.wait(1);
//! main.sys_create_bullets_batch(
//!     0, Fx::ZERO, Fx::from_int(100),
//!     8, Angle::ZERO, 8192, 1, Fx::from_int(2), Fx::ZERO,
//! );
//! main.end();
//! let main_id = ib.declare_sub("main", SubKind::Root, &[])?;
//! ib.define_sub(main_id, main)?;
//! let image = ib.build(0)?;
//! assert_eq!(image.root().unwrap().get(), 0);
//! # Ok::<(), stg_core::ecl::image::ImageBuildError>(())
//! ```

pub mod lang;

use stg_core::ecl::image::{
    EclImage, EclValueType, EntryInit, ImageBuildError, ImageParts, SubInit, SubKind,
};
use stg_core::ecl::ops::{
    OP_ADD, OP_CALL, OP_COSB, OP_DIV, OP_DIVF, OP_DUP, OP_END, OP_EQ, OP_GE, OP_GT, OP_JMP, OP_JZ,
    OP_KILL_CHILDREN, OP_KILL_SELF, OP_LE, OP_LT, OP_MOD, OP_MUL, OP_MULF, OP_NE, OP_NEG, OP_POP,
    OP_POPL, OP_PUSHI, OP_PUSHL, OP_RET, OP_SINB, OP_SPAWN, OP_SUB, OP_SYS, OP_WAIT,
};
use stg_core::ecl::syscall;
use stg_core::ecl::task::LOCALS;
use stg_core::math::{Angle, Fx};
use stg_core::xform::XformSlot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BuilderSubRef(u32);

#[derive(Clone, Copy, Debug)]
struct JumpFixup {
    operand: usize,
    target: usize,
}

#[derive(Clone, Copy, Debug)]
enum TargetUse {
    Call,
    Spawn { argc: u8 },
    Fire,
}

#[derive(Clone, Copy, Debug)]
struct TargetFixup {
    operand: usize,
    target: BuilderSubRef,
    usage: TargetUse,
}

/// 一段子程序的构建器：raw 发射器 + 结构化糖（回填跳转）+ 类型化 syscall 薄壳。
///
/// **locals 语义**（同 VM：任务全局共享，见 `ecl::task` 文档）：`repeat` 的计数器借用
/// locals **从高位往低位**分配（`LOCALS-1`=63 起，嵌套每层再让一格，body 执行完归还）——
/// 避免与脚本作者自己用的低位 locals 槽相撞；纯栈实现（不借 locals）在 VM 现有 op 集下
/// 做不到"计数值需要跨过 body 任意条指令存活"，故选 locals 方案（结构化糖内部细节，作者
/// 不应依赖具体槽号——如需与本区间重叠，自留槽请从低位起）。
pub struct SubBuilder {
    code: Vec<u32>,
    /// 位置（本 sub 本地 code 下标）→ 该处操作数已经是"本 sub 本地目标 pc"，`build()` 拼接时
    /// 整体 `+= base_offset` 即得全局绝对 pc（`JMP`/`JZ` 回填目标，`repeat`/`if_ge`/
    /// `loop_forever` 用它）。
    jump_fixups: Vec<JumpFixup>,
    target_fixups: Vec<TargetFixup>,
    /// `mark(id)` 落点垫片登记（Task 4；整局流程刀 spec §2.1）：`(id, 本地 code 位置)`——
    /// `here()` 的语义与 `jump_fixups` 的 `target` 同款"本 sub 本地下标"，`ImageBuilder::build`
    /// 拼接时同样 `+= base_offset` 得全局绝对 ip，汇总进 `ImageParts.marks`。
    pub(crate) marks: Vec<(i32, usize)>,
    next_repeat_slot: u8,
    ended: bool,
}

impl Default for SubBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SubBuilder {
    pub fn new() -> Self {
        SubBuilder {
            code: Vec::new(),
            jump_fixups: Vec::new(),
            target_fixups: Vec::new(),
            marks: Vec::new(),
            next_repeat_slot: (LOCALS - 1) as u8,
            ended: false,
        }
    }

    #[inline]
    pub(crate) fn emit(&mut self, word: u32) -> usize {
        let p = self.code.len();
        self.code.push(word);
        p
    }

    #[inline]
    pub(crate) fn here(&self) -> usize {
        self.code.len()
    }

    #[inline]
    pub(crate) fn patch(&mut self, pos: usize, local_target: usize) {
        self.jump_fixups
            .iter_mut()
            .find(|fixup| fixup.operand == pos)
            .expect("patch target must refer to a recorded jump operand")
            .target = local_target;
    }

    // ── codegen 专用 raw 原语（`lang::codegen` 消费；`pub(crate)`——仅本 crate 内部，
    // 不是公开契约）：表层语言控制流降低模板（if/while/for/`&&`/`||`）需要裸
    // `JZ`/`JMP` + 手动回填，既有的 `if_ge`/`loop_forever`/`repeat` 结构化糖形状太窄
    // （各自钉死一种控制流形状），故在既有 `emit`/`here`/`patch` 基础之上加这一薄层，
    // 复用同一套 `jump_fixups` 回填机制（不是另起一条路），符合"extend it, don't
    // fork"的指导──────────────────────────────────────────────────────────────

    /// 裸 `JZ`：占位操作数记入 `jump_fixups`（`build()` 时随 sub 基址平移），返回该
    /// 操作数在本 sub 本地 `code` 里的位置，供调用方稍后 `patch` 到真实（本地）目标。
    pub(crate) fn raw_jz(&mut self) -> usize {
        self.emit(OP_JZ as u32);
        let p = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: p,
            target: 0,
        });
        p
    }

    /// 同 [`Self::raw_jz`]，裸 `JMP`。
    pub(crate) fn raw_jmp(&mut self) -> usize {
        self.emit(OP_JMP as u32);
        let p = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: p,
            target: 0,
        });
        p
    }

    /// `mark(id)` 落点垫片降低（Task 4；整局流程刀 spec §2.1）：把**当前位置**登记为 `id`
    /// 的落点（垫片首指令，紧跟在 `raw_jmp()` 之后调用，见 `lang::codegen` 的
    /// `TypedStmt::Mark` 臂）——`ImageBuilder::build` 汇总 `base + 本地位置` 得全局绝对 ip，
    /// 塞 `ImageParts.marks`（Task 6 `resolve_mark` 消费）。id 重复在编译期
    /// `typeck::validate_marks` 已经拒绝，本方法不重复校验。
    pub(crate) fn mark_here(&mut self, id: i32) {
        let h = self.here();
        self.marks.push((id, h));
    }

    /// `wait(e)`（`e` 是任意表达式，不是编译期常量）：调用方已把等待帧数表达式的求值
    /// 结果留在栈顶，本方法只发 `OP_WAIT`——区别于 [`Self::wait`] 那个自己 `push_i`
    /// 一个 `u16` 编译期常量的糖。
    pub(crate) fn raw_wait(&mut self) {
        self.emit(OP_WAIT as u32);
    }

    /// `return;` 降低（call-style sub 专用——只被同步 `CALL` 过的 sub 必须以 `OP_RET`
    /// 收尾/早退，好让调用方从调用栈弹回；entry-style sub 用 [`Self::end`]，见
    /// `lang::codegen` 模块文档"call-style vs entry-style"判定）：发 `OP_RET` 并标记
    /// `ended`（阻止 [`ImageBuilder::build`] 误给这条早退路径再补一条多余 `OP_END`——
    /// `ended` 只在"这是本 sub 最终真正落地的收尾指令"时才该标记为真，早退路径同样
    /// 满足这个条件，故一律标记，不区分"是不是函数体最后一条语句"）。
    pub(crate) fn raw_ret(&mut self) {
        self.emit(OP_RET as u32);
        self.ended = true;
    }

    /// 直发一条无操作数 op（`sin`/`cos` 等 `is_op` 内建的直译，见 `lang::builtins`
    /// 模块文档"`is_op` 消歧"——这类内建底层不是 `OP_SYS` 派发，是一条裸算术 op）。
    pub(crate) fn raw_emit_op(&mut self, op: u8) {
        self.emit(op as u32);
    }

    // ── raw 发射器（薄壳，逐 op 一对一）─────────────────────────────────────

    pub fn push_i(&mut self, v: i32) {
        self.emit(OP_PUSHI as u32);
        self.emit(v as u32);
    }

    pub fn push_l(&mut self, slot: u8) {
        self.emit(OP_PUSHL as u32);
        self.emit(slot as u32);
    }

    pub fn pop_l(&mut self, slot: u8) {
        self.emit(OP_POPL as u32);
        self.emit(slot as u32);
    }

    pub fn dup(&mut self) {
        self.emit(OP_DUP as u32);
    }

    pub fn pop(&mut self) {
        self.emit(OP_POP as u32);
    }

    /// `wait(frames)` 糖：`push_i(frames) + OP_WAIT`。
    pub fn wait(&mut self, frames: u16) {
        self.push_i(frames as i32);
        self.emit(OP_WAIT as u32);
    }

    pub fn add(&mut self) {
        self.emit(OP_ADD as u32);
    }
    pub fn sub(&mut self) {
        self.emit(OP_SUB as u32);
    }
    pub fn mul(&mut self) {
        self.emit(OP_MUL as u32);
    }
    pub fn div(&mut self) {
        self.emit(OP_DIV as u32);
    }
    /// `mod` 是 Rust 保留字，方法名让开一格。
    pub fn rem(&mut self) {
        self.emit(OP_MOD as u32);
    }
    pub fn neg(&mut self) {
        self.emit(OP_NEG as u32);
    }
    pub fn mulf(&mut self) {
        self.emit(OP_MULF as u32);
    }
    pub fn divf(&mut self) {
        self.emit(OP_DIVF as u32);
    }
    pub fn sinb(&mut self) {
        self.emit(OP_SINB as u32);
    }
    pub fn cosb(&mut self) {
        self.emit(OP_COSB as u32);
    }
    pub fn eq(&mut self) {
        self.emit(OP_EQ as u32);
    }
    pub fn ne(&mut self) {
        self.emit(OP_NE as u32);
    }
    pub fn lt(&mut self) {
        self.emit(OP_LT as u32);
    }
    pub fn le(&mut self) {
        self.emit(OP_LE as u32);
    }
    pub fn gt(&mut self) {
        self.emit(OP_GT as u32);
    }
    pub fn ge(&mut self) {
        self.emit(OP_GE as u32);
    }

    pub fn kill_self(&mut self) {
        self.emit(OP_KILL_SELF as u32);
    }
    pub fn kill_children(&mut self) {
        self.emit(OP_KILL_CHILDREN as u32);
    }

    /// 子程序调用（`OP_CALL`）：`build()` 校验目标为 `CallOnly`，并将操作数回填为
    /// 按名字排序后分配的 canonical `SubId`；VM 再由镜像元数据解析绝对代码入口。
    pub fn call(&mut self, sub: BuilderSubRef) {
        self.emit(OP_CALL as u32);
        let p = self.emit(0);
        self.target_fixups.push(TargetFixup {
            operand: p,
            target: sub,
            usage: TargetUse::Call,
        });
    }

    /// 协程派生（`OP_SPAWN`）：操作数是 canonical `SubId` + **argc**。`build()` 校验
    /// 目标为 `Async` 且形参数量与 `argc` 完全相等，再回填 canonical `SubId`；VM 运行期
    /// 通过镜像元数据重复校验 kind/arity 并解析绝对代码入口。owner 继承自当前任务
    /// （VM 既定语义）；子句柄（池索引，失败 -1）留在求值栈顶。
    ///
    /// **调用前置条件**：调用方须已把 `argc` 个实参**按声明顺序正序压栈**（`push_i`/
    /// 表达式求值链），`OP_SPAWN` 会逆序弹出落进子任务 `locals[0..argc)`——弹完后
    /// `locals` 顺序仍是声明序，见 `vm.rs::exec` 的 `OP_SPAWN` 分支文档。`argc=0` 是
    /// 既有零参调用点的等价形态（不弹栈、子任务 locals 全零，逐位不变）。
    pub fn spawn(&mut self, sub: BuilderSubRef, argc: u8) {
        self.emit(OP_SPAWN as u32);
        let p = self.emit(0);
        self.emit(argc as u32);
        self.target_fixups.push(TargetFixup {
            operand: p,
            target: sub,
            usage: TargetUse::Spawn { argc },
        });
    }

    /// `end()`：追加 `OP_END`（`build()` 时若某 sub 未调用过本方法会自动补一次，
    /// 见 `ImageBuilder::build`）。
    pub fn end(&mut self) {
        self.emit(OP_END as u32);
        self.ended = true;
    }

    // ── 结构化糖（跳转回填）───────────────────────────────────────────────

    /// 无条件回环：`body` 执行完后 `JMP` 回 `body` 起点（`JMP` 操作数记入 `jump_fixups`，
    /// `build()` 拼接时整体加基址）。空 `body` 合法（生成一条自跳转，等价原 VM 的死循环
    /// 语义——作者自负预算撞墙的后果）。
    pub fn loop_forever(&mut self, body: impl FnOnce(&mut Self)) {
        let top = self.here();
        body(self);
        self.emit(OP_JMP as u32);
        let p = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: p,
            target: 0,
        });
        self.patch(p, top);
    }

    /// 条件块：栈顶为 0（假）时跳过 `body`（`JZ`），非 0 时落入执行——**不消费额外栈**，
    /// 调用方须在此之前自行把条件值（如 `ge()` 的结果）压好。空 `body` 合法（JZ 目标
    /// 落在紧随其后，等价一次纯粹的条件求值 + 落地）。
    pub fn if_ge(&mut self, body: impl FnOnce(&mut Self)) {
        self.emit(OP_JZ as u32);
        let jz_pos = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: jz_pos,
            target: 0,
        });
        body(self);
        let after = self.here();
        self.patch(jz_pos, after);
    }

    /// 计数循环：`body` 恰执行 `n` 次（`n<=0` 是 no-op，不发一条指令）。计数器借用一格
    /// locals（本 builder 实例内自动从 `LOCALS-1` 往下分配、嵌套安全，见结构体文档）；
    /// `body` 内部若自己也调用 `wait`，恢复执行落点由 VM 的 `Task.pc` 天然处理，
    /// 不需要 `repeat` 额外关照。
    pub fn repeat(&mut self, n: i32, body: impl FnOnce(&mut Self)) {
        if n <= 0 {
            return;
        }
        let slot = self.next_repeat_slot;
        self.next_repeat_slot = self.next_repeat_slot.saturating_sub(1);

        self.push_i(n);
        self.pop_l(slot); // locals[slot] = n

        let top = self.here();
        body(self);

        // locals[slot] -= 1；非零则回环，零则落出。
        self.push_l(slot);
        self.push_i(1);
        self.sub();
        self.dup();
        self.pop_l(slot);

        self.emit(OP_JZ as u32);
        let jz_pos = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: jz_pos,
            target: 0,
        });

        self.emit(OP_JMP as u32);
        let jmp_pos = self.emit(0);
        self.jump_fixups.push(JumpFixup {
            operand: jmp_pos,
            target: 0,
        });
        self.patch(jmp_pos, top);

        let after = self.here();
        self.patch(jz_pos, after);

        self.next_repeat_slot += 1;
    }

    // ── syscall 类型化薄壳（v1 号表逐一覆盖；参数按 `ecl::syscall` 各自文档声明顺序
    // 正序压栈——与 `dispatch` 的逆序弹出严格配对，见该模块文档）────────────────────

    #[inline]
    pub(crate) fn sys(&mut self, no: u16) {
        self.emit(OP_SYS as u32);
        self.emit(no as u32);
    }

    pub fn sys_frame(&mut self) {
        self.sys(syscall::SYS_FRAME);
    }
    pub fn sys_player_x(&mut self) {
        self.sys(syscall::SYS_PLAYER_X);
    }
    pub fn sys_player_y(&mut self) {
        self.sys(syscall::SYS_PLAYER_Y);
    }
    pub fn sys_self_x(&mut self) {
        self.sys(syscall::SYS_SELF_X);
    }
    pub fn sys_self_y(&mut self) {
        self.sys(syscall::SYS_SELF_Y);
    }
    pub fn sys_self_hp(&mut self) {
        self.sys(syscall::SYS_SELF_HP);
    }
    /// 任务龄（M1.5；`ctx.frame - task.born_frame`——**任务**龄非 ZUN 的敌龄，见 `syscall.rs`
    /// `SYS_SELF_AGE` 文档的语义偏离记档）。
    pub fn sys_self_age(&mut self) {
        self.sys(syscall::SYS_SELF_AGE);
    }
    /// owner 上限血量（M1.5；非敌 owner 恒 0，同 `sys_self_hp` 误用策略）。
    pub fn sys_self_hp_max(&mut self) {
        self.sys(syscall::SYS_SELF_HP_MAX);
    }
    pub fn sys_rand_range(&mut self, n: i32) {
        self.push_i(n);
        self.sys(syscall::SYS_RAND_RANGE);
    }
    pub fn sys_get_var(&mut self, slot: u16) {
        self.push_i(slot as i32);
        self.sys(syscall::SYS_GET_VAR);
    }
    pub fn sys_set_var(&mut self, slot: u16, val: i32) {
        self.push_i(slot as i32);
        self.push_i(val);
        self.sys(syscall::SYS_SET_VAR);
    }
    /// 与 [`Self::sys_set_var`] 等价，但把要写入的值留给调用方**预先压栈**（配合动态表达式，
    /// 例如 `get_var` 读回值 + 算术后再写回——`sys_set_var` 只能塞编译期常量 `val`）。
    /// 调用前栈序须已是 `[slot, val]`（正序，val 在顶）。
    pub fn sys_set_var_from_stack(&mut self) {
        self.sys(syscall::SYS_SET_VAR);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sys_create_bullet(
        &mut self,
        appearance: u16,
        x: Fx,
        y: Fx,
        speed: Fx,
        angle: Angle,
        xform_off: i32,
        xform_cnt: i32,
        task_script: Option<BuilderSubRef>,
    ) {
        self.push_i(appearance as i32);
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(speed.raw());
        self.push_i(angle.raw() as i32);
        self.push_i(xform_off);
        self.push_i(xform_cnt);
        self.push_task_ref(task_script);
        self.sys(syscall::SYS_CREATE_BULLET);
    }

    pub(crate) fn push_task_ref(&mut self, task: Option<BuilderSubRef>) {
        match task {
            None => self.push_i(-1),
            Some(target) => {
                self.emit(OP_PUSHI as u32);
                let operand = self.emit(0);
                self.target_fixups.push(TargetFixup {
                    operand,
                    target,
                    usage: TargetUse::Fire,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sys_create_bullets_batch(
        &mut self,
        appearance: u16,
        x: Fx,
        y: Fx,
        n_angle: u16,
        angle0: Angle,
        angle_step: i16,
        n_speed: u16,
        speed0: Fx,
        speed_step: Fx,
    ) {
        self.push_i(appearance as i32);
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(n_angle as i32);
        self.push_i(angle0.raw() as i32);
        self.push_i(angle_step as i32);
        self.push_i(n_speed as i32);
        self.push_i(speed0.raw());
        self.push_i(speed_step.raw());
        self.sys(syscall::SYS_CREATE_BULLETS_BATCH);
    }

    pub fn sys_spawn_enemy(&mut self, x: Fx, y: Fx, hp: i32, drop_table: u16, score: u16) {
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(hp);
        self.push_i(drop_table as i32);
        self.push_i(score as i32);
        self.sys(syscall::SYS_SPAWN_ENEMY);
    }

    pub fn sys_drop_item(&mut self, x: Fx, y: Fx, item_type: u8) {
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(item_type as i32);
        self.sys(syscall::SYS_DROP_ITEM);
    }

    pub fn sys_move_enemy_to(&mut self, dur: u16, x: Fx, y: Fx, easing: u8) {
        self.push_i(dur as i32);
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(easing as i32);
        self.sys(syscall::SYS_MOVE_ENEMY_TO);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sys_boss_set(
        &mut self,
        slot: u8,
        hp_ratio: Fx,
        spell_id: u16,
        timer_frames: u16,
        phase_left: u8,
        active: u8,
    ) {
        self.push_i(slot as i32);
        self.push_i(hp_ratio.raw());
        self.push_i(spell_id as i32);
        self.push_i(timer_frames as i32);
        self.push_i(phase_left as i32);
        self.push_i(active as i32);
        self.sys(syscall::SYS_BOSS_SET);
    }

    pub fn sys_pulse_signal(&mut self, ch: u8) {
        self.push_i(ch as i32);
        self.sys(syscall::SYS_PULSE_SIGNAL);
    }

    pub fn sys_set_bullet_speed(&mut self, speed: Fx) {
        self.push_i(speed.raw());
        self.sys(syscall::SYS_SET_BULLET_SPEED);
    }
    pub fn sys_set_bullet_angle(&mut self, angle: Angle) {
        self.push_i(angle.raw() as i32);
        self.sys(syscall::SYS_SET_BULLET_ANGLE);
    }
    pub fn sys_turn_bullet(&mut self, delta: Angle) {
        self.push_i(delta.raw() as i32);
        self.sys(syscall::SYS_TURN_BULLET);
    }
    pub fn sys_set_bullet_vel(&mut self, vx: Fx, vy: Fx) {
        self.push_i(vx.raw());
        self.push_i(vy.raw());
        self.sys(syscall::SYS_SET_BULLET_VEL);
    }
    pub fn sys_set_bullet_ang_vel(&mut self, w: i16) {
        self.push_i(w as i32);
        self.sys(syscall::SYS_SET_BULLET_ANG_VEL);
    }
    pub fn sys_set_bullet_accel(&mut self, accel: Fx) {
        self.push_i(accel.raw());
        self.sys(syscall::SYS_SET_BULLET_ACCEL);
    }
    pub fn sys_set_bullet_gravity(&mut self, ax: Fx, ay: Fx) {
        self.push_i(ax.raw());
        self.push_i(ay.raw());
        self.sys(syscall::SYS_SET_BULLET_GRAVITY);
    }
    pub fn sys_stop_bullet_fx(&mut self) {
        self.sys(syscall::SYS_STOP_BULLET_FX);
    }
    pub fn sys_aim_bullet_at_player(&mut self, delta: Angle) {
        self.push_i(delta.raw() as i32);
        self.sys(syscall::SYS_AIM_BULLET_AT_PLAYER);
    }
    pub fn sys_aim_player_angle(&mut self) {
        self.sys(syscall::SYS_AIM_PLAYER_ANGLE);
    }

    /// 把一段 `XformSlot` 序列按丙方案 3 词打包写入 `locals[off..]`
    /// （`word0=(wait<<16)|(op<<8)`，`word1/2=args`）——供 [`Self::sys_create_bullet`] 的
    /// `xform_off`/`xform_cnt` 联合读取。若要在循环内复用同一份模板，把本调用放在循环
    /// **之外**一次写入即可（locals 任务全局共享、跨帧存活）。`slots.len()` 必须 `<=16`
    /// 且 `off as usize + slots.len()*3 <= LOCALS`——本方法不做校验（生成期作者自查；
    /// 运行期 `SYS_CREATE_BULLET` 自己会对越界给 `Fault`）。
    pub fn write_xform_locals(&mut self, off: u8, slots: &[XformSlot]) {
        for (k, s) in slots.iter().enumerate() {
            let word0 = ((s.wait as u32) << 16) | ((s.op as u32) << 8);
            let base = off as usize + k * 3;
            self.push_i(word0 as i32);
            self.pop_l(base as u8);
            self.push_i(s.args[0]);
            self.pop_l((base + 1) as u8);
            self.push_i(s.args[1]);
            self.pop_l((base + 2) as u8);
        }
    }
}

struct SubDecl {
    name: String,
    kind: SubKind,
    params: Vec<EclValueType>,
    body: Option<SubBuilder>,
}

#[derive(Default)]
pub struct ImageBuilder {
    subs: Vec<SubDecl>,
}

impl ImageBuilder {
    pub fn new() -> Self {
        Self { subs: Vec::new() }
    }

    pub fn declare_sub(
        &mut self,
        name: &str,
        kind: SubKind,
        params: &[EclValueType],
    ) -> Result<BuilderSubRef, ImageBuildError> {
        if !valid_identifier(name) {
            return Err(ImageBuildError::InvalidEntryName {
                name: name.to_owned(),
            });
        }
        if self.subs.iter().any(|decl| decl.name == name) {
            return Err(ImageBuildError::DuplicateSubName {
                name: name.to_owned(),
            });
        }
        if kind == SubKind::Root && name != "main" {
            return Err(ImageBuildError::InvalidRootName {
                name: name.to_owned(),
            });
        }
        if name == "main" && kind != SubKind::Root {
            return Err(ImageBuildError::ReservedMainKind { kind });
        }
        if kind == SubKind::Root && !params.is_empty() {
            return Err(ImageBuildError::RootHasParameters);
        }
        if params.len() > LOCALS {
            return Err(ImageBuildError::TooManyParameters {
                sub: self.subs.len(),
                actual: params.len(),
            });
        }
        let id = u32::try_from(self.subs.len()).map_err(|_| ImageBuildError::OperandOverflow)?;
        self.subs.push(SubDecl {
            name: name.to_owned(),
            kind,
            params: params.to_vec(),
            body: None,
        });
        Ok(BuilderSubRef(id))
    }

    pub fn define_sub(
        &mut self,
        sub: BuilderSubRef,
        body: SubBuilder,
    ) -> Result<(), ImageBuildError> {
        let Some(decl) = self.subs.get_mut(sub.0 as usize) else {
            return Err(ImageBuildError::InvalidBuilderRef);
        };
        if decl.body.is_some() {
            return Err(ImageBuildError::DuplicateDefinition {
                name: decl.name.clone(),
            });
        }
        decl.body = Some(body);
        Ok(())
    }

    pub fn build(mut self, content_hash: u64) -> Result<EclImage, ImageBuildError> {
        if self.subs.is_empty() {
            return Ok(EclImage::empty());
        }
        if !self.subs.iter().any(|decl| decl.kind == SubKind::Root) {
            return Err(ImageBuildError::MissingRoot);
        }
        for decl in &self.subs {
            if decl.body.is_none() {
                return Err(ImageBuildError::UndefinedSub {
                    name: decl.name.clone(),
                });
            }
        }
        if self.subs.len() > u16::MAX as usize + 1 {
            return Err(ImageBuildError::TooManySubs {
                actual: self.subs.len(),
            });
        }

        for decl in &mut self.subs {
            let body = decl.body.as_mut().expect("definitions checked above");
            if !body.ended {
                if decl.kind == SubKind::CallOnly {
                    body.raw_ret();
                } else {
                    body.end();
                }
            }
        }

        let mut order: Vec<usize> = (0..self.subs.len()).collect();
        order.sort_by(|&left, &right| self.subs[left].name.cmp(&self.subs[right].name));

        let mut canonical = vec![0u16; self.subs.len()];
        let mut bases = vec![0usize; self.subs.len()];
        let mut code = Vec::new();
        for (canonical_index, &decl_index) in order.iter().enumerate() {
            canonical[decl_index] =
                u16::try_from(canonical_index).map_err(|_| ImageBuildError::OperandOverflow)?;
            bases[decl_index] = code.len();
            code.extend_from_slice(
                &self.subs[decl_index]
                    .body
                    .as_ref()
                    .expect("definitions checked above")
                    .code,
            );
            if code.len() > u32::MAX as usize {
                return Err(ImageBuildError::CodeTooLong { words: code.len() });
            }
        }

        for &decl_index in &order {
            let base = bases[decl_index];
            let body = self.subs[decl_index]
                .body
                .as_ref()
                .expect("definitions checked above");
            for fixup in &body.jump_fixups {
                let absolute = base
                    .checked_add(fixup.target)
                    .ok_or(ImageBuildError::OperandOverflow)?;
                let word = u32::try_from(absolute).map_err(|_| ImageBuildError::OperandOverflow)?;
                let operand = base
                    .checked_add(fixup.operand)
                    .ok_or(ImageBuildError::OperandOverflow)?;
                *code
                    .get_mut(operand)
                    .ok_or(ImageBuildError::OperandOverflow)? = word;
            }
            for fixup in &body.target_fixups {
                let target_index = fixup.target.0 as usize;
                let target = self
                    .subs
                    .get(target_index)
                    .ok_or(ImageBuildError::InvalidBuilderRef)?;
                let (expected_kind, expected_arity, actual_arity) = match fixup.usage {
                    TargetUse::Call => (SubKind::CallOnly, None, None),
                    TargetUse::Spawn { argc } => (
                        SubKind::Async,
                        Some(
                            u8::try_from(target.params.len())
                                .map_err(|_| ImageBuildError::OperandOverflow)?,
                        ),
                        Some(argc),
                    ),
                    TargetUse::Fire => (SubKind::Async, Some(0), Some(0)),
                };
                if target.kind != expected_kind {
                    return Err(ImageBuildError::WrongTargetKind {
                        target: target.name.clone(),
                        expected: expected_kind,
                        actual: target.kind,
                    });
                }
                if let (Some(expected), Some(actual)) = (expected_arity, actual_arity)
                    && expected != actual
                {
                    return Err(ImageBuildError::WrongTargetArity {
                        target: target.name.clone(),
                        expected,
                        actual,
                    });
                }
                let operand = base
                    .checked_add(fixup.operand)
                    .ok_or(ImageBuildError::OperandOverflow)?;
                *code
                    .get_mut(operand)
                    .ok_or(ImageBuildError::OperandOverflow)? = canonical[target_index] as u32;
            }
        }

        // 中段启动标记表汇总（Task 4；整局流程刀 spec §2）：`SubBuilder::mark_here` 登记的
        // 都是本地（本 sub 自己的 `code`）位置，同 `jump_fixups` 一样 `base + 本地位置` 得
        // 全局绝对 ip；`id` 重复在编译期 `typeck::validate_marks` 已经拒绝，这里只
        // `debug_assert`（P4-c：引擎自身 bug 才会撞上，不是正常源码能触发的路径）。
        let mut marks: Vec<(i32, u32)> = Vec::new();
        for &decl_index in &order {
            let base = bases[decl_index];
            let body = self.subs[decl_index]
                .body
                .as_ref()
                .expect("definitions checked above");
            for &(id, local_ip) in &body.marks {
                let absolute = base
                    .checked_add(local_ip)
                    .ok_or(ImageBuildError::OperandOverflow)?;
                let ip = u32::try_from(absolute).map_err(|_| ImageBuildError::OperandOverflow)?;
                marks.push((id, ip));
            }
        }
        marks.sort_by_key(|m| m.0);
        debug_assert!(
            marks.windows(2).all(|pair| pair[0].0 != pair[1].0),
            "mark id 重复应已在编译期 typeck::validate_marks 挡下"
        );

        let mut subs = Vec::with_capacity(order.len());
        let mut entries = Vec::new();
        let mut root = None;
        for (canonical_index, &decl_index) in order.iter().enumerate() {
            let decl = &self.subs[decl_index];
            let code_entry =
                u32::try_from(bases[decl_index]).map_err(|_| ImageBuildError::OperandOverflow)?;
            subs.push(SubInit::new(code_entry, decl.kind, decl.params.clone()));
            let sub =
                u16::try_from(canonical_index).map_err(|_| ImageBuildError::OperandOverflow)?;
            match decl.kind {
                SubKind::Root => root = Some(sub),
                SubKind::Async => entries.push(EntryInit::new(&decl.name, sub)),
                SubKind::CallOnly => {}
            }
        }
        EclImage::try_from_parts(ImageParts {
            code,
            subs,
            entries,
            root,
            marks,
            content_hash,
        })
    }
}

fn valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::ecl::image::{EclValueType, ImageBuildError, SubKind};

    #[test]
    fn builder_assigns_ids_and_code_layout_by_name_not_declaration_order() {
        fn build(reverse: bool) -> EclImage {
            let mut ib = ImageBuilder::new();
            let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
            let worker = ib
                .declare_sub("worker", SubKind::Async, &[EclValueType::Int])
                .unwrap();
            let helper = ib.declare_sub("helper", SubKind::CallOnly, &[]).unwrap();
            let order = if reverse {
                [worker, helper, main]
            } else {
                [main, helper, worker]
            };
            for id in order {
                let mut sub = SubBuilder::new();
                if id == helper {
                    sub.raw_ret();
                } else {
                    sub.end();
                }
                ib.define_sub(id, sub).unwrap();
            }
            ib.build(0).unwrap()
        }
        assert_eq!(build(false), build(true));
    }

    #[test]
    fn call_and_spawn_operands_are_canonical_sub_ids() {
        let mut ib = ImageBuilder::new();
        let worker = ib.declare_sub("worker", SubKind::Async, &[]).unwrap();
        let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        let helper = ib.declare_sub("helper", SubKind::CallOnly, &[]).unwrap();

        let mut main_body = SubBuilder::new();
        main_body.call(helper);
        main_body.spawn(worker, 0);
        main_body.end();
        ib.define_sub(main, main_body).unwrap();

        for id in [worker, helper] {
            let mut body = SubBuilder::new();
            if id == helper {
                body.raw_ret();
            } else {
                body.end();
            }
            ib.define_sub(id, body).unwrap();
        }

        let image = ib.build(0).unwrap();
        let pc = image.sub_meta(image.root().unwrap()).unwrap().code_entry() as usize;
        assert_eq!(
            &image.code()[pc..pc + 5],
            &[OP_CALL as u32, 0, OP_SPAWN as u32, 2, 0],
        );
    }

    fn defined_body(kind: SubKind) -> SubBuilder {
        let mut body = SubBuilder::new();
        if kind == SubKind::CallOnly {
            body.raw_ret();
        } else {
            body.end();
        }
        body
    }

    #[test]
    fn builder_rejects_duplicate_declaration_and_invalid_ref() {
        let mut ib = ImageBuilder::new();
        ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        assert_eq!(
            ib.declare_sub("main", SubKind::Root, &[]),
            Err(ImageBuildError::DuplicateSubName {
                name: "main".to_owned()
            })
        );
        assert_eq!(
            ib.define_sub(BuilderSubRef(u32::MAX), SubBuilder::new()),
            Err(ImageBuildError::InvalidBuilderRef)
        );
    }

    #[test]
    fn builder_rejects_undefined_and_duplicate_definitions() {
        let mut undefined = ImageBuilder::new();
        undefined.declare_sub("main", SubKind::Root, &[]).unwrap();
        assert_eq!(
            undefined.build(0),
            Err(ImageBuildError::UndefinedSub {
                name: "main".to_owned()
            })
        );

        let mut duplicate = ImageBuilder::new();
        let main = duplicate.declare_sub("main", SubKind::Root, &[]).unwrap();
        duplicate
            .define_sub(main, defined_body(SubKind::Root))
            .unwrap();
        assert_eq!(
            duplicate.define_sub(main, defined_body(SubKind::Root)),
            Err(ImageBuildError::DuplicateDefinition {
                name: "main".to_owned()
            })
        );
    }

    fn wrong_target_image(op: impl FnOnce(&mut SubBuilder, BuilderSubRef)) -> ImageBuilder {
        let mut ib = ImageBuilder::new();
        let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        let target = ib.declare_sub("target", SubKind::Async, &[]).unwrap();
        let mut body = SubBuilder::new();
        op(&mut body, target);
        body.end();
        ib.define_sub(main, body).unwrap();
        ib.define_sub(target, defined_body(SubKind::Async)).unwrap();
        ib
    }

    #[test]
    fn builder_rejects_call_to_root_or_async() {
        let async_call = wrong_target_image(|body, target| body.call(target));
        assert_eq!(
            async_call.build(0),
            Err(ImageBuildError::WrongTargetKind {
                target: "target".to_owned(),
                expected: SubKind::CallOnly,
                actual: SubKind::Async,
            })
        );

        let mut root_call = ImageBuilder::new();
        let main = root_call.declare_sub("main", SubKind::Root, &[]).unwrap();
        let helper = root_call
            .declare_sub("helper", SubKind::CallOnly, &[])
            .unwrap();
        let mut helper_body = SubBuilder::new();
        helper_body.call(main);
        helper_body.raw_ret();
        root_call
            .define_sub(main, defined_body(SubKind::Root))
            .unwrap();
        root_call.define_sub(helper, helper_body).unwrap();
        assert_eq!(
            root_call.build(0),
            Err(ImageBuildError::WrongTargetKind {
                target: "main".to_owned(),
                expected: SubKind::CallOnly,
                actual: SubKind::Root,
            })
        );
    }

    #[test]
    fn builder_rejects_spawn_to_root_or_call_only_and_wrong_arity() {
        for (kind, name) in [(SubKind::Root, "main"), (SubKind::CallOnly, "helper")] {
            let mut ib = ImageBuilder::new();
            let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
            let target = if kind == SubKind::Root {
                main
            } else {
                ib.declare_sub(name, kind, &[]).unwrap()
            };
            let mut body = SubBuilder::new();
            body.spawn(target, 0);
            body.end();
            ib.define_sub(main, body).unwrap();
            if target != main {
                ib.define_sub(target, defined_body(kind)).unwrap();
            }
            assert_eq!(
                ib.build(0),
                Err(ImageBuildError::WrongTargetKind {
                    target: name.to_owned(),
                    expected: SubKind::Async,
                    actual: kind,
                })
            );
        }

        let mut arity = ImageBuilder::new();
        let main = arity.declare_sub("main", SubKind::Root, &[]).unwrap();
        let worker = arity
            .declare_sub("worker", SubKind::Async, &[EclValueType::Int])
            .unwrap();
        let mut body = SubBuilder::new();
        body.spawn(worker, 0);
        body.end();
        arity.define_sub(main, body).unwrap();
        arity
            .define_sub(worker, defined_body(SubKind::Async))
            .unwrap();
        assert_eq!(
            arity.build(0),
            Err(ImageBuildError::WrongTargetArity {
                target: "worker".to_owned(),
                expected: 1,
                actual: 0,
            })
        );
    }

    #[test]
    fn builder_requires_zero_param_root_named_main() {
        let mut missing = ImageBuilder::new();
        let worker = missing.declare_sub("worker", SubKind::Async, &[]).unwrap();
        missing
            .define_sub(worker, defined_body(SubKind::Async))
            .unwrap();
        assert_eq!(missing.build(0), Err(ImageBuildError::MissingRoot));

        let mut params = ImageBuilder::new();
        assert_eq!(
            params.declare_sub("main", SubKind::Root, &[EclValueType::Int]),
            Err(ImageBuildError::RootHasParameters)
        );
    }

    #[test]
    fn builder_checks_jump_operand_overflow() {
        let mut ib = ImageBuilder::new();
        let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        let body = SubBuilder {
            code: vec![OP_JMP as u32, 0],
            jump_fixups: vec![JumpFixup {
                operand: 1,
                target: usize::MAX,
            }],
            target_fixups: vec![],
            marks: vec![],
            next_repeat_slot: (LOCALS - 1) as u8,
            ended: true,
        };
        ib.define_sub(main, body).unwrap();
        assert_eq!(ib.build(0), Err(ImageBuildError::OperandOverflow));
    }

    #[test]
    fn fire_target_is_canonicalized_and_must_be_zero_arg_async() {
        let mut ib = ImageBuilder::new();
        let worker = ib.declare_sub("worker", SubKind::Async, &[]).unwrap();
        let main = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        let mut body = SubBuilder::new();
        body.sys_create_bullet(
            0,
            Fx::ZERO,
            Fx::ZERO,
            Fx::ZERO,
            Angle::ZERO,
            0,
            0,
            Some(worker),
        );
        body.end();
        ib.define_sub(main, body).unwrap();
        ib.define_sub(worker, defined_body(SubKind::Async)).unwrap();
        let image = ib.build(0).unwrap();
        assert!(
            image
                .code()
                .windows(2)
                .any(|pair| pair == [OP_PUSHI as u32, 1])
        );
    }

    /// `write_xform_locals`：逐槽生成 3 对 `PUSHI+POPL`，`word0` 打包位精确。
    #[test]
    fn write_xform_locals_packs_word0_bitwise() {
        let mut s = SubBuilder::new();
        let slot = XformSlot {
            wait: 3,
            op: 10,
            _pad: 0,
            args: [111, 222],
        };
        s.write_xform_locals(4, std::slice::from_ref(&slot));
        let expect_word0 = (3u32 << 16) | (10u32 << 8);
        assert_eq!(
            s.code,
            vec![
                OP_PUSHI as u32,
                expect_word0,
                OP_POPL as u32,
                4,
                OP_PUSHI as u32,
                111,
                OP_POPL as u32,
                5,
                OP_PUSHI as u32,
                222,
                OP_POPL as u32,
                6,
            ]
        );
    }

    // ── 生成码经真实 VM 跑一遍（stg-core::step 全公开面；`TaskPool`/池 SoA 数组按 P1
    // 纪律是 `pub(crate)`，外部 crate 摸不到——一度想用 stg-core 对本 crate 的
    // dev-dependency 绕过，撞上 Cargo"自引用 dev 依赖"的重复编译单元限制（同一 crate
    // 两份类型不互认），故改走 `globals`/`diag`/`iter_alive().count()` 这些真正公开的
    // 世界读口，见 `stg-core/Cargo.toml` 踩坑记录）───────────────────────────────

    use stg_core::input::InputFrame;
    use stg_core::step::{World, step};
    use stg_core::tables::TABLES_V0;

    /// 生成码经真实 VM 跑一遍：`repeat(3, body)` 里用 `sys_get_var`/`sys_set_var_from_stack`
    /// 把 `globals[20]` 累加 3 次（0→3）——`push_i`/`add`/`repeat` 回填三方合验，观测点走
    /// `WorldBody::globals`（真正公开字段，不借道任何 `pub(crate)` 内部）。槽号取 20（≥
    /// `stg_core::world::GLOBALS_SYS_SEGMENT`=16 的自由段）——M1.5 起 slot<16 是系统段，
    /// 脚本经 `sys_set_var` 写会被 no-op 守卫挡下（见该常量文档），本测试关心的是 `repeat`/
    /// `get_var`/`set_var_from_stack` 回填链路本身，不是系统段语义，故避开之。
    #[test]
    fn generated_repeat_code_roundtrips_through_real_vm() {
        const SLOT: u16 = 20;
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.sys_set_var(SLOT, 0); // globals[SLOT] = 0
        s.repeat(3, |b| {
            b.push_i(SLOT as i32); // 待写槽号（sys_set_var_from_stack 要求栈序 [slot, val]）
            b.sys_get_var(SLOT); // 读 globals[SLOT]
            b.push_i(1);
            b.add(); // 栈：[SLOT, globals[SLOT]+1]
            b.sys_set_var_from_stack(); // globals[SLOT] = 旧值+1
        });
        s.end();
        let main_id = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        ib.define_sub(main_id, s).unwrap();
        let image = ib.build(0).unwrap();

        let mut w = World::new(1);
        let _ = w.start_main(&image).expect("start_main 应成功");

        // 出生帧跳过；次帧首跑——无 WAIT，一次 exec 应跑到 END（3 次迭代远小于 1024 预算）。
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(0));
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(1));

        assert_eq!(
            w.body.view().globals()[SLOT as usize],
            3,
            "repeat(3) 应恰累加 3 次（读写走 globals，真实 VM 执行）"
        );
        assert_eq!(w.body.view().diag().task_faults, 0, "全程不应产生 Fault");
    }

    /// M1.5：`sys_self_age`/`sys_self_hp_max` 两个新读口 DSL 薄壳——经真实 VM 跑一遍，
    /// 结果走 globals relay 观测（同上一测试的观测惯例）。owner=STAGE：`self_age` 次帧首跑时
    /// 应为 1（出生帧 born_frame=0 不跑，`ctx.frame=1` 首次执行，`1-0=1`，与 `stg-core`
    /// `step.rs` 的端到端 off-by 判别同一钉死值）；`self_hp_max` 非敌 owner 恒 0。
    #[test]
    fn sys_self_age_and_hp_max_wrappers_roundtrip_through_real_vm() {
        const AGE_SLOT: u16 = 20;
        const HP_MAX_SLOT: u16 = 21;
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.push_i(AGE_SLOT as i32);
        s.sys_self_age();
        s.sys_set_var_from_stack();
        s.push_i(HP_MAX_SLOT as i32);
        s.sys_self_hp_max();
        s.sys_set_var_from_stack();
        s.end();
        let main_id = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        ib.define_sub(main_id, s).unwrap();
        let image = ib.build(0).unwrap();

        let mut w = World::new(1);
        w.start_main(&image).expect("start_main 应成功");
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(0)); // born 帧：门禁跳过
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(1)); // 次帧首跑

        assert_eq!(
            w.body.view().globals()[AGE_SLOT as usize],
            1,
            "born_frame=0，次帧首跑 ctx.frame=1，self_age=1-0=1"
        );
        assert_eq!(
            w.body.view().globals()[HP_MAX_SLOT as usize],
            0,
            "STAGE owner 的 self_hp_max 恒 0"
        );
        assert_eq!(w.body.view().diag().task_faults, 0, "全程不应产生 Fault");
    }

    /// 端到端小脚本（plan 既定简化版：不涉及敌人）——`wait(1)` 后 `repeat(2)` 两轮 4-way
    /// 批量环，`step` N 帧后断言弹数（`iter_alive().count()` 是 `BulletPool` 上真正公开
    /// 的方法，位置级细节留给 stg-core 自身对 `create_bullets_batch`/`SYS_CREATE_BULLETS_BATCH`
    /// 的判别式单测——本测试的职责是证明 DSL 生成的 `wait`/`repeat`/`sys_create_bullets_batch`
    /// 链路经真实调度跑通，不是重新验证批量创建几何，那条已在 Commit A 钉死）。
    #[test]
    fn end_to_end_script_wait_then_repeat_batch_ring_produces_expected_bullet_count() {
        let mut ib = ImageBuilder::new();
        let mut main = SubBuilder::new();
        main.wait(1);
        main.repeat(2, |s| {
            s.sys_create_bullets_batch(
                0,
                Fx::ZERO,
                Fx::from_int(100),
                4,
                Angle::ZERO,
                16384,
                1,
                Fx::from_int(2),
                Fx::ZERO,
            );
            s.wait(5);
        });
        main.end();
        let main_id = ib.declare_sub("main", SubKind::Root, &[]).unwrap();
        ib.define_sub(main_id, main).unwrap();
        let image = ib.build(0).unwrap();

        let mut w = World::new(1);
        w.start_main(&image).expect("start_main 应成功");

        for f in 0..20u32 {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
        }

        assert_eq!(
            w.body.view().bullets().iter_alive().count(),
            8,
            "repeat(2) × 4-way batch = 8 弹"
        );
        assert_eq!(w.body.view().diag().task_faults, 0, "全程不应产生 Fault");
    }
}
