//! 共用的彩虹风铃诊断场景：源码编译与世界初始化。

// ═══════════════════════════════════════════════════════════════════════════
// M1.9 T4：彩虹风铃卡（金向量二号）—— `.ecl` 表层语言源码，启动时编译。
// ═══════════════════════════════════════════════════════════════════════════

/// RANK（难度）读取槽——scene 2 建场时 `world.body.set_var(RANK_SLOT, 2)` 写入
/// （spec 拍板 5：难度是脚本变量，VM 零支持，脚本自己读）；rainbow.ecl 里的 `main` 用它给
/// 五环整体密度算式 `28 + global(RANK_SLOT) * 2` 供值（C13①还账，见 rainbow.ecl 头部注释）。
/// 系统段纪律（`stg_core::world::GLOBALS_SYS_SEGMENT`）：本槽即系统段内的 `GVAR_RANK`，
/// 本常量直接复用该单一权威定义，不重复写字面量 0。
const RANK_SLOT: u16 = stg_core::world::GVAR_RANK;

/// boss 出生点（`.ecl` 版弹幕原点改读运行期 `$self_x`/`$self_y`，只有 boss 的初始出生坐标
/// 仍需要 Rust 侧字面量——见 rainbow.ecl 头部注释"附带修复一处摩擦"）。
const BOSS_X: i32 = 0;
const BOSS_Y: i32 = 100;

/// rainbow.ecl 源码文本——`include_str!` 相对路径从本文件（`stg-harness/src/scenes.rs`）
/// 出发，`scenes/` 是其同级 crate 根下的兄弟目录。
const RAINBOW_SRC: &str = include_str!("../scenes/rainbow.ecl");

/// 编译彩虹风铃卡（M1.9 T4——`.ecl` 表层语言，启动时源码编译，spec 拍板 4）。编译失败即
/// 引擎/场景搭建代码自身的 bug（不是运行期可恢复的场景），照 P4-c 就地 panic，把渲染好的
/// 逐条错误（文件:行:列 + 源行摘录 + `^` 定位）打到终端，不吞诊断信息。
fn compile_rainbow_image() -> stg_core::ecl::image::EclImage {
    match stg_ecl_compiler::lang::compile(RAINBOW_SRC, "rainbow.ecl") {
        Ok(image) => image,
        Err(errors) => {
            let rendered: String = errors
                .iter()
                .map(|e| e.render("rainbow.ecl"))
                .collect::<Vec<_>>()
                .join("\n\n");
            panic!("rainbow.ecl 编译失败：\n{rendered}");
        }
    }
}

/// 彩虹风铃卡开局搭建（golden、storm 与 viewer 共用）。
/// 返回 boss 句柄供 golden 后续断言；serve 忽略之。
pub(crate) fn build_rainbow_world(
    seed: u64,
) -> (
    Box<stg_core::step::World>,
    stg_core::ecl::image::EclImage,
    stg_core::enemy::EnemyHandle,
) {
    use stg_core::ecl::binding::EclOwner;
    use stg_core::enemy::EnemyInit;
    use stg_core::math::{Angle, Fx};
    use stg_core::step::World;

    let image = compile_rainbow_image();
    let mut world2 = World::new(seed);
    world2.body.set_var(RANK_SLOT, 2); // 环密度算式：28+rank×2 → rank=2 时 32-way

    let boss = world2.body.create_enemy(EnemyInit {
        x: Fx::from_int(BOSS_X),
        y: Fx::from_int(BOSS_Y),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        dx: Fx::ZERO,
        dy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        vel_from_0: 0,
        vel_from_1: 0,
        vel_to_0: 0,
        vel_to_1: 0,
        vel_t: 0,
        vel_dur: 0,
        vel_easing: 0,
        vel_active: 0,
        vel_space: 0,
        vel_touched: 0,
        mv_from_x: Fx::ZERO,
        mv_from_y: Fx::ZERO,
        mv_to_x: Fx::ZERO,
        mv_to_y: Fx::ZERO,
        mv_t: 0,
        mv_dur: 0,
        mv_easing: 0,
        mv_active: 0,
        hp: 9999,
        hp_max: 9999,
        radius: Fx::from_int(20),
        hurtbox: Fx::from_int(24),
        invuln: 0,
        hit_flash: 0,
        flags: 0,
        sprite: 0,
        anm_state: 0,
        anm_state_frame: 0,
        main_task: 0,
        death_script: 0,
        script_int: [0; stg_core::enemy::ENEMY_INT_SLOTS],
        script_fx: [stg_core::math::Fx::ZERO; stg_core::enemy::ENEMY_FX_SLOTS],
        drop_count: [0; stg_core::items::ITEM_TYPE_COUNT],
        score: 10000,
    });
    world2
        .start_main_with_owner(&image, EclOwner::Enemy(boss))
        .expect("main 任务应能派生（新镜像/新池，容量均未耗尽）");
    (world2, image, boss)
}

#[cfg(test)]
mod ecl_rainbow_tests {
    use super::*;
    use stg_core::ecl::binding::EclOwner;
    use stg_core::ecl::image::ResolveError;
    use stg_core::enemy::EnemyInit;
    use stg_core::input::InputFrame;
    use stg_core::math::{Angle, Fx};
    use stg_core::step::{World, step_with_director};

    fn boss_init() -> EnemyInit {
        EnemyInit {
            x: Fx::from_int(BOSS_X),
            y: Fx::from_int(BOSS_Y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            dx: Fx::ZERO,
            dy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            vel_from_0: 0,
            vel_from_1: 0,
            vel_to_0: 0,
            vel_to_1: 0,
            vel_t: 0,
            vel_dur: 0,
            vel_easing: 0,
            vel_active: 0,
            vel_space: 0,
            vel_touched: 0,
            mv_from_x: Fx::ZERO,
            mv_from_y: Fx::ZERO,
            mv_to_x: Fx::ZERO,
            mv_to_y: Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 9999,
            hp_max: 9999,
            radius: Fx::from_int(20),
            hurtbox: Fx::from_int(24),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            anm_state_frame: 0,
            main_task: 0,
            death_script: 0,
            script_int: [0; stg_core::enemy::ENEMY_INT_SLOTS],
            script_fx: [stg_core::math::Fx::ZERO; stg_core::enemy::ENEMY_FX_SLOTS],
            drop_count: [0; stg_core::items::ITEM_TYPE_COUNT],
            score: 10000,
        }
    }

    /// 编译干净：rainbow.ecl 应零错误编译（狗粮验收的地基——语言真的能表达这张卡）。
    /// 镜像结构：三 sub（patrol/windchime_pattern/main，规范排序；符卡机构狗粮化后
    /// `timer_ui` 已删除，五环弹幕 loop 搬进 `windchime_pattern`，见 rainbow.ecl 头部
    /// 注释）、named entry 解析正确、Root 不可通过 resolve_entry("main") 获取
    /// （须走 start_main）。
    #[test]
    fn rainbow_ecl_compiles_clean_with_named_entries() {
        let image = compile_rainbow_image();
        assert_eq!(image.sub_count(), 3, "patrol + windchime_pattern + main");
        assert!(image.root().is_some(), "main 应被标记为 Root");
        assert!(
            image.resolve_entry("patrol").is_ok(),
            "async sub patrol 应是可解析的 named entry"
        );
        assert!(
            image.resolve_entry("windchime_pattern").is_ok(),
            "async sub windchime_pattern（spell_begin 的模式引用）应是可解析的 named entry"
        );
        assert_eq!(
            image.resolve_entry("main"),
            Err(ResolveError::RootRequiresStartMain),
            "main 作为 Root 只能通过 start_main 启动"
        );
    }

    /// 编译确定性（声明序置换测试）：交换 sub 的声明顺序后两次编译应产出逐字节相同的
    /// `EclImage`——因为规范排序按名称字典序，与声明序无关。
    #[test]
    fn compile_twice_declaration_order_permutation_yields_equal_image() {
        let src1 = "sub a() { wait(1); } sub b() { wait(2); } sub main() { a(); b(); }";
        let src2 = "sub b() { wait(2); } sub a() { wait(1); } sub main() { a(); b(); }";
        let img1 =
            stg_ecl_compiler::lang::compile(src1, "test.ecl").expect("order 1 should compile");
        let img2 =
            stg_ecl_compiler::lang::compile(src2, "test.ecl").expect("order 2 should compile");
        assert_eq!(
            img1, img2,
            "canonical sort by name = declaration-order independent"
        );
    }

    /// 稳态判别（输入脚本同金向量二号真实建场路径——全程持 BTN_SHOT + 左右缓移，自机弹真的
    /// 会打中 boss，见下方 `hp_ratio` 断言）：600 帧后弹数 >100（持续环流）、boss 存活
    /// （hp_max 9999、`spell_begin` 血线 `threshold=0`，600 帧自机火力打不穿，符卡收卡
    /// 路径走不到，卡全程 active）、`boss_ui[0].active==1` 且 `hp_ratio` 是真值（非零、
    /// ≤1.0fx——符卡机构 settle 趟五"boss_ui 自动喂"用真定点除法算逐卡血条，见 spec
    /// 2026-07-24 §3；真实建场下自机弹确实打中 boss，`hp_ratio` 实测 <1.0，不是恒为 1.0
    /// 的占位符）、任务数 >=3（main+patrol+windchime_pattern 三子全存活——`patrol`/
    /// `windchime_pattern` 皆 `loop {}` 不自灭，`windchime_pattern` 是 spell_begin 生出的
    /// 模式任务、随卡生死但卡本身 600 帧内不结束）、全程零 Fault。
    #[test]
    fn rainbow_scene_reaches_steady_state() {
        let image = compile_rainbow_image();
        let mut w = World::new(0x524E_424F_5701);
        w.body.set_var(RANK_SLOT, 2);
        let boss = w.body.create_enemy(boss_init());
        w.start_main_with_owner(&image, EclOwner::Enemy(boss))
            .expect("spawn 应成功（新镜像/新池）");

        for frame in 0..600u32 {
            let mut input = InputFrame::empty(frame);
            let mut btn = stg_core::input::BTN_SHOT;
            btn |= if (frame / 60) % 2 == 0 {
                stg_core::input::BTN_LEFT
            } else {
                stg_core::input::BTN_RIGHT
            };
            input.actions[0].buttons = btn;
            step_with_director(&mut w, &stg_core::tables::TABLES_V0, &image, &input, |_| {});
        }
        assert_eq!(w.body.view().diag().task_faults, 0, "全程不应产生 Fault");
        let bullet_count = w.body.view().bullets().iter_alive().count();
        assert!(bullet_count > 100, "稳态弹数应 >100（实测 {bullet_count}）");
        assert!(
            w.body.view().enemies().get(boss).is_some(),
            "boss 应存活满 600 帧（hp_max 9999 扛住火力）"
        );
        assert_eq!(
            w.body.view().boss_ui()[0].active,
            1,
            "符卡计时器应保持 active=1"
        );
        let ratio = w.body.view().boss_ui()[0].hp_ratio;
        assert!(
            ratio.raw() > 0 && ratio.raw() <= Fx::ONE.raw(),
            "hp_ratio 应是非零、≤1.0fx 的真值（实测 raw={}）——C13②证据",
            ratio.raw()
        );
        let task_count = w.tasks().iter_alive().count();
        assert!(
            task_count >= 3,
            "main+patrol+windchime_pattern 三任务应全存活（实测 {task_count}）"
        );
    }
}
