use super::*;

// ── 敌号编解码（敌句柄打包刀 2026-07-31；六处产/消口共用，别各写各的）──────────
//
// 打包编码 [`crate::enemy::pack_handle`] 2026-09-15 起公开于 `enemy` 模块（stg-rl
// 观测编码复用同一约定），本文件只保留解码侧 `resolve_enemy_handle`。

/// 打包敌号 → 活槽索引；`None` = 无效（负值哨兵 / 越界 / 死槽 / **generation 不符**）。
///
/// 判据 = `packed >= 0 && idx < CAP && is_alive(idx) && (generation[idx] & 0x7FFF) == g`。
/// 前三条与打包前逐字相同（含 dying 语义：`is_alive` 是占用位，`ENEMY_DYING` 只是 flag，
/// 槽活到相位 9 才回收），第四条是本刀新增的那条——槽回收再复用之后旧句柄不再指向占了
/// 这个槽的另一只敌。
///
/// ⚠️ **两边都要 `& 0x7FFF`**：池的 `generation` 是完整 `u16` 而句柄只带低 15 位。
/// 拿 `generation[idx] == g` 直接比，gen 一旦越过 `0x7FFF` 就永远比不中——那会变成
/// "敌活着但所有读口都说它没了"，且要跑 32768 次同槽复用才撞得到（钉在
/// `packed_handle_stays_non_negative_and_resolves_with_a_high_generation`）。
pub(super) fn resolve_enemy_handle(packed: i32, ctx: &VmCtx) -> Option<usize> {
    if packed < 0 {
        return None;
    }
    let idx = (packed & 0xFFFF) as usize;
    let g = ((packed >> 16) & 0x7FFF) as u16;
    if idx < crate::enemy::EnemyPool::CAP
        && ctx.body.enemies.is_alive(idx)
        && (ctx.body.enemies.generation[idx] & 0x7FFF) == g
    {
        Some(idx)
    } else {
        None
    }
}

/// `SYS_ENEMY_HP`（100；A5 补遗）：1 参 `handle`（**打包敌号**，含 generation——槽复用后
/// 旧句柄可辨，见 [`resolve_enemy_handle`]；无效句柄仍是读族误用降级口径，不 Fault）。
/// 活敌返当前 hp；越界/死槽/gen 不符/负值一律 -1——stage 编排"等 boss 死"的轮询原语。
pub(super) fn sys_enemy_hp(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let hp = match resolve_enemy_handle(handle, ctx) {
        Some(idx) => ctx.body.enemies.hp[idx],
        None => -1,
    };
    push(task, hp)
}

/// `SYS_ENEMY_X`(101)/`SYS_ENEMY_Y`(102) 共享的读法——存活判据逐字同 `sys_enemy_hp`
/// （同一个 [`resolve_enemy_handle`]；dying 仍算活），只有降级值不同（坐标无哨兵位可用
/// → `Fx::ZERO`，理由见 [`SYS_ENEMY_X`] 号表注释）。`want_y` 选轴：`false`=x，`true`=y。
pub(super) fn sys_enemy_pos(task: &mut Task, ctx: &mut VmCtx, want_y: bool) -> Result<(), u8> {
    let handle = pop(task)?;
    let v = match resolve_enemy_handle(handle, ctx) {
        Some(idx) => {
            if want_y {
                ctx.body.enemies.y[idx]
            } else {
                ctx.body.enemies.x[idx]
            }
        }
        None => Fx::ZERO,
    };
    push(task, v.raw())
}

pub(super) fn sys_nearest_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let y = pop(task)?;
    let x = pop(task)?;
    // 世界侧返的本来就是**完整句柄**——打包刀之前这里只押 `h.index`、把 gen 丢了。
    let handle = match ctx.body.nearest_enemy(Fx::from_raw(x), Fx::from_raw(y)) {
        Some(h) => crate::enemy::pack_handle(h),
        None => -1,
    };
    push(task, handle)
}

/// `SYS_ENEMY_ALIVE`(103)：存活判据**逐字同** `sys_enemy_hp`/`sys_enemy_pos`（同一个
/// [`resolve_enemy_handle`]：负句柄/越界/死槽/gen 不符 → 0；`ENEMY_DYING` 仍算活——
/// `is_alive` 是存活位，dying 只是 flag，槽活到相位 9 才回收），只是把那个判据**本身**
/// 押出去而不是拿它选一个值。
///
/// 存在的理由见 [`SYS_ENEMY_ALIVE`] 号表注释：`enemy_hp(e) != -1` 这个旧探针在"活敌血量
/// 恰为 −1"那一格会误判，专用口没有这条缝。
pub(super) fn sys_enemy_alive(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let alive = resolve_enemy_handle(handle, ctx).is_some();
    push(task, if alive { 1 } else { 0 })
}

pub(super) fn sys_set_invuln(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let n = pop(task)?;
    let Ok(frames) = u16::try_from(n) else {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    };
    ctx.body.set_enemy_invuln(h, frames);
    Ok(())
}

pub(super) fn sys_set_hitbox(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let r = pop(task)?;
    ctx.body.set_enemy_hitbox(h, Fx::from_raw(r));
    Ok(())
}

pub(super) fn sys_set_hurtbox(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let r = pop(task)?;
    ctx.body.set_enemy_hurtbox(h, Fx::from_raw(r));
    Ok(())
}

pub(super) fn sys_set_enemy_flag(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let on = pop(task)?;
    let mask = pop(task)?;
    const SETTABLE: i32 = (crate::enemy::ENEMY_NO_BODY | crate::enemy::ENEMY_KILLALL_EXEMPT) as i32;
    if mask == 0 || mask & !SETTABLE != 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    ctx.body.set_enemy_flags(h, mask as u8, on != 0);
    Ok(())
}

pub(super) fn sys_drop_clear(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.clear_enemy_drops(h);
    Ok(())
}

pub(super) fn sys_drop_add(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    // 逆序弹出（模块文档"参数传递约定"）：`drop_add(type, n)` 故先 `n` 后 `type`。
    let n = pop(task)?;
    let item_type = pop(task)?;
    ctx.body.add_enemy_drop(h, item_type, n);
    Ok(())
}

pub(super) fn sys_drop_items(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.spill_enemy_drops(h, ctx.tables);
    Ok(())
}

pub(super) fn sys_kill_all_enemies(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let mode = pop(task)?;
    let die = match mode {
        m if m == crate::enemy::KILL_SILENT as i32 => false,
        m if m == crate::enemy::KILL_DIE as i32 => true,
        _ => {
            ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
            ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            return Ok(());
        }
    };
    let except = (task.owner_kind == OWNER_ENEMY).then_some(EnemyHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    });
    ctx.body.kill_all_enemies(except, die, ctx.tables);
    Ok(())
}

pub(super) fn sys_die(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.kill_enemy_by_handle(h, ctx.tables);
    Ok(())
}

/// 敌人创建（SYS 210；A5 乙案，append-only：旧 5 参前缀不动，尾追 sprite/task）：
/// 7 参逆序弹出。半径/受击盒用固定默认值（同 `world::test_support::spawn_enemy` 惯例）；
/// `task` 号先验后建（镜像 `sys_create_bullet` 的 task 路径：坏号/非 0 参 Async → Fault，
/// 零副作用，敌未建）；`main_task` 在敌句柄产出**之后**回填（任务的 owner 三元组需要敌
/// index/gen，敌必须先于任务存在）；`death_script` 仍恒 0（脚本面缺口留 follow-ups）。
pub(super) fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    // boss 换段刀 spec §4.2：压栈序 `x,y,hp,drop,score,sprite,task_script, arg0..arg(n-1), argc`。
    // 门禁全部先于建敌（任一门不过都零副作用）；argc/栈深口径同 `OP_SPAWN`。
    let argc = usize::try_from(pop(task)?)
        .ok()
        .filter(|&n| n <= LOCALS)
        .ok_or(FAULT_STACK)?;
    if (task.sp as usize) < argc + 7 {
        return Err(FAULT_STACK);
    }
    let mut args = [0i32; LOCALS];
    for k in (0..argc).rev() {
        args[k] = pop(task)?;
    }
    let task_script = pop(task)?;
    let sprite = pop(task)?;
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    // task 号先验后建（镜像 sys_create_bullet：坏号 FAULT_BAD_OP，敌未建）：在册 + Async +
    // 形参个数 == argc；none 不许带参。
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async || ctx.ecl.param_types(sub).is_none_or(|p| p.len() != argc)
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else if argc > 0 {
        return Err(FAULT_BAD_OP);
    } else {
        None
    };

    // 掉落表号在**生成时**展开成逐类型计数（此前存表号、死时才查表）。
    // P4-b：越界表号 → 视同空表 + 计数（原检查在 `settle::damage_enemy`，随状态前移）。
    // `contract_viol` 与 `last_status` **两样都写**——邻居的每条 P4-b 都是这个口径
    // （`world::add_enemy_drop` / `world::move_enemy_to` / `sys_spell_begin`），搬到 syscall
    // 层之后不该变成异类。`last_status` 进校验和，故这一处会改世界状态（金向量两侧表号
    // 恒不越界，压不到这条路径，实测逐字节不变）。
    let (drop_count, table_ok) = crate::tables::drop_counts(ctx.tables, drop_table as u16);
    if !table_ok {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
    }

    let init = EnemyInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
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
        hp,
        hp_max: hp,
        radius: Fx::from_int(12),
        hurtbox: Fx::from_int(16),
        invuln: 0,
        hit_flash: 0,
        flags: 0,
        sprite: sprite as u16,
        anm_state: 0,
        anm_state_frame: ctx.body.frame,
        main_task: 0, // 任务 spawn 后回填（敌句柄先于任务存在）
        death_script: 0,
        drop_count,
        score: score as u16,
    };
    let handle = ctx.body.create_enemy(init);
    if handle == EnemyHandle::NULL {
        return push(task, -1);
    }
    push(task, crate::enemy::pack_handle(handle))?;

    if let Some(sub) = task_sub {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），敌已建、句柄已押，不 Fault。
        let pc0 = ctx.ecl.sub_meta(sub).expect("已在上面校验过").code_entry();
        let owner = (OWNER_ENEMY, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(sub, pc0, owner, parent, ctx.frame) {
            Some(slot) => {
                ctx.tasks.write_args(slot, &args[..argc]);
                ctx.body.enemies.main_task[handle.index as usize] = slot as u32 + 1;
            }
            None => {
                ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                    ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// 敌人限时缓动位移（SYS 400）：self owner 必须是 ENEMY（否则 Fault，misuse 策略）；
/// 4 参逆序弹出：`easing, y, x, dur`。无返回值。
pub(super) fn sys_move_enemy_to(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19：越界 dur/easing → P4-b 整条 no-op
    };
    ctx.body
        .move_enemy_to(h, Fx::from_raw(x_raw), Fx::from_raw(y_raw), dur, easing);
    Ok(())
}

/// 运动动词族的 `dur`/`easing` **参数收窄**（D19，2026-09-03 裁定：收窄成拒收）。
///
/// 此前五处一律裸 `as u16` / `as u8`，制造了两个坏路径：
/// - `easing = 256` → `256 as u8 == 0` → **静默变成 Linear**，绕过世界层 `easing >= 8` 的
///   判据（那条判据收到的已经是截断后的值）；写 `easing = 264` 反而落 8、被正确拒掉
///   ——**能不能拒取决于越界值模 256 落在哪里，毫无规律**。
/// - `dur = -1` → `-1 as u16 == 65535` → 一条本该报错的笔误变成"缓动 18 分钟"。
///
/// 现在两个都走 `try_from`：任一失败即 **P4-b**（`contract_viol` +1 + `last_status =
/// BAD_ARGS` + **整条 no-op**），与既有的 `easing >= 8` 是同一条腿，判据顺势前移到收窄这步。
/// 不 Fault——参数笔误属"调用方违约"，确定性安全结果比杀任务有用。
///
/// 返回 `None` 即调用方应当整条 no-op。
fn narrow_dur_easing(ctx: &mut VmCtx, dur: i32, easing: i32) -> Option<(u16, u8)> {
    match (u16::try_from(dur), u8::try_from(easing)) {
        (Ok(d), Ok(e)) => Some((d, e)),
        _ => {
            ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
            ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            None
        }
    }
}

/// `SYS_MOVE_VEL`（410）：逆序弹出 `easing, speed, angle, dur`。
pub(super) fn sys_move_vel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body.set_enemy_vel_polar(
        h,
        crate::math::Angle(angle as u16),
        Fx::from_raw(speed),
        dur,
        easing,
    );
    Ok(())
}

/// `SYS_MOVE_VEL_XY`（411）：逆序弹出 `easing, vy, vx, dur`。
pub(super) fn sys_move_vel_xy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let vy = pop(task)?;
    let vx = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_vel_cart(h, Fx::from_raw(vx), Fx::from_raw(vy), dur, easing);
    Ok(())
}

/// `SYS_MOVE_ANGLE`（420）：逆序弹出 `easing, angle, dur`。
pub(super) fn sys_move_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let angle = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_angle(h, crate::math::Angle(angle as u16), dur, easing);
    Ok(())
}

/// `SYS_MOVE_SPEED`（421）：逆序弹出 `easing, speed, dur`。
pub(super) fn sys_move_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let speed = pop(task)?;
    let dur = pop(task)?;
    let Some((dur, easing)) = narrow_dur_easing(ctx, dur, easing) else {
        return Ok(()); // D19
    };
    ctx.body
        .set_enemy_speed(h, Fx::from_raw(speed), dur, easing);
    Ok(())
}

/// 敌人表现状态号（[`SYS_SET_ANM_STATE`]=430）：1 参逆序弹出；self-only（非敌 → Fault）。
/// 悬垂 owner 句柄由世界层 `set_anm_state` 计 P4-b（no-op + `STALE_HANDLE`）。
pub(super) fn sys_set_anm_state(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let state = pop(task)?;
    ctx.body.set_anm_state(h, state as u16);
    Ok(())
}
