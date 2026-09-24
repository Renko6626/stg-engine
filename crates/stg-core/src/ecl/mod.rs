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

/// safe named-entry binding API (Task 3)
pub mod binding;
pub mod image;
pub mod ops;
/// 预存发射参数集（shooter 刀 2026-07-31）——每任务 4 个发射器槽，存储挂 `TaskPool`。
pub mod shooter;
/// syscall 号表 v1（`SYS_*` 常量，`pub`）——`stg-ecl-compiler` 的 builder DSL 靠它拼
/// `OP_SYS` 指令（依赖方向 compiler→core 单向，只取常量，不碰 `dispatch`）。`dispatch` 本身
/// 仍 `pub(crate)`：只有 `vm::exec` 能调用，编译器够不到派发逻辑，只够到号表。
pub mod syscall;
pub mod task;
pub(crate) mod vm;

pub use image::EclImage;

/// Fault 码与其规范短名（F9）——`vm` 模块本身是 `pub(crate)`，这几件却是**断层线以上要用的
/// 契约**（`stg-harness run` 把 fault 打给脚本作者看），故在这里单独开口。只出常量与名字表，
/// 不出 `Exec`/`run_tasks` 那些派发内脏。
pub use vm::{
    FAULT_BAD_OP, FAULT_BUDGET, FAULT_CALL_DEPTH, FAULT_DIV_ZERO, FAULT_NAMES, FAULT_PC_OOB,
    FAULT_STACK, FAULT_UNIMPLEMENTED,
};

/// VM fuzz 冒烟（§9 承诺的最小版，M1-T5；引擎第二刀 §4 起随加载时校验改口径，
/// Task 2 复审 Important 修复：改成"两种镜像各半"）：确定性 PRNG 生成 64 字镜像，断言
/// 三件事——① 每次抽样按下标奇偶在**两种生成策略间交替**：偶数下标"biased"（op 从
/// `SAFE_ARITY0`/`SAFE_ARITY1` 里挑、操作数按 op 语义配好，头字高位天然为 0）——这类
/// 镜像**恒能通过**加载闸，专门用来把运行时路径（栈/预算/除零/调用深度/pc 越界/syscall
/// 动态检查）真正跑起来；奇数下标"raw"（`rng.next_u32()` 直出全域随机字，同 T5 原版）——
/// 头字高 24 位恰好全 0 的概率只有 2⁻²⁴，实质上**恒被拒**，专门覆盖"加载闸拒绝路径不
/// panic"。② 通过校验、真的跑起来的那些镜像永不 panic，Fault 只能来自运行时仍保留的那
/// 几类（栈/预算/除零/调用深度/pc 越界，外加 syscall 内部动态检查复用的 BAD_OP）。
/// ③ 剥去副作用 op（SYS/SPAWN）的垃圾码**无法越权触碰 WorldBody**（对照空镜像参考世界
/// 逐字段哈希全等——白名单沙箱的实证）。`accepted`/`rejected` 两端在固定种子下都远超
/// 各自 10% 的下限（见两条测试末尾的断言与其旁注的实测数字）。
#[cfg(test)]
mod fuzz_smoke {
    use crate::checksum::Checksum;
    use crate::ecl::image::{EclImage, ImageBuildError, ImageParts, SubInit, SubKind};
    use crate::ecl::ops;
    use crate::ecl::syscall;
    use crate::ecl::task::{LOCALS, OWNER_STAGE, Task};
    use crate::ecl::vm::{Exec, VmCtx, exec};
    use crate::ecl::{
        FAULT_BAD_OP, FAULT_BUDGET, FAULT_CALL_DEPTH, FAULT_DIV_ZERO, FAULT_PC_OOB, FAULT_STACK,
    };
    use crate::events::EVT_TASK_FAULT;
    use crate::input::InputFrame;
    use crate::rng::Pcg32;
    use crate::step::{World, step};
    use crate::tables::TABLES_V0;

    /// "biased"生成用的 0 元 op 池——都是已实现、且**不带任何静态可判约束**的 op（没有
    /// JMP/JZ/CALL/SPAWN：它们的目标合法性是加载闸的重头戏，样例已经躺在 `image.rs` 的
    /// `validate_*` 单测和 `vm.rs` 的既有单测里，这里不必也不想重复造轮子）。运行时它们
    /// 各自可能栈上溢/下溢（空栈 `POP`/`DUP`/算术族）、除零（`DIV`/`MOD`/`DIVF`）、调用
    /// 栈空 `RET`——都落在 `ALLOWED_RUNTIME_FAULTS` 里。
    const SAFE_ARITY0: &[u8] = &[
        ops::OP_END,
        ops::OP_WAIT,
        ops::OP_RET,
        ops::OP_DUP,
        ops::OP_POP,
        ops::OP_ADD,
        ops::OP_SUB,
        ops::OP_MUL,
        ops::OP_DIV,
        ops::OP_MOD,
        ops::OP_NEG,
        ops::OP_MULF,
        ops::OP_DIVF,
        ops::OP_SINB,
        ops::OP_COSB,
        ops::OP_EQ,
        ops::OP_NE,
        ops::OP_LT,
        ops::OP_LE,
        ops::OP_GT,
        ops::OP_GE,
        ops::OP_KILL_SELF,
        ops::OP_KILL_CHILDREN,
    ];

    /// "biased"生成用的 1 元 op 池（含 `SYS`；`strip_effects` 时去掉 `SYS`，同 raw 生成器
    /// 把副作用 op 中性化的用意一致）。`PUSHI` 操作数任意；`PUSHL`/`POPL` 钳进
    /// `[0, LOCALS)`；`SYS` 从 `SAFE_SYSCALLS` 里挑，保证号在白名单内。
    const SAFE_ARITY1_WITH_SYS: &[u8] = &[ops::OP_PUSHI, ops::OP_PUSHL, ops::OP_POPL, ops::OP_SYS];
    const SAFE_ARITY1_NO_SYS: &[u8] = &[ops::OP_PUSHI, ops::OP_PUSHL, ops::OP_POPL];

    /// 挑几个 owner 无关（STAGE 也能安全派发、不 panic——`self_pos`/`self_hp` 对非
    /// ENEMY/BULLET 恒返回零值，见 `syscall.rs` 文档）、参数个数为 0 的号，好在 biased
    /// 生成器里直接内联，不必额外压参数。
    const SAFE_SYSCALLS: &[u16] = &[
        syscall::SYS_FRAME,
        syscall::SYS_SELF_X,
        syscall::SYS_SELF_HP,
        syscall::SYS_RAND_RANGE,
        syscall::SYS_GET_VAR,
    ];

    /// biased 64 字：按 `ARITY` 语义逐条拼装，头字高位天然为 0、操作数天然合法——
    /// **恒能通过** `validate_code`（不含 JMP/JZ/CALL/SPAWN，没有跳转目标/CALL·SPAWN
    /// 目标这类需要跨指令核对的约束；`PUSHL`/`POPL`/`SYS` 各自的约束在生成时就地满足）。
    fn biased_code(rng: &mut Pcg32, strip_effects: bool) -> Vec<u32> {
        let arity1 = if strip_effects {
            SAFE_ARITY1_NO_SYS
        } else {
            SAFE_ARITY1_WITH_SYS
        };
        let mut code = Vec::with_capacity(64);
        while code.len() < 64 {
            let remaining = 64 - code.len();
            let use_arity1 = remaining >= 2 && rng.next_u32().is_multiple_of(2);
            let op = if use_arity1 {
                arity1[(rng.next_u32() as usize) % arity1.len()]
            } else {
                SAFE_ARITY0[(rng.next_u32() as usize) % SAFE_ARITY0.len()]
            };
            code.push(op as u32);
            match op {
                ops::OP_PUSHI => code.push(rng.next_u32()),
                ops::OP_PUSHL | ops::OP_POPL => code.push(rng.next_u32() % LOCALS as u32),
                ops::OP_SYS => {
                    code.push(SAFE_SYSCALLS[(rng.next_u32() as usize) % SAFE_SYSCALLS.len()] as u32)
                }
                _ => {}
            }
        }
        debug_assert_eq!(code.len(), 64, "biased 生成器必须恰好填满 64 字，不多不少");
        code
    }

    /// raw 64 字：`rng.next_u32()` 直出全域随机字（T5 原版逻辑）；`strip_effects` 把
    /// SYS（改世界）与 SPAWN（池满时改 diag）换成 POP。头字高 24 位恰好全 0 的概率只有
    /// 2⁻²⁴，64 字连续通过整条校验器的概率实质为零——这条生成器专门覆盖"加载闸拒绝路径
    /// 不 panic"，不指望它产出能跑的镜像。
    fn raw_random_code(rng: &mut Pcg32, strip_effects: bool) -> Vec<u32> {
        let mut code = Vec::with_capacity(64);
        for _ in 0..64 {
            let mut w = rng.next_u32();
            if strip_effects {
                let op = (w & 0xFF) as u8;
                if op == ops::OP_SYS || op == ops::OP_SPAWN {
                    w = (w & !0xFF) | ops::OP_POP as u32;
                }
            }
            code.push(w);
        }
        code
    }

    /// 64 字镜像；`biased` 选生成策略（见 `biased_code`/`raw_random_code` 文档），
    /// `strip_effects` 把 SYS（改世界）与 SPAWN（池满时改 diag）中性化。直接过
    /// `try_from_parts`（不再 `test_image` 的 `.expect()`）——加载时校验器会真的审这段
    /// 字节码；调用方按 `Err` 计数、`continue`，不 panic。
    fn random_image(
        rng: &mut Pcg32,
        strip_effects: bool,
        biased: bool,
    ) -> Result<EclImage, ImageBuildError> {
        let code = if biased {
            biased_code(rng, strip_effects)
        } else {
            raw_random_code(rng, strip_effects)
        };
        EclImage::try_from_parts(ImageParts {
            code,
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        })
    }

    /// 运行时仍可能真的产出的 fault 码——静态可判的坏码（坏 op/保留位/操作数越界/非法跳转/
    /// CALL·SPAWN 目标不对/局部下标越界/坏 syscall 号）已被加载闸挡在了镜像构造期；这里只
    /// 剩动态检查。`FAULT_BAD_OP` 仍在表里：syscall 内部的动态检查（坏 owner 类别等）复用它。
    const ALLOWED_RUNTIME_FAULTS: [u8; 6] = [
        FAULT_PC_OOB,
        FAULT_STACK,
        FAULT_BUDGET,
        FAULT_DIV_ZERO,
        FAULT_CALL_DEPTH,
        FAULT_BAD_OP,
    ];

    fn assert_faults_are_allowed(events: &[crate::events::Event], ctx: &str) {
        for e in events {
            if e.kind == EVT_TASK_FAULT {
                let code = e.data[0] as u8;
                assert!(
                    ALLOWED_RUNTIME_FAULTS.contains(&code),
                    "{ctx}：运行时产出了加载时理应已经挡住的 fault 码 {code}"
                );
            }
        }
    }

    /// ① 256 次抽样（偶数下标 biased、奇数下标 raw，各 128 次）× 60 帧：被校验器拒的记
    /// 个数、跳过；通过的照原来的方式跑——不 panic，fault 码只能来自运行时仍保留的那几类。
    ///
    /// **固定种子 `0xF0_22_5E_ED` 下的实测**：`accepted == 128`、`rejected == 128`——biased
    /// 那 128 份全过（生成时已保证语义合法，见 `biased_code` 文档），raw 那 128 份全被拒
    /// （`ReservedBits` 门槛概率 2⁻²⁴，raw 生成器不指望产出能跑的镜像）。两端都遠超"各
    /// ≥10%"的下限，断言按 `iterations / 20`（=12，≈4.7%，留出充足余量）卡下限，不写死
    /// 128 这个随生成器实现细节可能变的精确值。
    #[test]
    fn fuzz_random_code_never_panics() {
        let mut rng = Pcg32::new(0xF0_22_5E_ED, 0x1);
        const ITERATIONS: u64 = 256;
        let mut rejected = 0u32;
        let mut accepted = 0u32;
        for i in 0..ITERATIONS {
            let biased = i % 2 == 0;
            let img = match random_image(&mut rng, false, biased) {
                Ok(img) => img,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
            accepted += 1;
            let mut w = World::new(0x1000 + i);
            w.spawn_sub_internal(&img, img.root().unwrap(), &[], (OWNER_STAGE, 0, 0));
            for f in 0..60u32 {
                step(&mut w, &TABLES_V0, &img, &InputFrame::empty(f));
                assert_faults_are_allowed(w.body.frame_events(), "fuzz_random_code_never_panics");
            }
        }
        let min_substantial = (ITERATIONS / 20) as u32; // ≥5% 下限，远低于实测的 50%
        assert!(
            rejected >= min_substantial,
            "64 字镜像不该只有 {rejected}/{ITERATIONS} 份被加载时校验拒绝（下限 {min_substantial}）"
        );
        assert!(
            accepted >= min_substantial,
            "64 字镜像不该只有 {accepted}/{ITERATIONS} 份通过加载时校验、真正跑起来（下限 {min_substantial}）"
        );
    }

    /// ② 128 次去副作用抽样（偶数下标 biased、奇数下标 raw，各 64 次）vs 空镜像参考
    /// 世界：被拒的同样跳过、计数；通过的除 tasks/diag（task_faults 计数）外，WorldBody
    /// 逐字段哈希全等——垃圾码只能烧自己的预算，摸不到世界（syscall 白名单沙箱）。
    ///
    /// **固定种子 `0xF0_22_5E_EE` 下的实测**：`accepted == 64`、`rejected == 64`（biased
    /// 那 64 份全过、raw 那 64 份全被拒，理由同 ①）。断言按 `iterations / 20`（=6，
    /// ≈4.7%）卡下限，不写死精确值。
    #[test]
    fn fuzz_stripped_code_cannot_touch_world_body() {
        let empty = EclImage::empty();
        let mut rng = Pcg32::new(0xF0_22_5E_EE, 0x2);
        const ITERATIONS: u64 = 128;
        let mut rejected = 0u32;
        let mut accepted = 0u32;
        for i in 0..ITERATIONS {
            let biased = i % 2 == 0;
            let img = match random_image(&mut rng, true, biased) {
                Ok(img) => img,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
            accepted += 1;
            let seed = 0x2000 + i;
            let mut wa = World::new(seed);
            wa.spawn_sub_internal(&img, img.root().unwrap(), &[], (OWNER_STAGE, 0, 0));
            let mut wb = World::new(seed);
            for f in 0..60u32 {
                step(&mut wa, &TABLES_V0, &img, &InputFrame::empty(f));
                step(&mut wb, &TABLES_V0, &empty, &InputFrame::empty(f));
                assert_faults_are_allowed(
                    wa.body.frame_events(),
                    "fuzz_stripped_code_cannot_touch_world_body",
                );
            }
            let a = &wa.body;
            let b = &wb.body;
            assert_eq!(a.frame, b.frame);
            assert_eq!(
                a.rng.checksum(),
                b.rng.checksum(),
                "镜像 {i} 越权消耗了世界 RNG"
            );
            assert_eq!(
                a.bullets.checksum(),
                b.bullets.checksum(),
                "镜像 {i} 越权产弹"
            );
            assert_eq!(a.shots.checksum(), b.shots.checksum(), "镜像 {i}");
            assert_eq!(a.enemies.checksum(), b.enemies.checksum(), "镜像 {i}");
            assert_eq!(a.fields.checksum(), b.fields.checksum(), "镜像 {i}");
            assert_eq!(a.items.checksum(), b.items.checksum(), "镜像 {i}");
            assert_eq!(
                a.globals.checksum(),
                b.globals.checksum(),
                "镜像 {i} 越权写 globals"
            );
            assert_eq!(
                a.boss_ui.checksum(),
                b.boss_ui.checksum(),
                "镜像 {i} 越权写 boss_ui"
            );
            assert_eq!(a.players.checksum(), b.players.checksum(), "镜像 {i}");
        }
        let min_substantial = (ITERATIONS / 20) as u32; // ≥5% 下限，远低于实测的 50%
        assert!(
            rejected >= min_substantial,
            "64 字镜像（去副作用）不该只有 {rejected}/{ITERATIONS} 份被拒（下限 {min_substantial}）"
        );
        assert!(
            accepted >= min_substantial,
            "64 字镜像（去副作用）不该只有 {accepted}/{ITERATIONS} 份通过、真正跑起来（下限 {min_substantial}）"
        );
    }

    /// Review Focus 2：存档恢复出来的 task pc 落在合法指令的操作数字上（不是指令边界）——
    /// 跨镜像存档，或者手工改过的存档，都能把 `task.pc` 带到这里。加载时校验器只审「镜像
    /// 自身合法」，管不到运行时被外部注入的 pc；这条钉的是 `vm::exec` 自己必须扛住：镜像是
    /// `PUSHI 50; END`——合法、能过校验器。把 `task.pc` 手工设成 1（PUSHI 的操作数字，值恰
    /// 是 50），从这里起解码会把 `50` 误读成 `OP_SPAWN`（元数 2），要求的两个操作数字读出
    /// `code`（长度 3）边界——必须确定性 `Fault(FAULT_PC_OOB)`，不能 panic 越界读（P4）。
    #[test]
    fn exec_faults_deterministically_when_pc_lands_mid_instruction() {
        let code = vec![ops::OP_PUSHI as u32, 50, ops::OP_END as u32];
        let img = EclImage::try_from_parts(ImageParts {
            code,
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        })
        .expect("PUSHI 50; END 是一段合法镜像（能过加载时校验）");

        let mut w = World::new(1);
        let mut task = Task {
            pc: 1, // 模拟存档带回的坏 pc：落在 PUSHI 的操作数字上，不是指令边界
            ..Task::default()
        };
        let mut budget = u32::MAX;
        let mut ctx = VmCtx {
            code: img.code(),
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl: &img,
            body: &mut w.body,
            tables: &TABLES_V0,
            self_index: 0,
            frame: 0,
        };
        let r = exec(&mut task, &mut ctx);
        assert_eq!(
            r,
            Exec::Fault(FAULT_PC_OOB),
            "坏 pc 落进操作数字，误读出的 SPAWN 需要的操作数字越出 code 边界，须确定性 PC_OOB"
        );
    }
}
