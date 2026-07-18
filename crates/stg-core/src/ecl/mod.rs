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

pub mod image;
pub mod ops;
/// syscall 号表 v1（`SYS_*` 常量，`pub`）——`stg-ecl-compiler` 的 builder DSL 靠它拼
/// `OP_SYS` 指令（依赖方向 compiler→core 单向，只取常量，不碰 `dispatch`）。`dispatch` 本身
/// 仍 `pub(crate)`：只有 `vm::exec` 能调用，编译器够不到派发逻辑，只够到号表。
pub mod syscall;
pub mod task;
pub(crate) mod vm;

pub use image::EclImage;

/// VM fuzz 冒烟（§9 承诺的最小版，M1-T5）：确定性 PRNG 生成随机字节码，断言两件事——
/// ① 任意垃圾码永不 panic（只许确定性 Fault）；② 剥去副作用 op（SYS/SPAWN）的垃圾码
/// **无法越权触碰 WorldBody**（对照空镜像参考世界逐字段哈希全等——白名单沙箱的实证）。
#[cfg(test)]
mod fuzz_smoke {
    use crate::checksum::Checksum;
    use crate::ecl::image::EclImage;
    use crate::ecl::ops;
    use crate::ecl::task::OWNER_STAGE;
    use crate::input::InputFrame;
    use crate::rng::Pcg32;
    use crate::step::{World, step};
    use crate::tables::TABLES_V0;

    /// 64 字随机镜像；`strip_effects` 把 SYS（改世界）与 SPAWN（池满时改 diag）换成 POP。
    fn random_image(rng: &mut Pcg32, strip_effects: bool) -> EclImage {
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
        EclImage {
            code,
            subs: vec![0],
            content_hash: 0,
        }
    }

    /// ① 256 份全随机镜像 × 60 帧：不 panic、不越界，Fault 走确定性通道。
    #[test]
    fn fuzz_random_code_never_panics() {
        let mut rng = Pcg32::new(0xF0_22_5E_ED, 0x1);
        for i in 0..256u64 {
            let img = random_image(&mut rng, false);
            let mut w = World::new(0x1000 + i);
            w.spawn_task(&img, 0, (OWNER_STAGE, 0, 0));
            for f in 0..60u32 {
                step(&mut w, &TABLES_V0, &img, &InputFrame::empty(f));
            }
        }
    }

    /// ② 128 份去副作用镜像 vs 空镜像参考世界：除 tasks/diag（task_faults 计数）外，
    /// WorldBody 逐字段哈希全等——垃圾码只能烧自己的预算，摸不到世界（syscall 白名单沙箱）。
    #[test]
    fn fuzz_stripped_code_cannot_touch_world_body() {
        let empty = EclImage::empty();
        let mut rng = Pcg32::new(0xF0_22_5E_EE, 0x2);
        for i in 0..128u64 {
            let img = random_image(&mut rng, true);
            let seed = 0x2000 + i;
            let mut wa = World::new(seed);
            wa.spawn_task(&img, 0, (OWNER_STAGE, 0, 0));
            let mut wb = World::new(seed);
            for f in 0..60u32 {
                step(&mut wa, &TABLES_V0, &img, &InputFrame::empty(f));
                step(&mut wb, &TABLES_V0, &empty, &InputFrame::empty(f));
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
    }
}
