//! 弹幕创建、外观、变换与清弹。

use super::*;

/// 丙方案 create_bullet：dumb 路径（xform_cnt=0）——位置精确、appearance 半径/逻辑编号
/// 逐位查表命中。
#[test]
fn sys_create_bullet_dumb_path_position_and_appearance_bitwise() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // 正序：appearance,x,y,speed,angle,xform_off,xform_cnt,task_script
    let args = [
        ROW_B,
        Fx::from_int(10).raw(),
        Fx::from_int(-20).raw(),
        Fx::from_int(3).raw(),
        Angle::QUARTER.raw() as i32,
        0,
        0,
        -1,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
    let idx = task.stack[0];
    assert!(idx >= 0, "创建成功压回句柄 index");
    let i = idx as usize;
    assert_eq!(w.body.bullets.x[i], Fx::from_int(10));
    assert_eq!(w.body.bullets.y[i], Fx::from_int(-20));
    assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));
    assert_eq!(w.body.bullets.angle[i], Angle::QUARTER);
    let cfg = &TABLES_V0.appearances[ROW_B as usize];
    assert_eq!(
        w.body.bullets.radius[i], cfg.radius,
        "appearance 半径逐位命中"
    );
    assert_eq!(
        w.body.bullets.sprite[i], ROW_B as u16,
        "appearance 逻辑编号逐位命中"
    );
    assert_eq!(
        w.body.bullets.transform_head[i],
        crate::xform::XFORM_NONE,
        "xform_cnt=0 → 哑弹"
    );
}

#[test]
fn sys_create_bullet_appearance_oob_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let bad_appearance = TABLES_V0.appearances.len() as i32; // 恰过界
    let args = [bad_appearance, 0, 0, 0, 0, 0, 0, -1];
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(
        w.body.bullets.iter_alive().count(),
        0,
        "校验先于创建，零副作用"
    );
}

/// 空格 appearance（图集该格没有图）→ Fault，且**弹未被创建**（先验后建）。
/// 这是"隐形弹"（有判定无图像）的运行期闸；编译期同款判据见 lang::typeck。
#[test]
fn sys_create_bullet_blank_cell_faults_without_creating() {
    const BLANK: i32 = 3 * 16 + 7; // 合成表里挖的那一格
    let holed = tables_with_hole(BLANK as usize);
    assert!(
        !holed.appearances[BLANK as usize].valid && TABLES_V0.appearances[BLANK as usize].valid,
        "前提：{BLANK} 在合成表里是空格、在内建表里不是——否则本测试无判别力"
    );
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = [BLANK, 0, 0, 0, 0, 0, 0, -1];
    let r = call_with_tables(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args, &holed);
    assert_eq!(r, Err(FAULT_BAD_OP), "空格 appearance 必须 Fault");
    assert_eq!(
        w.body.bullets.iter_alive().count(),
        0,
        "Fault 时不得留下半成品弹（先验后建）"
    );
}

/// 物理贴图可共用；出生时仍保留逻辑形色与该弹型的判定半径。
#[test]
fn firing_paths_preserve_logical_appearance_with_shared_sprite() {
    let mut tables = crate::tables::build_tables_v0();
    for a in &mut tables.appearances[16..32] {
        a.sprite = 5;
        a.radius = Fx::from_int(2);
    }
    for a in &mut tables.appearances[32..48] {
        a.sprite = 5;
        a.radius = Fx::from_int(7);
    }
    for syscall in [SYS_CREATE_BULLET, SYS_CREATE_BULLETS_BATCH, SYS_SH_FIRE] {
        let (mut w, ecl) = fresh_with_shooters();
        for appearance in [21, 37] {
            let mut task = Task::default();
            let args = match syscall {
                SYS_CREATE_BULLET => vec![appearance, 0, 0, 0, 0, 0, 0, -1],
                SYS_CREATE_BULLETS_BATCH => vec![appearance, 0, 0, 1, 0, 0, 1, 0, 0],
                SYS_SH_FIRE => {
                    call_with_tables(
                        &mut w,
                        &ecl,
                        &mut task,
                        SYS_SH_SPRITE,
                        &[0, appearance],
                        &tables,
                    )
                    .unwrap();
                    vec![0]
                }
                _ => unreachable!(),
            };
            call_with_tables(&mut w, &ecl, &mut task, syscall, &args, &tables).unwrap();
        }
        let ids: Vec<_> = w.body.bullets.iter_alive().collect();
        assert_eq!(ids.len(), 2, "syscall {syscall}");
        assert_eq!(w.body.bullets.sprite[ids[0]], 21, "syscall {syscall}");
        assert_eq!(w.body.bullets.sprite[ids[1]], 37, "syscall {syscall}");
        assert_eq!(w.body.bullets.radius[ids[0]], Fx::from_int(2));
        assert_eq!(w.body.bullets.radius[ids[1]], Fx::from_int(7));
    }
}

/// 换色、换形、完整换图都在逻辑域运算，并保留出生时半径。
#[test]
fn appearance_xforms_preserve_logical_axes_and_birth_radius() {
    use crate::xform::{OP_SET_COLOR, OP_SET_SHAPE, OP_SET_SPRITE};
    let mut tables = crate::tables::build_tables_v0();
    tables.appearances[37].sprite = 5;
    tables.appearances[37].radius = Fx::from_int(7);
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    for (k, (op, arg, stride)) in [
        (OP_SET_COLOR, 9, 16),
        (OP_SET_SHAPE, 48, 16),
        (OP_SET_SPRITE, 18, 0),
    ]
    .into_iter()
    .enumerate()
    {
        task.locals[k * 3] = ((1u32 << 16) | ((op as u32) << 8)) as i32;
        task.locals[k * 3 + 1] = arg;
        task.locals[k * 3 + 2] = stride;
    }
    call_with_tables(
        &mut w,
        &ecl,
        &mut task,
        SYS_CREATE_BULLET,
        &[37, 0, 0, 0, 0, 0, 3, -1],
        &tables,
    )
    .unwrap();
    let i = w.body.bullets.iter_alive().next().unwrap();
    for expected in [41, 57, 18] {
        #[cfg(debug_assertions)]
        {
            w.body.phase_guard = crate::world::PH_XFORM;
        }
        w.body.run_transforms(ecl.xforms(), &tables);
        assert_eq!(w.body.bullets.sprite[i], expected);
        assert_eq!(w.body.bullets.radius[i], Fx::from_int(7));
    }
}

/// xform 区间引用解包逐位：locals 手填 3 词槽（word0=(wait<<16)|(op<<8)，word1/2=args），
/// 创建后段内容与手搭 `XformSlot` 逐位相等。
#[test]
fn sys_create_bullet_xform_unpack_bitwise() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // 槽 0：wait=3, op=OP_SET_SPEED(10), args=[Fx::from_int(2).raw(), 0]
    let off = 4usize;
    task.locals[off] = ((3u32 << 16) | (crate::xform::OP_SET_SPEED as u32) << 8) as i32;
    task.locals[off + 1] = Fx::from_int(2).raw();
    task.locals[off + 2] = 0;
    let args = [
        ROW_A, 0, 0, 0, 0, off as i32, 1, // xform_cnt=1
        -1,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
    let idx = task.stack[0] as usize;
    let seg = w.body.bullets.transform_head[idx];
    assert_ne!(seg, crate::xform::XFORM_NONE, "xform_cnt>0 → 带段");
    let expect = XformSlot {
        wait: 3,
        op: crate::xform::OP_SET_SPEED,
        _pad: 0,
        args: [Fx::from_int(2).raw(), 0],
    };
    assert_eq!(w.body.xforms.seg_slots(seg)[0], expect, "解包逐位相等");
}

/// off+cnt*3 越出 LOCALS(64) → Fault，零副作用（不建弹）。
#[test]
fn sys_create_bullet_xform_bounds_violation_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = [
        ROW_A, 0, 0, 0, 0, 60, // off
        2,  // cnt：60+2*3=66 > LOCALS(64)
        -1,
    ];
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.bullets.iter_alive().count(), 0);
}

/// 坏 task_script 号（不在册）→ Fault，且先查后建：弹不应被创建（零副作用）。
#[test]
fn sys_create_bullet_bad_task_script_faults_before_creating() {
    let (mut w, ecl) = fresh(); // subs 空——任何脚本号都越界
    let mut task = Task::default();
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, 0]; // task_script=0 越界
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
}

/// P4-b 支路①（B25）：`fire`（`sys_create_bullet`）的 task_script 超出 `u16` 范围
/// （`u16::try_from` 失败）→ Fault，且先查后建：弹不应被创建（零副作用）。
/// 镜像 `spawn_enemy_task_script_out_of_u16_faults_without_enemy`。
///
/// **取值刻意挑过**（变异检验陷阱，见 task-3 报告）：`65537 as u16 == 1`，而
/// `1` 在 `async_pattern_image()` 里恰是一个合法在册的零参 Async sub——若把
/// `u16::try_from` 检查误删成 `as u16` 截断，这条参数会"活下来"变成合法号，
/// 从而真的建成弹（支路②③都拦不住它）。用一个恰好越界又恰好在册的取值，才能
/// 让"删掉①检查"这个变异被**这条测试**真正捕捉，而不是被②顺手挡住。
#[test]
fn sys_create_bullet_task_script_out_of_u16_faults_before_creating() {
    let ecl = async_pattern_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    // 正序：appearance,x,y,speed,angle,xform_off,xform_cnt,task_script
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, 65537]; // 65537 > u16::MAX，但截断后=1（在册）
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
}

/// P4-b 支路③（B25）：`fire` 的 task_script 号在册但不是零参 `Async`（这里用
/// `CallOnly`）→ Fault，且先查后建：弹不应被创建（零副作用）。镜像
/// `spawn_enemy_task_script_wrong_kind_faults_without_enemy`。
#[test]
fn sys_create_bullet_task_script_wrong_kind_faults_before_creating() {
    let ecl = wrong_kind_task_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1，在册但 CallOnly
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
}

/// P4-b 支路③另一半（终审 M-4）：`fire` 的 task_script 号在册、`kind()==Async`，
/// 唯独**带参**（非零参）→ 同样必须 Fault，且先查后建（弹不应被创建）。
/// 与 `sys_create_bullet_task_script_wrong_kind_faults_before_creating` 互补——
/// 那条打 kind 半边，这条打 param_types 半边，合起来才覆盖判据的完整析取。
/// 镜像 `spawn_enemy_task_script_with_params_faults_without_enemy`。
#[test]
fn sys_create_bullet_task_script_with_params_faults_before_creating() {
    let ecl = async_with_param_task_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1，在册、Async，但带参
    let r = call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.bullets.iter_alive().count(), 0, "先查后建：零副作用");
}

/// 池满 → 押 -1，不 Fault，不派任务（NULL 分支）。
#[test]
fn sys_create_bullet_pool_full_pushes_neg1() {
    let (mut w, ecl) = fresh();
    for _ in 0..crate::bullets::BulletPool::CAP {
        w.body.create_bullet(crate::bullets::BulletInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            vx: Fx::ZERO,
            vy: Fx::ZERO,
            speed: Fx::ZERO,
            angle: Angle::ZERO,
            ang_vel: 0,
            accel: Fx::ZERO,
            ax: Fx::ZERO,
            ay: Fx::ZERO,
            sprite: 0,
            radius: Fx::from_int(2),
            delay: 0,
            life: 0xFFFF,
            flags: 0,
            grazed_by: 0,
            transform_head: 0xFFFF,
            xform_wait: 0,
            xform_next: 0,
            born_frame: 0,
        });
    }
    let mut task = Task::default();
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, -1];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
    assert_eq!(task.stack[0], -1, "池满押 -1");
}

/// task_script ≥ 0 挂弹派任务：owner=(BULLET, 新弹 index/gen)，`parent`/`born_frame` 戳记
/// 与 `OP_SPAWN` 同款；弹死后任务次帧被静默回收（owner 门禁，非 Fault）。
#[test]
fn sys_create_bullet_task_script_spawns_owner_bound_task_and_dies_with_bullet() {
    let ecl = test_image(
        vec![
            crate::ecl::ops::OP_PUSHI as u32,
            999,
            crate::ecl::ops::OP_WAIT as u32,
        ],
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(0, SubKind::Async, vec![]),
        ],
        vec![EntryInit::new("bullet_task", 1)],
        Some(0),
    );
    let mut w = World::new(1);
    w.body.frame = 5;
    let mut task = Task::default();
    let args = [ROW_A, 0, 0, 0, 0, 0, 0, 1]; // task_script=1（在册）
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
    let bidx = task.stack[0] as u16;
    let bgen = w.body.bullets.generation[bidx as usize];

    // 恰新增一个任务，owner 绑定新弹，born_frame 戳为 ctx.frame（=5，次帧首跑）。
    let child = (0..crate::ecl::task::TASK_CAP)
        .find(|&i| w.tasks.is_alive(i))
        .expect("应已派生子任务");
    assert_eq!(w.tasks.slots[child].owner_kind, OWNER_BULLET);
    assert_eq!(w.tasks.slots[child].owner_index, bidx);
    assert_eq!(w.tasks.slots[child].owner_gen, bgen);
    assert_eq!(w.tasks.slots[child].born_frame, 5);

    // 弹死 → 次帧任务被静默回收（owner 门禁，通过完整 step 驱动验证端到端链路）。
    w.body.bullets.free(crate::bullets::BulletHandle {
        index: bidx,
        generation: bgen,
    });
    let frame = w.body.frame;
    crate::step::step(
        &mut w,
        &TABLES_V0,
        &ecl,
        &crate::input::InputFrame::empty(frame),
    );
    assert!(!w.tasks.is_alive(child), "owner 弹死后任务应被静默回收");
    assert_eq!(w.body.diag.task_faults, 0, "owner 死不是 Fault");
}

#[test]
fn sys_create_bullets_batch_ring_count_and_appearance() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
    let args = [
        ROW_C,
        0,
        Fx::from_int(100).raw(),
        8,
        0,
        8192,
        1,
        Fx::from_int(2).raw(),
        0,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
    assert_eq!(task.stack[0], 8, "8-way 环实发 8");
    assert_eq!(w.body.bullets.iter_alive().count(), 8);
    let cfg = &TABLES_V0.appearances[ROW_C as usize];
    for i in 0..8 {
        assert_eq!(w.body.bullets.radius[i], cfg.radius);
        assert_eq!(w.body.bullets.sprite[i], ROW_C as u16);
    }
}

/// 批量入口同款（两族同构，别只补一边）：空格 appearance → Fault，且**弹未被创建**
/// （先验后建）；编译期同款判据见 lang::typeck。
#[test]
fn sys_create_bullets_batch_blank_cell_faults_without_creating() {
    const BLANK: i32 = 3 * 16 + 7; // 同上,合成表
    let holed = tables_with_hole(BLANK as usize);
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
    let args = [BLANK, 0, 0, 4, 0, 4096, 1, Fx::from_int(1).raw(), 0];
    let r = call_with_tables(
        &mut w,
        &ecl,
        &mut task,
        SYS_CREATE_BULLETS_BATCH,
        &args,
        &holed,
    );
    assert_eq!(r, Err(FAULT_BAD_OP), "空格 appearance 必须 Fault");
    assert_eq!(
        w.body.bullets.iter_alive().count(),
        0,
        "Fault 时不得留下半成品弹（先验后建）"
    );
}

/// 弹 setter 族快乐路径抽查（self owner=BULLET）：speed/vel/ang_vel/gravity/stop_fx。
#[test]
fn sys_bullet_setter_family_happy_path() {
    let (mut w, ecl) = fresh();
    let bh = crate::world::test_support::bullet_at(&mut w, 0, 100);
    let mut task = Task {
        owner_kind: OWNER_BULLET,
        owner_index: bh.index,
        owner_gen: bh.generation,
        ..Task::default()
    };
    let i = bh.index as usize;

    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_BULLET_SPEED,
            &[Fx::from_int(3).raw()]
        )
        .is_ok()
    );
    assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));

    task.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_BULLET_VEL,
            &[Fx::from_int(1).raw(), Fx::from_int(2).raw()]
        )
        .is_ok(),
        "正序 vx,vy"
    );
    assert_eq!(w.body.bullets.vx[i], Fx::from_int(1));
    assert_eq!(w.body.bullets.vy[i], Fx::from_int(2));

    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_BULLET_ANG_VEL, &[300]).is_ok());
    assert_eq!(w.body.bullets.ang_vel[i], 300);

    task.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_BULLET_GRAVITY,
            &[Fx::from_raw(11).raw(), Fx::from_raw(22).raw()]
        )
        .is_ok(),
        "正序 ax,ay"
    );
    assert_eq!(w.body.bullets.ax[i].raw(), 11);
    assert_eq!(w.body.bullets.ay[i].raw(), 22);

    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_STOP_BULLET_FX, &[]).is_ok());
    assert_eq!(
        w.body.bullets.flags[i]
            & (crate::bullets::BULLET_POLAR_FX | crate::bullets::BULLET_CART_FX),
        0
    );
}

/// 弹 setter 族误用策略：self owner != BULLET → Fault（响亮报错，非静默 no-op）。
/// **B15 覆盖缺口补齐**：九支 dispatch 分支各自手写内联 `self_bullet_handle(task)?`
/// （非共享 loop/宏），此前只有 `SYS_SET_BULLET_SPEED` 一支独立实测过——其余 8 支
/// 理论上"该有这一句"，但没有测试证明真的有；有人加新 setter 或手滑漏抄这一句都
/// 不会被任何测试发现。一刀补齐全九支：`self_bullet_handle` 在摸任何参数前就返回
/// Fault，故 `&[]` 空参足够触发，不依赖各 syscall 各自的真实 arity。
#[test]
fn sys_bullet_setter_wrong_owner_faults() {
    const ALL_NINE: [(u16, &str); 9] = [
        (SYS_SET_BULLET_SPEED, "set_speed"),
        (SYS_SET_BULLET_ANGLE, "set_angle"),
        (SYS_TURN_BULLET, "turn"),
        (SYS_SET_BULLET_VEL, "set_vel"),
        (SYS_SET_BULLET_ANG_VEL, "set_ang_vel"),
        (SYS_SET_BULLET_ACCEL, "set_accel"),
        (SYS_SET_BULLET_GRAVITY, "set_gravity"),
        (SYS_STOP_BULLET_FX, "stop_fx"),
        (SYS_AIM_BULLET_AT_PLAYER, "aim_at_player"),
    ];
    for (sys, name) in ALL_NINE {
        let (mut w, ecl) = fresh();
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            ..Task::default()
        };
        let r = call(&mut w, &ecl, &mut task, sys, &[]);
        assert_eq!(
            r,
            Err(FAULT_BAD_OP),
            "'{name}'（sys={sys}）坏 owner 应 Fault"
        );
    }
}

// ── SYS_CLEAR_BULLETS（B19；整局流程刀 Task 2）──────────────────────────

/// B19：`clear_bullets()` 铺一个覆盖全场、存活 1 帧的消弹区。
/// **消弹转星星是白送的**（M0-15：settle 趟一对每颗被消的弹原位转一颗星星）,
/// 故断言"弹没了"**和**"星星出现了"——后者是"真走了 FieldPool 那条路"的判别腿
/// （若实现者绕开 field、自己写个循环把弹 free 掉，星星那条立刻红）。
#[test]
fn clear_bullets_lays_fullscreen_field_and_converts_to_stars() {
    use crate::bullets::BULLET_CLEARED;
    use crate::events::EVT_FIELD_CLEARED;
    use crate::items::ITEM_STAR;
    #[cfg(debug_assertions)]
    use crate::world::PH_COLLIDE;
    use crate::world::test_support::bullet_at;

    let (mut w, ecl) = fresh();
    // 三颗弹散在场内不同位置（场界 x∈[-192,192]、y∈[0,448]），全部落在
    // FIELD_RADIUS_FULLSCREEN 的覆盖范围内。
    bullet_at(&mut w, -100, 50);
    bullet_at(&mut w, 100, 400);
    bullet_at(&mut w, 0, 224);

    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_CLEAR_BULLETS, &[]).is_ok());

    // 手动驱动到相位 6/7：field 是 life=1，相位5 减到 0、相位6 alive 位仍在
    // 照常判定（field.rs 模块文档），故 collide 仍能吃到它。
    #[cfg(debug_assertions)]
    {
        w.body.phase_guard = PH_COLLIDE;
    }
    w.body.collide(&TABLES_V0);
    w.body.settle(&TABLES_V0);

    assert_ne!(w.body.bullets.flags[0] & BULLET_CLEARED, 0, "弹0 应被消");
    assert_ne!(w.body.bullets.flags[1] & BULLET_CLEARED, 0, "弹1 应被消");
    assert_ne!(w.body.bullets.flags[2] & BULLET_CLEARED, 0, "弹2 应被消");

    // 判别腿：星星必须出现（真走了 FieldPool），不是"弹没了"就算数。
    assert_eq!(
        w.body.items.iter_alive().count(),
        3,
        "每颗被消的弹应原位转一颗星星"
    );
    for i in 0..3 {
        assert_eq!(w.body.items.item_type[i], ITEM_STAR, "槽 {i} 应为星星");
    }

    assert_eq!(w.body.frame_events_len, 1, "field 消弹应聚合发一条事件");
    assert_eq!(w.body.frame_events[0].kind, EVT_FIELD_CLEARED);
    assert_eq!(w.body.frame_events[0].data[0], 3, "data[0] == 弹数");
}

/// P4-a：field 池满 → 确定性降级（不 panic、不 Fault，计 pool_full[POOL_FIELD]）。
#[test]
fn clear_bullets_field_pool_full_degrades() {
    use crate::field::{FieldHandle, FieldInit, FieldPool};
    use crate::world::POOL_FIELD;

    let (mut w, ecl) = fresh();
    for _ in 0..FieldPool::CAP {
        let h = w.body.create_field(FieldInit {
            x: Fx::ZERO,
            y: Fx::ZERO,
            radius: Fx::from_int(10),
            dmg_per_frame: 0,
            life: 1,
            owner: 0,
            flags: 0,
        });
        assert_ne!(h, FieldHandle::NULL, "灌池阶段不该失败");
    }
    let before = w.body.diag.pool_full[POOL_FIELD];

    let mut task = Task::default();
    let result = call(&mut w, &ecl, &mut task, SYS_CLEAR_BULLETS, &[]);

    assert!(result.is_ok(), "池满降级不应 Fault");
    assert_eq!(w.body.diag.pool_full[POOL_FIELD], before + 1, "池满须计数");
}

/// 弹的三条创建路径都盖 `born_frame = 创建帧`（spec §3.1）：fire(200) / batch(201) /
/// sh_fire(660)。帧号取 42 而非 0——零初始化的槽也是 0，0 无判别力。
#[test]
fn all_three_bullet_creation_paths_stamp_born_frame() {
    let (mut w, ecl) = fresh();
    w.body.frame = 42;
    // fire
    let mut task = Task::default();
    let args = [ROW_B, 0, 0, Fx::from_int(1).raw(), 0, 0, 0, -1];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
    let i = task.stack[0] as usize;
    assert_eq!(w.body.bullets.born_frame[i], 42, "fire 路径");
    // batch：3 颗
    let mut task = Task::default();
    let n0 = w.body.bullets.iter_alive().count();
    // 正序：appearance,x,y,n_angle,angle0,angle_step,n_speed,speed0,speed_step
    let args = [ROW_B, 0, 0, 3, 0, 1000, 1, Fx::from_int(1).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLETS_BATCH, &args).is_ok());
    assert!(w.body.bullets.iter_alive().count() > n0, "batch 至少造了弹");
    for j in w.body.bullets.iter_alive() {
        assert_eq!(w.body.bullets.born_frame[j], 42, "batch 路径槽 {j}");
    }
    // sh_fire：reset 槽 0 → sprite ROW_B → 开火
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_RESET, &[0]).is_ok());
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, ROW_B]).is_ok());
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, 2, 1]).is_ok());
    let n1 = w.body.bullets.iter_alive().count();
    assert!(call(&mut w, &ecl, &mut task, SYS_SH_FIRE, &[0]).is_ok());
    assert!(
        w.body.bullets.iter_alive().count() > n1,
        "sh_fire 至少造了弹"
    );
    for j in w.body.bullets.iter_alive() {
        assert_eq!(w.body.bullets.born_frame[j], 42, "sh_fire 路径槽 {j}");
    }
}

/// 541 `clear_bullets_at`：按参数铺一帧清弹区；stars=0 带 FIELD_NO_STAR。
#[test]
fn clear_bullets_at_creates_one_frame_field_with_star_bit() {
    use crate::field::{FIELD_CLEAR_BULLETS, FIELD_NO_STAR};
    let (mut w, ecl) = fresh();
    let mut stage = Task::default();
    let args = [
        Fx::from_int(-30).raw(),
        Fx::from_int(200).raw(),
        Fx::from_int(40).raw(),
        0,
    ];
    assert!(call(&mut w, &ecl, &mut stage, SYS_CLEAR_BULLETS_AT, &args).is_ok());
    let f = w.body.fields.iter_alive().next().expect("应建清弹区");
    assert_eq!(w.body.fields.x[f], Fx::from_int(-30));
    assert_eq!(w.body.fields.y[f], Fx::from_int(200));
    assert_eq!(w.body.fields.radius[f], Fx::from_int(40));
    assert_eq!(w.body.fields.life[f], 1);
    assert_eq!(w.body.fields.dmg_per_frame[f], 0);
    assert_eq!(w.body.fields.flags[f], FIELD_CLEAR_BULLETS | FIELD_NO_STAR);

    let (mut w2, _) = fresh();
    let mut args2 = args;
    args2[3] = 1;
    let mut stage2 = Task::default();
    assert!(call(&mut w2, &ecl, &mut stage2, SYS_CLEAR_BULLETS_AT, &args2).is_ok());
    let f2 = w2.body.fields.iter_alive().next().unwrap();
    assert_eq!(w2.body.fields.flags[f2], FIELD_CLEAR_BULLETS);
}
