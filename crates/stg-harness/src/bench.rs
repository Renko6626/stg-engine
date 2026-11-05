//! step、快照与校验和的性能基准及其场景。

use std::process::ExitCode;

use crate::xformbench;

/// benchmark（M0-18）—— step 性能基线 + rollback 成本账（ECL 指令预算/M3 快照环的实测依据）。
///
/// 场景阶梯：哑弹 1024/2048/4096/8192（纯积分+行1/2 碰撞）· xform 弹 512/1024/2048
/// （SET_ANG_VEL 逐帧 sincos 回填 + 段池满载）· 全混合（敌/杀敌掉落/道具/每150帧消弹转星/
/// 自机满火力四路+子机）。每档预热 120 帧、实测 `--frames`（默认 600）帧。
///
/// **饱和四档**（2026-08-01，follow-ups F5 还账）追加在阶梯之后：`ecl 任务 256` /
/// `syscall 密` / `shooter 大环`（脚本源 `scenes/bench.ecl`）+ `敌插值 256`（Rust 导演，
/// 笛卡尔/极坐标各一行）。口径与前八档**不同**：前八档里的「全混合」是典型帧，这四档
/// 一律是**把某一根轴打满的最烂情况**——见 `docs/bench-baseline.md` 末节。
///
/// 三组耗时分开计：step 本体（含导演补弹）/ `copy_into` 整块快照 / 全量校验和——后两者
/// 是 rollback 与联机采样的预算数。计时用 `std::time::Instant`（断层线之上，仅测不喂）。
/// **务必 `--release` 跑**；debug 构建会打印警告（数字仅供相对比较）。
pub(crate) fn cmd_bench(rest: &[String]) -> ExitCode {
    use stg_core::step::World;

    let frames: u32 = rest
        .iter()
        .position(|a| a == "--frames")
        .and_then(|i| rest.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

    if rest.iter().any(|arg| arg == "--xform-expr") {
        return xformbench::cmd_xform_bench(frames);
    }

    if cfg!(debug_assertions) {
        println!("⚠ debug 构建——绝对数字无意义，请用 cargo run --release -p stg-harness -- bench");
    }
    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    let world_sz = std::mem::size_of::<World>();
    println!("── 内存账（扁平无堆，size_of 即全部）──");
    println!(
        "World 总计 {:.2} MB（16 帧快照环 ≈ {:.1} MB）",
        mb(world_sz),
        mb(world_sz * 16)
    );
    println!(
        "  弹池 {:.2} MB · 自机弹池 {:.3} MB · 敌池 {:.3} MB · 道具池 {:.3} MB · 其余(含变换段/globals/缓冲) {:.2} MB",
        mb(std::mem::size_of::<stg_core::bullets::BulletPool>()),
        mb(std::mem::size_of::<stg_core::shots::ShotPool>()),
        mb(std::mem::size_of::<stg_core::enemy::EnemyPool>()),
        mb(std::mem::size_of::<stg_core::items::ItemPool>()),
        mb(world_sz
            - std::mem::size_of::<stg_core::bullets::BulletPool>()
            - std::mem::size_of::<stg_core::shots::ShotPool>()
            - std::mem::size_of::<stg_core::enemy::EnemyPool>()
            - std::mem::size_of::<stg_core::items::ItemPool>()),
    );
    println!();
    println!(
        "{:<16} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "场景", "稳态弹", "step均µs", "p50µs", "p99µs", "快照µs", "校验和µs"
    );
    // 静默预热一轮：吃掉首场景的页错误/缓存冷启动/频率爬坡污染（首行数据曾实测虚高）。
    bench_ladder("(预热丢弃)", 512, false, 60, false);
    for &(name, target, xf) in &[
        ("哑弹 1024", 1024usize, false),
        ("哑弹 2048", 2048, false),
        ("哑弹 4096", 4096, false),
        ("哑弹 8192", 8192, false),
        ("xform 512", 512, true),
        ("xform 1024", 1024, true),
        ("xform 2048", 2048, true),
    ] {
        bench_ladder(name, target, xf, frames, true);
    }
    bench_mix(frames);

    // ── 饱和四档（F5 还账）：每档只打满一根轴，不是"跑一局真实游戏"────────────
    // 前三档共用 `scenes/bench.ecl` 一份镜像（三个 named entry），第四档不用 ECL——
    // 用 Rust 导演直接打满敌池，好把敌人运动的价钱和任务/VM 开销隔离开。
    let image = compile_bench_image();
    for &(name, entry) in &[
        ("ecl 任务 256", "bench_tasks"),
        ("syscall 密", "bench_syscalls"),
        ("shooter 大环", "bench_shooter"),
    ] {
        bench_ecl(name, entry, &image, frames);
    }
    bench_enemy_interp("敌插值 256 笛", VelSpace::Cart, frames);
    bench_enemy_interp("敌插值 256 极", VelSpace::Polar, frames);
    ExitCode::SUCCESS
}

/// 单档阶梯：导演每帧把弹池补到 target（批量环，慢速外扩长寿命），自机满火力全程射击。
fn bench_ladder(name: &str, target: usize, with_xform: bool, frames: u32, print: bool) {
    use stg_core::bullets::BulletInit;
    use stg_core::input::{BTN_LEFT, BTN_RIGHT, BTN_SHOT, InputFrame};
    use stg_core::math::{Angle, Fx};
    use stg_core::step::World;
    use stg_core::xform::{OP_SET_ANG_VEL, XformSlot};

    let template = BulletInit {
        x: Fx::ZERO,
        y: Fx::from_int(200),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: 0,
        radius: Fx::from_int(3),
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: 0xFFFF,
        xform_wait: 0,
        xform_next: 0,
        born_frame: 0,
    };
    let seq = [XformSlot {
        wait: 0,
        op: OP_SET_ANG_VEL,
        _pad: 0,
        args: [96, 0], // 慢旋：POLAR_FX 逐帧 sincos 回填路径
    }];
    let mut w = World::new(0xBE9C);
    w.body.set_player_power(0, 400); // 满火力：四路+子机的自机弹稳态负载
    run_measured(
        name,
        print,
        &mut w,
        &stg_core::ecl::image::EclImage::empty(), // 无脚本场景：显式空镜像（零任务零成本）
        frames,
        |b, frame| {
            let alive = b.view().bullets().iter_alive().count();
            if alive < target {
                let deficit = (target - alive).min(512) as u16;
                let astep = ((65536u32 / deficit.max(2) as u32) as u16) as i16;
                let xform: &[XformSlot] = if with_xform { &seq } else { &[] };
                b.create_bullets_batch(
                    template,
                    xform,
                    deficit,
                    Angle((frame.wrapping_mul(7919) & 0xFFFF) as u16),
                    astep,
                    1,
                    Fx::from_raw(16384), // 0.25 px/帧外扩——OOB 前 >1000 帧，稳态可保
                    Fx::ZERO,
                );
            }
        },
        |frame| {
            let mut input = InputFrame::empty(frame);
            input.actions[0].buttons = BTN_SHOT
                | if (frame / 40) % 2 == 0 {
                    BTN_RIGHT
                } else {
                    BTN_LEFT
                };
            input
        },
    );
}

/// 全混合：敌补位+杀敌掉落+道具磁吸+每 150 帧消弹转星+xform 环+自机满火力低速。
fn bench_mix(frames: u32) {
    use stg_core::bullets::BulletInit;
    use stg_core::enemy::EnemyInit;
    use stg_core::field::{FIELD_CLEAR_BULLETS, FIELD_RADIUS_FULLSCREEN, FieldInit};
    use stg_core::input::{BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame};
    use stg_core::math::{Angle, Fx};
    use stg_core::step::World;
    use stg_core::xform::{OP_SET_ANG_VEL, XformSlot};

    let bullet = |y: i32, life: u16| BulletInit {
        x: Fx::ZERO,
        y: Fx::from_int(y),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: 0,
        radius: Fx::from_int(3),
        delay: 0,
        life,
        flags: 0,
        grazed_by: 0,
        transform_head: 0xFFFF,
        xform_wait: 0,
        xform_next: 0,
        born_frame: 0,
    };
    let enemy = |x: i32| EnemyInit {
        x: Fx::from_int(x),
        y: Fx::from_int(80),
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
        hp: 5,
        hp_max: 5,
        radius: Fx::from_int(12),
        hurtbox: Fx::from_int(16),
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
        drop_count: stg_core::tables::drop_counts(&stg_core::tables::TABLES_V0, 1).0,
        score: 100,
    };
    let seq = [XformSlot {
        wait: 0,
        op: OP_SET_ANG_VEL,
        _pad: 0,
        args: [128, 0],
    }];
    let mut w = World::new(0xBE9C);
    w.body.set_player_power(0, 400);
    run_measured(
        "全混合",
        true,
        &mut w,
        &stg_core::ecl::image::EclImage::empty(), // 无脚本场景：显式空镜像（零任务零成本）
        frames,
        move |b, frame| {
            if frame % 60 == 0 {
                let alive = b.view().enemies().iter_alive().count();
                for &ex in [-80i32, 0, 80].iter().skip(alive) {
                    b.create_enemy(enemy(ex));
                }
            }
            if frame % 8 == 0 {
                b.create_bullets_batch(
                    bullet(100, 300),
                    &[],
                    10,
                    Angle((frame.wrapping_mul(797) & 0xFFFF) as u16),
                    6554,
                    1,
                    Fx::from_int(2),
                    Fx::ZERO,
                );
            }
            if frame % 40 == 20 {
                b.create_bullets_batch(
                    bullet(150, 400),
                    &seq,
                    16,
                    Angle::ZERO,
                    4096,
                    1,
                    Fx::from_int(1),
                    Fx::ZERO,
                );
            }
            if frame % 150 == 145 {
                b.create_field(FieldInit {
                    x: Fx::ZERO,
                    y: Fx::from_int(224),
                    radius: FIELD_RADIUS_FULLSCREEN,
                    dmg_per_frame: 0,
                    life: 1,
                    owner: 0,
                    flags: FIELD_CLEAR_BULLETS,
                });
            }
        },
        |frame| {
            let mut input = InputFrame::empty(frame);
            input.actions[0].buttons =
                BTN_SHOT | BTN_SLOW | if frame % 90 < 50 { BTN_UP } else { 0 };
            input
        },
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 饱和四档（F5 还账，2026-08-01）—— 每档只打满一根轴
// ═══════════════════════════════════════════════════════════════════════════
//
// **口径（与前八档不同，改这段前先读）**：bench 量的是**最烂情况的帧率**，不是实际
// 凑巧的帧率。所以这四档一律把某根轴推到池上限/预算上限，其余轴压到最低——不要往
// 这里加"跑一局真实游戏看看"的档，那个位置已经被「全混合（真实规模）」占了。

/// `scenes/bench.ecl` 源码文本（三个饱和入口 + 占位 main）。`include_str!` 相对路径
/// 从本文件出发，同 `RAINBOW_SRC`。
const BENCH_SRC: &str = include_str!("../scenes/bench.ecl");

/// 编译饱和档脚本。失败即 harness 自身的 bug，照 P4-c 就地 panic + 打印渲染好的逐条
/// 错误（同 `compile_rainbow_image`，不吞诊断）。
fn compile_bench_image() -> stg_core::ecl::image::EclImage {
    match stg_ecl_compiler::lang::compile(BENCH_SRC, "bench.ecl") {
        Ok(image) => image,
        Err(errors) => {
            let rendered: String = errors
                .iter()
                .map(|e| e.render("bench.ecl"))
                .collect::<Vec<_>>()
                .join("\n\n");
            panic!("bench.ecl 编译失败：\n{rendered}");
        }
    }
}

/// 脚本饱和档通用跑法：建空世界 → 按名启动一个 `async sub` 入口 → 零导演、零输入实测。
///
/// 走 `spawn_entry_named` 而不是 `start_main`：一份 `EclImage` 只能有一个 `main`，而这里
/// 三档共用同一份镜像（`bench.ecl` 里的 `main` 是永不启动的占位）。owner 取 `Stage`——
/// 三档都不需要敌人，敌池空着正是"只压这根轴"的一部分。
///
/// 输入恒空（不按射击键）：自机弹是另一根轴，混进来会污染这三行的读数。
fn bench_ecl(name: &str, entry: &str, image: &stg_core::ecl::image::EclImage, frames: u32) {
    use stg_core::ecl::binding::EclOwner;
    use stg_core::input::InputFrame;
    use stg_core::step::World;

    let mut w = World::new(0xBE9C);
    w.spawn_entry_named(image, entry, &[], EclOwner::Stage)
        .unwrap_or_else(|e| panic!("bench 场景入口 `{entry}` 启动失败：{e:?}"));
    run_measured(
        name,
        true,
        &mut w,
        image,
        frames,
        |_, _| {},
        InputFrame::empty,
    );
}

/// 敌速度插值档要压的插值空间。
#[derive(Clone, Copy, PartialEq, Eq)]
enum VelSpace {
    /// 笛卡尔：每帧插 `vx/vy` 后 **`backfill_enemy_polar`**（`isqrt` + CORDIC `atan2` 各一次）。
    Cart,
    /// 极坐标：每帧插 `speed/angle` 后 `refresh_enemy_vel_from_polar`（只查 sin/cos 表）。
    Polar,
}

/// 打满敌池（cap = 256）+ 敌人速度插值热路径。**刻意不用 ECL**——用 Rust 导演直接武装，
/// 好把这一档和任务/VM 开销隔离开，读数里只剩敌人运动本身。
///
/// 两行的差就是**笛卡尔回填的价钱**：`Cart` 每帧每敌一次 `isqrt` + 一次 16 轮 CORDIC
/// `atan2`（不回填 `$self_speed`/`$self_angle` 会读到陈值，故它是必须的），256 敌 =
/// **256 次 CORDIC/帧**；`Polar` 走 `polar_to_vec`，只查两次三角表、无 CORDIC。
/// 这条热路径是敌人运动动词族刀（2026-07-31）加进来的，此前从来没量过。
///
/// **保持 `vel_active != 0` 全程为真**：每 `DUR` 帧无条件重新武装。导演在相位 3、插值
/// tick 在相位 5，故重新武装恒**早于**那一帧的 tick —— `vel_t` 每次被抹回 0，永远追不上
/// `vel_dur`，`done` 分支一次都不会走到，插值器全程在飞。
///
/// **稳态**（p50/p99 才有意义的前提）：目标速度逐段取反、easing 取 Linear ⇒ 一整个周期
/// 的平均速度≈0，敌人在出生点附近小幅摆动而不是往一个方向漂到越界被回收（敌越界边距
/// `ENEMY_OOB_MARGIN = 256`）。极坐标那行同理：目标角每段翻 180°、速率恒定，最短弧恒定
/// 朝一个方向扫 ⇒ 走一个半径 ≈ V·DUR/π ≈ 10px 的小圆，闭合、不漂。
fn bench_enemy_interp(name: &str, space: VelSpace, frames: u32) {
    use stg_core::input::InputFrame;
    use stg_core::step::World;

    let mut w = World::new(0xBE9C);
    let handles = bench_fill_enemy_grid(&mut w);
    assert_eq!(handles.len(), 256, "敌池应恰好被填满 256 只");

    run_measured(
        name,
        true,
        &mut w,
        &stg_core::ecl::image::EclImage::empty(), // 这一档不用 ECL（与任务开销隔离）
        frames,
        move |b, frame| bench_rearm_enemy_vel(b, &handles, space, frame),
        InputFrame::empty,
    );
}

/// 插值段长（帧）。重新武装周期同此值。
const BENCH_VEL_DUR: u16 = 30;

/// 摆幅速度（px/帧）。1.0 × 30 帧 ⇒ 摆幅十几 px，离任何边界都远。
const BENCH_VEL_V: stg_core::math::Fx = stg_core::math::Fx::from_raw(65536);

/// 把敌池填满 256 只（16×16 网格，摊在场内上半区），返回句柄表。
/// bench 与押运测试共用同一份建场，免得测的和跑的是两个东西。
fn bench_fill_enemy_grid(w: &mut stg_core::step::World) -> Vec<stg_core::enemy::EnemyHandle> {
    use stg_core::enemy::{EnemyHandle, EnemyInit};
    use stg_core::math::{Angle, Fx};

    /// 16×16 = 256 = 敌池 cap，一只不剩。
    const SIDE: i32 = 16;

    let mut handles: Vec<EnemyHandle> = Vec::with_capacity((SIDE * SIDE) as usize);
    for row in 0..SIDE {
        for col in 0..SIDE {
            let h = w.body.create_enemy(EnemyInit {
                // 摊在场内上半区（自机开局在 (0, 384)），体碰不到自机
                x: Fx::from_int(-180 + col * 24),
                y: Fx::from_int(40 + row * 12),
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
                hp: 100_000,
                hp_max: 100_000,
                radius: Fx::from_int(12),
                hurtbox: Fx::from_int(16),
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
                score: 0,
            });
            handles.push(h);
        }
    }
    handles
}

/// 每 `BENCH_VEL_DUR` 帧无条件重新武装全部 256 只敌的速度插值（逐段取反，见
/// [`bench_enemy_interp`] 的稳态说明）。
///
/// ⚠️ **重新武装周期恰等于 `BENCH_VEL_DUR`，所以每段的最后一帧插值是"刚到期"而非"在飞"**
/// （复审实测：f = 29/59/89… 上 256 只敌全部 `vel_active == 0`、`vel_t == 30`）。导演位
/// （相位 3）恒早于插值 tick（相位 5）只保证"同帧先武装后 tick"，不保证"永不完成"。
/// 对读数的影响可忽略——`integrate` 的 `done` 路径**照样调 `backfill_enemy_polar`**、只是
/// 跳过一次 `ease()`，30 帧里 1 帧。
///
/// **别试图靠缩短周期来"永不完成"**：往返两段必须等长才净位移为零，周期是摆幅的分母，
/// 取 `dur − 1` 会让 256 只敌慢慢漂出界（实测末帧只剩 16 只）。真要永不完成得加长 `dur`
/// 且同步重算摆幅，收益（30 帧里省 1 次 `ease`）不值这个风险。
fn bench_rearm_enemy_vel(
    b: &mut stg_core::world::WorldBody,
    handles: &[stg_core::enemy::EnemyHandle],
    space: VelSpace,
    frame: u32,
) {
    use stg_core::math::{Angle, Fx};

    if !frame.is_multiple_of(BENCH_VEL_DUR as u32) {
        return;
    }
    // 逐段取反：偶数段朝 (+V,+V)/0°，奇数段朝 (−V,−V)/180°
    let flip = (frame / BENCH_VEL_DUR as u32) % 2 == 1;
    for &h in handles {
        match space {
            VelSpace::Cart => {
                let v = if flip {
                    Fx::ZERO - BENCH_VEL_V
                } else {
                    BENCH_VEL_V
                };
                b.set_enemy_vel_cart(h, v, v, BENCH_VEL_DUR, 0 /* Linear */);
            }
            VelSpace::Polar => {
                let a = if flip { Angle(32768) } else { Angle(0) };
                b.set_enemy_vel_polar(h, a, BENCH_VEL_V, BENCH_VEL_DUR, 0 /* Linear */);
            }
        }
    }
}

/// 预热 120 帧 → 实测 N 帧；step/快照/校验和三组分计，出一行报告。
///
/// `ecl` 是本档的脚本镜像。**无脚本场景传 `&EclImage::empty()`**（零任务零成本）——
/// M0-18 那八档本来把 `EclImage::empty()` 写死在函数体里，饱和四档刀（2026-08-01）把它
/// 提成参数，好让 `bench_ecl` 传真镜像进来；八档传的值与写死时逐位同一个东西，行为不变。
fn run_measured(
    name: &str,
    print: bool,
    w: &mut stg_core::step::World,
    ecl: &stg_core::ecl::image::EclImage,
    frames: u32,
    mut director: impl FnMut(&mut stg_core::world::WorldBody, u32),
    mut input_of: impl FnMut(u32) -> stg_core::input::InputFrame,
) {
    use std::time::Instant;
    use stg_core::step::{World, step_with_director};
    use stg_core::tables::TABLES_V0;

    const WARMUP: u32 = 120;
    let mut snap = World::new(0);
    let mut step_ns: Vec<u64> = Vec::with_capacity(frames as usize);
    let mut snap_ns: u64 = 0;
    let mut sum_ns: u64 = 0;
    for frame in 0..(WARMUP + frames) {
        let input = input_of(frame);
        let t0 = Instant::now();
        step_with_director(w, &TABLES_V0, ecl, &input, |b| director(b, frame));
        let dt = t0.elapsed().as_nanos() as u64;
        if frame >= WARMUP {
            step_ns.push(dt);
            let t1 = Instant::now();
            w.copy_into(&mut snap);
            snap_ns += t1.elapsed().as_nanos() as u64;
            let t2 = Instant::now();
            std::hint::black_box(w.checksum());
            sum_ns += t2.elapsed().as_nanos() as u64;
        }
    }
    step_ns.sort_unstable();
    if !print {
        return;
    }
    let n = step_ns.len().max(1) as u64;
    let us = |ns: u64| ns as f64 / 1000.0;
    println!(
        "{:<16} {:>6} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
        name,
        w.body.view().bullets().iter_alive().count(),
        us(step_ns.iter().sum::<u64>() / n),
        us(step_ns[step_ns.len() / 2]),
        us(step_ns[(step_ns.len() * 99 / 100).min(step_ns.len() - 1)]),
        us(snap_ns / n),
        us(sum_ns / n),
    );
}

#[cfg(test)]
mod tests {
    use stg_core::input::InputFrame;
    use stg_core::step::step_with_director;

    // ── 饱和四档的稳态押运（F5 还账，2026-08-01）────────────────────────────
    //
    // 这四档的数字只有在**稳态**下才可读：任务被 `FAULT_BUDGET` 静默杀掉、敌人漂到
    // 越界被回收、弹池撞满导致 `sh_fire` 短路，三者任一发生，那一行就从"打满某根轴"
    // 变成"某根轴打了一半就塌了"，而 bench 自己是**不会报错**的（它只印时间）。
    // 所以稳态由这组测试押运，不由跑 bench 的人肉眼看。

    /// 跑 N 帧饱和 ECL 档，返回 (末帧存活任务数, 全程 fault 事件数, 末帧存活弹数)。
    fn run_bench_entry(entry: &str, frames: u32) -> (usize, usize, usize) {
        use stg_core::ecl::binding::EclOwner;
        let image = super::compile_bench_image();
        let mut w = stg_core::step::World::new(0xBE9C);
        w.spawn_entry_named(&image, entry, &[], EclOwner::Stage)
            .unwrap_or_else(|e| panic!("入口 `{entry}` 启动失败：{e:?}"));
        let mut faults = 0usize;
        for f in 0..frames {
            stg_core::step::step(
                &mut w,
                &stg_core::tables::TABLES_V0,
                &image,
                &InputFrame::empty(f),
            );
            faults += w
                .frame_events()
                .iter()
                .filter(|e| e.kind == stg_core::events::EVT_TASK_FAULT)
                .count();
        }
        (
            w.tasks().iter_alive().count(),
            faults,
            w.body.view().bullets().iter_alive().count(),
        )
    }

    /// ① `ecl 任务 256`：任务池必须**恰好**被填满并保持住。
    ///
    /// 判别力：`TASK_CAP` 是 256，入口任务自己占 1 槽 ⇒ 255 个 drone。若 `bench.ecl` 把
    /// 数量写成 256（多一个）、或某一帧的 `spawn` 撞破 1024 条指令预算，这里立刻红。
    #[test]
    fn bench_tasks_saturates_task_pool_and_holds() {
        let (alive, faults, _) = run_bench_entry("bench_tasks", 300);
        assert_eq!(alive, 256, "任务池应被打满（TASK_CAP=256）并保持");
        assert_eq!(faults, 0, "不应有任务 fault（预算/坏参数）");
    }

    /// ② `syscall 密`：60 个烧 syscall 的任务 + 入口 = 61 个，全程一个都不能死。
    ///
    /// 这一档最容易翻车的是 **`GLOBAL_BUDGET`（65536 条/帧）**：`run_tasks` 按池索引
    /// 升序消耗共享余额，撞破以后**排在后面的任务**逐个 `FAULT_BUDGET` 被杀，bench
    /// 那一行会变成"前半程贵、后半程白送"的双峰——mean 比 p50 还小，一眼可辨但
    /// 没人保证会去看。故在这里钉死。
    #[test]
    fn bench_syscalls_holds_all_burner_tasks() {
        let (alive, faults, _) = run_bench_entry("bench_syscalls", 300);
        assert_eq!(
            faults, 0,
            "烧 syscall 的任务不应被预算杀（全局 65536 条/帧）"
        );
        assert_eq!(alive, 61, "60 个 burner + 1 个入口任务应全程存活");
    }

    /// ③ `shooter 大环`：稳态弹数必须**明显低于**弹池 cap（8192）。
    ///
    /// 撞满 cap 的后果不是报错而是 `sh_fire` **短路**（P4-a：从满的那一颗起停止本次
    /// 开火），于是这一档会悄悄地少干活、数字反而变好看。留 ≥15% 余量。
    #[test]
    fn bench_shooter_stays_under_bullet_cap() {
        let (alive, faults, bullets) = run_bench_entry("bench_shooter", 400);
        assert_eq!(faults, 0, "发射器任务不应 fault");
        assert_eq!(alive, 9, "8 个环 + 1 个入口任务应全程存活");
        assert!(
            (1000..7000).contains(&bullets),
            "稳态弹数应稳定在 8192 上限之下且确实打满了发射路径，实测 {bullets}"
        );
    }

    /// ④ `敌插值 256`：256 只敌全程不许漂到越界被回收，两条插值空间各押一次。
    ///
    /// 判别力：若把目标速度改成不取反（单向恒定），敌人会一路漂出
    /// `ENEMY_OOB_MARGIN`（256）被 cleanup 收走，末帧存活数掉下来 —— 这正是"稳态"
    /// 二字在这一档的全部含义。
    #[test]
    fn bench_enemy_interp_keeps_all_256_enemies_in_field() {
        for space in [super::VelSpace::Cart, super::VelSpace::Polar] {
            let mut w = stg_core::step::World::new(0xBE9C);
            let handles = super::bench_fill_enemy_grid(&mut w);
            assert_eq!(handles.len(), 256, "敌池应恰好被填满");
            let ecl = stg_core::ecl::image::EclImage::empty();
            // 跑到 720（`BENCH_VEL_DUR` 的整数倍 ⇒ 末帧正好重新武装过），避开
            // 每段最后一帧那个"刚到期"的相位，好让下面的 `vel_active` 断言问的是稳态。
            for f in 0..=720u32 {
                step_with_director(
                    &mut w,
                    &stg_core::tables::TABLES_V0,
                    &ecl,
                    &InputFrame::empty(f),
                    |b| super::bench_rearm_enemy_vel(b, &handles, space, f),
                );
            }
            assert_eq!(
                w.body.view().enemies().iter_alive().count(),
                256,
                "全程 721 帧后 256 只敌应一只不少（不漂出边界）"
            );
            // ⚠️ 上面那条**不押这一档的那根轴**（复审逮到）：把武装用的 easing 写成越界值，
            // `enemy_vel_precheck` 会按 P4-b 拒收 ⇒ 每次重新武装都是空操作、256 只敌全程
            // 静止零插值，而"没漂出边界"照样成立、测试照绿。真正塌掉的是这一档的头条结论
            // （笛/极两行之差 = `backfill_enemy_polar` 的价钱），它会静默变成 ~0。
            // 故必须直接押"插值器确实在飞"。
            let pool = w.body.view().enemies();
            let armed = pool
                .iter_alive()
                .filter(|&i| pool.vel_active()[i] == 1)
                .count();
            assert!(
                armed >= 250,
                "末帧应有几乎全部敌人带着在飞的速度插值器，实得 {armed}/256\
                 （若为 0，多半是武装参数被 P4-b 拒收 ⇒ 这一档测的是 256 只静止敌）"
            );
        }
    }
}
