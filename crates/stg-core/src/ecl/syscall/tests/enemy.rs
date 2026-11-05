//! 敌人创建、运动、属性与死亡效果。

use super::*;

/// 打包敌号 → 池槽索引（敌句柄打包刀 2026-07-31）。`spawn_enemy` 押的不再是裸 index，
/// 想拿"哪个槽"去戳池内存的测试走这里；想拿"敌号"喂读口的测试用
/// [`crate::enemy::pack_handle`]。
fn enemy_slot(packed: i32) -> usize {
    (packed & 0xFFFF) as usize
}

#[test]
fn sys_spawn_enemy_creates_with_fields() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)（A5 乙案：7 参，尾追 sprite/task）
    let args = [
        Fx::from_int(5).raw(),
        Fx::from_int(6).raw(),
        42,
        1,
        100,
        0,
        -1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let handle = task.stack[0];
    assert!(handle >= 0);
    let i = enemy_slot(handle);
    assert_eq!(w.body.enemies.x[i], Fx::from_int(5));
    assert_eq!(w.body.enemies.y[i], Fx::from_int(6));
    assert_eq!(w.body.enemies.hp[i], 42);
    // 表号在生成时就展开成逐类型计数（表 1 = POWER×2 + POINT×1）。
    assert_eq!(
        w.body.enemies.drop_count[i],
        crate::tables::drop_counts(&TABLES_V0, 1).0
    );
    assert_eq!(
        w.body.enemies.drop_count[i][crate::items::ITEM_POWER as usize],
        2
    );
    assert_eq!(w.body.enemies.score[i], 100);
}

/// P4-b：`spawn_enemy` 的越界 `drop_table` → 视同空表 + 计 contract_viol +
/// `last_status=BAD_ARGS`，敌照建、不 panic、死时不掉道具
/// （原 B11，随掉落状态从 settle 前移到生成时）。
/// 负数表号同样走这条（`as u16` 回绕成大正数 → 仍越界）。
///
/// `last_status` 那条断言是全支线复审 Minor #6 补的：兄弟测试
/// `world::tests::enemy_drop_and_kill_apis_degrade_on_stale_handle` 一直断言两样，
/// 这条搬到 syscall 层时只剩了 `contract_viol`。
#[test]
fn spawn_enemy_out_of_range_drop_table_degrades_to_empty() {
    // 内建表只有 2 张掉落表；取一个必然越界的正数号，再取 -1 走回绕那条腿。
    let oob = TABLES_V0.drop_tables.len() as i32 + 9;
    for &bad in &[oob, -1] {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        let viol_before = w.body.diag.contract_viol;
        assert_eq!(
            w.body.last_status,
            crate::world::STATUS_OK,
            "前提：开局 last_status 干净（表号 {bad}）"
        );
        // 正序：x,y,hp,drop_table,score,sprite,task(none=-1)
        let args = [
            Fx::ZERO.raw(),
            Fx::from_int(80).raw(),
            10,
            bad,
            100,
            0,
            -1,
            0,
        ];
        assert!(
            call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok(),
            "越界表号不得 Fault（P4-b 降级，不是违约方的锅）"
        );
        let handle = task.stack[0];
        assert!(handle >= 0, "敌照建（表号 {bad}）");
        assert_eq!(
            w.body.diag.contract_viol,
            viol_before + 1,
            "越界表号须计一次违约（表号 {bad}）"
        );
        assert_eq!(
            w.body.last_status,
            crate::world::STATUS_BAD_ARGS,
            "越界表号须写 last_status（同邻居 P4-b 口径；表号 {bad}）"
        );
        assert_eq!(
            w.body.enemies.drop_count[enemy_slot(handle)],
            [0u8; crate::items::ITEM_TYPE_COUNT],
            "视同空表（表号 {bad}）"
        );
    }
}

/// task_script ≥0 挂敌派任务：owner=(ENEMY, 新敌 index/gen)，`main_task` 回填槽号+1。
/// sprite 传判别值 5（非默认 0）——S1 纪律：圆心重合式测试对字段映射是瞎的，钉非默认值。
#[test]
fn spawn_enemy_with_task_binds_owner_and_main_task() {
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
        vec![EntryInit::new("enemy_task", 1)],
        Some(0),
    );
    let mut w = World::new(1);
    w.body.frame = 3;
    let mut task = Task::default();
    // 正序：x,y,hp,drop_table,score,sprite,task_script；sprite=5（判别值），task=1（在册）
    let args = [
        Fx::from_int(0).raw(),
        Fx::from_int(80).raw(),
        10,
        0,
        0,
        5,
        1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let eidx = enemy_slot(task.stack[0]) as u16;
    assert_eq!(
        w.body.enemies.sprite[eidx as usize], 5,
        "sprite 判别值应落池（S1：非默认判别）"
    );
    let egen = w.body.enemies.generation[eidx as usize];

    let child = (0..crate::ecl::task::TASK_CAP)
        .find(|&i| w.tasks.is_alive(i))
        .expect("应已派生子任务");
    assert_eq!(w.tasks.slots[child].owner_kind, OWNER_ENEMY);
    assert_eq!(w.tasks.slots[child].owner_index, eidx);
    assert_eq!(w.tasks.slots[child].owner_gen, egen);
    assert_eq!(w.tasks.slots[child].born_frame, 3);
    assert_eq!(
        w.body.enemies.main_task[eidx as usize],
        child as u32 + 1,
        "main_task 应回填为任务槽号+1"
    );
}

/// task_script = -1（none）：敌建成、不派任何任务，`main_task` 保持 0。
#[test]
fn spawn_enemy_task_none_leaves_main_task_zero() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = [
        Fx::from_int(0).raw(),
        Fx::from_int(80).raw(),
        10,
        0,
        0,
        0,
        -1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let handle = task.stack[0];
    assert!(handle >= 0);
    assert_eq!(
        w.body.enemies.main_task[enemy_slot(handle)],
        0,
        "task=none 不应回填 main_task"
    );
    assert_eq!(
        (0..crate::ecl::task::TASK_CAP)
            .filter(|&i| w.tasks.is_alive(i))
            .count(),
        0,
        "task=none 不应派生任何任务"
    );
}

/// 坏 task_script 号（不在册）→ Fault，且先验后建：敌不应被创建（零副作用），
/// 镜像 `sys_create_bullet_bad_task_script_faults_before_creating`。
#[test]
fn spawn_enemy_bad_task_script_faults_without_enemy() {
    let (mut w, ecl) = fresh(); // subs 空——任何脚本号都越界
    let mut task = Task::default();
    let args = [
        Fx::from_int(0).raw(),
        Fx::from_int(80).raw(),
        10,
        0,
        0,
        0,
        9999, // 不在册
        0,    // argc（boss 换段刀：210 调用约定追加）
    ];
    let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.enemies.iter_alive().count(), 0, "先验后建：零副作用");
}

/// P4-b 支路①：task 号超出 u16 范围 → Fault，且敌未创建（B25）。
///
/// **取值刻意挑过**（同 `sys_create_bullet_task_script_out_of_u16_faults_before_creating`
/// 的陷阱说明）：`65537 as u16 == 1`，而 `1` 在 `async_pattern_image()` 里恰是一个
/// 合法在册的零参 Async sub——若把 `u16::try_from` 检查误删成 `as u16` 截断，这条
/// 参数会"活下来"变成合法号，敌真的会建成（支路②③都拦不住）。选一个恰好越界又
/// 恰好截断落进"在册"的取值，才能让这条测试真正命中支路①，而不是被②顺手挡住。
#[test]
fn spawn_enemy_task_script_out_of_u16_faults_without_enemy() {
    let ecl = async_pattern_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    // 正序参：x, y, hp, drop_table, score, sprite, task_script
    let args = [0, 0, 100, 0, 0, 0, 65537, 0]; // 65537 > u16::MAX，但截断后=1（在册）
    let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(
        w.body.enemies.iter_alive().count(),
        0,
        "先验后建：不留半成品"
    );
}

/// P4-b 支路③：task 号在册但不是零参 `Async`（这里用 `CallOnly`）→ Fault，
/// 且敌未创建（B25）。镜像 `sys_create_bullet_task_script_wrong_kind_faults_before_creating`。
#[test]
fn spawn_enemy_task_script_wrong_kind_faults_without_enemy() {
    let ecl = wrong_kind_task_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    // 正序参：x, y, hp, drop_table, score, sprite, task_script
    let args = [0, 0, 100, 0, 0, 0, 1, 0]; // task_script=1，在册但 CallOnly
    let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(
        w.body.enemies.iter_alive().count(),
        0,
        "先验后建：不留半成品"
    );
}

/// P4-b 支路③另一半（终审 M-4）：task 号在册、`kind()==Async`，唯独**带参**
/// （非零参）→ 同样必须 Fault，且敌未创建。镜像
/// `sys_create_bullet_task_script_with_params_faults_before_creating`。
#[test]
fn spawn_enemy_task_script_with_params_faults_without_enemy() {
    let ecl = async_with_param_task_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    // 正序参：x, y, hp, drop_table, score, sprite, task_script
    let args = [0, 0, 100, 0, 0, 0, 1, 0]; // task_script=1，在册、Async，但带参
    let r = call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(
        w.body.enemies.iter_alive().count(),
        0,
        "先验后建：不留半成品"
    );
}

/// 任务池满（P4-a 降级）：敌仍建成、`diag.pool_full[POOL_TASK]` 计数 +1、
/// `main_task` 保持 0（挂任务失败不影响敌本身创建成功）。
#[test]
fn spawn_enemy_task_pool_full_degrades() {
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
        vec![EntryInit::new("enemy_task", 1)],
        Some(0),
    );
    let mut w = World::new(1);
    // 先灌满任务池（循环 tasks.spawn；owner 值任意，本测试只关心池满信号）。
    while w
        .tasks
        .spawn(SubId::default(), 0, (OWNER_STAGE, 0, 0), 0, 0)
        .is_some()
    {}
    let mut task = Task::default();
    let args = [
        Fx::from_int(0).raw(),
        Fx::from_int(80).raw(),
        10,
        0,
        0,
        0,
        1, // 在册但池满
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let handle = task.stack[0];
    assert!(handle >= 0, "任务池满不应阻止敌建成");
    assert_eq!(
        w.body.diag.pool_full[crate::world::POOL_TASK],
        1,
        "任务池满应计一次 P4-a 降级"
    );
    assert_eq!(
        w.body.enemies.main_task[enemy_slot(handle)],
        0,
        "main_task 应保持 0（挂任务失败）"
    );
}

/// e2e：敌绑定的 task 随敌死亡被 owner-liveness gate 静默收走（相位 2 门禁，同
/// `sys_create_bullet_task_script_spawns_owner_bound_task_and_dies_with_bullet` 先例）。
#[test]
fn enemy_owned_task_dies_with_enemy() {
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
        vec![EntryInit::new("enemy_task", 1)],
        Some(0),
    );
    let mut w = World::new(1);
    w.body.frame = 5;
    let mut task = Task::default();
    let args = [
        Fx::from_int(0).raw(),
        Fx::from_int(80).raw(),
        50,
        0,
        0,
        0,
        1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let eidx = enemy_slot(task.stack[0]) as u16;
    let egen = w.body.enemies.generation[eidx as usize];

    let child = (0..crate::ecl::task::TASK_CAP)
        .find(|&i| w.tasks.is_alive(i))
        .expect("应已派生子任务");
    assert!(w.tasks.is_alive(child), "敌死之前任务应活");

    // free 敌 → 次帧任务被静默回收（owner 门禁，通过完整 step 驱动验证端到端链路）。
    w.body.enemies.free(EnemyHandle {
        index: eidx,
        generation: egen,
    });
    let frame = w.body.frame;
    crate::step::step(
        &mut w,
        &TABLES_V0,
        &ecl,
        &crate::input::InputFrame::empty(frame),
    );
    assert!(!w.tasks.is_alive(child), "owner 敌死后任务应被静默回收");
    assert_eq!(w.body.diag.task_faults, 0, "owner 死不是 Fault");
}

/// `enemy_hp`（SYS 100）：活敌返当前 hp（判别值 77，非默认）；死敌/越界句柄均返 -1
/// （P4-b，不 Fault）。
#[test]
fn enemy_hp_reads_alive_and_rejects_dead() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 77);
    // `test_support::spawn_enemy` 令 hp==hp_max==77——单独把 hp_max 拉开，逐位命中排除
    // "读混 hp/hp_max 两个同族字段"的变异（同 `sys_self_hp_max_dispatch_by_owner_kind`
    // 先例：hp_max=9999 排除读混字段）。
    w.body.enemies.hp_max[eh.index as usize] = 9999;
    let mut task = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_ENEMY_HP,
            &[crate::enemy::pack_handle(eh)]
        )
        .is_ok()
    );
    assert_eq!(task.stack[0], 77, "活敌返当前 hp（判别值，非 hp_max）");

    w.body.enemies.free(eh);
    task.sp = 0;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_ENEMY_HP,
            &[crate::enemy::pack_handle(eh)]
        )
        .is_ok()
    );
    assert_eq!(task.stack[0], -1, "死敌返 -1");

    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ENEMY_HP, &[9999]).is_ok());
    assert_eq!(task.stack[0], -1, "越界句柄返 -1（P4-b 不 Fault）");
}

/// self owner=ENEMY 时 `move_enemy_to` 正确武装插值器（`mv_active`/终点/时长/easing）。
#[test]
fn sys_move_enemy_to_arms_interpolator() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    // 正序：dur,x,y,easing
    let args = [30, Fx::from_int(50).raw(), Fx::from_int(60).raw(), 2];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.mv_active[i], 1);
    assert_eq!(w.body.enemies.mv_to_x[i], Fx::from_int(50));
    assert_eq!(w.body.enemies.mv_to_y[i], Fx::from_int(60));
    assert_eq!(w.body.enemies.mv_dur[i], 30);
    assert_eq!(w.body.enemies.mv_easing[i], 2);
}

/// D19：`dur`/`easing` 越界 → P4-b 整条 no-op（不是静默截断、也不是 Fault）。
///
/// **旧行为的荒谬正是本条要钉死的**：裸 `as u8` 之下 `easing = 256` 截断成 0、**静默变
/// Linear** 并照常武装插值器，而 `easing = 264` 落 8、被世界层正确拒掉——能不能拒取决于
/// 越界值**模 256 落在哪里**。`dur = -1` 同理变成"缓动 65535 帧"。
///
/// **判别力**：`easing = 256` 那条腿是核心——它在旧实现下**会成功武装**（`mv_active == 1`、
/// `mv_easing == 0`），只有收窄之后才 no-op。若有人把 `try_from` 改回 `as`，或把处置从
/// no-op 改成钳位，这条立刻红。`dur = 65536`（截断成 0）与 `dur = -1`（截断成 65535）
/// 两条覆盖 `dur` 的两侧。第四条正例押住"合法值一切照旧"，防"收窄收过头把好参数也拒了"。
#[test]
fn move_verbs_reject_out_of_range_dur_and_easing_instead_of_truncating() {
    for (label, args) in [
        // 正序 dur,x,y,easing
        (
            "easing=256（旧实现截断成 0 = Linear，会静默成功）",
            [30, 0, 0, 256],
        ),
        ("easing=-1", [30, 0, 0, -1]),
        ("dur=65536（旧实现截断成 0）", [65536, 0, 0, 2]),
        ("dur=-1（旧实现截断成 65535 ≈ 18 分钟）", [-1, 0, 0, 2]),
    ] {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let before = w.body.diag.contract_viol;
        assert!(
            call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok(),
            "{label}：坏参数是调用方违约，走 P4-b 不 Fault"
        );
        let i = eh.index as usize;
        assert_eq!(
            w.body.enemies.mv_active[i], 0,
            "{label}：必须整条 no-op，不得武装"
        );
        assert_eq!(w.body.diag.contract_viol - before, 1, "{label}：违约计一次");
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS, "{label}");
    }

    // 正例：边界上的合法值照常武装（收窄不得收过头）
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let args = [65535, Fx::from_int(50).raw(), 0, 29]; // dur 上沿 + easing 上沿
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.mv_active[i], 1, "边界合法值必须照常武装");
    assert_eq!(w.body.enemies.mv_dur[i], 65535);
    assert_eq!(w.body.enemies.mv_easing[i], 29);
}

/// D19：四条速度动词与 `move_enemy_to` **同一条腿**——收窄是共用 helper，五个入口不得分家。
///
/// **判别力**：逐个 syscall 号试 `easing = 256`。若只给 `move_enemy_to` 加了收窄而漏掉
/// 某条速度动词（本条最可能的实现失误），那一条的 `vel_active` 会是 1，立刻红。
#[test]
fn all_five_move_verbs_share_the_same_narrowing_leg() {
    // (号, 正序参数——末位恒为 easing)
    let cases: [(u16, &[i32]); 4] = [
        (SYS_MOVE_VEL, &[30, 0, Fx::from_int(3).raw(), 256]),
        (SYS_MOVE_VEL_XY, &[30, Fx::from_int(3).raw(), 0, 256]),
        (SYS_MOVE_ANGLE, &[30, 0, 256]),
        (SYS_MOVE_SPEED, &[30, Fx::from_int(3).raw(), 256]),
    ];
    for (sys, args) in cases {
        let (mut w, ecl) = fresh();
        let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
        let mut task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        let before = w.body.diag.contract_viol;
        assert!(
            call(&mut w, &ecl, &mut task, sys, args).is_ok(),
            "syscall {sys}"
        );
        let i = eh.index as usize;
        assert_eq!(
            w.body.enemies.vel_active[i], 0,
            "syscall {sys}：easing=256 必须整条 no-op（收窄漏了这一条入口？）"
        );
        assert_eq!(w.body.diag.contract_viol - before, 1, "syscall {sys}");
    }
}

/// self owner != ENEMY → Fault（误用策略：move_enemy_to 要求 self 是敌）。
#[test]
fn sys_move_enemy_to_wrong_owner_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    let args = [30, 0, 0, 0];
    let r = call(&mut w, &ecl, &mut task, SYS_MOVE_ENEMY_TO, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
}

/// move_vel（410）：4 参正序 dur,angle,speed,easing；owner 取自 self。
#[test]
fn sys_move_vel_arms_polar_interpolator() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let args = [
        20,
        crate::math::Angle::QUARTER.raw() as i32,
        Fx::from_int(4).raw(),
        3,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.vel_active[i], 1);
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_POLAR);
    assert_eq!(w.body.enemies.vel_to_0[i], Fx::from_int(4).raw());
    assert_eq!(
        w.body.enemies.vel_to_1[i],
        crate::math::Angle::QUARTER.raw() as i32
    );
    assert_eq!(w.body.enemies.vel_dur[i], 20);
    assert_eq!(w.body.enemies.vel_easing[i], 3);
}

/// move_vel_xy（411）：落 CART 空间——**参数序 dur,vx,vy,easing**（vx 在前）。
#[test]
fn sys_move_vel_xy_arms_cartesian_interpolator() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    // vx=3, vy=-7：**x≠y 且异号**，两条派发臂写反立刻可辨
    let args = [12, Fx::from_int(3).raw(), Fx::from_int(-7).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL_XY, &args).is_ok());
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.vel_space[i], crate::enemy::VEL_SPACE_CART);
    assert_eq!(
        w.body.enemies.vel_to_0[i],
        Fx::from_int(3).raw(),
        "slot0 = vx"
    );
    assert_eq!(
        w.body.enemies.vel_to_1[i],
        Fx::from_int(-7).raw(),
        "slot1 = vy"
    );
}

/// move_angle（420）/ move_speed（421）：3 参，各自保持另一分量。
#[test]
fn sys_move_angle_and_speed_are_single_axis() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let i = eh.index as usize;
    // 先摆一个 speed=7、angle=0 的起点（dur=0 瞬时）
    let seed = [0, 0, Fx::from_int(7).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_VEL, &seed).is_ok());
    // move_angle(0, QUARTER, 0) → 只转向
    let a = [0, crate::math::Angle::QUARTER.raw() as i32, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_ANGLE, &a).is_ok());
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(7), "转向不改速率");
    assert_eq!(w.body.enemies.angle[i], crate::math::Angle::QUARTER);
    // move_speed(0, 2.0, 0) → 只调速
    let s = [0, Fx::from_int(2).raw(), 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_MOVE_SPEED, &s).is_ok());
    assert_eq!(
        w.body.enemies.angle[i],
        crate::math::Angle::QUARTER,
        "调速不改方向"
    );
    assert_eq!(w.body.enemies.speed[i], Fx::from_int(2));
}

/// 四条动词 self owner != ENEMY → Fault（同 move_to 现状）。
#[test]
fn motion_verbs_fault_on_non_enemy_owner() {
    let (mut w, ecl) = fresh();
    for (sys, argc) in [
        (SYS_MOVE_VEL, 4),
        (SYS_MOVE_VEL_XY, 4),
        (SYS_MOVE_ANGLE, 3),
        (SYS_MOVE_SPEED, 3),
    ] {
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        let args = vec![0i32; argc];
        let r = call(&mut w, &ecl, &mut task, sys, &args);
        assert_eq!(r, Err(FAULT_BAD_OP), "syscall {sys} 非敌 owner 应 Fault");
    }
}

/// 把内建掉落表 1 展开（`ITEM_POWER`×2 + `ITEM_POINT`×1 = **3 颗**）灌进敌的掉落计数。
fn load_drop_table_1(w: &mut World, eh: EnemyHandle) {
    let (counts, ok) = crate::tables::drop_counts(&TABLES_V0, 1);
    assert!(ok, "内建掉落表 1 必须存在");
    assert_eq!(
        counts.iter().map(|&n| n as u32).sum::<u32>(),
        3,
        "本组测试的 3 颗判别值依赖表 1 的内容"
    );
    w.body.enemies.drop_count[eh.index as usize] = counts;
}

/// D-3 判别腿：`drop_items()` 吐完**不清空**计数 → 再死一次会掉**双份**。
/// 这是人类裁定（spec §3 D-3），不是 bug——将来有人"顺手修好"成清零语义，这条会红。
#[test]
fn drop_items_does_not_clear_counts_so_dying_after_drops_twice() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
    load_drop_table_1(&mut w, eh);

    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ITEMS, &[]).is_ok());
    assert_eq!(w.body.items.iter_alive().count(), 3, "表 1 展开 = 3 颗");

    // 计数没被清空 —— 随后的死亡效果把同一批再撒一遍。
    w.body.kill_enemy(eh.index as usize, &TABLES_V0);
    assert_eq!(
        w.body.items.iter_alive().count(),
        6,
        "吐完不清空（人类裁定 D-3）：`drop_items(); die();` 掉双份，作者自负"
    );
}

/// `die()` 走**完整**死亡效果：四件齐。判别腿——只标 dying 不跑效果的实现会红。
#[test]
fn die_runs_the_full_death_effect() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 100);
    load_drop_table_1(&mut w, eh);
    let score_before = w.body.players[0].score;

    assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());

    let i = eh.index as usize;
    assert_eq!(w.body.items.iter_alive().count(), 3, "掉落");
    assert_eq!(
        w.body.players[0].score,
        score_before + 100,
        "加分（测试内不拾取道具，故这 100 只能来自 enemies.score）"
    );
    assert!(
        w.body
            .frame_events()
            .iter()
            .any(|e| e.kind == crate::events::EVT_ENEMY_DIED),
        "死亡事件"
    );
    assert_ne!(
        w.body.enemies.flags[i] & crate::enemy::ENEMY_DYING,
        0,
        "dying 标记"
    );
    let reqs = w.body.take_requests();
    assert!(
        reqs.iter().any(|r| r.id == crate::consts::REQ_ENEMY_DEATH),
        "死亡特效请求"
    );
}

/// `die()` 打**满血** boss → hp 归 0（不是留在 8888）。
/// 防"HUD 当帧显示满血死人"；这也是 `min(0)` 与"什么都不做"的判别点。
#[test]
fn die_on_full_hp_enemy_zeroes_hp() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
    let i = eh.index as usize;
    assert_eq!(w.body.enemies.hp_max[i], 8888, "前提：满血");

    assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());
    assert_eq!(w.body.enemies.hp[i], 0, "满血 die → hp 压到 0");

    // 反向腿：overkill 打成的负血不得被抹平（负值可观测性是
    // `settle_overkill_two_shots_one_death_event` 的依赖）——证明用的是 `min(0)`
    // 而不是无条件置 0。
    let (eh2, _t2) = enemy_owner_task(&mut w, 10, 0);
    let j = eh2.index as usize;
    w.body.enemies.hp[j] = -5;
    w.body.kill_enemy(j, &TABLES_V0);
    assert_eq!(w.body.enemies.hp[j], -5, "min(0) 而非无条件置 0");
}

/// `drop_clear()` 清空计数 → 随后的死亡效果一颗都不掉（`clear_enemy_drops` 的正腿；
/// 七条钉裁定的测试里只有 misuse 腿碰过 `SYS_DROP_CLEAR`，那条不区分"清空"与"no-op"）。
#[test]
fn drop_clear_zeroes_counts_so_death_drops_nothing() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
    load_drop_table_1(&mut w, eh);

    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_CLEAR, &[]).is_ok());
    assert_eq!(
        w.body.enemies.drop_count[eh.index as usize],
        [0; crate::items::ITEM_TYPE_COUNT]
    );
    w.body.kill_enemy(eh.index as usize, &TABLES_V0);
    assert_eq!(w.body.items.iter_alive().count(), 0, "清空后死亡不掉落");
}

/// P4-b：`drop_add` 坏类型（≥ `ITEM_TYPE_COUNT` / 负数）→ no-op + contract_viol，无掉落。
#[test]
fn drop_add_bad_type_degrades_and_counts() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
    let i = eh.index as usize;
    let viol0 = w.body.diag.contract_viol;

    // 腿一：越界正数类型。取值 `(type=ITEM_TYPE_COUNT=5, n=3)` 是**刻意挑的判别对**：
    // `n=3` 自己是个**合法**类型号（`ITEM_BOMB_PIECE`），所以一旦实现把两个 `pop` 写反
    // （正序参 `type, n` → 逆序弹栈 `n` 先 `type` 后），这条腿就不再是"坏类型 no-op"，
    // 而是老老实实执行 `drop_count[3] += 5` —— 被下面的"全零"断言当场逮住。
    // 换成 `n=1` 之类也合法的值同样能逮，但换成 `n` 越界（如 99）就两路都 no-op，
    // 这条腿会退化成对参数序瞎的测试。别顺手改这两个数。
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_DROP_ADD,
            &[crate::items::ITEM_TYPE_COUNT as i32, 3]
        )
        .is_ok(),
        "坏类型是 P4-b 降级，不 Fault"
    );
    // 腿二：负数类型
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[-1, 3]).is_ok());

    assert_eq!(
        w.body.enemies.drop_count[i],
        [0; crate::items::ITEM_TYPE_COUNT],
        "两条坏类型腿都必须 no-op"
    );
    assert_eq!(
        w.body.diag.contract_viol,
        viol0 + 2,
        "各计一次 contract_viol"
    );
    w.body.kill_enemy(i, &TABLES_V0);
    assert_eq!(w.body.items.iter_alive().count(), 0, "无掉落");
}

/// P4-b：`drop_add` 的 n 为负或巨大 → 钳位后饱和，不 panic 不回绕。
/// `n = -5` → 计数不变（视同 0）；`n = i32::MAX` → 计数封顶 255。
#[test]
fn drop_add_clamps_and_saturates_n() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 8888, 0);
    let i = eh.index as usize;
    let ty = crate::items::ITEM_POINT as i32;
    let slot = crate::items::ITEM_POINT as usize;

    // 负 n 视同 0（本刀不做"减掉落"；只饱和不钳的实现会让 `-5 as u8` 回绕成 251）。
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, -5]).is_ok());
    assert_eq!(w.body.enemies.drop_count[i][slot], 0, "负 n 视同 0");

    // 增量语义：两次调用累加，不是覆盖。
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 2]).is_ok());
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 3]).is_ok());
    assert_eq!(w.body.enemies.drop_count[i][slot], 5, "增量累加");

    // 巨大 n：先钳 [0,255] 再 saturating_add —— 不 panic、不回绕。
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, i32::MAX]).is_ok());
    assert_eq!(w.body.enemies.drop_count[i][slot], 255, "封顶 255");
    // 已封顶再加仍是 255（饱和，非回绕）——只钳不饱和会在这里溢出 panic。
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ADD, &[ty, 200]).is_ok());
    assert_eq!(w.body.enemies.drop_count[i][slot], 255, "饱和不回绕");
}

/// spec §5 的断言腿：**`die()` 打在绑卡 boss 上，经现有的破卡三路 OR 自动收卡结算——
/// 无需新增机制。** 这是设计里一句"现成机制够用"的论断，不是推论，要验（全支线复审
/// Important #3 补：spec §8 点名的这一行在 T4 交接中蒸发了，全仓再无 `SYS_DIE` 与
/// 符卡槽同框的测试）。
///
/// 与 `vm::tests::spell_bound_boss_self_destruct_settles_spell_with_hp_above_threshold`
/// 的分工：那条走 **D9 自燃**（不碰 hp，故 hp 远高于血线，判的是三路 OR 里
/// `ENEMY_DYING` 那一路的判别力）；本条走 **`SYS_DIE`**，`kill_enemy` 的 `hp.min(0)`
/// 让第三路 `hp<=threshold` 同真，故它**不**是 OR 分支的判别腿——它验的是别的东西：
/// syscall → 世界 → `settle_spells` 这条链在 `die()` 上真的接通了。
#[test]
fn die_on_spell_bound_boss_settles_the_spell() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 10_000, 0);
    assert!(
        w.body.spell_begin_internal(0, eh, 77, 100, 1000, 0, 300),
        "前提：卡开起来了"
    );
    assert_eq!(w.body.spells[0].active, 1, "前提：槽 active");

    assert!(call(&mut w, &ecl, &mut task, SYS_DIE, &[]).is_ok());
    assert_eq!(
        w.body.enemies.hp[eh.index as usize], 0,
        "前提：`die()` 压过血线下钳（spec §4.2 有意为之），不是停在 threshold=300"
    );

    w.body.settle_spells(&TABLES_V0);
    assert_eq!(
        w.body.spells[0].active, 0,
        "spec §5 断言：`die()` 打绑卡 boss 无需新增机制即收卡结算"
    );
    assert!(
        w.body
            .frame_events()
            .iter()
            .any(|e| e.kind == crate::events::EVT_SPELL_CAPTURED && e.data[0] == 77),
        "走的是正常结算路径（CAPTURED），不是把槽抹了"
    );
}

/// misuse：非 enemy-owner 调这四个 → Fault（照 `self_enemy_handle` 既有口径）。
#[test]
fn drop_and_die_syscalls_fault_for_non_enemy_owner() {
    let (mut w, ecl) = fresh();
    for (no, args) in [
        (SYS_DROP_CLEAR, &[][..]),
        (SYS_DROP_ADD, &[0, 1][..]),
        (SYS_DROP_ITEMS, &[][..]),
        (SYS_DIE, &[][..]),
    ] {
        let mut task = Task {
            owner_kind: OWNER_STAGE,
            ..Task::default()
        };
        assert_eq!(
            call(&mut w, &ecl, &mut task, no, args),
            Err(FAULT_BAD_OP),
            "syscall {no} 的非敌 owner 腿"
        );
    }
}

// ── 表现契约 v2（2026-09-07）：430 set_anm_state / 721 fx_at / 722 fx_on / born_frame ──

/// 430：写状态并盖帧；**同状态重设也盖**（= 重播，ZUN interrupt 重触发语义）。
#[test]
fn set_anm_state_restamps_frame_even_for_same_state() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    let i = eh.index as usize;
    w.body.frame = 10;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[3]).is_ok());
    assert_eq!(w.body.enemies.anm_state[i], 3);
    assert_eq!(w.body.enemies.anm_state_frame[i], 10);
    w.body.frame = 25;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[3]).is_ok());
    assert_eq!(w.body.enemies.anm_state[i], 3, "状态不变");
    assert_eq!(w.body.enemies.anm_state_frame[i], 25, "帧号必须重盖");
    assert_eq!(w.body.diag.contract_viol, 0);
}

/// `spawn_enemy`（210）盖 `anm_state_frame = 创建帧`、`anm_state = 0`。
#[test]
fn spawn_enemy_stamps_anm_state_frame() {
    let (mut w, ecl) = fresh();
    w.body.frame = 42;
    let mut task = Task::default();
    let args = [
        Fx::from_int(5).raw(),
        Fx::from_int(6).raw(),
        42,
        1,
        100,
        0,
        -1,
        0, // argc（boss 换段刀：210 调用约定追加）
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    let i = enemy_slot(task.stack[0]);
    assert_eq!(w.body.enemies.anm_state[i], 0);
    assert_eq!(w.body.enemies.anm_state_frame[i], 42);
}

// ── boss 换段与敌人钩子刀 Task 2：敌判定族 440-443 / $self_enemy 026 / 清场 531 ──────────

/// 440 `set_invuln`：写入；`-1`/`65536` 越界整条 no-op + 违约；非敌 owner Fault。
#[test]
fn set_invuln_writes_rejects_out_of_range_and_faults_for_non_enemy() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
    let i = eh.index as usize;
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[120]).is_ok());
    assert_eq!(w.body.enemies.invuln[i], 120);
    for bad in [-1, 65536] {
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_INVULN, &[bad]).is_ok());
        assert_eq!(w.body.enemies.invuln[i], 120, "越界 {bad} 整条 no-op");
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
    }
    let mut stage = Task::default();
    assert_eq!(
        call(&mut w, &ecl, &mut stage, SYS_SET_INVULN, &[1]),
        Err(FAULT_BAD_OP)
    );
}

/// 441/442：各写各的半径，互不串位；负半径钳 0 并计违约。
#[test]
fn set_hitbox_and_hurtbox_write_their_own_radius_and_clamp() {
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
    let i = eh.index as usize;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_HITBOX,
            &[Fx::from_int(40).raw()]
        )
        .is_ok()
    );
    assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
    assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(16));
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_HURTBOX,
            &[Fx::from_int(24).raw()]
        )
        .is_ok()
    );
    assert_eq!(w.body.enemies.hurtbox[i], Fx::from_int(24));
    assert_eq!(w.body.enemies.radius[i], Fx::from_int(40));
    let v0 = w.body.diag.contract_viol;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_HITBOX,
            &[Fx::from_int(-5).raw()]
        )
        .is_ok()
    );
    assert_eq!(w.body.enemies.radius[i], Fx::ZERO);
    assert_eq!(w.body.diag.contract_viol, v0 + 1);
}

/// 443 `set_enemy_flag`：只收 NO_BODY|KILLALL_EXEMPT 的非空子集；含 DYING/未知位/0 整条拒。
#[test]
fn set_enemy_flag_accepts_only_settable_bits() {
    use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, ENEMY_NO_BODY};
    let (mut w, ecl) = fresh();
    let (eh, mut task) = enemy_owner_task(&mut w, 100, 0);
    let i = eh.index as usize;
    let (nb, ex) = (ENEMY_NO_BODY as i32, ENEMY_KILLALL_EXEMPT as i32);
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[nb | ex, 1]).is_ok());
    assert_eq!(
        w.body.enemies.flags[i],
        ENEMY_NO_BODY | ENEMY_KILLALL_EXEMPT
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[ex, 0]).is_ok());
    assert_eq!(w.body.enemies.flags[i], ENEMY_NO_BODY);
    for bad in [0, ENEMY_DYING as i32, 8, nb | ENEMY_DYING as i32] {
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut task, SYS_SET_ENEMY_FLAG, &[bad, 1]).is_ok());
        assert_eq!(
            w.body.enemies.flags[i], ENEMY_NO_BODY,
            "坏掩码 {bad} 不改 flags"
        );
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
    }
}

/// 531 静默模式：跳过调用者 / 免清 / 已死；被杀者无事件、无加分、无掉落。
#[test]
fn kill_all_enemies_silent_skips_caller_exempt_and_dying() {
    use crate::enemy::{ENEMY_DYING, ENEMY_KILLALL_EXEMPT, KILL_SILENT};
    let (mut w, ecl) = fresh();
    let (caller, mut task) = enemy_owner_task(&mut w, 10, 0);
    let exempt = crate::world::test_support::spawn_enemy(&mut w, 10, 0, 10);
    let dying = crate::world::test_support::spawn_enemy(&mut w, 20, 0, 10);
    let normal = crate::world::test_support::spawn_enemy(&mut w, 30, 0, 10);
    w.body.enemies.flags[exempt.index as usize] |= ENEMY_KILLALL_EXEMPT;
    w.body.enemies.flags[dying.index as usize] |= ENEMY_DYING;
    load_drop_table_1(&mut w, normal);
    let score0 = w.body.players[0].score;
    let ev0 = w.body.frame_events_len;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_KILL_ALL_ENEMIES,
            &[KILL_SILENT as i32]
        )
        .is_ok()
    );
    let is_dying =
        |w: &World, h: EnemyHandle| w.body.enemies.flags[h.index as usize] & ENEMY_DYING != 0;
    assert!(!is_dying(&w, caller), "调用者自己不杀");
    assert!(!is_dying(&w, exempt), "免清位不杀");
    assert!(is_dying(&w, normal));
    assert_eq!(w.body.players[0].score, score0, "静默不加分");
    assert_eq!(w.body.frame_events_len, ev0, "静默不发事件");
    assert_eq!(w.body.items.iter_alive().count(), 0, "静默不掉落");
}

/// 531 击破模式走 die() 全套；坏 mode 整条 no-op + 违约。STAGE owner 无调用者豁免。
#[test]
fn kill_all_enemies_die_mode_runs_full_death_and_bad_mode_is_noop() {
    use crate::enemy::{ENEMY_DYING, KILL_DIE};
    use crate::events::EVT_ENEMY_DIED;
    let (mut w, ecl) = fresh();
    let e = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 10);
    load_drop_table_1(&mut w, e);
    let mut stage = Task::default();
    let v0 = w.body.diag.contract_viol;
    assert!(call(&mut w, &ecl, &mut stage, SYS_KILL_ALL_ENEMIES, &[2]).is_ok());
    assert_eq!(w.body.diag.contract_viol, v0 + 1);
    assert_eq!(
        w.body.enemies.flags[e.index as usize] & ENEMY_DYING,
        0,
        "坏 mode 不杀"
    );

    let score0 = w.body.players[0].score;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut stage,
            SYS_KILL_ALL_ENEMIES,
            &[KILL_DIE as i32]
        )
        .is_ok()
    );
    assert_ne!(w.body.enemies.flags[e.index as usize] & ENEMY_DYING, 0);
    assert_eq!(
        w.body.players[0].score,
        score0 + 100,
        "test_support 敌 score=100"
    );
    assert_eq!(
        w.body.frame_events[w.body.frame_events_len as usize - 1].kind,
        EVT_ENEMY_DIED
    );
    assert_eq!(w.body.items.iter_alive().count(), 3, "掉落表 1 = 3 颗");
}

// ── boss 换段与敌人钩子刀 Task 4：spawn_enemy 带参（210 调用约定）──────────────

/// 1 参 Async sub（raw=1）的镜像——spawn_enemy 带参测试专用。
fn one_arg_async_image() -> EclImage {
    use crate::ecl::image::EclValueType;
    test_image(
        vec![
            crate::ecl::ops::OP_PUSHI as u32,
            999,
            crate::ecl::ops::OP_WAIT as u32,
        ],
        vec![
            SubInit::new(0, SubKind::Root, vec![]),
            SubInit::new(0, SubKind::Async, vec![EclValueType::Int]),
        ],
        vec![EntryInit::new("zako", 1)],
        Some(0),
    )
}

/// 带参生成（spec §4.2）：同一帧两只敌各拿自己的实参——globals 顶替做不到的判别腿。
#[test]
fn spawn_enemy_with_args_gives_each_task_its_own_args_same_frame() {
    let ecl = one_arg_async_image();
    let mut w = World::new(1);
    let mut task = Task::default();
    for v in [111, 222] {
        let args = [0, Fx::from_int(80).raw(), 10, 0, 0, 0, 1, v, 1];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
    }
    let got: Vec<i32> = (0..crate::ecl::task::TASK_CAP)
        .filter(|&i| w.tasks.is_alive(i))
        .map(|i| w.tasks.slots[i].locals[0])
        .collect();
    assert_eq!(got, vec![111, 222]);
}

/// 带参门禁：个数不符 / none 带参 → Fault(0) 且敌未建；argc 超 LOCALS 或栈不够 → Fault(2)。
#[test]
fn spawn_enemy_with_args_rejects_bad_arity_before_creating_enemy() {
    let ecl = one_arg_async_image();
    let cases: [(&[i32], u8); 4] = [
        (&[0, 0, 10, 0, 0, 0, 1, 0], FAULT_BAD_OP), // 1 参 sub 给 0 参
        (&[0, 0, 10, 0, 0, 0, -1, 7, 1], FAULT_BAD_OP), // none 带参
        (&[0, 0, 10, 0, 0, 0, 1, 65], FAULT_STACK), // argc > LOCALS
        (&[0, 0, 10, 0, 0, 0, 1, 3], FAULT_STACK),  // 栈里不够 argc+7
    ];
    for (args, fault) in cases {
        let mut w = World::new(1);
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, args),
            Err(fault),
            "{args:?}"
        );
        assert_eq!(
            w.body.enemies.iter_alive().count(),
            0,
            "{args:?}：敌不得建出"
        );
    }
}
