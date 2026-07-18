//! ecl/vm.rs —— 单任务解释核（T1：字流解码 + 双层预算完整；`SPAWN`/`KILL_*`/`SYS` 解码
//! 通过但派发未接线——踩到即 `Exec::Fault(FAULT_UNIMPLEMENTED)`，T2/T3 接管实际语义）。
//!
//! 指令编码：1 头字（opcode 在低 8 位，余位留白供未来 mask）+ N 操作数字（N 由
//! `ops::ARITY` 钉死）。循环：预算门（任务 1024 + 全局）→ 取头字（pc 越界 Fault(1)）→
//! op 低 8 位 → `op_implemented`? → 按 `ARITY` 取操作数字（越界 Fault(1)）→ 语义分派 →
//! `WAIT` 写 `wait` 并 `Yield` / `END` → `End`。
//!
//! **T1 地基无消费者**：本文件的全部公开面（`exec`/`VmCtx`/`Exec`/`FAULT_*`/`TASK_BUDGET`）
//! 只被本文件自己的单测调用——协程调度接入相位 2 导演槽是 T2 的事，届时这里的 dead_code
//! 允许会随第一个真调用点自然解除。
#![allow(dead_code)]

use crate::ecl::ops::{self, ARITY};
use crate::ecl::task::{CALL_DEPTH, EVAL_DEPTH, LOCALS, Task};
use crate::math::Angle;
use crate::math::trig;

/// 单任务每帧指令预算（spec 拍板 2：双层，超限确定性杀）。
pub const TASK_BUDGET: u32 = 1024;

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
/// **T1 占位**：`SPAWN`/`KILL_SELF`/`KILL_CHILDREN`/`SYS` 解码已实现但派发未接线
/// （T2 接协程调度、T3 接 syscall 表）。与 0-5 五个"确定性坏行为"物理区分——命中这里
/// 不是作者 bug，是本刀故意留白；T2/T3 落地后这个变体的测试会被替换为真实语义测试。
pub const FAULT_UNIMPLEMENTED: u8 = 6;

/// 单次 `exec` 调用的执行结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exec {
    Yield,
    End,
    Fault(u8),
}

/// 单帧执行上下文：本任务将要解码的字节码 + 全局剩余预算（跨任务共享，按池序消耗，I4；
/// T1 未做调度层，测试直接构造）。
pub(crate) struct VmCtx<'a> {
    pub code: &'a [u32],
    pub budget: &'a mut u32,
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
            ops::OP_SPAWN | ops::OP_KILL_SELF | ops::OP_KILL_CHILDREN | ops::OP_SYS => {
                return Exec::Fault(FAULT_UNIMPLEMENTED);
            }
            _ => return Exec::Fault(FAULT_BAD_OP), // 防御：op_implemented 与本 match 若失步，不 panic
        }
        task.pc = next_pc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::ops::*;
    use crate::math::Fx;

    fn run(code: &[u32]) -> (Exec, Task) {
        let mut task = Task::default();
        let mut budget = u32::MAX;
        let mut ctx = VmCtx {
            code,
            budget: &mut budget,
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
        let mut ctx = VmCtx {
            code: &code,
            budget: &mut budget,
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
        let mut ctx = VmCtx {
            code: &code,
            budget: &mut budget,
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
        let mut ctx = VmCtx {
            code: &[OP_PUSHL as u32, 3],
            budget: &mut budget,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(r, Exec::Fault(FAULT_PC_OOB)); // 无 END：code 耗尽后下一次 fetch 越界
        assert_eq!(task.stack[0], 77);

        let mut task2 = Task::default();
        let mut budget2 = u32::MAX;
        let mut ctx2 = VmCtx {
            code: &[OP_PUSHI as u32, 55, OP_POPL as u32, 9],
            budget: &mut budget2,
        };
        let r2 = exec(&mut task2, &mut ctx2);
        assert_eq!(r2, Exec::Fault(FAULT_PC_OOB));
        assert_eq!(task2.locals[9], 55);
    }

    /// SPAWN/KILL_SELF/KILL_CHILDREN/SYS：T1 解码通过但派发未接线，统一 `Fault(FAULT_UNIMPLEMENTED)`
    /// ——与既定 0-5 五个"确定性坏行为" fault 码物理区分，T2/T3 接线时把这里替换为真实语义测试。
    #[test]
    fn unwired_task_and_syscall_ops_return_dedicated_fault() {
        assert_eq!(FAULT_UNIMPLEMENTED, 6, "占位 fault 码钉死");
        let (r, _) = run(&[OP_SPAWN as u32, 0]);
        assert_eq!(r, Exec::Fault(FAULT_UNIMPLEMENTED));
        let (r, _) = run(&[OP_KILL_SELF as u32]);
        assert_eq!(r, Exec::Fault(FAULT_UNIMPLEMENTED));
        let (r, _) = run(&[OP_KILL_CHILDREN as u32]);
        assert_eq!(r, Exec::Fault(FAULT_UNIMPLEMENTED));
        let (r, _) = run(&[OP_SYS as u32, 0]);
        assert_eq!(r, Exec::Fault(FAULT_UNIMPLEMENTED));
    }
}
