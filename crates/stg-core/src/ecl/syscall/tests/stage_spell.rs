//! 关卡、符卡、玩家账面与表现请求。

use super::*;

#[test]
fn sys_drop_item_creates_item() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = [
        Fx::from_int(1).raw(),
        Fx::from_int(2).raw(),
        crate::items::ITEM_POINT as i32,
    ];
    assert!(call(&mut w, &ecl, &mut task, SYS_DROP_ITEM, &args).is_ok());
    assert!(task.stack[0] >= 0);
    assert_eq!(w.body.items.iter_alive().count(), 1);
}

/// boss_set：self owner=ENEMY 时 `enemy` 字段自动取自 owner；6 参落槽逐位命中。
#[test]
fn sys_boss_set_writes_slot_with_self_owner_enemy() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    // 正序：slot,hp_ratio,spell_id,timer_frames,phase_left,active
    let args = [0, Fx::from_raw(32768).raw(), 7, 3600, 2, 1];
    assert!(call(&mut w, &ecl, &mut task, SYS_BOSS_SET, &args).is_ok());
    let ui = w.body.boss_ui[0];
    assert_eq!(ui.enemy, eh);
    assert_eq!(ui.hp_ratio, Fx::from_raw(32768));
    assert_eq!(ui.spell_id, 7);
    assert_eq!(ui.timer_frames, 3600);
    assert_eq!(ui.phase_left, 2);
    assert_eq!(ui.active, 1);
}

/// boss_set：self owner 非 ENEMY → `enemy` 字段写 NULL，不 Fault（例外，见模块文档）。
#[test]
fn sys_boss_set_non_enemy_owner_writes_null_enemy_no_fault() {
    let (mut w, ecl) = fresh();
    let mut task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    let args = [0, 0, 0, 0, 0, 1];
    assert!(call(&mut w, &ecl, &mut task, SYS_BOSS_SET, &args).is_ok());
    assert_eq!(w.body.boss_ui[0].enemy, EnemyHandle::NULL);
    assert_eq!(w.body.boss_ui[0].active, 1);
}

#[test]
fn sys_pulse_signal_sets_channel() {
    let (mut w, ecl) = fresh();
    w.body.frame = 10;
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_PULSE_SIGNAL, &[3]).is_ok());
    assert_eq!(w.body.signals[3], 11, "脉冲戳为 frame+1（边沿语义）");
}

#[test]
fn sys_stage_clear_pushes_fact_event_not_request() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_STAGE_CLEAR, &[3]).is_ok());
    let evs: Vec<_> = w
        .frame_events()
        .iter()
        .filter(|e| e.kind == crate::events::EVT_STAGE_CLEARED)
        .collect();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].data[0], 3, "载荷 = stage");
    assert_eq!(
        w.body.take_requests().len(),
        0,
        "走通道 A 事实流，不发通道 B 请求"
    );
    assert_eq!(task.sp, 0, "参数弹尽");
}

#[test]
fn sys_emit_req_pushes_request_and_drains_stack() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_EMIT_REQ,
            &[64, 1, 2, 3, 4, 5, 6]
        )
        .is_ok()
    );
    let reqs = w.body.take_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!((reqs[0].id, reqs[0].seq), (64, 0));
    assert_eq!(
        reqs[0].args,
        [1, 2, 3, 4, 5, 6],
        "声明序 id,a0..a5 ↔ 弹栈逆序还原"
    );
    assert_eq!(task.sp, 0, "七值全弹栈");
}

#[test]
fn sys_emit_req_bad_id_is_p4b_noop() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let cv0 = w.body.diag.contract_viol;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_EMIT_REQ,
            &[-1, 0, 0, 0, 0, 0, 0]
        )
        .is_ok()
    );
    assert_eq!(w.body.take_requests().len(), 0, "坏 id no-op 不入缓冲");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    assert_eq!(
        task.sp, 0,
        "坏 id 路径同样七值全弹栈（先弹后验，栈效应与成功路径一致）"
    );
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_EMIT_REQ,
            &[65536, 0, 0, 0, 0, 0, 0]
        )
        .is_ok()
    );
    assert_eq!(w.body.take_requests().len(), 0, "越上界同款");
    assert_eq!(w.body.diag.contract_viol, cv0 + 2);
    assert_eq!(
        task.sp, 0,
        "坏 id 路径同样七值全弹栈（先弹后验，栈效应与成功路径一致）"
    );
}

#[test]
fn sys_emit_req_stack_underflow_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert_eq!(
        call(&mut w, &ecl, &mut task, SYS_EMIT_REQ, &[1, 2]),
        Err(FAULT_STACK),
        "参数不足 → 栈下溢 Fault（同全族处置）"
    );
}

/// misuse 策略：owner != ENEMY → Fault，零副作用（同 `move_enemy_to` 口径）。
#[test]
fn sys_spell_begin_owner_stage_faults() {
    let (mut w, ecl) = fresh();
    let mut task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    let args = [0, 1, -1, 100, 1000, 0, 0];
    let r = call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.spells[0].active, 0, "misuse Fault：零副作用");
}

/// 快乐路径：owner=ENEMY 成功 → 槽 active + 模式任务已 spawn，`spell_bound==slot+1`，
/// owner 绑定新任务到 boss（"生"，spec §2.1）。
#[test]
fn sys_spell_begin_success_spawns_pattern_task_bound_to_slot() {
    let mut w = World::new(1);
    let ecl = async_pattern_image();
    let (boss, mut task) = enemy_task(&mut w, 1000);
    // 声明序：slot, spell_id, pattern_ref, time_limit, bonus0, flags, threshold
    let args = [0, 5, 1, 100, 1000, 0, 300];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.spells[0].active, 1, "槽应已 active");
    let child = (0..crate::ecl::task::TASK_CAP)
        .find(|&i| w.tasks.is_alive(i))
        .expect("应已 spawn 模式任务");
    assert_eq!(w.tasks.slots[child].owner_kind, OWNER_ENEMY);
    assert_eq!(w.tasks.slots[child].owner_index, boss.index);
    assert_eq!(w.tasks.slots[child].owner_gen, boss.generation);
    assert_eq!(
        w.tasks.slots[child].spell_bound, 1,
        "槽 0 → spell_bound = slot+1 = 1"
    );
    assert_eq!(
        w.tasks.slots[child].spell_epoch, w.body.spells[0].epoch,
        "模式任务应捕获本槽刚铸出的代际戳（ABA 修复，复审 Task 2）"
    );
    assert_ne!(w.body.spells[0].epoch, 0, "首次 begin 后 epoch 应已非零");
}

/// `pattern=none`（负值）→ begin 仍成功但不 spawn 任何任务。
#[test]
fn sys_spell_begin_pattern_none_does_not_spawn() {
    let mut w = World::new(1);
    let ecl = async_pattern_image();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let args = [0, 5, -1, 100, 1000, 0, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.spells[0].active, 1);
    assert_eq!(w.tasks.iter_alive().count(), 0, "pattern=none 不 spawn");
}

/// 坏 `pattern` 号（不在册）→ Fault，先查后建：`spell_begin_internal` 不应已被调用
/// （槽仍空闲，同 `sys_create_bullet` 的坏 task_script 口径）。
#[test]
fn sys_spell_begin_pattern_bad_sub_faults_before_mutating_world() {
    let mut w = World::new(1);
    let ecl = async_pattern_image(); // 只有 raw=1 在册
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let args = [0, 5, 99, 100, 1000, 0, 0]; // pattern_ref=99 不在册
    let r = call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args);
    assert_eq!(r, Err(FAULT_BAD_OP));
    assert_eq!(w.body.spells[0].active, 0, "先查后建：坏号不应已写入槽");
    assert_eq!(w.tasks.iter_alive().count(), 0);
}

/// 槽越界 → P4-b no-op + 计数（`spell_begin_internal` 自身判定，syscall 层直通）。
#[test]
fn sys_spell_begin_slot_oob_is_p4b_noop() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let cv0 = w.body.diag.contract_viol;
    let args = [99, 5, -1, 100, 1000, 0, 0]; // slot=99 越界
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    assert_eq!(w.tasks.iter_alive().count(), 0);
}

/// `threshold > 当前 hp` → P4-b no-op + 计数。
#[test]
fn sys_spell_begin_threshold_over_hp_is_p4b_noop() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 100); // hp=100
    let cv0 = w.body.diag.contract_viol;
    let args = [0, 5, -1, 100, 1000, 0, 200]; // threshold=200 > hp=100
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.spells[0].active, 0);
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
}

/// 该槽已 active（重复 begin 同一槽）→ P4-b no-op + 计数，原槽内容不被覆写。
#[test]
fn sys_spell_begin_duplicate_active_slot_is_p4b_noop() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let args = [0, 5, -1, 100, 1000, 0, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.spells[0].active, 1);

    task.sp = 0;
    let cv0 = w.body.diag.contract_viol;
    let args2 = [0, 6, -1, 100, 1000, 0, 0]; // 同槽再次 begin
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args2).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.spells[0].spell_id, 5, "第一次的槽内容未被覆写");
}

/// 控制器补充决策（复审 T1-m3）：`threshold < 0` → P4-b no-op + 计数，不 begin、不
/// spawn（血线语义非负；世界 API 层仍宽松接受负值供白盒测）。
#[test]
fn sys_spell_begin_negative_threshold_is_p4b_noop_no_spawn() {
    let mut w = World::new(1);
    let ecl = async_pattern_image();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let cv0 = w.body.diag.contract_viol;
    let args = [0, 5, 1, 100, 1000, 0, -1]; // threshold=-1，pattern 合法在册
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    assert_eq!(w.body.spells[0].active, 0, "拒负 threshold：不 begin");
    assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");
}

/// 控制器补充决策（Task 2 复审 Important #2，spec §5 P4-b 拒收条件）：`bonus0 < 0` →
/// P4-b no-op + 计数，不 begin、不 spawn——必须在 `bonus0 as u32` 转型前拒收，否则
/// `-1i32 as u32` 变 `u32::MAX`，等于凭空发 ~42.9 亿分（未拒时的真实后果）。
#[test]
fn sys_spell_begin_negative_bonus0_is_p4b_noop_no_spawn() {
    let mut w = World::new(1);
    let ecl = async_pattern_image();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let cv0 = w.body.diag.contract_viol;
    let args = [0, 5, 1, 100, -1, 0, 0]; // bonus0=-1，pattern 合法在册
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    assert_eq!(w.body.spells[0].active, 0, "拒负 bonus0：不 begin");
    assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");
}

/// 控制器补充决策（Task 2 复审 Important #3，spec §5 P4-b 拒收条件）：`time_limit <= 0`
/// → P4-b no-op + 计数，不 begin、不 spawn——负值经 `as u16` 抹符号会变成巨大正数
/// （如 `-1 → 65535`），绕过 `spell_begin_internal` 内"`time_limit == 0`"那条守卫，
/// 未拒时会带着离谱的时限成功 begin。
#[test]
fn sys_spell_begin_non_positive_time_limit_is_p4b_noop_no_spawn() {
    let mut w = World::new(1);
    let ecl = async_pattern_image();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let cv0 = w.body.diag.contract_viol;
    let args = [0, 5, 1, -1, 1000, 0, 0]; // time_limit=-1，pattern 合法在册
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    assert_eq!(w.body.spells[0].active, 0, "拒非正 time_limit：不 begin");
    assert_eq!(w.tasks.iter_alive().count(), 0, "不 spawn");

    // 对照：time_limit=0 同样应被拒（非仅负值——`<= 0` 覆盖两态）。
    task.sp = 0;
    let cv1 = w.body.diag.contract_viol;
    let args0 = [0, 5, 1, 0, 1000, 0, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args0).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv1 + 1);
    assert_eq!(w.body.spells[0].active, 0, "time_limit==0 同样拒收");
}

/// 控制器补充决策（复审 T1-m1）：该 boss 已绑到另一 active 槽 → P4-b no-op + 计数
/// （单卡/boss 是设计用法；同 boss 连续 begin 两个不同槽应在第二次被拒）。
#[test]
fn sys_spell_begin_boss_already_bound_to_another_slot_is_p4b_noop() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let args0 = [0, 5, -1, 100, 1000, 0, 0];
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args0).is_ok());
    assert_eq!(w.body.spells[0].active, 1);

    task.sp = 0;
    let cv0 = w.body.diag.contract_viol;
    let args1 = [1, 6, -1, 100, 1000, 0, 0]; // 同 boss，另一空闲槽 1
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_BEGIN, &args1).is_ok());
    assert_eq!(
        w.body.diag.contract_viol,
        cv0 + 1,
        "同 boss 二次绑定应计数拒绝"
    );
    assert_eq!(w.body.spells[1].active, 0, "槽 1 不应被写入");
}

/// 逃生舱口：绑定 owner 调用 → 槽走 HP 路径结算清空。
#[test]
fn sys_spell_end_bound_owner_settles_slot() {
    let (mut w, ecl) = fresh();
    let (boss, mut task) = enemy_task(&mut w, 1000);
    assert!(w.body.spell_begin_internal(0, boss, 5, 100, 1000, 0, 0));
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_END, &[]).is_ok());
    assert_eq!(w.body.spells[0].active, 0, "逃生舱口应已结算清槽");
}

/// 无绑定 → no-op，且不计 contract_viol（重复调用安全，同世界 API 文档）。
#[test]
fn sys_spell_end_unbound_owner_is_noop() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    let cv0 = w.body.diag.contract_viol;
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_END, &[]).is_ok());
    assert_eq!(w.body.diag.contract_viol, cv0, "无绑定重复调用安全，不计数");
}

/// 绑定槽 → 压 `frames_left`（begin 当刻即 `time_limit`）。
#[test]
fn sys_spell_timer_bound_pushes_frames_left() {
    let (mut w, ecl) = fresh();
    let (boss, mut task) = enemy_task(&mut w, 1000);
    assert!(w.body.spell_begin_internal(0, boss, 5, 100, 1000, 0, 0));
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
    assert_eq!(task.stack[0], 100);
}

/// 无绑定（owner=ENEMY 但从未 begin）→ 押 -1。
#[test]
fn sys_spell_timer_unbound_pushes_neg1() {
    let (mut w, ecl) = fresh();
    let (_boss, mut task) = enemy_task(&mut w, 1000);
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
    assert_eq!(task.stack[0], -1);
}

/// 读族误用降级（同 `SYS_SELF_HP`）：owner 非 ENEMY → 押 -1，不 Fault。
#[test]
fn sys_spell_timer_non_enemy_owner_pushes_neg1_no_fault() {
    let (mut w, ecl) = fresh();
    let mut task = Task {
        owner_kind: OWNER_STAGE,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_SPELL_TIMER, &[]).is_ok());
    assert_eq!(task.stack[0], -1);
}

// ── SYS_ADD_SCORE/SYS_BGM/SYS_BG/SYS_BG_PHASE（整局流程刀 Task 2；5xx 族）────────

#[test]
fn sys_bgm_writes_field_and_emits_req() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_BGM, &[5]).is_ok());
    assert_eq!(w.body.bgm_id, 5);
    let reqs = w.body.take_requests();
    let last = reqs.last().expect("SYS_BGM 必须发一条 req");
    assert_eq!(last.id, crate::consts::REQ_BGM);
    assert_eq!(last.args[0], 5);
}

#[test]
fn sys_bg_phase_stamps_current_frame() {
    let (mut w, ecl) = fresh();
    w.body.frame = 42;
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_BG_PHASE, &[3]).is_ok());
    assert_eq!(w.body.bg_phase, 3);
    assert_eq!(w.body.bg_phase_frame, 42);
    let reqs = w.body.take_requests();
    let last = reqs.last().expect("SYS_BG_PHASE 必须发一条 req");
    assert_eq!(last.id, crate::consts::REQ_BG_PHASE);
}

/// 越界 → P4-b no-op + viol（与 `sys_emit_req` 的 id 收窄同款口径，syscall.rs:622 参照）。
#[test]
fn sys_anchor_out_of_range_is_noop_with_viol() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let cv0 = w.body.diag.contract_viol;
    assert!(call(&mut w, &ecl, &mut task, SYS_BGM, &[-1]).is_ok());
    assert_eq!(w.body.bgm_id, 0, "字段不动");
    assert_eq!(w.body.take_requests().len(), 0, "无 req");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
}

#[test]
fn sys_add_score_saturates_at_zero() {
    let (mut w, ecl) = fresh();
    w.body.players[0].score = 10;
    let mut task = Task::default();

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[-100]).is_ok());
    assert_eq!(w.body.players[0].score, 0, "下溢钳 0，不回绕");

    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[7]).is_ok());
    assert_eq!(w.body.players[0].score, 7);

    w.body.players[0].score = u64::MAX - 1;
    task.sp = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_SCORE, &[100]).is_ok());
    assert_eq!(w.body.players[0].score, u64::MAX, "上溢钳 u64::MAX");
}

// ── SYS_SET_PLAYER_HITBOX（561；判定写口 2026-09-25）──────────────────────────

/// 写的是自机 0 的中弹半径（不碰擦弹半径）；钳上界计违约。判别力：写 5 与写 5000 各一腿。
#[test]
fn sys_set_player_hitbox_writes_hit_radius_and_clamps() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let graze = w.body.players[0].graze_radius;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_PLAYER_HITBOX,
            &[Fx::from_int(5).raw()]
        )
        .is_ok()
    );
    assert_eq!(w.body.players[0].hit_radius, Fx::from_int(5));
    assert_eq!(w.body.players[0].graze_radius, graze, "擦弹半径不动");
    let viol = w.body.diag.contract_viol;
    assert!(
        call(
            &mut w,
            &ecl,
            &mut task,
            SYS_SET_PLAYER_HITBOX,
            &[Fx::from_int(5000).raw()]
        )
        .is_ok()
    );
    assert_eq!(
        w.body.players[0].hit_radius,
        crate::world::MAX_ENTITY_RADIUS
    );
    assert_eq!(w.body.diag.contract_viol, viol + 1);
}

// ── SYS_TIME_STOP_PLAYER（560；自机能力刀 T6）───────────────────────────

/// 写 freeze_left[1]（冻 A+B），不碰 [0]。
#[test]
fn sys_time_stop_player_writes_the_cutscene_slot() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
    assert_eq!(w.body.freeze_left, [0, 90]);
}

/// 重入取**覆盖**（后写为准），不取最大、不叠加：脚本是权威，覆盖可预测。
/// 判别力：先写大值再写小值——取最大或叠加的实现在这里会给出 ≥ 90 的值。
#[test]
fn sys_time_stop_player_reentry_overwrites() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[90]).is_ok());
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[10]).is_ok());
    assert_eq!(w.body.freeze_left[1], 10, "后写为准");
}

/// `time_stop_player(0)` = **立即解除**，天然的取消 API（不另开 syscall）。
#[test]
fn sys_time_stop_player_zero_cancels() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.freeze_left[1] = 50;
    assert!(call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[0]).is_ok());
    assert_eq!(w.body.freeze_left[1], 0);
}

/// 参数收窄照 D19 判例：`u16::try_from` 失败 ⇒ P4-b **整条 no-op**、计 contract_viol
/// 与 BAD_ARGS，**不钳位、不 Fault**。判别力：负值与 65536 各一腿——钳位实现会给出
/// 0 或 65535 而非"原值不动"。
#[test]
fn sys_time_stop_player_rejects_out_of_range() {
    for bad in [-1i32, 65536] {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        w.body.freeze_left[1] = 7;
        let cv0 = w.body.diag.contract_viol;
        assert!(
            call(&mut w, &ecl, &mut task, SYS_TIME_STOP_PLAYER, &[bad]).is_ok(),
            "P4-b 是安全结果不是 Fault"
        );
        assert_eq!(
            w.body.freeze_left[1], 7,
            "越界 {bad} 必须整条 no-op（不钳位）"
        );
        assert_eq!(
            w.body.diag.contract_viol,
            cv0 + 1,
            "越界 {bad} 须计一次违约"
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
    }
}

// ── SYS_ADD_LIVES/SYS_ADD_BOMBS/SYS_ADD_POWER（B20；账面增量 setter）──────────

/// B20：`add_lives(d)` 增量记账，双边钳位（P4-b），不回绕不 panic。
#[test]
fn add_lives_clamps_both_ends() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.players[0].lives = 3;

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[1]).is_ok());
    assert_eq!(w.body.players[0].lives, 4, "正增");

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[-1]).is_ok());
    assert_eq!(w.body.players[0].lives, 3, "负减");

    w.body.players[0].lives = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[-1]).is_ok());
    assert_eq!(w.body.players[0].lives, 0, "下钳 0，不回绕成 255");

    w.body.players[0].lives = u8::MAX;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_LIVES, &[1]).is_ok());
    assert_eq!(w.body.players[0].lives, u8::MAX, "上钳 u8::MAX，不回绕成 0");
}

/// `add_bombs(d)`：钳 `[0, STOP_STOCK_MAX=5]`（玩法刀；原 `[0,255]`）。判别腿：写错字段会让 lives 动。
#[test]
fn add_bombs_clamps_both_ends() {
    use crate::player::STOP_STOCK_MAX;
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.players[0].bombs = 3;
    let lives0 = w.body.players[0].lives;

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[1]).is_ok());
    assert_eq!(w.body.players[0].bombs, 4, "正增");
    assert_eq!(w.body.players[0].lives, lives0, "判别腿：不得误写 lives");

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
    assert_eq!(w.body.players[0].bombs, 3, "负减");

    w.body.players[0].bombs = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[-1]).is_ok());
    assert_eq!(w.body.players[0].bombs, 0, "下钳 0，不回绕成 255");

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[9]).is_ok());
    assert_eq!(w.body.players[0].bombs, STOP_STOCK_MAX, "上钳 5");
}

/// `power` 上限是 `POWER_MAX`（400，= 显示 4.00），**不是 `u16::MAX`**——判别腿：
/// 钳错成 `u16::MAX` 时 401 会被放行，`power_tier` 索引随后 OOB（见 `set_player_power` 文档）。
#[test]
fn add_power_clamps_to_power_max_not_u16_max() {
    use crate::items::POWER_MAX;

    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    w.body.players[0].power = 100;

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[50]).is_ok());
    assert_eq!(w.body.players[0].power, 150, "正增");

    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[-50]).is_ok());
    assert_eq!(w.body.players[0].power, 100, "负减");

    w.body.players[0].power = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[-1]).is_ok());
    assert_eq!(w.body.players[0].power, 0, "下钳 0");

    w.body.players[0].power = POWER_MAX;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[1]).is_ok());
    assert_eq!(
        w.body.players[0].power, POWER_MAX,
        "上钳 POWER_MAX(400)，不是 u16::MAX"
    );

    // 一步跨过上限也得钳住（不是"只在恰好 +1 时钳"）。
    w.body.players[0].power = 0;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[10_000]).is_ok());
    assert_eq!(w.body.players[0].power, POWER_MAX, "一步跨越同样钳 400");
}

/// P4-b：脚本可以 push 任意 `i32`，极值 delta 不得让中间量溢出（debug 下裸 `+` 会 panic）。
/// 三个 setter × `{i32::MAX, i32::MIN}` 全走一遍：不 panic、不 Fault，且落在各自钳位边界上。
#[test]
fn add_counters_survive_extreme_deltas() {
    use crate::items::POWER_MAX;

    let (mut w, ecl) = fresh();
    let mut task = Task::default();

    for &no in &[SYS_ADD_LIVES, SYS_ADD_BOMBS] {
        w.body.players[0].lives = 200;
        w.body.players[0].bombs = 200;
        assert!(call(&mut w, &ecl, &mut task, no, &[i32::MAX]).is_ok());
        let v = if no == SYS_ADD_LIVES {
            w.body.players[0].lives
        } else {
            w.body.players[0].bombs
        };
        let cap = if no == SYS_ADD_LIVES {
            u8::MAX
        } else {
            crate::player::STOP_STOCK_MAX
        };
        assert_eq!(v, cap, "syscall {no}：i32::MAX 应钳到上限");

        assert!(call(&mut w, &ecl, &mut task, no, &[i32::MIN]).is_ok());
        let v = if no == SYS_ADD_LIVES {
            w.body.players[0].lives
        } else {
            w.body.players[0].bombs
        };
        assert_eq!(v, 0, "syscall {no}：i32::MIN 应钳到 0");
    }

    w.body.players[0].power = 200;
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[i32::MAX]).is_ok());
    assert_eq!(
        w.body.players[0].power, POWER_MAX,
        "i32::MAX 应钳到 POWER_MAX"
    );
    assert!(call(&mut w, &ecl, &mut task, SYS_ADD_POWER, &[i32::MIN]).is_ok());
    assert_eq!(w.body.players[0].power, 0, "i32::MIN 应钳到 0");
}

/// 430 / 722 self-only：owner 非 ENEMY → Fault（同 400 族误用策略）；栈不留垃圾。
#[test]
fn set_anm_state_and_fx_on_fault_on_non_enemy_owner() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default(); // STAGE owner
    assert_eq!(
        call(&mut w, &ecl, &mut task, SYS_SET_ANM_STATE, &[1]),
        Err(FAULT_BAD_OP)
    );
    let mut task = Task::default();
    assert_eq!(
        call(&mut w, &ecl, &mut task, SYS_FX_ON, &[1, 2]),
        Err(FAULT_BAD_OP)
    );
    assert!(w.body.take_requests().is_empty(), "Fault 路径不得发请求");
}

/// 721：布局钉死 `REQ_FX_AT, [x raw, y raw, kind, param, 0, 0]`；owner 无限制。
#[test]
fn fx_at_emits_pinned_request_layout() {
    let (mut w, ecl) = fresh();
    let mut task = Task::default();
    let args = [Fx::from_int(12).raw(), Fx::from_int(-34).raw(), 7, 99];
    assert!(call(&mut w, &ecl, &mut task, SYS_FX_AT, &args).is_ok());
    let r = w.body.take_requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].id, crate::consts::REQ_FX_AT);
    assert_eq!(
        r[0].args,
        [Fx::from_int(12).raw(), Fx::from_int(-34).raw(), 7, 99, 0, 0]
    );
    assert_eq!(task.sp, 0, "无返回值，栈应回到空");
}

/// 722：布局钉死 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`，句柄取自 owner。
#[test]
fn fx_on_emits_owner_handle_as_two_raw_slots() {
    let (mut w, ecl) = fresh();
    let eh = crate::world::test_support::spawn_enemy(&mut w, 0, 0, 5);
    let mut task = Task {
        owner_kind: OWNER_ENEMY,
        owner_index: eh.index,
        owner_gen: eh.generation,
        ..Task::default()
    };
    assert!(call(&mut w, &ecl, &mut task, SYS_FX_ON, &[5, -8]).is_ok());
    let r = w.body.take_requests();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].id, crate::consts::REQ_FX_ATTACHED);
    assert_eq!(
        r[0].args,
        [eh.index as i32, eh.generation as i32, 5, -8, 0, 0]
    );
}

// ── boss 换段与敌人钩子刀 Task 3：spell_result 131 / clear_bullets_at 541 ──────────

/// 131 `spell_result`：读槽值；越界 / 负槽号押 0 + 违约。owner 无限制（STAGE 可读）。
#[test]
fn spell_result_reads_slot_and_rejects_out_of_range() {
    let (mut w, ecl) = fresh();
    w.body.spell_last_result[1] = crate::spell::SPELL_END_TIMEOUT;
    let mut stage = Task::default();
    assert!(call(&mut w, &ecl, &mut stage, SYS_SPELL_RESULT, &[1]).is_ok());
    assert_eq!(stage.stack[0], crate::spell::SPELL_END_TIMEOUT as i32);
    for bad in [-1, crate::boss::MAX_BOSSES as i32] {
        let mut t = Task::default();
        let v0 = w.body.diag.contract_viol;
        assert!(call(&mut w, &ecl, &mut t, SYS_SPELL_RESULT, &[bad]).is_ok());
        assert_eq!(t.stack[0], 0);
        assert_eq!(w.body.diag.contract_viol, v0 + 1);
    }
}
