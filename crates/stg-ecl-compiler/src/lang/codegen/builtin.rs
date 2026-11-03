//! 内建调用的参数处理与特殊指令降低。
//!
//! 通用表达式仍调用父模块的 `Gen::gen_expr`；Builtin 注册与两参糖起点继续由
//! `lang::builtins` 提供。本模块保持参数求值、引用压栈和尾指令的既有顺序。
//!
//! ## 弹 setter 族 `handle:int` 首参：求值后丢弃（T3 落地拍板）
//!
//! `lang::builtins` 报告点名的悬而未决项——`set_speed`/`set_angle`/`turn`/`set_vel`/
//! `set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player` 九个 setter 的首参
//! `handle:int`，底层 syscall 实际操作的是 `self`（任务 owner），并不消费一个显式句柄。
//! 本趟落地策略：**求值后立即丢弃**（保留副作用——万一作者写了带副作用的表达式——但
//! 不把它压进 syscall 实际吃的参数序列），不是"Fault if handle != self"（更严格但当前
//! 无法在编译期证明等值，运行期也没有校验入口）——最小惊讶、不新增运行期检查。
use super::Gen;
use crate::SubBuilder;
use crate::lang::builtins::{self, ParamKind};
use crate::lang::slots::SubSlots;
use crate::lang::typeck::{CallArg, const_val};

/// 弹 setter 族名单（模块文档"弹 setter 族 handle:int 首参"）。
fn is_self_bullet_setter(name: &str) -> bool {
    matches!(
        name,
        "set_speed"
            | "set_angle"
            | "turn"
            | "set_vel"
            | "set_ang_vel"
            | "set_accel"
            | "set_gravity"
            | "stop_fx"
            | "aim_at_player"
    )
}

/// 降低后要追发 `OP_KILL_SELF` 的内建（当前仅 `die`）。名字键控，与
/// [`is_self_bullet_setter`] 同款——集中在一处，免得散落在 `gen_builtin_call` 里。
fn emits_kill_self_after(name: &str) -> bool {
    name == "die"
}

/// 降低后要追发 `wait(1)` 的内建（当前仅 `stage_clear`）：`emit`/`SYS` 本身不让出帧，
/// 而「本关到此为止」的语义要求下一条语句在宿主放行后的第一帧才跑（壳子刀 2026-09-07）。
fn emits_wait_one_after(name: &str) -> bool {
    name == "stage_clear"
}

/// `phase_begin` 糖的常量注入（boss 换段刀 spec §3.1）：返回「在表层第 `i` 位**之前**要追压的立即数」。
/// 表层 `(slot, pattern, time_limit, hp_threshold)` → 字节码 `spell_begin` 7 参
/// `(slot, spell_id=0, pattern, time_limit, bonus0=0, flags=SPELL_NONSPELL, hp_threshold)`。
fn phase_begin_injects(name: &str, i: usize) -> &'static [i32] {
    const NONSPELL: i32 = stg_core::spell::SPELL_NONSPELL as i32;
    match (name, i) {
        ("phase_begin", 1) => &[0],
        ("phase_begin", 3) => &[0, NONSPELL],
        _ => &[],
    }
}

impl Gen<'_> {
    pub(super) fn gen_builtin_call(
        &mut self,
        b: &mut SubBuilder,
        slots: &SubSlots,
        bi: &'static builtins::Builtin,
        args: &[CallArg],
    ) {
        let mut program_call = false;
        let discard_first_handle = is_self_bullet_setter(bi.name);
        // 颜色轴糖：表层 (shape, color) 两参 → 字节码单个 appearance 值。折叠掉两位，
        // 故循环改成索引推进式（`zip(...).enumerate()` 一位一步走不了这个合并）。
        // 折叠**起点不总是 0**：`fire`/`batch` 折前两参，`sh_sprite(id, shape, color)` 折的
        // 是第 2、3 参。起点由 `builtins::fold_start` 单一权威给出（曾硬编码 `i == 0`）。
        let fold_start = builtins::fold_start(bi.name);
        let mut i = 0usize;
        while i < args.len() {
            if fold_start == Some(i) {
                match (const_val(&args[i]), const_val(&args[i + 1])) {
                    // 常量对：折成单个字面量——与手写单参字节码逐字节相同（零运行期开销）。
                    // `wrapping_add`：未绑定表时判据被跳过（`fire(i32::MAX, 1, …)` 可编），
                    // 编译器不该因整数溢出在 debug 构建里 abort——诊断拒绝可以，崩不行。
                    // 绑定表时判据已把两参钳在表内，绕不回来。
                    (Some(s), Some(c)) => b.push_i(s.wrapping_add(c)),
                    _ => {
                        let (CallArg::Val(se), CallArg::Val(ce)) = (&args[i], &args[i + 1]) else {
                            unreachable!("typeck 已保证两参糖的那两位是 Val")
                        };
                        self.gen_expr(b, slots, se);
                        self.gen_expr(b, slots, ce);
                        b.add();
                    }
                }
                i += 2;
                continue;
            }
            for &v in phase_begin_injects(bi.name, i) {
                b.push_i(v);
            }
            let (a, pk) = (&args[i], &bi.params[i]);
            match (a, pk) {
                (CallArg::Val(e), ParamKind::Val(_) | ParamKind::RawVal) => {
                    self.gen_expr(b, slots, e);
                    if i == 0 && discard_first_handle {
                        b.pop();
                    }
                }
                (CallArg::XformRef(name_opt), ParamKind::XformRef) => match name_opt {
                    Some(name) => match slots.xform_regions[name] {
                        crate::lang::slots::XformRegion::Legacy { off, cnt } => {
                            b.push_i(off as i32);
                            b.push_i(cnt as i32);
                        }
                        crate::lang::slots::XformRegion::ProgramArgs { off, argc } => {
                            b.push_i(self.xform_program_ids[name] as i32);
                            b.push_i(off as i32);
                            b.push_i(argc as i32);
                            program_call = true;
                        }
                    },
                    None => {
                        b.push_i(0);
                        b.push_i(0);
                    }
                },
                (CallArg::XformRefArgs(name, exprs), ParamKind::XformRef) => {
                    self.gen_xform_program_ref(b, slots, name, exprs);
                    program_call = true;
                }
                (CallArg::SubRef(name_opt), ParamKind::SubRef) => {
                    match name_opt {
                        Some(name) => b.push_task_ref(Some(self.name_to_ref[name])),
                        None => b.push_task_ref(None),
                    }
                    if bi.name == "spawn_enemy" {
                        b.push_i(0); // argc = 0（syscall 210 调用约定，boss 换段刀）
                    }
                }
                (CallArg::SubRefArgs(name, exprs), ParamKind::SubRef) => {
                    b.push_task_ref(Some(self.name_to_ref[name]));
                    for e in exprs {
                        self.gen_expr(b, slots, e);
                    }
                    b.push_i(exprs.len() as i32); // argc（syscall 210 调用约定）
                }
                _ => unreachable!("typeck 已保证 CallArg 与 ParamKind 一一对应"),
            }
            i += 1;
        }
        if bi.is_op {
            // `is_op` 直发路径的 `syscall` 字段实际装的是 VM op 码本身（`lang::builtins`
            // 模块文档"Builtin 字段形状"）——v1 只有 sin/cos 两个硬编码小常量，但断言
            // 挡住未来任何新增 `is_op:true` 条目手滑填了个超出 u8 的号（C19 复审修复）。
            debug_assert!(
                bi.syscall <= u8::MAX as u16,
                "'{}' 的 is_op 直发 op 码 {} 超出 u8 编码范围",
                bi.name,
                bi.syscall
            );
            b.raw_emit_op(bi.syscall as u8);
        } else {
            b.sys(if program_call {
                match bi.syscall {
                    stg_core::ecl::syscall::SYS_CREATE_BULLET => {
                        stg_core::ecl::syscall::SYS_CREATE_BULLET_PROGRAM
                    }
                    stg_core::ecl::syscall::SYS_SH_XFORM => {
                        stg_core::ecl::syscall::SYS_SH_XFORM_PROGRAM
                    }
                    _ => unreachable!("xform program used outside fire/sh_xform"),
                }
            } else {
                bi.syscall
            });
            if emits_kill_self_after(bi.name) {
                // `die()` 降低成两条指令：`SYS(SYS_DIE)` 跑死亡效果，`OP_KILL_SELF` 终止本
                // 任务（人类裁定 D-4）。这样做是因为 `syscall::dispatch` 的签名是
                // `Result<(), u8>`，没有"结束本任务"的返回通道——与其给六十个 match arm
                // 换返回类型，不如在这里发第二条指令（零 VM 改动、零 op 表改动）。
                b.raw_emit_op(stg_core::ecl::ops::OP_KILL_SELF);
            }
            if emits_wait_one_after(bi.name) {
                // `stage_clear(n)` = `SYS 723` + `wait(1)`：让出一帧，宿主看见事件就停拍。
                b.push_i(1);
                b.raw_wait();
            }
        }
    }
}
