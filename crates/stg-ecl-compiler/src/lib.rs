//! # stg-ecl-compiler —— ECL 字节码编译器（design_doc.md §4.5）
//!
//! 面向手感的表层语言（或 M1 先用的 Rust builder / 宏 DSL）→ 编译器 →
//! **字节码 + 常量表 = `EclImage`**（只读，内容哈希写入回放头与联机握手）。
//!
//! 只在**离线 / 加载期**运行，绝不进任何热路径（design_doc.md §1.1）。
//!
//! ## 分期
//!
//! - **M1**：先用 Rust builder / 宏 DSL 直接拼字节码，把 VM 语义与 syscall 表跑通
//!   （本刀，T3——**临时形态**，糖层语义即未来表层语言编译器的后端，非丢弃件）；
//! - 表层语言与其编译器随后再上（可与 M3+ 并行，联动 THTK 工具链）。
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
//! let main_id = ib.add_sub(main);
//! let image = ib.build();
//! assert_eq!(image.subs[main_id.0 as usize], 0);
//! ```

use stg_core::ecl::image::EclImage;
use stg_core::ecl::ops::{
    OP_ADD, OP_CALL, OP_COSB, OP_DIV, OP_DIVF, OP_DUP, OP_END, OP_EQ, OP_GE, OP_GT, OP_JMP, OP_JZ,
    OP_KILL_CHILDREN, OP_KILL_SELF, OP_LE, OP_LT, OP_MOD, OP_MUL, OP_MULF, OP_NE, OP_NEG, OP_POP,
    OP_POPL, OP_PUSHI, OP_PUSHL, OP_SINB, OP_SPAWN, OP_SUB, OP_SYS, OP_WAIT,
};
use stg_core::ecl::syscall;
use stg_core::ecl::task::LOCALS;
use stg_core::math::{Angle, Fx};
use stg_core::xform::XformSlot;

/// 脚本号——只能经 [`ImageBuilder::add_sub`] 取得（构造顺序即号——`build()` 时按此顺序
/// 把各 sub 的本地字节码依次拼进同一份扁平 `code`）。`call`/`spawn` 用它跨 sub 引用。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScriptId(pub u16);

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
    jump_fixups: Vec<usize>,
    /// 位置 → 目标 `ScriptId`：`build()` 时改写为该脚本的绝对入口（`call` 用；`spawn` 的
    /// 操作数是纯 script id 数值，VM 运行期自己查 `ecl.entry()`，故不需要这张表）。
    call_fixups: Vec<(usize, ScriptId)>,
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
            call_fixups: Vec::new(),
            next_repeat_slot: (LOCALS - 1) as u8,
            ended: false,
        }
    }

    #[inline]
    fn emit(&mut self, word: u32) -> usize {
        let p = self.code.len();
        self.code.push(word);
        p
    }

    #[inline]
    fn here(&self) -> usize {
        self.code.len()
    }

    #[inline]
    fn patch(&mut self, pos: usize, local_target: usize) {
        self.code[pos] = local_target as u32;
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

    /// 子程序调用（`OP_CALL`）：目标是另一 sub 的绝对入口，`build()` 时回填
    /// （`call_fixups`——此刻还不知道目标 sub 在最终拼接后的绝对偏移）。
    pub fn call(&mut self, sub: ScriptId) {
        self.emit(OP_CALL as u32);
        let p = self.emit(0);
        self.call_fixups.push((p, sub));
    }

    /// 协程派生（`OP_SPAWN`）：操作数是**纯 script id 数值**（VM 运行期自己
    /// `ecl.entry(script)` 查入口，见 `vm.rs::OP_SPAWN`），故不需要回填——立即写定。
    /// owner 继承自当前任务（VM 既定语义）；子句柄（池索引，失败 -1）留在求值栈顶。
    pub fn spawn(&mut self, sub: ScriptId) {
        self.emit(OP_SPAWN as u32);
        self.emit(sub.0 as u32);
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
        self.jump_fixups.push(p);
        self.patch(p, top);
    }

    /// 条件块：栈顶为 0（假）时跳过 `body`（`JZ`），非 0 时落入执行——**不消费额外栈**，
    /// 调用方须在此之前自行把条件值（如 `ge()` 的结果）压好。空 `body` 合法（JZ 目标
    /// 落在紧随其后，等价一次纯粹的条件求值 + 落地）。
    pub fn if_ge(&mut self, body: impl FnOnce(&mut Self)) {
        self.emit(OP_JZ as u32);
        let jz_pos = self.emit(0);
        self.jump_fixups.push(jz_pos);
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
        self.jump_fixups.push(jz_pos);

        self.emit(OP_JMP as u32);
        let jmp_pos = self.emit(0);
        self.jump_fixups.push(jmp_pos);
        self.patch(jmp_pos, top);

        let after = self.here();
        self.patch(jz_pos, after);

        self.next_repeat_slot += 1;
    }

    // ── syscall 类型化薄壳（v1 号表逐一覆盖；参数按 `ecl::syscall` 各自文档声明顺序
    // 正序压栈——与 `dispatch` 的逆序弹出严格配对，见该模块文档）────────────────────

    #[inline]
    fn sys(&mut self, no: u16) {
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
        task_script: Option<ScriptId>,
    ) {
        self.push_i(appearance as i32);
        self.push_i(x.raw());
        self.push_i(y.raw());
        self.push_i(speed.raw());
        self.push_i(angle.raw() as i32);
        self.push_i(xform_off);
        self.push_i(xform_cnt);
        self.push_i(task_script.map_or(-1, |s| s.0 as i32));
        self.sys(syscall::SYS_CREATE_BULLET);
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

/// 镜像构建器：`add_sub` 收集各 sub 的构建器（赋 [`ScriptId`] = 加入序），`build` 拼接成
/// 一份扁平 `EclImage`（回填跨 sub `call` 目标 + 平移每 sub 内部的局部跳转目标）。
#[derive(Default)]
pub struct ImageBuilder {
    subs: Vec<SubBuilder>,
}

impl ImageBuilder {
    pub fn new() -> Self {
        ImageBuilder { subs: Vec::new() }
    }

    /// 登记一个已构建的子程序，返回其 [`ScriptId`]（= 当前 `subs.len()`，即加入顺序）。
    pub fn add_sub(&mut self, sub: SubBuilder) -> ScriptId {
        let id = ScriptId(self.subs.len() as u16);
        self.subs.push(sub);
        id
    }

    /// 拼接为 `EclImage`：
    /// 1. 未调用过 [`SubBuilder::end`] 的 sub 自动补一条 `OP_END`（宁可正常收尾，不留
    ///    悬空字节码——脚本作者忘写 `end()` 不该变成 `FAULT_PC_OOB`）。
    /// 2. 逐 sub 顺序拼接本地 `code` 进最终扁平数组，记下各自的绝对基址（= 入口，
    ///    `EclImage.subs[i]`）。
    /// 3. 局部跳转回填（`jump_fixups`）：每处操作数原本是"sub 本地目标 pc"，整体
    ///    `+= base_offset` 变成全局绝对 pc。
    /// 4. 跨 sub 调用回填（`call_fixups`）：每处操作数原本是占位 0，改写为目标 sub 的
    ///    绝对入口（`base_offset[target]`）。
    ///
    /// `content_hash` 占位 0（同 `WorldTables`/`EclImage` 现有惯例，文件加载刀再补真哈希）。
    pub fn build(mut self) -> EclImage {
        for s in &mut self.subs {
            if !s.ended {
                s.end();
            }
        }

        let mut base_offsets: Vec<usize> = Vec::with_capacity(self.subs.len());
        let mut code: Vec<u32> = Vec::new();
        for s in &self.subs {
            base_offsets.push(code.len());
            code.extend_from_slice(&s.code);
        }

        for (i, s) in self.subs.iter().enumerate() {
            let base = base_offsets[i];
            for &p in &s.jump_fixups {
                code[base + p] += base as u32;
            }
            for &(p, target) in &s.call_fixups {
                code[base + p] = base_offsets[target.0 as usize] as u32;
            }
        }

        let subs: Vec<u32> = base_offsets.into_iter().map(|o| o as u32).collect();
        EclImage {
            code,
            subs,
            content_hash: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空 `ImageBuilder`（无 sub）→ 空镜像，等价 `EclImage::empty()` 的形状。
    #[test]
    fn empty_builder_yields_empty_image() {
        let image = ImageBuilder::new().build();
        assert!(image.code.is_empty());
        assert!(image.subs.is_empty());
    }

    /// `end()` 未显式调用 → `build()` 自动补一条（入口仍指向 sub 起点，序列以 END 收尾）。
    #[test]
    fn missing_end_is_auto_appended() {
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.push_i(1);
        let id = ib.add_sub(s);
        let image = ib.build();
        assert_eq!(image.subs[id.0 as usize], 0);
        assert_eq!(*image.code.last().unwrap(), OP_END as u32, "自动补 END");
    }

    /// 两个 sub 顺序拼接：第二个 sub 的入口 = 第一个 sub 的 code 长度（基址平移正确）。
    #[test]
    fn two_subs_entries_are_sequential_base_offsets() {
        let mut ib = ImageBuilder::new();
        let mut a = SubBuilder::new();
        a.push_i(1);
        a.end();
        let a_len = a.code.len() as u32;
        let a_id = ib.add_sub(a);

        let mut b = SubBuilder::new();
        b.push_i(2);
        b.end();
        let b_id = ib.add_sub(b);

        let image = ib.build();
        assert_eq!(image.subs[a_id.0 as usize], 0);
        assert_eq!(image.subs[b_id.0 as usize], a_len);
    }

    /// `loop_forever`：回填的 `JMP` 目标必须精确落在 `body` 起点（哪怕空 `body`）。
    #[test]
    fn loop_forever_backpatches_to_body_start_even_when_empty() {
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.loop_forever(|_| {}); // 空 body：应生成恰一条自跳转 JMP 0
        let id = ib.add_sub(s);
        let image = ib.build();
        let entry = image.subs[id.0 as usize] as usize;
        assert_eq!(image.code[entry], OP_JMP as u32);
        assert_eq!(
            image.code[entry + 1],
            entry as u32,
            "空 body 回环：JMP 目标 = 自身起点"
        );
    }

    /// `if_ge`：`JZ` 回填目标必须精确落在 `body` 之后（空 body 时紧挨 JZ 自身之后）。
    #[test]
    fn if_ge_backpatches_past_empty_body() {
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.if_ge(|_| {});
        s.push_i(42);
        s.end();
        let id = ib.add_sub(s);
        let image = ib.build();
        let entry = image.subs[id.0 as usize] as usize;
        assert_eq!(image.code[entry], OP_JZ as u32);
        // JZ 头字(entry) + 操作数字(entry+1) 之后紧跟 PUSHI 42（entry+2）——目标应指向此处。
        assert_eq!(image.code[entry + 1], (entry + 2) as u32);
        assert_eq!(image.code[entry + 2], OP_PUSHI as u32);
    }

    /// 嵌套 `repeat`：外层 slot=63、内层 slot=62（自动降一格，不与外层计数器相撞）——
    /// 通过读取生成码里 `POPL`/`PUSHL` 的槽号操作数间接验证（结构细节，非公开 API，
    /// 但值得钉死以防"忘记降格"回归）。
    #[test]
    fn nested_repeat_uses_distinct_locals_slots() {
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.repeat(2, |outer| {
            outer.repeat(3, |_inner| {});
        });
        s.end();
        let id = ib.add_sub(s);
        let image = ib.build();
        let entry = image.subs[id.0 as usize] as usize;
        // 布局：PUSHI 2, POPL slot_outer, [PUSHI 3, POPL slot_inner, ... inner loop-back ...],
        // PUSHL slot_outer, PUSHI 1, SUB, DUP, POPL slot_outer, JZ, JMP, END
        assert_eq!(image.code[entry], OP_PUSHI as u32);
        assert_eq!(image.code[entry + 1], 2);
        assert_eq!(image.code[entry + 2], OP_POPL as u32);
        let outer_slot = image.code[entry + 3];
        assert_eq!(outer_slot, 63, "外层 repeat 计数槽 = LOCALS-1");
        assert_eq!(image.code[entry + 4], OP_PUSHI as u32);
        assert_eq!(image.code[entry + 5], 3);
        assert_eq!(image.code[entry + 6], OP_POPL as u32);
        let inner_slot = image.code[entry + 7];
        assert_eq!(inner_slot, 62, "内层 repeat 降一格，不与外层相撞");
    }

    /// 空 `repeat` body（`n>0`）：body 内无指令，但计数/跳转骨架仍完整生成
    /// （回填不因空 body 而错位——恰跳过 0 条指令）。
    #[test]
    fn repeat_empty_body_boundary() {
        let mut ib = ImageBuilder::new();
        let mut s = SubBuilder::new();
        s.repeat(5, |_| {});
        s.push_i(9);
        s.end();
        let id = ib.add_sub(s);
        let image = ib.build();
        let entry = image.subs[id.0 as usize] as usize;
        // PUSHI 5, POPL slot [body 空], PUSHL slot, PUSHI 1, SUB, DUP, POPL slot, JZ tgt, JMP top
        // top = entry+4（body 起点，空）；JZ 落在 body 之后的判定序列结束处。
        let top = entry + 4;
        assert_eq!(image.code[entry], OP_PUSHI as u32);
        assert_eq!(image.code[entry + 1], 5);
        assert_eq!(image.code[entry + 2], OP_POPL as u32);
        assert_eq!(
            image.code[top], OP_PUSHL as u32,
            "body 空——判定序列紧跟计数器初始化"
        );
        // JZ/JMP 位置：top + [PUSHL,slot(2), PUSHI,1(2), SUB(1), DUP(1), POPL,slot(2)] = top+8
        let jz_pos = top + 8;
        assert_eq!(image.code[jz_pos], OP_JZ as u32);
        let jmp_pos = jz_pos + 2;
        assert_eq!(image.code[jmp_pos], OP_JMP as u32);
        assert_eq!(
            image.code[jmp_pos + 1],
            top as u32,
            "JMP 回环目标 = body 起点（top）"
        );
        let after = jmp_pos + 2;
        assert_eq!(
            image.code[jz_pos + 1],
            after as u32,
            "JZ 目标 = 循环之后（PUSHI 9）"
        );
        assert_eq!(image.code[after], OP_PUSHI as u32);
        assert_eq!(image.code[after + 1], 9);
    }

    /// `call`：跨 sub 回填——操作数最终等于目标 sub 的绝对入口（`base_offsets[target]`），
    /// 不论目标 sub 是在调用方之前还是之后 `add_sub`。
    #[test]
    fn call_backpatches_to_absolute_target_entry_regardless_of_add_order() {
        // 目标 sub 在调用方**之前**添加（callee 先 add_sub，拿到 ScriptId 后 caller 才引用）。
        let mut ib = ImageBuilder::new();
        let mut callee = SubBuilder::new();
        callee.push_i(77);
        callee.end();
        let callee_id = ib.add_sub(callee);

        let mut caller = SubBuilder::new();
        caller.call(callee_id);
        caller.end();
        let caller_id = ib.add_sub(caller);

        let image = ib.build();
        let caller_entry = image.subs[caller_id.0 as usize] as usize;
        let callee_entry = image.subs[callee_id.0 as usize] as usize;
        assert_eq!(image.code[caller_entry], OP_CALL as u32);
        assert_eq!(
            image.code[caller_entry + 1],
            callee_entry as u32,
            "CALL 操作数回填为目标绝对入口"
        );
    }

    /// `spawn`：操作数是纯 script id 数值（不回填、不随拼接偏移变化）。
    #[test]
    fn spawn_operand_is_plain_script_id_no_fixup() {
        let mut ib = ImageBuilder::new();
        let mut a = SubBuilder::new();
        a.push_i(0);
        a.end();
        let a_id = ib.add_sub(a);

        let mut b = SubBuilder::new();
        b.spawn(a_id);
        b.end();
        let b_id = ib.add_sub(b);

        let image = ib.build();
        let b_entry = image.subs[b_id.0 as usize] as usize;
        assert_eq!(image.code[b_entry], OP_SPAWN as u32);
        assert_eq!(
            image.code[b_entry + 1],
            a_id.0 as u32,
            "SPAWN 操作数恒 = script id"
        );
    }

    /// `sys_create_bullet` 生成的压栈序 == 手写 8 参正序（`SYS_CREATE_BULLET` 号紧随其后）。
    #[test]
    fn sys_create_bullet_emits_forward_order_args_and_sys_number() {
        let mut s = SubBuilder::new();
        s.sys_create_bullet(
            2,
            Fx::from_int(10),
            Fx::from_int(-20),
            Fx::from_int(3),
            Angle::QUARTER,
            5,
            1,
            Some(ScriptId(7)),
        );
        let expect = vec![
            OP_PUSHI as u32,
            2,
            OP_PUSHI as u32,
            Fx::from_int(10).raw() as u32,
            OP_PUSHI as u32,
            Fx::from_int(-20).raw() as u32,
            OP_PUSHI as u32,
            Fx::from_int(3).raw() as u32,
            OP_PUSHI as u32,
            Angle::QUARTER.raw() as u32,
            OP_PUSHI as u32,
            5,
            OP_PUSHI as u32,
            1,
            OP_PUSHI as u32,
            7,
            OP_SYS as u32,
            syscall::SYS_CREATE_BULLET as u32,
        ];
        assert_eq!(s.code, expect);
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

    use stg_core::ecl::task::OWNER_STAGE;
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
        let main_id = ib.add_sub(s);
        let image = ib.build();

        let mut w = World::new(1);
        let idx = w
            .spawn_task(&image, main_id.0, (OWNER_STAGE, 0, 0))
            .unwrap();

        // 出生帧跳过；次帧首跑——无 WAIT，一次 exec 应跑到 END（3 次迭代远小于 1024 预算）。
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(0));
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(1));

        assert_eq!(
            w.body.globals[SLOT as usize], 3,
            "repeat(3) 应恰累加 3 次（读写走 globals，真实 VM 执行）"
        );
        assert_eq!(w.body.diag.task_faults, 0, "全程不应产生 Fault");
        let _ = idx; // 任务已跑完自灭；本测试只关心可观测的世界效应
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
        let main_id = ib.add_sub(s);
        let image = ib.build();

        let mut w = World::new(1);
        w.spawn_task(&image, main_id.0, (OWNER_STAGE, 0, 0))
            .unwrap();
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(0)); // born 帧：门禁跳过
        step(&mut w, &TABLES_V0, &image, &InputFrame::empty(1)); // 次帧首跑

        assert_eq!(
            w.body.globals[AGE_SLOT as usize], 1,
            "born_frame=0，次帧首跑 ctx.frame=1，self_age=1-0=1"
        );
        assert_eq!(
            w.body.globals[HP_MAX_SLOT as usize], 0,
            "STAGE owner 的 self_hp_max 恒 0"
        );
        assert_eq!(w.body.diag.task_faults, 0, "全程不应产生 Fault");
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
        let main_id = ib.add_sub(main);
        let image = ib.build();

        let mut w = World::new(1);
        w.spawn_task(&image, main_id.0, (OWNER_STAGE, 0, 0))
            .unwrap();

        for f in 0..20u32 {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
        }

        assert_eq!(
            w.body.bullets.iter_alive().count(),
            8,
            "repeat(2) × 4-way batch = 8 弹"
        );
        assert_eq!(w.body.diag.task_faults, 0, "全程不应产生 Fault");
    }
}
