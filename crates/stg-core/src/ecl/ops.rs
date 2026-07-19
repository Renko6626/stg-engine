//! ecl/ops.rs —— opcode 常量（编号即契约，冻结纪律同 `xform.rs` D4）+ 元数表 + 实现集判定。
//! 指令集 v1（`docs/superpowers/specs/2026-07-18-m1-ecl-vm-design.md`）：十位 = 族号，
//! 族内留空隙；新 op 落族内，永不乱序追加。

// ── 0x：控制 ──────────────────────────────────────────────────────────────
pub const OP_END: u8 = 0;
pub const OP_WAIT: u8 = 1;
pub const OP_JMP: u8 = 2;
pub const OP_JZ: u8 = 3;
pub const OP_CALL: u8 = 4;
pub const OP_RET: u8 = 5;

// ── 1x：栈 ────────────────────────────────────────────────────────────────
pub const OP_PUSHI: u8 = 10;
pub const OP_PUSHL: u8 = 11;
pub const OP_POPL: u8 = 12;
pub const OP_DUP: u8 = 13;
pub const OP_POP: u8 = 14;

// ── 2x：整数算术 ──────────────────────────────────────────────────────────
pub const OP_ADD: u8 = 20;
pub const OP_SUB: u8 = 21;
pub const OP_MUL: u8 = 22;
pub const OP_DIV: u8 = 23;
pub const OP_MOD: u8 = 24;
pub const OP_NEG: u8 = 25;

// ── 3x：定点/角度 ─────────────────────────────────────────────────────────
pub const OP_MULF: u8 = 30;
pub const OP_DIVF: u8 = 31;
pub const OP_SINB: u8 = 32;
pub const OP_COSB: u8 = 33;

// ── 4x：比较 ──────────────────────────────────────────────────────────────
pub const OP_EQ: u8 = 40;
pub const OP_NE: u8 = 41;
pub const OP_LT: u8 = 42;
pub const OP_LE: u8 = 43;
pub const OP_GT: u8 = 44;
pub const OP_GE: u8 = 45;

// ── 5x：任务（语义 T2 落地：SPAWN/KILL_SELF/KILL_CHILDREN，见 vm.rs）─────────
pub const OP_SPAWN: u8 = 50;
pub const OP_KILL_SELF: u8 = 51;
pub const OP_KILL_CHILDREN: u8 = 52;

// ── 6x：syscall（语义 T3 落地：派发进 ecl::syscall::dispatch，见 vm.rs/syscall.rs）──
pub const OP_SYS: u8 = 60;

/// 元数表：opcode → 内联操作数字数（JMP/JZ/CALL=1 目标字；PUSHI/PUSHL/POPL=1；
/// SPAWN=**2**（script id 字 + argc 字，M1.9 T3 起——表层语言 `spawn f(args)` 传参地基，
/// 见 `vm.rs::exec` 的 `OP_SPAWN` 分支文档）；SYS=1 syscall 号；其余 0；未实现 op 元数 0）。
/// 全 u8 域可安全索引。
pub const ARITY: [u8; 256] = {
    let mut a = [0u8; 256];
    a[OP_JMP as usize] = 1;
    a[OP_JZ as usize] = 1;
    a[OP_CALL as usize] = 1;
    a[OP_PUSHI as usize] = 1;
    a[OP_PUSHL as usize] = 1;
    a[OP_POPL as usize] = 1;
    a[OP_SPAWN as usize] = 2;
    a[OP_SYS as usize] = 1;
    a
};

/// 解码层已知的 op 集合——**按表查而非比大小**（族号制下编号非连续，`xform.rs`
/// 同款纪律）。`SPAWN`/`KILL_SELF`/`KILL_CHILDREN`（T2）与 `SYS`（T3）均已落地真实派发
/// 语义，见 `vm.rs`/`syscall.rs`。
pub const fn op_implemented(op: u8) -> bool {
    matches!(
        op,
        OP_END
            | OP_WAIT
            | OP_JMP
            | OP_JZ
            | OP_CALL
            | OP_RET
            | OP_PUSHI
            | OP_PUSHL
            | OP_POPL
            | OP_DUP
            | OP_POP
            | OP_ADD
            | OP_SUB
            | OP_MUL
            | OP_DIV
            | OP_MOD
            | OP_NEG
            | OP_MULF
            | OP_DIVF
            | OP_SINB
            | OP_COSB
            | OP_EQ
            | OP_NE
            | OP_LT
            | OP_LE
            | OP_GT
            | OP_GE
            | OP_SPAWN
            | OP_KILL_SELF
            | OP_KILL_CHILDREN
            | OP_SYS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编号冻结钉死（指令集 v1）。重排 = 契约变更，本测试逼改动者有意识确认。
    #[test]
    fn op_numbering_frozen_v1() {
        assert_eq!(OP_END, 0);
        assert_eq!((OP_WAIT, OP_JMP, OP_JZ, OP_CALL, OP_RET), (1, 2, 3, 4, 5));
        assert_eq!(
            (OP_PUSHI, OP_PUSHL, OP_POPL, OP_DUP, OP_POP),
            (10, 11, 12, 13, 14)
        );
        assert_eq!(
            (OP_ADD, OP_SUB, OP_MUL, OP_DIV, OP_MOD, OP_NEG),
            (20, 21, 22, 23, 24, 25)
        );
        assert_eq!((OP_MULF, OP_DIVF, OP_SINB, OP_COSB), (30, 31, 32, 33));
        assert_eq!(
            (OP_EQ, OP_NE, OP_LT, OP_LE, OP_GT, OP_GE),
            (40, 41, 42, 43, 44, 45)
        );
        assert_eq!((OP_SPAWN, OP_KILL_SELF, OP_KILL_CHILDREN), (50, 51, 52));
        assert_eq!(OP_SYS, 60);
    }

    /// 有效性按表查而非比大小；本刀已实现集恰为 31 个（含 5x/6x 的"解码已实现但派发未接线"
    /// 占位 op）；未实现/垃圾值一律 false；ARITY 全 u8 域可索引。
    #[test]
    fn op_implemented_table_and_arity_full_domain() {
        let implemented = [
            OP_END,
            OP_WAIT,
            OP_JMP,
            OP_JZ,
            OP_CALL,
            OP_RET,
            OP_PUSHI,
            OP_PUSHL,
            OP_POPL,
            OP_DUP,
            OP_POP,
            OP_ADD,
            OP_SUB,
            OP_MUL,
            OP_DIV,
            OP_MOD,
            OP_NEG,
            OP_MULF,
            OP_DIVF,
            OP_SINB,
            OP_COSB,
            OP_EQ,
            OP_NE,
            OP_LT,
            OP_LE,
            OP_GT,
            OP_GE,
            OP_SPAWN,
            OP_KILL_SELF,
            OP_KILL_CHILDREN,
            OP_SYS,
        ];
        assert_eq!(implemented.len(), 31);
        for op in 0..=255u8 {
            assert_eq!(
                op_implemented(op),
                implemented.contains(&op),
                "op {op} 的有效性判定错误"
            );
        }
        assert_eq!(ARITY[OP_JMP as usize], 1);
        assert_eq!(ARITY[OP_JZ as usize], 1);
        assert_eq!(ARITY[OP_CALL as usize], 1);
        assert_eq!(ARITY[OP_PUSHI as usize], 1);
        assert_eq!(ARITY[OP_PUSHL as usize], 1);
        assert_eq!(ARITY[OP_POPL as usize], 1);
        assert_eq!(
            ARITY[OP_SPAWN as usize], 2,
            "M1.9 T3：script id + argc 两字"
        );
        assert_eq!(ARITY[OP_SYS as usize], 1);
        assert_eq!(ARITY[OP_END as usize], 0);
        assert_eq!(ARITY[OP_RET as usize], 0);
        assert_eq!(ARITY[OP_KILL_SELF as usize], 0);
        assert_eq!(ARITY[OP_KILL_CHILDREN as usize], 0);
        assert_eq!(ARITY[255], 0);
    }
}
