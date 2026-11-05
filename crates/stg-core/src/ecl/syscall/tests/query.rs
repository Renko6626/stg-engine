//! 世界、owner、数学与敌句柄查询。

use super::*;

#[test]
fn sys_frame_reads_world_frame() {
    let (mut w, ecl) = fresh();
    w.body.frame = 123;
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_FRAME, &[]).is_ok());
    assert_eq!(task.stack[0], 123);
}

#[test]
fn sys_player_xy_reads_player0_position() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_PLAYER_X, &[]).is_ok());
    assert_eq!(task.stack[0], w.body.players[0].x.raw());
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_PLAYER_Y, &[]).is_ok());
    assert_eq!(task.stack[0], w.body.players[0].y.raw());
}

/// self_x/self_y/self_hp 三态：ENEMY→敌池读、BULLET→弹池位置(hp 恒 0)、STAGE→恒 0。
#[test]
fn sys_self_reads_dispatch_by_owner_kind() {
    let (mut w, ecl) = fresh();
    let eh = w.body.create_enemy(crate::enemy::EnemyInit {
        x: Fx::from_int(11),
        y: Fx::from_int(22),
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
        hp: 7,
        hp_max: 7,
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
        script_int: [0; crate::enemy::ENEMY_INT_SLOTS],
        script_fx: [Fx::ZERO; crate::enemy::ENEMY_FX_SLOTS],
        drop_count: [0; crate::items::ITEM_TYPE_COUNT],
        score: 0,
    });
    let mut enemy_task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_X, &[]).is_ok());
    assert_eq!(enemy_task.stack[0], Fx::from_int(11).raw());
    enemy_task.sp = 0;
    assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_Y, &[]).is_ok());
    assert_eq!(enemy_task.stack[0], Fx::from_int(22).raw());
    enemy_task.sp = 0;
    assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_HP, &[]).is_ok());
    assert_eq!(enemy_task.stack[0], 7);

    let bh = crate::world::test_support::bullet_at(&mut w, 33, 44);
    let mut bullet_task = Task {
        owner_kind: OWNER_BULLET,
        owner_index: bh.index,
        owner_gen: bh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut bullet_task, SYS_SELF_X, &[]).is_ok());
    assert_eq!(bullet_task.stack[0], Fx::from_int(33).raw());
    bullet_task.sp = 0;
    assert!(call(&mut w, &ecl, &mut bullet_task, SYS_SELF_HP, &[]).is_ok());
    assert_eq!(bullet_task.stack[0], 0, "弹 hp 读恒 0");

    let mut stage_task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut stage_task, SYS_SELF_X, &[]).is_ok());
    assert_eq!(stage_task.stack[0], 0, "STAGE self_x 恒 0");
}

/// 四个 $self_* 在敌 owner 下读到池字段。取 (3, -7) 这个 x≠y 且异号的速度——
/// 派发臂写反立刻可辨。
#[test]
fn self_velocity_vars_read_enemy_pool() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = eh.index as usize;
    w.body.enemies.vx[i] = Fx::from_int(3);
    w.body.enemies.vy[i] = Fx::from_int(-7);
    w.body.enemies.speed[i] = Fx::from_int(9);
    w.body.enemies.angle[i] = crate::math::Angle::QUARTER;
    let mk = || Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    for (sys, want) in [
        (SYS_SELF_VX, Fx::from_int(3).raw()),
        (SYS_SELF_VY, Fx::from_int(-7).raw()),
        (SYS_SELF_SPEED, Fx::from_int(9).raw()),
        (SYS_SELF_ANGLE, crate::math::Angle::QUARTER.raw() as i32),
    ] {
        let mut task = mk();
        assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
        assert_eq!(pop(&mut task).unwrap(), want, "syscall {sys}");
    }
}

/// 弹 owner 下同样有效（弹池本就有这四个字段——双表示是从弹抄来的）。
#[test]
fn self_velocity_vars_dispatch_to_bullet_pool() {
    let (mut w, ecl) = fresh();
    let bh = crate::world::test_support::bullet_at(&mut w, 0, 0);
    let i = bh.index as usize;
    w.body.bullets.vx[i] = Fx::from_int(2);
    let mut task = Task {
        owner_kind: OWNER_BULLET,
        owner_index: bh.index,
        owner_gen: bh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_VX, &[]).is_ok());
    assert_eq!(pop(&mut task).unwrap(), Fx::from_int(2).raw());
}

/// CART_FX 惰性化（引擎第二刀 §5）：弹积分 1 帧后 `speed`/`angle` 是陈值（标脏），
/// 读 `$self_angle` 必须先 materialize —— 值等于 `atan2(vy,vx)`，且**读取会顺带回写**
/// （读完 `BULLET_POLAR_STALE` 应已清零）。
#[test]
fn self_angle_read_materializes_stale_cart_fx_bullet() {
    let (mut w, ecl) = fresh();
    let bh = crate::world::test_support::bullet_at(&mut w, 0, 200);
    let i = bh.index as usize;
    w.body.bullets.vx[i] = Fx::from_int(3);
    w.body.bullets.vy[i] = Fx::from_int(4);
    w.body.set_bullet_gravity(bh, Fx::ZERO, Fx::from_raw(16384)); // 开 CART_FX
    let frame = w.body.frame;
    crate::step::step(
        &mut w,
        &TABLES_V0,
        &ecl,
        &crate::input::InputFrame::empty(frame),
    );
    assert_ne!(
        w.body.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE,
        0,
        "CART_FX 积分一帧后应标脏"
    );
    let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
    let mut task = Task {
        owner_kind: OWNER_BULLET,
        owner_index: bh.index,
        owner_gen: bh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ANGLE, &[]).is_ok());
    assert_eq!(
        pop(&mut task).unwrap(),
        crate::math::cordic::atan2(vy, vx).raw() as i32,
        "$self_angle 应等于 atan2(vy,vx)"
    );
    assert_eq!(
        w.body.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE,
        0,
        "读取应顺带回写，清掉陈值位"
    );
}

/// 非敌非弹 owner → 0（同 $self_x 的既有降级）。
#[test]
fn self_velocity_vars_degrade_to_zero_for_stage_owner() {
    let (mut w, ecl) = fresh();
    for sys in [SYS_SELF_VX, SYS_SELF_VY, SYS_SELF_SPEED, SYS_SELF_ANGLE] {
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert!(call(&mut w, &ecl, &mut task, sys, &[]).is_ok());
        assert_eq!(pop(&mut task).unwrap(), 0, "syscall {sys} 非敌非弹应降级 0");
    }
}

#[test]
fn sys_get_set_var_roundtrip() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // SET_VAR(slot=20, val=42) —— 正序压栈 slot,val；slot=20 落自由段（≥
    // GLOBALS_SYS_SEGMENT=16），非本测试关注点的系统段写保护见
    // `sys_set_var_system_segment_slot_is_guarded_no_op` 单开测试。
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[20, 42]).is_ok());
    assert_eq!(w.body.globals[20], 42);
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_GET_VAR, &[20]).is_ok());
    assert_eq!(task.stack[0], 42);
}

/// globals 段纪律（甲案，M1.5）：`SYS_SET_VAR` 写系统段（slot < `GLOBALS_SYS_SEGMENT`=16）
/// → **no-op**（真槽值不变）+ `diag.contract_viol` +1 + `last_status=BAD_ARGS`，**不 Fault**
/// （`dispatch` 仍返回 `Ok(())`——P4-b 脚本作者违约的确定性安全结果，非引擎 bug，调度层不杀
/// 任务）。边界精确钉死：slot=15（系统段最高位）挡、slot=16（自由段最低位）放行。
/// `SYS_GET_VAR` 两段皆不受限（读无保护）。世界 API `set_var` 直写不经此guard（仍可写系统段
/// ——game 层建场用它写 RANK，见 `world::GLOBALS_SYS_SEGMENT` 文档）。
#[test]
fn sys_set_var_system_segment_slot_is_guarded_no_op() {
    let (mut w, ecl) = fresh();
    w.body.set_var(crate::world::GVAR_RANK, 111); // 世界 API 先写系统段一个已知值（建场惯例）
    let mut task = Task::default();

    // 脚本经 SYS_SET_VAR 写 slot=0（系统段）→ no-op，dispatch 仍 Ok（非 Fault）。
    assert!(
        call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[0, 999]).is_ok(),
        "系统段写保护是 no-op，不是 Fault"
    );
    assert_eq!(w.body.globals[0], 111, "真槽值不变（未被脚本 999 覆盖）");
    assert_eq!(w.body.diag.contract_viol, 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);

    // 边界：slot=15 仍属系统段 → 同样挡。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[15, 999]).is_ok());
    assert_eq!(w.body.globals[15], 0, "slot=15 系统段边界仍挡");
    assert_eq!(w.body.diag.contract_viol, 2);

    // 边界：slot=16 是自由段最低位 → 正常写入（guard 不越界误伤）。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[16, 999]).is_ok());
    assert_eq!(w.body.globals[16], 999, "slot=16 自由段最低位正常写");
    assert_eq!(w.body.diag.contract_viol, 2, "自由段写不应额外计数");

    // 世界 API `set_var` 直写系统段不受本 guard 影响（不同门，见模块文档）。
    w.body.set_var(crate::world::GVAR_RANK, 777);
    assert_eq!(w.body.globals[0], 777, "世界 API 写系统段仍畅通");

    // `SYS_GET_VAR` 读系统段不受限。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_GET_VAR, &[0]).is_ok());
    assert_eq!(task.stack[0], 777, "脚本读系统段不受限");
}

/// 任务龄 `SYS_SELF_AGE`（M1.5）：`ctx.frame - task.born_frame`（wrapping，i32 域）——
/// 纯 dispatch 层直调（不经调度门禁），钉死算式本身；跨帧调度语义的端到端 off-by 判别见
/// `step.rs` 的 `sys_self_age_ticks_task_age_each_frame_via_globals_relay`。
#[test]
fn sys_self_age_computes_frame_minus_born_frame() {
    let (mut w, ecl) = fresh();
    let mut task = Task {
        born_frame: 10,
        ..Task::default()
    };
    w.body.frame = 13;
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_AGE, &[]).is_ok());
    assert_eq!(task.stack[0], 3, "13-10=3");

    // wrapping：born_frame 数值上"晚于" frame（理论不该发生，但算式必须 wrapping 不 panic）。
    let mut task2 = Task {
        born_frame: 5,
        ..Task::default()
    };
    w.body.frame = 0;
    assert!(call(&mut w, &ecl, &mut task2, SYS_SELF_AGE, &[]).is_ok());
    assert_eq!(task2.stack[0], 0i32.wrapping_sub(5), "wrapping 不 panic");
}

/// owner 上限血量 `SYS_SELF_HP_MAX`（M1.5）：ENEMY→`hp_max` 字段（与 `hp` 不同值，逐位命中
/// 排除"读混 hp/hp_max 两个同族字段"的变异）；非敌（STAGE/BULLET）→ 恒 0（同 `SYS_SELF_HP`
/// 误用策略）。
#[test]
fn sys_self_hp_max_dispatch_by_owner_kind() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let i = eh.index as usize;
    w.body.enemies.hp_max[i] = 9999; // 与 hp=5 明显不同，逐位命中排除读混字段
    let mut enemy_task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut enemy_task, SYS_SELF_HP_MAX, &[]).is_ok());
    assert_eq!(enemy_task.stack[0], 9999);

    let mut stage_task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut stage_task, SYS_SELF_HP_MAX, &[]).is_ok());
    assert_eq!(stage_task.stack[0], 0, "非敌 owner 恒 0");
}

/// `n==0` → 押 0、不消耗 RNG 流（拍板钉死：连续两次 n=0 调用后世界 RNG 状态与初始一致，
/// 通过"随后一次真实 rand_range 调用的结果与从未调用过 n=0 的另一世界一致"来判别）。
#[test]
fn sys_rand_range_zero_does_not_consume_rng() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[0]).is_ok());
    assert_eq!(task.stack[0], 0, "n=0 押 0");
    let after_zero_calls = w.body.rng;
    let mut w2 = World::new(1);
    assert_eq!(
        after_zero_calls, w2.body.rng,
        "n=0 不消耗 RNG 流——与全新同种子世界的 RNG 状态相同"
    );
    // 消耗判别式的另一半：真实调用（n>0）必须真的推进流，且与直接调 rand_range 序列一致。
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[100]).is_ok());
    let direct = w2.body.rng.rand_range(100) as i32;
    assert_eq!(
        task.stack[0], direct,
        "syscall 消费的 RNG 流与直接调用序列一致"
    );
}

/// B15 覆盖缺口补齐：负 `n` 与 `n==0` 走同一分支（`if n <= 0`），但从未有测试独立
/// 拿负数实测过——这里钉死同样的判别式（不消耗 RNG 流 + 押 0），防止未来有人把
/// 判别条件悄悄改成 `n == 0`（丢负数分支）而没有任何测试炸。
#[test]
fn sys_rand_range_negative_n_does_not_consume_rng() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_RAND_RANGE, &[-5]).is_ok());
    assert_eq!(task.stack[0], 0, "负 n 押 0，同 n=0");
    let after_negative_call = w.body.rng;
    let w2 = World::new(1);
    assert_eq!(
        after_negative_call, w2.body.rng,
        "负 n 不消耗 RNG 流——与全新同种子世界的 RNG 状态相同"
    );
}

/// self 位置朝向 P0 的角度 == 参考 `atan2` 计算（判别式：换公式即红）。
#[test]
fn sys_aim_player_angle_matches_atan2_reference() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 100, 100, 5);
    // 自机出场点 (0, 384)（`World::new` 既定）。
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
    let expect = crate::math::cordic::atan2(Fx::from_int(384 - 100), Fx::from_int(0 - 100));
    assert_eq!(task.stack[0] as u16 as u32, expect.raw() as u32);
}

/// 判别式（F8 瞄准口径统一）：`aim_player()` 取的是**最近的可瞄自机**，不是恒
/// `players[0]`。P0 已 GAMEOVER 冻在左上、P1 存活在右下——两个方向截然相反，取错
/// 人一眼可辨。摆位不可重合：圆心重合式的摆法对"瞄谁"这条映射是瞎的
/// （`CLAUDE.md` 点名的 M0-7 变异检验教训）。
#[test]
fn sys_aim_player_angle_skips_unaimable_player() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body.players[0].x = Fx::from_int(-200);
    w.body.players[0].y = Fx::from_int(-150);
    w.body.players[0].life_state = crate::player::LIFE_GAMEOVER;
    w.body.players[1] = crate::player::PlayerState::spawn(0, &TABLES_V0.characters[0]);
    w.body.players[1].x = Fx::from_int(200);
    w.body.players[1].y = Fx::from_int(150);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
    let to_p1 = crate::math::cordic::atan2(Fx::from_int(150), Fx::from_int(200));
    let to_p0 = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
    assert_eq!(
        task.stack[0] as u16 as u32,
        to_p1.raw() as u32,
        "应瞄唯一可瞄的 P1"
    );
    assert_ne!(
        task.stack[0] as u16 as u32,
        to_p0.raw() as u32,
        "瞄了已 GAMEOVER 的 P0 尸体坐标"
    );
}

/// 一个可瞄自机都没有时的 fallback 口径（F8 拍板，本条押的是"别改成别的"）：查询与
/// 发射这两条**必须产出一个角度**的路回退到 `players[0]` 的最后坐标——不是朝下的
/// 约定角、不是 0、更不是 no-op。no-op 是弹上 setter 那半边的处置，两半有意不同：
/// 查询必须给值，弹上的 setter 可以拒绝改动。
#[test]
fn sys_aim_player_angle_falls_back_to_p0_when_none_aimable() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    w.body.players[0].x = Fx::from_int(-200);
    w.body.players[0].y = Fx::from_int(-150);
    w.body.players[0].life_state = crate::player::LIFE_GAMEOVER; // players[1] 本就 ABSENT
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_AIM_PLAYER_ANGLE, &[]).is_ok());
    let to_corpse = crate::math::cordic::atan2(Fx::from_int(-150), Fx::from_int(-200));
    assert_eq!(
        task.stack[0] as u16 as u32,
        to_corpse.raw() as u32,
        "无可瞄自机时应回退到 P0 的最后坐标"
    );
}

// ── 数学/查询面（110/140/141；小清洗刀 2026-07-31）────────────────────────────────

/// **参数序判别腿**：`atan2(y, x)` 两位同为 `Fx`，对调不会有任何判型报错，只会静默
/// 把角度镜像到另一条对角线上。故必须**两个取值**一起断言——只测 `atan2(1,0)==16384`
/// 是抓不到"实现写成 `atan2(x, y)`"的（`atan2(0,1)` 那腿才把它钉死）。
#[test]
fn atan2_argument_order_is_y_then_x() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    let one = Fx::from_int(1).raw();
    assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[one, 0]).is_ok());
    assert_eq!(t.sp, 1, "atan2 押一个返回值");
    assert_eq!(t.stack[0], 16384, "atan2(y=1, x=0) = +90° = BAM 16384");
    t.sp = 0;
    assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[0, one]).is_ok());
    assert_eq!(t.stack[0], 0, "atan2(y=0, x=1) = 0° = BAM 0");
}

/// `(0,0)` 无 P4 分支——CORDIC 对原点有定义（0），不 Fault、不计违约。
#[test]
fn atan2_at_origin_is_zero_without_fault() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    let viol = w.body.diag.contract_viol;
    assert!(call(&mut w, &ecl, &mut t, SYS_ATAN2, &[0, 0]).is_ok());
    assert_eq!(t.stack[0], 0);
    assert_eq!(w.body.diag.contract_viol, viol, "原点不是违约");
}

/// **开根判别腿**：3-4-5 直角三角形。若实现漏了 `isqrt`（直接押 `len_sq`）或把
/// Q32.32 当 Q16.16 塞回去，这条立刻红——只断言"结果 > 0"是抓不到的
/// （`len_sq(3,4)` 也 > 0）。
#[test]
fn dist_is_the_square_root_of_the_squared_length() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_DIST,
            &[Fx::from_int(3).raw(), Fx::from_int(4).raw()]
        )
        .is_ok()
    );
    assert_eq!(t.sp, 1, "dist 押一个返回值");
    assert_eq!(
        t.stack[0],
        Fx::from_int(5).raw(),
        "dist(3.0fx, 4.0fx) 必须正好是 5.0fx"
    );
}

/// `dist` 是**向量模**（对称、恒非负）——负分量与正分量同结果。
#[test]
fn dist_of_negative_components_is_the_same_magnitude() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_DIST,
            &[Fx::from_int(-3).raw(), Fx::from_int(-4).raw()]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], Fx::from_int(5).raw());
}

/// 极端输入的收窄腿（复审 Minor）：`dist` 是**通用两参 syscall**，脚本能直接喂任意
/// `fx`——安全论证里那句"满屏最大 1.7e15"只覆盖世界坐标差这个域，覆盖不到字面量。
///
/// `dx = dy = i32::MAX`：`len_sq` = 2×(2147483647²) = 9223372028264841218
/// （**贴着 i64 上限 9223372036854775807 但不溢出**，故 debug 下 `len_sq` 自身不 panic），
/// `isqrt` = **3037000498** > `i32::MAX`(2147483647) ⇒ 裸 `as i32` 回绕成
/// **-1257966798**。断言必须钉"等于 `i32::MAX`"而**不是**"结果 ≥ 0"——后者对
/// "回绕后恰好落在正半区"的输入是瞎的。
#[test]
fn dist_saturates_instead_of_wrapping_negative() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    assert!(call(&mut w, &ecl, &mut t, SYS_DIST, &[i32::MAX, i32::MAX]).is_ok());
    assert_eq!(
        t.stack[0],
        i32::MAX,
        "开根超 i32 上限时饱和到最大可表示距离，不得回绕成负数"
    );
}

/// **"取最近"判别腿**：场上放**两只**不同距离的敌——只放一只的话，"取最近"与
/// "取第一个活着的"无法区分（同本仓「圆心重合式测试对半径映射是瞎的」那条推论）。
/// 更进一步：近的那只**池索引更大**，故"返回最低存活索引"这个变异也被钉死。
#[test]
fn nearest_enemy_returns_the_nearer_of_two_enemies() {
    let (mut w, ecl) = fresh();
    let far = crate::world::test_support::spawn_enemy(&mut w, 10, 0, 5);
    let near = crate::world::test_support::spawn_enemy(&mut w, 3, 0, 5);
    assert!(
        near.index > far.index,
        "近敌须是后建的（索引更大）才有判别力"
    );
    let mut t = Task::default();
    assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
    assert_eq!(t.sp, 1, "nearest_enemy 押一个返回值");
    assert_eq!(
        t.stack[0],
        crate::enemy::pack_handle(near),
        "查询点 (0,0) 附近的是近敌"
    );

    // 反过来查：从远敌那侧看，最近的换成远敌——防"恒返回某个固定槽"。
    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_NEAREST_ENEMY,
            &[Fx::from_int(20).raw(), 0]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], crate::enemy::pack_handle(far));
}

/// 空场 → -1（同 `enemy_hp` 的"查不到押 -1"口径，不 Fault）。
#[test]
fn nearest_enemy_on_empty_field_is_minus_one() {
    let (mut w, ecl) = fresh();
    let mut t = Task::default();
    assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
    assert_eq!(t.stack[0], -1);
}

/// **轴判别腿**：敌放在 **x ≠ y** 的位置（30, −70）分别断言——放 (5,5) 的话把两条
/// 派发臂写反是完全看不出来的（同「圆心重合式测试对半径映射是瞎的」那条推论）。
/// 负坐标那一半还顺带钉死"别把 `Fx` raw 当无符号搬"。
#[test]
fn enemy_x_and_enemy_y_read_their_own_axis() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let mut t = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_X,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.sp, 1, "enemy_x 押一个返回值");
    assert_eq!(
        t.stack[0],
        Fx::from_int(30).raw(),
        "enemy_x 读的是 x（30），不是 y（-70）"
    );

    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_Y,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(
        t.stack[0],
        Fx::from_int(-70).raw(),
        "enemy_y 读的是 y（-70），不是 x（30）"
    );
}

/// 无效句柄三腿（负 / 越界 / 死槽）一律 **0**（P4-b 纯读族降级，不 Fault、不计
/// `contract_viol`）。坐标没有 `enemy_hp` 那样的哨兵位可用（−1 是合法 `fx`），故取
/// `Fx::ZERO`——探活归脚本（`enemy_hp(e) != -1`），手册写死了这条惯例。
#[test]
fn enemy_pos_on_invalid_handle_degrades_to_zero() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let before = w.body.diag.contract_viol;
    let mut t = Task::default();

    for (no, name) in [(SYS_ENEMY_X, "enemy_x"), (SYS_ENEMY_Y, "enemy_y")] {
        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, no, &[-1]).is_ok());
        assert_eq!(t.stack[0], 0, "{name}：负句柄降级返 0");

        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, no, &[9999]).is_ok());
        assert_eq!(t.stack[0], 0, "{name}：越界句柄降级返 0");
    }

    w.body.enemies.free(h);
    for (no, name) in [(SYS_ENEMY_X, "enemy_x"), (SYS_ENEMY_Y, "enemy_y")] {
        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, no, &[crate::enemy::pack_handle(h)]).is_ok());
        assert_eq!(t.stack[0], 0, "{name}：死槽降级返 0");
    }
    assert_eq!(
        w.body.diag.contract_viol, before,
        "纯读族降级不计违约（同 enemy_hp 口径）"
    );
}

/// `ENEMY_DYING` 的敌**仍可读**——`is_alive` 是存活位，dying 只是 flag，槽要活到相位 9
/// 才回收。别顺手把 dying 也降级了，那与 `enemy_hp` 的既有读族口径不一致。
#[test]
fn enemy_pos_is_readable_while_dying() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    w.body.enemies.flags[h.index as usize] |= crate::enemy::ENEMY_DYING;
    let mut t = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_X,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], Fx::from_int(30).raw(), "dying 的敌坐标仍读得到");
    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_Y,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], Fx::from_int(-70).raw());
}

/// owner 类别无限制（同 `enemy_hp`/`nearest_enemy`）：STAGE 任务照样能读。
#[test]
fn enemy_pos_is_callable_from_a_stage_task() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let mut t = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_X,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], Fx::from_int(30).raw());
}

// ── 探活读口 103（enemy_alive；探活读口刀 2026-07-31）──────────────────────

/// 活敌返 **1**、三种无效句柄（负 / 越界 / 死槽）各返 **0**，且**不计 `contract_viol`**
/// （纯读族口径，同 `enemy_hp`/`sys_enemy_pos`）。
#[test]
fn enemy_alive_is_one_for_a_live_enemy_and_zero_for_invalid_handles() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let before = w.body.diag.contract_viol;
    let mut t = Task::default();

    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_ALIVE,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.sp, 1, "enemy_alive 押一个返回值");
    assert_eq!(t.stack[0], 1, "活敌返 1");

    t.sp = 0;
    assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[-1]).is_ok());
    assert_eq!(t.stack[0], 0, "负句柄返 0");

    t.sp = 0;
    assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[9999]).is_ok());
    assert_eq!(t.stack[0], 0, "越界句柄返 0");

    w.body.enemies.free(h);
    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_ALIVE,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], 0, "死槽返 0");

    assert_eq!(
        w.body.diag.contract_viol, before,
        "纯读族降级不计违约（同 enemy_hp 口径）"
    );
}

/// **判别腿①——本刀唯一的语义裁定**：`ENEMY_DYING` 的敌 `enemy_alive` 仍返 **1**。
/// 判的是「槽有效」而不是「还能打」：读族四条（`enemy_hp`/`enemy_x`/`enemy_y`/
/// `enemy_alive`）必须用**完全相同**的三判据——dying 的槽要活到相位 9（坐标仍读得到），
/// 四条里单独给一条换判据会让这组口径散掉。
///
/// "排除 dying"是最自然的错法（名字读起来就像"还能打"），而它**只有这条测试逮得住**：
/// dying 在正常路径上是个转瞬即逝的中间态，e2e 与其余各腿全都照绿。
/// 顺带钉住与 `enemy_x` 的同判：同一个 dying 句柄，两条口必须给出同一个"槽有效"结论。
#[test]
fn enemy_alive_stays_one_while_dying() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    w.body.enemies.flags[h.index as usize] |= crate::enemy::ENEMY_DYING;
    let mut t = Task::default();

    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_ALIVE,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(
        t.stack[0], 1,
        "判的是「槽有效」不是「还能打」——dying 的敌仍返 1"
    );

    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_X,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(
        t.stack[0],
        Fx::from_int(30).raw(),
        "坐标读口对 dying 同判（读族四条判据逐字一致）"
    );
}

/// **判别腿②——与 `enemy_hp` 的判据一致性**：同一个句柄，`enemy_alive == 1` ⟺
/// `enemy_hp` **不因"槽无效"**降级。三种无效（负 / 越界 / 死槽）各断言两者同步
/// （`0` ⟺ `-1`）。
///
/// 外加本 syscall **存在的全部理由**那一格：活敌血量**恰为 −1** 时（overkill 的敌 hp
/// 是真实负值，`settle::kill_enemy` 只 `min(0)` 不抹平）`enemy_hp` 返 −1，与降级值
/// 撞车——旧探针 `enemy_hp(e) != -1` 就在这一格误判，而 `enemy_alive` 照返 1。
/// 这一格红了就说明新口只是 `enemy_hp` 的花哨包装、白加一号。
#[test]
fn enemy_alive_agrees_with_enemy_hp_and_closes_the_minus_one_seam() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let mut t = Task::default();

    // ① 缝本身：活敌的 hp 恰好撞上降级哨兵 −1。
    w.body.enemies.hp[h.index as usize] = -1;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_HP,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], -1, "旧探针的盲区：活敌 hp 与降级值不可辨");
    t.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_ALIVE,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], 1, "槽有效 ⇒ 1（这一格是本刀存在的全部理由）");

    // ② 三种无效：两条口必须同步（alive=0 ⟺ hp 降级成 −1）。
    w.body.enemies.free(h);
    for (handle, name) in [
        (-1i32, "负句柄"),
        (9999, "越界"),
        (crate::enemy::pack_handle(h), "死槽"),
    ] {
        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_ALIVE, &[handle]).is_ok());
        let alive = t.stack[0];
        t.sp = 0;
        assert!(call(&mut w, &ecl, &mut t, SYS_ENEMY_HP, &[handle]).is_ok());
        let hp = t.stack[0];
        assert_eq!((alive, hp), (0, -1), "{name}：两条口同步降级");
    }
}

/// owner 类别无限制（同 `enemy_hp`/`enemy_x`/`nearest_enemy`）：STAGE 任务照样能探。
#[test]
fn enemy_alive_is_callable_from_a_stage_task() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 30, -70, 5);
    let mut t = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(
        call(
            &mut w,
            &ecl,
            &mut t,
            SYS_ENEMY_ALIVE,
            &[crate::enemy::pack_handle(h)]
        )
        .is_ok()
    );
    assert_eq!(t.stack[0], 1);
}

/// owner 类别无限制：STAGE 任务（关卡编排）照样能查。
#[test]
fn nearest_enemy_is_callable_from_a_stage_task() {
    let (mut w, ecl) = fresh();
    let h = crate::world::test_support::spawn_enemy(&mut w, 5, 5, 3);
    let mut t = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
    assert_eq!(t.stack[0], crate::enemy::pack_handle(h));
}

// ── 敌句柄打包 generation（敌句柄打包刀 2026-07-31）─────────────────────────

/// 用 `spawn_enemy`(210) 造敌并取**脚本视角**的敌号——即 syscall 真正押出去的那个值。
/// 本族测试必须走 syscall 边界：被测的就是那个边界上的编码，绕过它（直接拿
/// `test_support::spawn_enemy` 的 `EnemyHandle`）就把要证的东西假设掉了。
fn spawn_enemy_via_syscall(w: &mut World, ecl: &EclImage, x: i32, y: i32, hp: i32) -> i32 {
    let mut t = Task::default();
    // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)
    let args = [
        Fx::from_int(x).raw(),
        Fx::from_int(y).raw(),
        hp,
        0,
        100,
        0,
        -1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(w, ecl, &mut t, SYS_SPAWN_ENEMY, &args).is_ok());
    t.stack[0]
}

/// 单参读口的一次调用（新 `Task`，返回押回的那一个值）。
fn read_enemy_port(w: &mut World, ecl: &EclImage, no: u16, handle: i32) -> i32 {
    let mut t = Task::default();
    assert!(call(w, ecl, &mut t, no, &[handle]).is_ok());
    assert_eq!(t.sp, 1, "读口押且只押一个返回值");
    t.stack[0]
}

/// **这刀存在的全部理由 —— ABA**：敌 A 死、槽被 cleanup 回收、敌 B 落进**同一个槽**
/// 之后，A 的**旧句柄**必须读到降级值，而不是静默变成 B 的号读到 B 的数据。
///
/// 改动前这条是红的（敌号是裸池 index，A/B 同槽 ⇒ 两者的号逐位相同），红的正是
/// `enemy_hp` 读到 B 的血那一格——问题真实存在的实证。
///
/// 判别力靠三件事：① B 必须落进 A 的**同一个槽**（分配器取最低空位，故先断言 A 占
/// 0 号槽、回收后最低空位仍是 0），否则整条测试什么也没证明；② A/B 的坐标与血量
/// **全部不同**，读到 B 必然可辨；③ 末尾用 B 的**新**句柄再读一遍四个口——防
/// "resolve 恒失败、全都读不到"那种假绿。
#[test]
fn a_stale_enemy_handle_does_not_read_the_enemy_that_took_its_slot() {
    let (mut w, ecl) = fresh();

    // A：空池 ⇒ 落最低空位（0 号槽）。
    let a = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
    let a_idx = (a & 0xFFFF) as usize;
    assert_eq!(a_idx, 0, "前提：A 占最低空位（否则 B 未必落回同一个槽）");

    // A 死 + 槽被相位 9 回收（走完整 step，不手 `free`——回收纪律本身也在链路里）。
    w.body.enemies.flags[a_idx] |= crate::enemy::ENEMY_DYING;
    let frame = w.body.frame;
    crate::step::step(
        &mut w,
        &TABLES_V0,
        &ecl,
        &crate::input::InputFrame::empty(frame),
    );
    assert!(
        !w.body.enemies.is_alive(a_idx),
        "前提：A 的槽已被 cleanup 回收"
    );

    // B：最低空位仍是 0 ⇒ 落进 A 的旧槽。坐标/血量与 A 全不同。
    let b = spawn_enemy_via_syscall(&mut w, &ecl, 55, 66, 123);
    assert_eq!(
        (b & 0xFFFF) as usize,
        a_idx,
        "前提：B 必须落进 A 的旧槽，否则本测试什么都没证明"
    );

    // 用 A 的**旧句柄**读四个口：全部降级，不得读到 B 的任何一个字段。
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, a),
        -1,
        "旧句柄的 enemy_hp 必须降级 —— 读到 123 就是读到了 B 的血（ABA）"
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, a),
        0,
        "旧句柄的 enemy_x 必须降级 —— 读到 55 就是 B 的 x"
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, a),
        0,
        "旧句柄的 enemy_y 必须降级 —— 读到 66 就是 B 的 y"
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, a),
        0,
        "旧句柄的 enemy_alive 必须返 0 —— A 已经不在了"
    );

    assert_ne!(
        b, a,
        "同槽而不同敌 ⇒ 两个句柄必须可辨（generation 就是干这个的）"
    );

    // 反向腿（防"全都读不到"的假绿）：B 的**新**句柄一切正常。
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, b), 123);
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, b),
        Fx::from_int(55).raw()
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, b),
        Fx::from_int(66).raw()
    );
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, b), 1);

    // `nearest_enemy` 押的必须是**同一套编码**（配对使用：拿号 → 轮询）。
    let mut t = Task::default();
    assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
    assert_eq!(
        t.stack[0], b,
        "nearest_enemy 与 spawn_enemy 同口径（含 gen）"
    );
}

/// **掩码腿**：`generation` 是完整 `u16` 而句柄只带低 15 位，故比对两边都要
/// `& 0x7FFF`。gen 高位置位（`0xF00D`）的敌，其句柄必须 ① 仍然**非负**
/// （否则 `-1` 哨兵不再唯一）、② 喂回四个读口全部照常工作。
///
/// 漏掉掩码的实现在这里红：`generation[idx] == g` 会拿 `0xF00D` 比 `0x700D`，
/// 于是"敌活着但所有读口都说它没了"——正常路径上要跑 32768 次同槽复用才撞得到，
/// 只有这条测试逮得住。
#[test]
fn packed_handle_stays_non_negative_and_resolves_with_a_high_generation() {
    let (mut w, ecl) = fresh();
    // alloc 会把槽的 gen +1，故预置 0xF00C ⇒ 敌的 generation = 0xF00D（最高位置位）。
    w.body.enemies.generation[0] = 0xF00C;
    let h = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
    assert_eq!(
        w.body.enemies.generation[0], 0xF00D,
        "前提：这只敌的 generation 最高位已置位"
    );

    assert!(
        h >= 0,
        "打包值必须恒非负 —— 只押 gen 的低 15 位就是为了这个"
    );
    assert_eq!((h & 0xFFFF) as usize, 0, "低 16 位仍是池 index");
    assert_eq!(
        (h >> 16) & 0x7FFF,
        0xF00D & 0x7FFF,
        "高位押的是 generation 的低 15 位"
    );

    // 往返：打包出来的句柄喂回四个读口全部工作（比对侧漏掩码则四条全红）。
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, h), 77);
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, h),
        Fx::from_int(30).raw()
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, h),
        Fx::from_int(-70).raw()
    );
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, h), 1);
}

/// 正常路径不变：`spawn_enemy` 押的号直接喂四个读口全部工作（打包不该把"刚建好的敌
/// 读不到"当代价）。顺带钉住 `nearest_enemy` 返的就是同一个值。
#[test]
fn freshly_spawned_enemy_handle_feeds_all_four_read_ports() {
    let (mut w, ecl) = fresh();
    let h = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
    assert!(h >= 0, "敌应建成");
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, h), 77);
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, h),
        Fx::from_int(30).raw()
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, h),
        Fx::from_int(-70).raw()
    );
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, h), 1);

    let mut t = Task::default();
    assert!(call(&mut w, &ecl, &mut t, SYS_NEAREST_ENEMY, &[0, 0]).is_ok());
    assert_eq!(t.stack[0], h, "两条产号口必须同编码");
}

/// `-1` 仍是四个读口眼里的唯一无效哨兵——打包后**不得**有哪个 `-1` 意外解包成合法槽。
///
/// ⚠️ 注意这条**打不中** `resolve_enemy_handle` 的 `packed >= 0` 那道闸：`-1` 的低 16 位
/// 是 `0xFFFF` = 65535，越界判据自己就兜住了。专打非负闸的判别腿见下一条。
#[test]
fn minus_one_is_still_invalid_for_every_read_port() {
    let (mut w, ecl) = fresh();
    // 场上放一只活敌：空场的话"恒降级"的实现也会绿。
    let live = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, live), 1);

    let before = w.body.diag.contract_viol;
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, -1), -1);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, -1), 0);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, -1), 0);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, -1), 0);
    assert_eq!(
        w.body.diag.contract_viol, before,
        "纯读族降级仍不计违约（口径未变）"
    );
}

/// **非负闸的判别腿**（复审 ②）：`resolve_enemy_handle` 的 `packed >= 0` 那道闸此前
/// 是**被越界判据遮住的**——常见负值（`-1`）的低 16 位是 `0xFFFF` = 65535 ≥ CAP(256)，
/// 删掉非负闸测试照绿。今天没事，但将来谁放宽越界判据或扩了敌池容量，洞就露出来。
///
/// 能单独打中它的取值是 **`-65536`**（`0xFFFF0000`）：低 16 位 = `0`（合法槽），
/// `(p >> 16) & 0x7FFF` = `0x7FFF`（算术右移把符号位铺满）。拿它去打一只
/// `generation & 0x7FFF == 0x7FFF` 的**活敌**——index 合法、gen 也对得上，
/// **只有非负闸能拒绝它**。
///
/// 反向腿钉住这不是"什么都读不到"：同一只敌的**正**句柄 `0x7FFF0000` 必须照常读通。
#[test]
fn a_negative_handle_whose_low_bits_alias_a_live_enemy_is_still_rejected() {
    let (mut w, ecl) = fresh();
    // alloc 会 +1，故预置 0x7FFE ⇒ 这只敌的 generation = 0x7FFF（低 15 位全 1）。
    w.body.enemies.generation[0] = 0x7FFE;
    let good = spawn_enemy_via_syscall(&mut w, &ecl, 30, -70, 77);
    assert_eq!(
        w.body.enemies.generation[0], 0x7FFF,
        "前提：gen 的低 15 位必须全 1，否则 -65536 解出来的 gen 对不上、本腿失去判别力"
    );
    assert_eq!(good, 0x7FFF_0000, "前提：正句柄就是 -65536 的非负孪生");

    // 反向腿：正句柄照常读通（否则下面四条退化成"什么都读不到"的假绿）。
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, good), 77);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, good), 1);

    // 正题：同 index、同 gen，只差一个符号位 —— 必须被非负闸拒掉。
    const ALIAS: i32 = -65536; // 0xFFFF0000
    assert_eq!(ALIAS & 0xFFFF, 0, "低 16 位确实指向 0 号槽（活敌）");
    assert_eq!(
        (ALIAS >> 16) & 0x7FFF,
        0x7FFF,
        "解出的 gen 确实与那只敌相符"
    );
    assert_eq!(
        read_enemy_port(&mut w, &ecl, SYS_ENEMY_HP, ALIAS),
        -1,
        "负句柄必须被拒 —— 删掉 `packed >= 0` 这条就红"
    );
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_X, ALIAS), 0);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_Y, ALIAS), 0);
    assert_eq!(read_enemy_port(&mut w, &ecl, SYS_ENEMY_ALIVE, ALIAS), 0);
}

/// 026 `$self_enemy`：敌 owner 押打包敌号；非敌押 -1（打包值 0 合法，故不读 0）。
#[test]
fn self_enemy_pushes_packed_handle_or_minus_one() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
    assert!(call(&mut w, &ecl, &mut task, SYS_SELF_ENEMY, &[]).is_ok());
    assert_eq!(task.stack[0], crate::enemy::pack_handle(eh));
    let mut stage = Task::default();
    assert!(call(&mut w, &ecl, &mut stage, SYS_SELF_ENEMY, &[]).is_ok());
    assert_eq!(stage.stack[0], -1);
}
