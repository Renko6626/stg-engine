//! stg-harness —— 金向量对拍 + 烘焙表工具的命令行入口。
//!
//! 子命令：
//!   golden [--out FILE]   跑金向量，逐帧输出校验和（CI 跨平台对拍的数据源）
//!   bake-tables           用 f64 生成 sin/cos/easing 烘焙表原始字节（M0 落地）
//!   verify-tables         断言现生成的表字节 == 已 commit 的字节（CI 防漂移）
//!
//! 本 crate 在断层线【以上】，可用浮点；stg-core 只消费 commit 的表字节。

use std::process::ExitCode;

mod tables;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("golden") => cmd_golden(&args[2..]),
        Some("bench") => cmd_bench(&args[2..]),
        Some("bake-tables") => cmd_bake_tables(),
        Some("verify-tables") => cmd_verify_tables(),
        _ => {
            eprintln!(
                "usage: stg-harness <golden [--out FILE] | bench [--frames N] | bake-tables | verify-tables>"
            );
            ExitCode::FAILURE
        }
    }
}

/// 从 `rest` 中解析 `--out FILE`。
fn parse_out(rest: &[String]) -> Option<String> {
    rest.iter()
        .position(|a| a == "--out")
        .and_then(|i| rest.get(i + 1).cloned())
}

/// benchmark（M0-18）—— step 性能基线 + rollback 成本账（ECL 指令预算/M3 快照环的实测依据）。
///
/// 场景阶梯：哑弹 1024/2048/4096/8192（纯积分+行1/2 碰撞）· xform 弹 512/1024/2048
/// （SET_ANG_VEL 逐帧 sincos 回填 + 段池满载）· 全混合（敌/杀敌掉落/道具/每150帧消弹转星/
/// 自机满火力四路+子机）。每档预热 120 帧、实测 `--frames`（默认 600）帧。
///
/// 三组耗时分开计：step 本体（含导演补弹）/ `copy_into` 整块快照 / 全量校验和——后两者
/// 是 rollback 与联机采样的预算数。计时用 `std::time::Instant`（断层线之上，仅测不喂）。
/// **务必 `--release` 跑**；debug 构建会打印警告（数字仅供相对比较）。
fn cmd_bench(rest: &[String]) -> ExitCode {
    use stg_core::step::World;

    let frames: u32 = rest
        .iter()
        .position(|a| a == "--frames")
        .and_then(|i| rest.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);

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
    };
    let enemy = |x: i32| EnemyInit {
        x: Fx::from_int(x),
        y: Fx::from_int(80),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
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
        main_task: 0,
        death_script: 0,
        drop_table: 1,
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

/// 预热 120 帧 → 实测 N 帧；step/快照/校验和三组分计，出一行报告。
fn run_measured(
    name: &str,
    print: bool,
    w: &mut stg_core::step::World,
    frames: u32,
    mut director: impl FnMut(&mut stg_core::world::WorldBody, u32),
    mut input_of: impl FnMut(u32) -> stg_core::input::InputFrame,
) {
    use std::time::Instant;
    use stg_core::ecl::image::EclImage;
    use stg_core::step::{World, step_with_director};
    use stg_core::tables::TABLES_V0;

    const WARMUP: u32 = 120;
    let mut snap = World::new(0);
    let ecl = EclImage::empty(); // 本刀无脚本场景：显式传空镜像（零任务零成本）
    let mut step_ns: Vec<u64> = Vec::with_capacity(frames as usize);
    let mut snap_ns: u64 = 0;
    let mut sum_ns: u64 = 0;
    for frame in 0..(WARMUP + frames) {
        let input = input_of(frame);
        let t0 = Instant::now();
        step_with_director(w, &TABLES_V0, &ecl, &input, |b| director(b, frame));
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

/// 金向量 —— 真实 step 演化的碰撞病态诊断场景，逐帧 World 校验和（CI 跨平台对拍的数据源）。
///
/// 导演每 60 帧把敌人补到顶部固定 3 位（场外 (ex, -100) 生 + `move_enemy_to` 40 帧 QuadOut
/// 飘入原位——压大边界内存活 + move_to 插值路径，飘入途中同样可被自机弹打中/体碰），每 8 帧从
/// 顶部中心铺一圈 10 发敌弹（rng 抖动 → 压 sincos + PCG32），每 150 帧全屏消弹一次
/// （`FIELD_RADIUS_FULLSCREEN` 作用区，`life=1`）。脚本自机全程射击、90 帧周期
/// 上冲吃弹/下退喘息，串联自机弹杀敌→dying→EnemyDied→cleanup 回收→导演补位、敌弹中弹→
/// 决死窗口→死亡→重生、graze 累积等碰撞/结算全链路。
///
/// D3 加戏（每帧压运动双表示）：每 40 帧铺一圈 8 发螺旋弹（POLAR_FX，ang_vel 驱动逐帧
/// sincos 回填 v）；每 90 帧发 3 发上抛重力弹（CART_FX，逐帧 CORDIC atan2 + isqrt 回填
/// 作者视图，顶点前后扫过 `BACKFILL_MIN_SPEED` 阈值两侧）；每 75 帧对螺旋圈最近一发 setter
/// 骚扰（turn/aim/set_vel 轮转，句柄可能已随生死回收变成悬垂——P4-b no-op 路径顺带入金向量）。
///
/// D4 加戏（变换游标压段池/游标推进）：每 50 帧（`frame % 50 == 10`）发一对之字加速弹
/// （LOOP 跳回 TURN ±90° 无限循环 + 开局 SET_ACCEL 常量加速，压单游标多 op 连发与段池长驻）；
/// 每 70 帧（`frame % 70 == 30`）发一发 SET_LIFE 自爆弹——排程于相位 4，45 帧后寿命改判 1；
/// 相位 5 integrate **同帧**减到 0；相位 9 cleanup **当帧**回收（不是"下一帧"）→还段路径入对拍。
///
/// 11b 加戏（三压力源，续 D4 编号）：每 40 帧偏移 25（`frame % 40 == 25`）停驻一发双重
/// WAIT_SIGNAL 弹（两级 TURN 各挂一道信号门，形成"双重驻停链"，第一道门后 `wait: 1` 隔开
/// 同帧边沿的连锁放行）；每 120 帧偏移 60（`frame % 120 == 60`）在信号通道 0 上
/// `pulse_signal`——两段停驻、两次脉冲各放行一段，真双停驻、轨迹可见变化（黑板齐转语义
/// 压 signals 数组 + 边沿命中路径）。每 90 帧偏移 45
/// （`frame % 90 == 45`）发一发 BOUNCE_ARM 三墙武装弹（左右上，`n=3`），POLAR 速度域入
/// 墙反弹镜像对拍。每 65 帧偏移 20（`frame % 65 == 20`）发一发 STEP_SPEED 缓动弹——
/// Smoothstep（easing id 7）40 帧从 0.5 缓到 3.0，压连续插值 scratch 与游标并发路径。
///
/// 道具趟加戏（续 11b 编号）：导演敌人 `drop_table` 由 0 改 1（标准杂鱼表），敌死走 settle
/// 趟二自动按表掉落——散布 RNG、重力/终速下落物理、近距/PoC/`attract_all_items` 三源磁吸、
/// 拾取入账全部进对拍流。块 ⑫ 每 200 帧偏移 90（`frame % 200 == 90`）额外调一次全场磁吸
/// （导演/bomb 入口），排在相位 3 `update_players` 之前——自机彼时若非 ALIVE（决死窗口/
/// 等待重生），走确定性 P4-b no-op + 计数，与块 ⑥ setter 骚扰 `spiral_h` 悬垂同款哲学。
///
/// batch 压力源（续 ⑫ 编号，M0-14 `create_bullets_batch` 三态直通）：⑬ 每 90 帧偏移 35
/// （`frame % 90 == 35`）铺一发 32-way 哑弹整环（`n_speed=1`，角步 2048=65536/32 整环闭合，
/// 速 1.2px/帧）——压批量哑弹分配序、径向匀散必越界回收。⑭ 每 110 帧偏移 70
/// （`frame % 110 == 70`）发一列 5 重速度正下弹（`n_angle=1`、`Angle(16384)` 竖直向下，
/// 1.0→3.0px/帧步 0.5）——压批量单角度速度轴列、匀速直线必越界回收。⑮ 每 150 帧偏移 130
/// （`frame % 150 == 130`）发 3 角 × 4 速 12 发扇形网格弹，每发自带两槽 xform
/// （`SET_ANG_VEL(256)` + 隐式尾零 END）——段消耗账 12 段/次入对拍，压批量 xform 分支的
/// 逐颗先段后弹路径；出生点刻意贴近下边界（常量角速度令轨迹趋近小半径圆弧，center-ish
/// 出生半径不足以够到任一边界，靠近边界出生才能让扇面多数越界回收，慢速小半径个别残留
/// 打转无损确定性）。
///
/// 消弹转星星（M0-15，**导演零改动**）：块 ③ 每 150 帧的全屏消弹自动逐弹原位转星
/// （一律转化）——星星雨/出生即磁吸/30 分入账/可能的道具池满降级全链入对拍
/// （实测改前后首个分歧帧 = 150，与首次消弹帧吻合）。
///
/// 火力拔档（M0-17 T5，续 ⑮ 编号 ⑯）：帧 200 导演经 `set_player_power(0, 250)`
/// （拔到 tier 2，shottype 两路弹型入流）、帧 400 拔到 400（tier 4，三路本体 + 1 路
/// 子机入流）——诊断场景导演直写档位合法，压相位 3 解释器逐档分支 + 子机弹随即参与
/// 碰撞/擦弹结算链，换档瞬时生效。
/// 600 帧 @ 60Hz。
fn cmd_golden(rest: &[String]) -> ExitCode {
    use stg_core::bullets::{BulletHandle, BulletInit};
    use stg_core::ecl::image::EclImage;
    use stg_core::enemy::EnemyInit;
    use stg_core::field::{FIELD_CLEAR_BULLETS, FIELD_RADIUS_FULLSCREEN, FieldInit};
    use stg_core::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame};
    use stg_core::math::{Angle, Fx, polar_to_vec};
    use stg_core::step::{World, step_with_director};
    use stg_core::xform::{
        OP_BOUNCE_ARM, OP_END, OP_LOOP, OP_SET_ACCEL, OP_SET_ANG_VEL, OP_SET_ANGLE, OP_SET_LIFE,
        OP_SET_SPEED, OP_STEP_SPEED, OP_TURN, OP_WAIT_SIGNAL, XformSlot,
    };

    const FRAMES: u32 = 600; // 10 秒 @ 60Hz
    const SEED: u64 = 0x5147_4f4c_4445_4e00; // "GOLDEN"
    let mut world = World::new(SEED);
    let ecl = EclImage::empty(); // 一号场景无脚本：显式传空镜像（零任务零成本，T2 不变门）
    let mut lines = String::new();
    // D3 压力源状态：螺旋圈最近一发的句柄，供 setter 骚扰块追打（可能随生命周期死亡/回收）。
    let mut spiral_h = BulletHandle::NULL;

    // 全字段 EnemyInit 助手（顶部三敌人固定位；move_to/挂钩惰性）。
    let enemy_at = |x: i32, y: i32| EnemyInit {
        x: Fx::from_int(x),
        y: Fx::from_int(y),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
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
        main_task: 0,
        death_script: 0,
        drop_table: 1,
        score: 100,
    };

    // 全字段 BulletInit 助手（D3 压力源哑弹：静止、半径 3、life 走满、无变换挂钩）。
    let bullet_at = |x: i32, y: i32| BulletInit {
        x: Fx::from_int(x),
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
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: 0xFFFF,
        xform_wait: 0,
        xform_next: 0,
    };

    for frame in 0..FRAMES {
        // 脚本化输入：多数时间上冲吃弹 + 全程射击（确定性触发中弹/擦弹/杀敌）。
        let mut input = InputFrame::empty(frame);
        let mut btn = BTN_SHOT; // 全程射击 → 自机弹上飞杀顶部敌人（行 4）
        // 每 90 帧一个周期：前 50 帧上冲（吃弹/擦弹/逼近敌体），后 40 帧下退（喘息）
        let phase = frame % 90;
        if phase < 50 {
            btn |= BTN_UP;
        } else {
            btn |= BTN_DOWN;
        }
        // 左右缓移增加位形多样性
        btn |= if (frame / 45) % 2 == 0 {
            BTN_RIGHT
        } else {
            BTN_LEFT
        };
        if (frame / 120) % 2 == 0 {
            btn |= BTN_SLOW;
        }
        input.actions[0].buttons = btn;
        step_with_director(
            &mut world,
            &stg_core::tables::TABLES_V0,
            &ecl,
            &input,
            |b| {
                // ① 每 60 帧把敌人补到 3 个（顶部固定三点；被自机弹打死→cleanup 回收→补位 churn）。
                // 场外飘入：出生在 (ex, -100)——旧共用界（y∈[-64,512]）外必死、新敌人大边界
                // （y∈[-256,704]，ENEMY_OOB_MARGIN=256）内存活——再 `move_enemy_to` 40 帧
                // QuadOut 飘到原位 (ex, 80)。到位时序因此推迟：补位帧起飞入途中一切照旧参与
                // 演化（可被自机弹打中/体碰/擦弹），只是 40 帧后才真正落在旧的静止靶位。
                if frame % 60 == 0 {
                    let alive = b.view().enemies().iter_alive().count();
                    let slots = [(-80, 80), (0, 80), (80, 80)];
                    for &(ex, ey) in slots.iter().skip(alive) {
                        let h = b.create_enemy(enemy_at(ex, -100));
                        b.move_enemy_to(h, Fx::from_int(ex), Fx::from_int(ey), 40, 2); // QuadOut
                    }
                }
                // ② 每 8 帧从顶部中心铺一圈 10 发敌弹（rng 抖动；部分下行抵达自机 → 中弹/擦弹）
                if frame % 8 == 0 {
                    let base = (frame.wrapping_mul(797) & 0xFFFF) as u16;
                    let n: u16 = 10;
                    let astep = (65536u32 / n as u32) as u16;
                    for k in 0..n {
                        let spread = b.rng.rand_range(384) as u16;
                        let a = Angle(
                            base.wrapping_add(k.wrapping_mul(astep))
                                .wrapping_add(spread),
                        );
                        let (vx, vy) = polar_to_vec(Fx::from_int(2), a);
                        b.create_bullet(BulletInit {
                            x: Fx::ZERO,
                            y: Fx::from_int(100),
                            vx,
                            vy,
                            speed: Fx::from_int(2),
                            angle: a,
                            ang_vel: 0,
                            accel: Fx::ZERO,
                            ax: Fx::ZERO,
                            ay: Fx::ZERO,
                            sprite: 0,
                            radius: Fx::from_int(3),
                            delay: 0,
                            life: 300,
                            flags: 0,
                            grazed_by: 0,
                            transform_head: 0xFFFF,
                            xform_wait: 0,
                            xform_next: 0,
                        });
                    }
                }
                // ③ 每 150 帧全屏消弹一次（压消弹标记/回收 churn + 聚合事件路径）
                if frame % 150 == 0 && frame > 0 {
                    b.create_field(FieldInit {
                        x: Fx::ZERO,
                        y: Fx::from_int(224), // 场心
                        radius: FIELD_RADIUS_FULLSCREEN,
                        dmg_per_frame: 0,
                        life: 1, // 只活本帧
                        owner: 0,
                        flags: FIELD_CLEAR_BULLETS,
                    });
                }
                // ④ D3 压力源一：螺旋圈（POLAR_FX——每帧 sincos 查表路径）
                if frame % 40 == 0 {
                    for k in 0..8u16 {
                        let h = b.create_bullet(bullet_at(0, 60));
                        b.set_bullet_speed(h, Fx::from_raw(98_304)); // 1.5 px/帧
                        b.set_bullet_angle(h, Angle(k * 8192)); // 八方位
                        b.set_bullet_ang_vel(h, if k % 2 == 0 { 512 } else { -512 });
                        spiral_h = h;
                    }
                }
                // ⑤ D3 压力源二：上抛重力弹（CART_FX——每帧 CORDIC+isqrt 回填，顶点扫过阈值两侧）
                if frame % 90 == 0 {
                    for k in 0..3i32 {
                        let h = b.create_bullet(bullet_at(-60 + 60 * k, 200));
                        b.set_bullet_vel(h, Fx::ZERO, Fx::from_int(-3));
                        b.set_bullet_gravity(h, Fx::ZERO, Fx::from_raw(16_384)); // 0.25 px/帧²
                    }
                }
                // ⑥ D3 压力源三：setter 骚扰（句柄可能已死——P4-b 路径顺带入金向量，确定性无损）
                if frame % 75 == 0 {
                    match (frame / 75) % 3 {
                        0 => b.turn_bullet(spiral_h, Angle::QUARTER),
                        1 => b.aim_bullet_at_player(spiral_h, Angle::ZERO),
                        _ => b.set_bullet_vel(spiral_h, Fx::from_int(2), Fx::from_int(1)),
                    }
                }
                // ⑦ D4 压力源一：之字加速弹——"单游标天然并发"招牌（LOOP TURN ±90° + 开局 SET_ACCEL）
                if frame % 50 == 10 {
                    let zig = [
                        XformSlot {
                            wait: 0,
                            op: OP_SET_SPEED,
                            _pad: 0,
                            args: [65_536, 0],
                        },
                        XformSlot {
                            wait: 0,
                            op: OP_SET_ACCEL,
                            _pad: 0,
                            args: [1_638, 0],
                        },
                        XformSlot {
                            wait: 20,
                            op: OP_TURN,
                            _pad: 0,
                            args: [16_384, 0],
                        },
                        XformSlot {
                            wait: 20,
                            op: OP_TURN,
                            _pad: 0,
                            args: [-16_384_i32, 0],
                        },
                        XformSlot {
                            wait: 0,
                            op: OP_LOOP,
                            _pad: 0,
                            args: [2, 0],
                        }, // 无限之字
                    ];
                    b.create_bullet_with_xform(bullet_at(-100, 60), &zig);
                    b.create_bullet_with_xform(bullet_at(100, 60), &zig);
                }
                // ⑧ D4 压力源二：SET_LIFE 自爆弹——排程改寿命 + 弹死还段路径入流
                if frame % 70 == 30 {
                    let fuse = [
                        XformSlot {
                            wait: 0,
                            op: OP_SET_SPEED,
                            _pad: 0,
                            args: [131_072, 0],
                        },
                        XformSlot {
                            wait: 45,
                            op: OP_SET_LIFE,
                            _pad: 0,
                            args: [1, 0],
                        },
                    ];
                    b.create_bullet_with_xform(bullet_at(0, 150), &fuse);
                }
                // ⑨ 11b 压力源一：真双停驻——两段信号门各自停驻，被两次不同脉冲分别放行
                if frame % 40 == 25 {
                    b.create_bullet_with_xform(
                        bullet_at(-120, 90),
                        &[
                            XformSlot {
                                wait: 0,
                                op: OP_SET_SPEED,
                                _pad: 0,
                                args: [49_152, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_WAIT_SIGNAL,
                                _pad: 0,
                                args: [0, 0],
                            },
                            XformSlot {
                                wait: 1,
                                op: OP_TURN,
                                _pad: 0,
                                args: [32_768, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_WAIT_SIGNAL,
                                _pad: 0,
                                args: [0, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_TURN,
                                _pad: 0,
                                args: [32_768, 0],
                            },
                        ],
                    );
                }
                if frame % 120 == 60 {
                    b.pulse_signal(0);
                }
                // ⑩ 11b 压力源二：三墙反弹弹（左右上，n=3）——POLAR 域镜像入对拍
                if frame % 90 == 45 {
                    b.create_bullet_with_xform(
                        bullet_at(0, 120),
                        &[
                            XformSlot {
                                wait: 0,
                                op: OP_SET_SPEED,
                                _pad: 0,
                                args: [196_608, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_SET_ANGLE,
                                _pad: 0,
                                args: [6_000, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_SET_ANG_VEL,
                                _pad: 0,
                                args: [0, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_BOUNCE_ARM,
                                _pad: 0,
                                args: [0b0111, 3],
                            },
                        ],
                    );
                }
                // ⑪ 11b 压力源三：STEP 缓动弹——Smoothstep 40 帧从 0.5 缓到 3.0
                if frame % 65 == 20 {
                    b.create_bullet_with_xform(
                        bullet_at(60, 70),
                        &[
                            XformSlot {
                                wait: 0,
                                op: OP_SET_SPEED,
                                _pad: 0,
                                args: [32_768, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_SET_ANGLE,
                                _pad: 0,
                                args: [16_384, 0],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_STEP_SPEED,
                                _pad: 0,
                                args: [196_608, 40 | (7 << 16)],
                            },
                            XformSlot {
                                wait: 0,
                                op: OP_END,
                                _pad: 0,
                                args: [0, 0],
                            }, // 扩展槽占位
                        ],
                    );
                }
                // ⑫ 道具趟压力源：每 200 帧偏移 90 全场磁吸一次（导演/bomb 入口，drop_table=1
                // 已令敌死掉落进流——散布 RNG、下落物理、拾取入账全部入对拍）。自机彼时非
                // ALIVE（决死窗口/等待重生）→ 确定性 P4-b no-op + 计数，与 ⑥ setter 骚扰
                // spiral_h 悬垂同款哲学：坏时机调用不崩、结果确定、计数入校验和。
                if frame % 200 == 90 {
                    b.attract_all_items(0);
                }
                // ⑬ batch 压力源一：32-way 哑弹整环（create_bullets_batch 环路径——n_speed=1，
                // 角步 2048=65536/32 整环闭合，速 1.2px/帧；32 发径向匀散，池分配序连号）。
                if frame % 90 == 35 {
                    b.create_bullets_batch(
                        bullet_at(0, 60),
                        &[],
                        32,
                        Angle::ZERO,
                        2048,
                        1,
                        Fx::from_raw(78_643), // 1.2px/帧
                        Fx::ZERO,
                    );
                }
                // ⑭ batch 压力源二：5 重速度正下列（n_angle=1，Angle(16384) 竖直向下——
                // 1.0→3.0px/帧步 0.5；压批量单角度速度轴列，匀速直线必越界回收）。
                if frame % 110 == 70 {
                    b.create_bullets_batch(
                        bullet_at(-150, 40),
                        &[],
                        1,
                        Angle(16384),
                        0,
                        5,
                        Fx::from_int(1),
                        Fx::from_raw(32_768), // 步 0.5
                    );
                }
                // ⑮ batch 压力源三：3 角 × 4 速扇形网格（12 发/次，每发自带两槽 xform
                // `SET_ANG_VEL(256)`+隐式尾零 END——段消耗账 12 段/次）。常量角速度令轨迹
                // 趋近小半径圆弧（周期 65536/256=256 帧，半径∝speed/ang_vel，约 41~102px）——
                // 出生点贴近下边界（y=460）让扇面高速侧越界回收；实测 600 帧内 4/12 扇位
                // （speed 1.0 全部三角 + 18432@1.5，max_y 492~508 差临门一脚够不到 y=512）
                // 永久打转、弹+段常驻——这是有意保留的长驻段占用压力（段位图入校验和），
                // 有界：600 帧共 4 次触发 ≤16 弹/段，远小于池容量；金向量若延长需重估。
                if frame % 150 == 130 {
                    b.create_bullets_batch(
                        bullet_at(0, 460),
                        &[XformSlot {
                            wait: 0,
                            op: OP_SET_ANG_VEL,
                            _pad: 0,
                            args: [256, 0],
                        }],
                        3,
                        Angle(14_336),
                        2048,
                        4,
                        Fx::from_int(1),
                        Fx::from_raw(32_768), // 步 0.5
                    );
                }
                // ⑯ M0-17 T5：导演直写火力拔档（诊断场景合法）——帧 200 拔到 250（tier 2，
                // shottype 两路入流）、帧 400 拔到 400（tier 4，三路本体 + 1 路子机入流，
                // 子机弹随即参与相位 6/7 碰撞/擦弹结算链）。换档瞬时生效，无过渡状态。
                if frame == 200 {
                    b.set_player_power(0, 250);
                }
                if frame == 400 {
                    b.set_player_power(0, 400);
                }
            },
        );
        lines.push_str(&format!("{frame} {:016x}\n", world.checksum()));
    }

    // ── 金向量二号：彩虹风铃卡（M1 T4）——续接一号场景之后，同一 `--out` 文件追加段 ──
    // 一号场景（上方 `world`/`lines`/`FRAMES`/`SEED`）逐字节不动；本段用独立全新 World +
    // 独立种子，`# scene: ecl-rainbow` 分隔行标记段界（CI 零改动：三平台仍只 diff 同一份
    // 文件的逐行文本，两段各自逐帧校验和天然对拍）。
    //
    // 输入拍板（brief 二选一）：全程持 BTN_SHOT + 左右缓移（非全程 idle）——让弹幕的
    // 碰撞/擦弹路径真的被自机踩到，符卡本体仍是主角，移动只是"不空闲"。
    {
        use stg_core::ecl::binding::EclOwner;

        const FRAMES2: u32 = 600;
        const SEED2: u64 = 0x524E_424F_5701; // "RNBW"

        lines.push_str("# scene: ecl-rainbow\n");

        let image = compile_rainbow_image();
        let mut world2 = World::new(SEED2);
        world2.body.set_var(RANK_SLOT, 2); // 环密度算式：28+rank×2 → rank=2 时 32-way

        let boss = world2.body.create_enemy(EnemyInit {
            x: Fx::from_int(BOSS_X),
            y: Fx::from_int(BOSS_Y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
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
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 10000,
        });
        world2
            .start_main_with_owner(&image, EclOwner::Enemy(boss))
            .expect("main 任务应能派生（新镜像/新池，容量均未耗尽）");

        for frame in 0..FRAMES2 {
            let mut input = InputFrame::empty(frame);
            let mut btn = BTN_SHOT;
            btn |= if (frame / 60) % 2 == 0 {
                BTN_LEFT
            } else {
                BTN_RIGHT
            };
            input.actions[0].buttons = btn;
            step_with_director(
                &mut world2,
                &stg_core::tables::TABLES_V0,
                &image,
                &input,
                |_| {},
            );
            lines.push_str(&format!("{frame} {:016x}\n", world2.checksum()));
        }
    }

    match parse_out(rest) {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, lines) {
                eprintln!("error: 写入 {path} 失败: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("golden: 两段场景校验和已写入 {path}");
        }
        None => print!("{lines}"),
    }
    ExitCode::SUCCESS
}

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

/// rainbow.ecl 源码文本——`include_str!` 相对路径从本文件（`stg-harness/src/main.rs`）
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

/// 烘焙 sin/cos/easing 表 —— 用 f64 生成原始字节并写入 stg-core 源目录（§2.1）。
fn cmd_bake_tables() -> ExitCode {
    match tables::bake_all() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bake-tables 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 断言现生成的表字节 == 已 commit 的字节（§2.1 CI 防漂移）。
fn cmd_verify_tables() -> ExitCode {
    match tables::verify_all() {
        Ok(()) => {
            eprintln!("verify-tables: 全部表与 commit 字节一致 ✔");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("verify-tables 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod ecl_rainbow_tests {
    use super::*;
    use stg_core::ecl::binding::EclOwner;
    use stg_core::ecl::image::ResolveError;
    use stg_core::enemy::EnemyInit;
    use stg_core::input::InputFrame;
    use stg_core::math::Fx;
    use stg_core::step::{World, step_with_director};

    fn boss_init() -> EnemyInit {
        EnemyInit {
            x: Fx::from_int(BOSS_X),
            y: Fx::from_int(BOSS_Y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
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
            main_task: 0,
            death_script: 0,
            drop_table: 0,
            score: 10000,
        }
    }

    /// 编译干净：rainbow.ecl 应零错误编译（狗粮验收的地基——语言真的能表达这张卡）。
    /// 镜像结构：三 sub（patrol/timer_ui/main，规范排序）、named entry 解析正确、
    /// Root 不可通过 resolve_entry("main") 获取（须走 start_main）。
    #[test]
    fn rainbow_ecl_compiles_clean_with_named_entries() {
        let image = compile_rainbow_image();
        assert_eq!(image.sub_count(), 3, "patrol + timer_ui + main");
        assert!(image.root().is_some(), "main 应被标记为 Root");
        assert!(
            image.resolve_entry("patrol").is_ok(),
            "async sub patrol 应是可解析的 named entry"
        );
        assert!(
            image.resolve_entry("timer_ui").is_ok(),
            "async sub timer_ui 应是可解析的 named entry"
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
    /// （hp_max 9999 扛得住 600 帧的自机火力，不会归零死亡）、`boss_ui[0].active==1` 且
    /// `hp_ratio` 是真值（非零、≤1.0fx——C13②"ratio = $self_hp/$self_hp_max 真算"的行为学
    /// 证据：真实建场下自机弹确实打中 boss，`hp_ratio` 实测 <1.0，不是恒为 1.0 的占位符）、
    /// 任务数 >=3（main+patrol+timer_ui 三子全存活——三者皆 `loop {}`，不自灭）、全程零 Fault。
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
        assert_eq!(w.body.diag.task_faults, 0, "全程不应产生 Fault");
        let bullet_count = w.body.view().bullets().iter_alive().count();
        assert!(bullet_count > 100, "稳态弹数应 >100（实测 {bullet_count}）");
        assert!(
            w.body.view().enemies().get(boss).is_some(),
            "boss 应存活满 600 帧（hp_max 9999 扛住火力）"
        );
        assert_eq!(w.body.boss_ui[0].active, 1, "符卡计时器应保持 active=1");
        let ratio = w.body.boss_ui[0].hp_ratio;
        assert!(
            ratio.raw() > 0 && ratio.raw() <= Fx::ONE.raw(),
            "hp_ratio 应是非零、≤1.0fx 的真值（实测 raw={}）——C13②证据",
            ratio.raw()
        );
        let task_count = w.tasks.iter_alive().count();
        assert!(
            task_count >= 3,
            "main+patrol+timer_ui 三任务应全存活（实测 {task_count}）"
        );
    }
}

#[cfg(test)]
mod table_disk_load_tests {
    /// D3 端到端自证（C11 收尾）：从磁盘 `std::fs::read` + `from_bytes` 载入已 commit 的
    /// `tables_v0.bin`，与 `include_bytes!` 内建 `TABLES_V0`（同一份字节）各自建 World 跑
    /// 120 帧，逐帧校验和须一致。二者字节相同不是巧合——本测试证的是**加载路径**：
    /// disk → `from_bytes` → `new_with_tables` → `step` 这条环真的产出可运行、位级一致的
    /// World，而不仅是"字节比对"。
    #[test]
    fn table_loaded_from_disk_runs_identically_to_builtin() {
        // 注：`World::checksum()` 是 `step.rs` 上的一个便利**固有方法**（内部转发到
        // `checksum::Checksum` trait），固有方法在方法解析中优先于同名 trait 方法命中——
        // 故此处不需要（也不能，否则 `-D warnings` 治下 unused_imports 报错）
        // `use stg_core::checksum::Checksum;` 把 trait 带入作用域。
        use stg_core::ecl::image::EclImage;
        use stg_core::input::{BTN_LEFT, BTN_SHOT, BTN_UP, InputFrame};

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../stg-core/src/tables/tables_v0.bin"
        );
        let bytes = std::fs::read(path).expect("committed tables_v0.bin exists");
        let loaded = stg_core::tables::WorldTables::from_bytes(&bytes).expect("disk table loads");

        let mut w_disk = stg_core::World::new_with_tables(0xC11, &loaded);
        let mut w_builtin = stg_core::World::new(0xC11);
        let ecl = EclImage::empty();
        for frame in 0..120u32 {
            // 复审强化（非 idle 输入）：全程持 SHOT + 左上对角移动——让
            // `char0_update_shot` 真正读 `tables.characters[..].shot`（发弹，命中
            // `debug_assert!(shooter.interval != 0)`）、`move_player` 真正读移动
            // `CharacterCfg` 参数（高/低速、对角归一），二者都把加载表的数值喂进
            // 校验和覆盖的世界状态——而不只是出生期的 `hit_radius`/`graze_radius`。
            // 两侧逐帧同一个 `input` 值，磁盘/内建两条腿除表来源外别无二致。
            let mut input = InputFrame::empty(frame);
            input.actions[0].buttons = BTN_SHOT | BTN_LEFT | BTN_UP;
            stg_core::step(&mut w_disk, &loaded, &ecl, &input);
            stg_core::step(&mut w_builtin, &stg_core::tables::TABLES_V0, &ecl, &input);
        }
        assert_eq!(
            w_disk.checksum(),
            w_builtin.checksum(),
            "磁盘加载表与内建表逐帧一致——文件加载闭环自证"
        );
        // 非退化守卫：证明上面不是一次「全程 idle」的空转——校验和非零、且真的发出了
        // 子弹（`char0_update_shot` 读到的 `interval=4` 的 shooter 在 120 帧内必命中
        // 多次），两侧防止本测试悄悄退化回「只比对出生期字段」。
        assert_ne!(w_disk.checksum(), 0, "非退化守卫：校验和不应为零");
        // 自机弹住 `shots` 池（`bullets` 是敌方弹池，D7/D8 分池）。
        let shot_count = w_disk.body.view().shots().iter_alive().count();
        assert!(
            shot_count > 0,
            "非退化守卫：120 帧全程持 SHOT 应产出自机弹（实测 {shot_count}）"
        );
    }
}
