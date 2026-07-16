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
        Some("bake-tables") => cmd_bake_tables(),
        Some("verify-tables") => cmd_verify_tables(),
        _ => {
            eprintln!("usage: stg-harness <golden [--out FILE] | bake-tables | verify-tables>");
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

/// 金向量 —— 真实 step 演化的碰撞病态诊断场景，逐帧 World 校验和（CI 跨平台对拍的数据源）。
///
/// 导演每 60 帧把敌人补到顶部固定 3 位（静止靶——AI/move_to 插值留后续切片），每 8 帧从
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
/// 600 帧 @ 60Hz。
fn cmd_golden(rest: &[String]) -> ExitCode {
    use stg_core::bullets::{BulletHandle, BulletInit};
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
        drop_table: 0,
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
        step_with_director(&mut world, &input, |b| {
            // ① 每 60 帧把敌人补到 3 个（顶部固定三点；被自机弹打死→cleanup 回收→补位 churn）
            if frame % 60 == 0 {
                let alive = b.enemies.iter_alive().count();
                let slots = [(-80, 80), (0, 80), (80, 80)];
                for &(ex, ey) in slots.iter().skip(alive) {
                    b.create_enemy(enemy_at(ex, ey));
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
        });
        lines.push_str(&format!("{frame} {:016x}\n", world.checksum()));
    }

    match parse_out(rest) {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, lines) {
                eprintln!("error: 写入 {path} 失败: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("golden: {FRAMES} 帧真实 step 演化校验和已写入 {path}");
        }
        None => print!("{lines}"),
    }
    ExitCode::SUCCESS
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
