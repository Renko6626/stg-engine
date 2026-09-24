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

/// VM fuzz 冒烟（§9 承诺的最小版，M1-T5；引擎第二刀 §4 起随加载时校验改口径）：
/// 确定性 PRNG 生成随机字节码，断言三件事——① 大多数随机抽样在加载时就被
/// `image::validate_code` 拒绝（统计个数，两端都不该是 0/全部）；② 通过校验、真的跑起来的
/// 那些镜像永不 panic，Fault 只能来自运行时仍保留的那几类（栈/预算/除零/调用深度/pc 越界，
/// 外加 syscall 内部动态检查复用的 BAD_OP）；③ 剥去副作用 op（SYS/SPAWN）的垃圾码**无法
/// 越权触碰 WorldBody**（对照空镜像参考世界逐字段哈希全等——白名单沙箱的实证）。
#[cfg(test)]
mod fuzz_smoke {
    use crate::checksum::Checksum;
    use crate::ecl::image::{EclImage, ImageBuildError, ImageParts, SubInit, SubKind};
    use crate::ecl::ops;
    use crate::ecl::task::{OWNER_STAGE, Task};
    use crate::ecl::vm::{Exec, VmCtx, exec};
    use crate::ecl::{
        FAULT_BAD_OP, FAULT_BUDGET, FAULT_CALL_DEPTH, FAULT_DIV_ZERO, FAULT_PC_OOB, FAULT_STACK,
    };
    use crate::events::EVT_TASK_FAULT;
    use crate::input::InputFrame;
    use crate::rng::Pcg32;
    use crate::step::{World, step};
    use crate::tables::TABLES_V0;

    /// 64 字随机镜像；`strip_effects` 把 SYS（改世界）与 SPAWN（池满时改 diag）换成 POP。
    /// 直接过 `try_from_parts`（不再 `test_image` 的 `.expect()`）——加载时校验器会真的审这
    /// 段随机字节码，多数抽样在这里就被拒；调用方按 `Err` 计数、`continue`，不 panic。
    fn random_image(rng: &mut Pcg32, strip_effects: bool) -> Result<EclImage, ImageBuildError> {
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

    /// ① 256 次随机抽样 × 60 帧：被校验器拒的记个数、跳过；通过的照原来的方式跑——不
    /// panic，fault 码只能来自运行时仍保留的那几类。
    ///
    /// **`accepted` 现实上恒为 0，这不是测试写坏了**：加载闸要求头字高 24 位必须为 0
    /// （`ReservedBits`，§4.1），而 `rng.next_u32()` 出的是全域随机字——首字高位恰好全 0
    /// 的概率仅 2⁻²⁴，64 字连续通过整条校验器的概率实质为零。这条测试真正守的是「大批
    /// 必被拒的垃圾码走 `try_from_parts` 不 panic」；一旦哪天真抽中一份能通过校验的（或
    /// 改了 PRNG/宽度让通过率不再是零），下面的运行时断言会自动接管，不用另外改测试。
    #[test]
    fn fuzz_random_code_never_panics() {
        let mut rng = Pcg32::new(0xF0_22_5E_ED, 0x1);
        let mut rejected = 0u32;
        for i in 0..256u64 {
            let img = match random_image(&mut rng, false) {
                Ok(img) => img,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
            let mut w = World::new(0x1000 + i);
            w.spawn_sub_internal(&img, img.root().unwrap(), &[], (OWNER_STAGE, 0, 0));
            for f in 0..60u32 {
                step(&mut w, &TABLES_V0, &img, &InputFrame::empty(f));
                assert_faults_are_allowed(w.body.frame_events(), "fuzz_random_code_never_panics");
            }
        }
        assert!(rejected > 0, "64 字全随机码不该一份都不被加载时校验拒绝");
    }

    /// ② 128 次去副作用抽样 vs 空镜像参考世界：被拒的同样跳过、计数；通过的除
    /// tasks/diag（task_faults 计数）外，WorldBody 逐字段哈希全等——垃圾码只能烧自己的
    /// 预算，摸不到世界（syscall 白名单沙箱）。
    /// 同 `fuzz_random_code_never_panics` 上方注释：`accepted` 现实上恒为 0（`ReservedBits`
    /// 门槛概率 2⁻²⁴），`rejected` 记数是给"被拒的路径也不 panic"留判别力，不是覆盖率断言。
    #[test]
    fn fuzz_stripped_code_cannot_touch_world_body() {
        let empty = EclImage::empty();
        let mut rng = Pcg32::new(0xF0_22_5E_EE, 0x2);
        let mut rejected = 0u32;
        for i in 0..128u64 {
            let img = match random_image(&mut rng, true) {
                Ok(img) => img,
                Err(_) => {
                    rejected += 1;
                    continue;
                }
            };
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
        assert!(rejected > 0, "64 字全随机码（去副作用）不该一份都不被拒");
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
