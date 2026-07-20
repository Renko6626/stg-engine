//! ecl/syscall.rs —— syscall 号表 v1（冻结起点）+ 绑定层派发（M1 T3）。
//!
//! **参数传递约定**（spec/plan 既定）：脚本按声明顺序**正序压栈**（param1, param2, ...,
//! paramN）；栈是 LIFO，故 `dispatch` 按**逆序弹出**（paramN 先弹 ... param1 后弹）——
//! 这与普通函数调用栈约定一致，不是额外规则。读类 syscall（`frame`/`player_x`/`self_x`/...）
//! 执行后压回 1 个返回值；创建类（`create_bullet`/`create_bullets_batch`/`spawn_enemy`/
//! `drop_item`）压 1 个返回值（句柄 index 转 i32，失败 -1；批量创建压实发数）；写类
//! （`set_var`/`move_enemy_to`/`boss_set`/`pulse_signal`/弹 setter 族）不压值。
//!
//! **坏号/参数不足（栈下溢）→ Fault**：`dispatch` 返回 `Result<(), u8>`，`Err(code)` 由
//! `vm::exec` 的 `OP_SYS` 分支转成 `Exec::Fault(code)`；坏 syscall 号统一复用
//! `vm::FAULT_BAD_OP`（同 `OP_SPAWN` 坏脚本号的处置口径）、栈下溢复用 `vm::FAULT_STACK`。
//!
//! **误用策略拍板（T3 worker 决定，记入报告）**：`move_enemy_to`/弹 setter族/`aim_player_angle`
//! 依赖"self 是某种 owner"的隐含契约（`move_enemy_to`/弹 setter族要求 owner 恰为对应池），
//! owner 类型不符 → **视为脚本作者违约，`Fault`**（响亮报错，非静默 no-op）——与
//! `docs/superpowers/sdd/task-3-brief.md` 建议一致："pick Fault unless codebase friction
//! suggests otherwise"，本仓一路复用既有 `FAULT_BAD_OP` 无额外摩擦，故采纳。`boss_set` 例外：
//! owner 非 ENEMY 时 `enemy` 字段写 `EnemyHandle::NULL`（不 Fault）——`boss_set` 语义本就
//! 允许 UI 槽先于 boss 敌人存在而调用（`world::boss_set` 文档："不校验 ui.enemy 句柄有效性"）。

use crate::boss::BossUiSlot;
use crate::bullets::{BulletHandle, BulletInit};
use crate::ecl::task::{LOCALS, OWNER_BULLET, OWNER_ENEMY, Task};
use crate::ecl::vm::{FAULT_BAD_OP, FAULT_STACK, VmCtx};
use crate::enemy::{EnemyHandle, EnemyInit};
use crate::math::geom::polar_to_vec;
use crate::math::{Angle, Fx};
use crate::xform::XformSlot;

// ── syscall 号表 v1（编号即契约；冻结纪律同 op 表——族内留空隙，新增落族内）───────

// 0x：读——世界/自机/随机/变量
pub const SYS_FRAME: u16 = 0;
pub const SYS_PLAYER_X: u16 = 1;
pub const SYS_PLAYER_Y: u16 = 2;
pub const SYS_SELF_X: u16 = 3;
pub const SYS_SELF_Y: u16 = 4;
pub const SYS_SELF_HP: u16 = 5;
pub const SYS_RAND_RANGE: u16 = 6;
pub const SYS_GET_VAR: u16 = 7;
pub const SYS_SET_VAR: u16 = 8;
/// 任务龄（帧数，**M1.5 新增**）：`ctx.frame - task.born_frame`（wrapping）。**语义故意偏离
/// ZUN**（ZUN `-9988` 是"敌出生以来帧数"，只对敌有意义）——我们量的是**任务**的龄，不是
/// owner 实体的龄：零新状态（复用既有 `Task.born_frame`/`ctx.frame`），对全部 owner 种类
/// （含 STAGE）均有意义。见 `docs/ecl-ops.md`/`docs/zun-ecl-v2-reference.md` 的偏离记档。
pub const SYS_SELF_AGE: u16 = 9;
/// owner 上限血量（M1.5 新增）：owner=ENEMY → `enemies.hp_max[idx]`；非敌 → 押 0
/// （同 `SYS_SELF_HP` 误用策略：静默降级，不 Fault）。
pub const SYS_SELF_HP_MAX: u16 = 10;

// 2x：写——创建/世界变更
/// 丙方案 8 参（正序压栈）：`appearance, x, y, speed, angle, xform_off, xform_cnt, task_script`。
pub const SYS_CREATE_BULLET: u16 = 20;
/// 9 参：`appearance, x, y, n_angle, angle0, angle_step, n_speed, speed0, speed_step`（无 xform）。
pub const SYS_CREATE_BULLETS_BATCH: u16 = 21;
/// v1 直参 5 个：`x, y, hp, drop_table, score`（appearance 敌表后补，见 follow-ups）。
pub const SYS_SPAWN_ENEMY: u16 = 22;
/// 3 参：`x, y, item_type`。
pub const SYS_DROP_ITEM: u16 = 23;
/// self owner 敌；4 参：`dur, x, y, easing`。
pub const SYS_MOVE_ENEMY_TO: u16 = 24;
/// slot + 5 字段（`enemy` 取自 self owner，非显式参）：
/// `slot, hp_ratio, spell_id, timer_frames, phase_left, active`。
pub const SYS_BOSS_SET: u16 = 25;
/// 1 参：`ch`。
pub const SYS_PULSE_SIGNAL: u16 = 26;

// 3x：写——弹 setter 族（self owner 必须是 BULLET；按 motion.rs 九连顺序编号）
pub const SYS_SET_BULLET_SPEED: u16 = 30;
pub const SYS_SET_BULLET_ANGLE: u16 = 31;
pub const SYS_TURN_BULLET: u16 = 32;
pub const SYS_SET_BULLET_VEL: u16 = 33;
pub const SYS_SET_BULLET_ANG_VEL: u16 = 34;
pub const SYS_SET_BULLET_ACCEL: u16 = 35;
pub const SYS_SET_BULLET_GRAVITY: u16 = 36;
pub const SYS_STOP_BULLET_FX: u16 = 37;
pub const SYS_AIM_BULLET_AT_PLAYER: u16 = 38;

// 4x：读——瞄准
/// 0 参：读 self 位置 → 朝向 P0 的角度（BAM，供脚本自算瞄准环）。
pub const SYS_AIM_PLAYER_ANGLE: u16 = 40;

// ── 求值栈存取（供各 syscall 实现复用；语义同 vm::exec 内的 pop!/push! 宏）───────

fn pop(task: &mut Task) -> Result<i32, u8> {
    if task.sp == 0 {
        return Err(FAULT_STACK);
    }
    task.sp -= 1;
    Ok(task.stack[task.sp as usize])
}

fn push(task: &mut Task, v: i32) -> Result<(), u8> {
    if task.sp as usize >= crate::ecl::task::EVAL_DEPTH {
        return Err(FAULT_STACK);
    }
    task.stack[task.sp as usize] = v;
    task.sp += 1;
    Ok(())
}

/// self 位置（owner 已被调度层 gate 校验存活——见 `vm::run_tasks` 文档）：
/// ENEMY→敌池位置 / BULLET→弹池位置 / STAGE→(0,0)。
fn self_pos(task: &Task, ctx: &VmCtx) -> (Fx, Fx) {
    match task.owner_kind {
        OWNER_ENEMY => {
            let i = task.owner_index as usize;
            (ctx.body.enemies.x[i], ctx.body.enemies.y[i])
        }
        OWNER_BULLET => {
            let i = task.owner_index as usize;
            (ctx.body.bullets.x[i], ctx.body.bullets.y[i])
        }
        _ => (Fx::ZERO, Fx::ZERO),
    }
}

fn self_hp(task: &Task, ctx: &VmCtx) -> i32 {
    match task.owner_kind {
        OWNER_ENEMY => ctx.body.enemies.hp[task.owner_index as usize],
        _ => 0,
    }
}

/// owner 上限血量：ENEMY→`hp_max`／非敌恒 0（`SYS_SELF_HP_MAX`，M1.5；镜像 `self_hp`）。
fn self_hp_max(task: &Task, ctx: &VmCtx) -> i32 {
    match task.owner_kind {
        OWNER_ENEMY => ctx.body.enemies.hp_max[task.owner_index as usize],
        _ => 0,
    }
}

/// self owner 必须是 BULLET，否则脚本作者违约 → `Fault`（misuse 策略，见模块文档）。
fn self_bullet_handle(task: &Task) -> Result<BulletHandle, u8> {
    if task.owner_kind != OWNER_BULLET {
        return Err(FAULT_BAD_OP);
    }
    Ok(BulletHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    })
}

/// self owner 必须是 ENEMY，否则脚本作者违约 → `Fault`（misuse 策略，见模块文档）。
fn self_enemy_handle(task: &Task) -> Result<EnemyHandle, u8> {
    if task.owner_kind != OWNER_ENEMY {
        return Err(FAULT_BAD_OP);
    }
    Ok(EnemyHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    })
}

/// 号表派发。坏号/参数不足 → `Err(fault_code)`（由 `vm::exec` 转 `Exec::Fault`）。
pub(crate) fn dispatch(no: u16, task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    match no {
        SYS_FRAME => push(task, ctx.frame as i32),
        SYS_PLAYER_X => push(task, ctx.body.players[0].x.raw()),
        SYS_PLAYER_Y => push(task, ctx.body.players[0].y.raw()),
        SYS_SELF_X => {
            let (x, _) = self_pos(task, ctx);
            push(task, x.raw())
        }
        SYS_SELF_Y => {
            let (_, y) = self_pos(task, ctx);
            push(task, y.raw())
        }
        SYS_SELF_HP => {
            let hp = self_hp(task, ctx);
            push(task, hp)
        }
        SYS_RAND_RANGE => sys_rand_range(task, ctx),
        SYS_GET_VAR => {
            let slot = pop(task)? as u16;
            let v = ctx.body.get_var(slot);
            push(task, v)
        }
        SYS_SET_VAR => {
            let val = pop(task)?;
            let slot = pop(task)? as u16;
            // globals 段纪律（甲案，M1.5）：脚本写系统段（slot < GLOBALS_SYS_SEGMENT）→
            // no-op + contract_viol 计数（P4-b 确定性安全结果，不 Fault——见
            // `world::GLOBALS_SYS_SEGMENT` 文档）。世界 API `set_var` 不经此门，见调用方。
            if slot < crate::world::GLOBALS_SYS_SEGMENT {
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            } else {
                ctx.body.set_var(slot, val);
            }
            Ok(())
        }
        SYS_SELF_AGE => {
            let age = ctx.frame.wrapping_sub(task.born_frame) as i32;
            push(task, age)
        }
        SYS_SELF_HP_MAX => {
            let hp_max = self_hp_max(task, ctx);
            push(task, hp_max)
        }
        SYS_CREATE_BULLET => sys_create_bullet(task, ctx),
        SYS_CREATE_BULLETS_BATCH => sys_create_bullets_batch(task, ctx),
        SYS_SPAWN_ENEMY => sys_spawn_enemy(task, ctx),
        SYS_DROP_ITEM => sys_drop_item(task, ctx),
        SYS_MOVE_ENEMY_TO => sys_move_enemy_to(task, ctx),
        SYS_BOSS_SET => sys_boss_set(task, ctx),
        SYS_PULSE_SIGNAL => {
            let ch = pop(task)?;
            ctx.body.pulse_signal(ch as usize);
            Ok(())
        }
        SYS_SET_BULLET_SPEED => {
            let h = self_bullet_handle(task)?;
            let speed = pop(task)?;
            ctx.body.set_bullet_speed(h, Fx::from_raw(speed));
            Ok(())
        }
        SYS_SET_BULLET_ANGLE => {
            let h = self_bullet_handle(task)?;
            let a = pop(task)?;
            ctx.body.set_bullet_angle(h, bam(a));
            Ok(())
        }
        SYS_TURN_BULLET => {
            let h = self_bullet_handle(task)?;
            let d = pop(task)?;
            ctx.body.turn_bullet(h, bam(d));
            Ok(())
        }
        SYS_SET_BULLET_VEL => {
            let h = self_bullet_handle(task)?;
            let vy = pop(task)?;
            let vx = pop(task)?;
            ctx.body
                .set_bullet_vel(h, Fx::from_raw(vx), Fx::from_raw(vy));
            Ok(())
        }
        SYS_SET_BULLET_ANG_VEL => {
            let h = self_bullet_handle(task)?;
            let w = pop(task)?;
            ctx.body.set_bullet_ang_vel(h, w as i16);
            Ok(())
        }
        SYS_SET_BULLET_ACCEL => {
            let h = self_bullet_handle(task)?;
            let a = pop(task)?;
            ctx.body.set_bullet_accel(h, Fx::from_raw(a));
            Ok(())
        }
        SYS_SET_BULLET_GRAVITY => {
            let h = self_bullet_handle(task)?;
            let ay = pop(task)?;
            let ax = pop(task)?;
            ctx.body
                .set_bullet_gravity(h, Fx::from_raw(ax), Fx::from_raw(ay));
            Ok(())
        }
        SYS_STOP_BULLET_FX => {
            let h = self_bullet_handle(task)?;
            ctx.body.stop_bullet_fx(h);
            Ok(())
        }
        SYS_AIM_BULLET_AT_PLAYER => {
            let h = self_bullet_handle(task)?;
            let d = pop(task)?;
            ctx.body.aim_bullet_at_player(h, bam(d));
            Ok(())
        }
        SYS_AIM_PLAYER_ANGLE => sys_aim_player_angle(task, ctx),
        _ => Err(FAULT_BAD_OP),
    }
}

/// 栈顶 i32 的低 16 位 → BAM（同 `OP_SINB`/`OP_COSB` 的取角惯例）。
#[inline]
fn bam(raw: i32) -> Angle {
    Angle((raw as u32 & 0xFFFF) as u16)
}

/// `n<=0` → 押 0、不消耗世界 RNG 流（拍板：n==0 既定钉死，n<0 视同"无合法范围"同律扩展、
/// 同样不消耗——一次坏参不该让世界 RNG 流分叉）。
fn sys_rand_range(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let n = pop(task)?;
    let v = if n <= 0 {
        0
    } else {
        ctx.body.rng.rand_range(n as u32) as i32
    };
    push(task, v)
}

/// 丙方案 create_bullet（SYS 20）：8 参逆序弹出；appearance 越界/xform 区间越界 → Fault；
/// task_script ≥0 时脚本号必须在册（否则 Fault，同 `OP_SPAWN` 口径）——**先查后建**：
/// 一切校验在任何世界写之前完成，避免"弹已建、任务绑定失败"的半成品状态。
/// 创建失败（appearance/xform 校验过关后，池满或 xform 内容仍被 `create_bullet_with_xform`
/// 拒绝）→ 押 -1，不派生任务。
fn sys_create_bullet(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let task_script = pop(task)?;
    let xform_cnt = pop(task)?;
    let xform_off = pop(task)?;
    let angle_raw = pop(task)?;
    let speed_raw = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let appearance = pop(task)?;

    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };

    // xform 区间越界（LOCALS 边界）→ Fault；内容层面的坏 xform（>16 槽/未知 op/...）留给
    // `create_bullet_with_xform` 自己的 P4-b 处置（NULL + BAD_ARGS，见下方"押 -1"分支）。
    let mut xform_buf = [XformSlot::default(); 16];
    let xform_len: usize = if xform_cnt == 0 {
        0
    } else {
        if xform_cnt < 0 || xform_cnt as usize > 16 || xform_off < 0 {
            return Err(FAULT_BAD_OP);
        }
        let cnt = xform_cnt as usize;
        let off = xform_off as usize;
        let Some(end) = off.checked_add(cnt * 3) else {
            return Err(FAULT_BAD_OP);
        };
        if end > LOCALS {
            return Err(FAULT_BAD_OP);
        }
        for (k, slot) in xform_buf.iter_mut().take(cnt).enumerate() {
            let base = off + k * 3;
            let word0 = task.locals[base] as u32;
            *slot = XformSlot {
                wait: (word0 >> 16) as u16,
                op: ((word0 >> 8) & 0xFF) as u8,
                _pad: 0,
                args: [task.locals[base + 1], task.locals[base + 2]],
            };
        }
        cnt
    };

    // task_script 号必须在册（先查，不建弹）——空指针式坏号同 OP_SPAWN 口径复用 FAULT_BAD_OP。
    if task_script >= 0 && ctx.ecl.entry(task_script as u16).is_none() {
        return Err(FAULT_BAD_OP);
    }

    let angle = bam(angle_raw);
    let speed = Fx::from_raw(speed_raw);
    let (vx, vy) = polar_to_vec(speed, angle);
    let init = BulletInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
        vx,
        vy,
        speed,
        angle,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: cfg.sprite,
        radius: cfg.radius,
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: crate::xform::XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
    };

    let handle = if xform_len == 0 {
        ctx.body.create_bullet(init)
    } else {
        ctx.body
            .create_bullet_with_xform(init, &xform_buf[..xform_len])
    };

    if handle == BulletHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)?;

    if task_script >= 0 {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），弹已建、句柄已押，不 Fault。
        let pc0 = ctx.ecl.entry(task_script as u16).expect("已在上面校验过");
        let owner = (OWNER_BULLET, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        if ctx
            .tasks
            .spawn(task_script as u16, pc0, owner, parent, ctx.frame)
            .is_none()
        {
            ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
        }
    }
    Ok(())
}

/// 批量环（SYS 21）：9 参逆序弹出，无 xform（性能语义原语，dumb 弹批量铺环）。
/// appearance 越界 → Fault；池满/坏轴由 `create_bullets_batch` 自身 P4 处置（压实发数，
/// 可能为 0——not 视为 Fault，与单发 create_bullet 的"NULL→-1"口径不同因为返回语义本就是数量）。
fn sys_create_bullets_batch(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let speed_step = pop(task)?;
    let speed0 = pop(task)?;
    let n_speed = pop(task)?;
    let angle_step = pop(task)?;
    let angle0 = pop(task)?;
    let n_angle = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let appearance = pop(task)?;

    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };

    let init = BulletInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle::ZERO,
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: cfg.sprite,
        radius: cfg.radius,
        delay: 0,
        life: 0xFFFF,
        flags: 0,
        grazed_by: 0,
        transform_head: crate::xform::XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
    };

    let n = ctx.body.create_bullets_batch(
        init,
        &[],
        n_angle.max(0) as u16,
        bam(angle0),
        angle_step as i16,
        n_speed.max(0) as u16,
        Fx::from_raw(speed0),
        Fx::from_raw(speed_step),
    );
    push(task, n as i32)
}

/// 敌人创建（SYS 22；v1 直参，无 appearance 表——见 follow-ups）：5 参逆序弹出。
/// 半径/受击盒/sprite 用固定默认值（同 `world::test_support::spawn_enemy` 惯例）；
/// `main_task`/`death_script` 恒 0（脚本层自行事后 `spawn_task` 绑定主控协程）。
fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    let init = EnemyInit {
        x: Fx::from_raw(x_raw),
        y: Fx::from_raw(y_raw),
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
        hp,
        hp_max: hp,
        radius: Fx::from_int(12),
        hurtbox: Fx::from_int(16),
        invuln: 0,
        hit_flash: 0,
        flags: 0,
        sprite: 0,
        anm_state: 0,
        main_task: 0,
        death_script: 0,
        drop_table: drop_table as u16,
        score: score as u16,
    };
    let handle = ctx.body.create_enemy(init);
    if handle == EnemyHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)
}

/// 道具掉落（SYS 23）：3 参逆序弹出。坏类型 → `drop_item` 自身 P4-b 处置（NULL + BAD_ARGS
/// 计数，不 Fault——世界既有 write API 契约，syscall 层不重复判定）。
fn sys_drop_item(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let item_type = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let handle = ctx.body.drop_item(
        Fx::from_raw(x_raw),
        Fx::from_raw(y_raw),
        item_type as u8,
        ctx.tables,
    );
    if handle == crate::items::ItemHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)
}

/// 敌人限时缓动位移（SYS 24）：self owner 必须是 ENEMY（否则 Fault，misuse 策略）；
/// 4 参逆序弹出：`easing, y, x, dur`。无返回值。
fn sys_move_enemy_to(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let easing = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let dur = pop(task)?;
    ctx.body.move_enemy_to(
        h,
        Fx::from_raw(x_raw),
        Fx::from_raw(y_raw),
        dur as u16,
        easing as u8,
    );
    Ok(())
}

/// boss 公告板整槽写（SYS 25）：`enemy` 字段取自 self owner（非 ENEMY → `EnemyHandle::NULL`，
/// 不 Fault——见模块文档"误用策略"boss_set 例外）；6 参逆序弹出：
/// `active, phase_left, timer_frames, spell_id, hp_ratio, slot`。无返回值。
fn sys_boss_set(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let active = pop(task)?;
    let phase_left = pop(task)?;
    let timer_frames = pop(task)?;
    let spell_id = pop(task)?;
    let hp_ratio = pop(task)?;
    let slot = pop(task)?;

    let enemy = if task.owner_kind == OWNER_ENEMY {
        EnemyHandle {
            index: task.owner_index,
            generation: task.owner_gen,
        }
    } else {
        EnemyHandle::NULL
    };

    ctx.body.boss_set(
        slot as u8,
        BossUiSlot {
            enemy,
            hp_ratio: Fx::from_raw(hp_ratio),
            spell_id: spell_id as u16,
            timer_frames: timer_frames as u16,
            phase_left: phase_left as u8,
            active: active as u8,
        },
    );
    Ok(())
}

/// 瞄准角查询（SYS 40）：0 参；self 位置（owner 未知/STAGE→原点）朝向 P0 的 `atan2`，
/// 押回 BAM raw（不消 RNG、不改世界，纯读）。
fn sys_aim_player_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (sx, sy) = self_pos(task, ctx);
    let px = ctx.body.players[0].x;
    let py = ctx.body.players[0].y;
    let angle = crate::math::cordic::atan2(py - sy, px - sx);
    push(task, angle.raw() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::EclImage;
    use crate::ecl::task::{OWNER_ENEMY, OWNER_STAGE, Task};
    use crate::step::World;
    use crate::tables::{APPEARANCE_LARGE, APPEARANCE_MEDIUM, APPEARANCE_SMALL, TABLES_V0};

    /// 派发测试助手：把 `args`（脚本**声明顺序**，正序）压栈，直连 `dispatch`（不经
    /// `OP_SYS`/`exec`——聚焦 syscall 语义本身，`OP_SYS` 派发链路已由 `vm.rs` 测试覆盖）。
    fn call(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
    ) -> Result<(), u8> {
        for &a in args {
            task.stack[task.sp as usize] = a;
            task.sp += 1;
        }
        let mut budget = u32::MAX;
        let frame = w.body.frame;
        let mut ctx = VmCtx {
            code: &[],
            budget: &mut budget,
            tasks: &mut w.tasks,
            ecl,
            body: &mut w.body,
            tables: &TABLES_V0,
            self_index: 0,
            frame,
        };
        dispatch(no, task, &mut ctx)
    }

    fn fresh() -> (Box<World>, EclImage) {
        (World::new(1), EclImage::empty())
    }

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
            main_task: 0,
            death_script: 0,
            drop_table: 0,
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

    /// 丙方案 create_bullet：dumb 路径（xform_cnt=0）——位置精确、appearance 半径/sprite
    /// 逐位查表命中。
    #[test]
    fn sys_create_bullet_dumb_path_position_and_appearance_bitwise() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：appearance,x,y,speed,angle,xform_off,xform_cnt,task_script
        let args = [
            APPEARANCE_MEDIUM as i32,
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
        let cfg = &TABLES_V0.appearances[APPEARANCE_MEDIUM as usize];
        assert_eq!(
            w.body.bullets.radius[i], cfg.radius,
            "appearance 半径逐位命中"
        );
        assert_eq!(
            w.body.bullets.sprite[i], cfg.sprite,
            "appearance sprite 逐位命中"
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
            APPEARANCE_SMALL as i32,
            0,
            0,
            0,
            0,
            off as i32,
            1, // xform_cnt=1
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
            APPEARANCE_SMALL as i32,
            0,
            0,
            0,
            0,
            60, // off
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
        let args = [APPEARANCE_SMALL as i32, 0, 0, 0, 0, 0, 0, 0]; // task_script=0 越界
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
            });
        }
        let mut task = Task::default();
        let args = [APPEARANCE_SMALL as i32, 0, 0, 0, 0, 0, 0, -1];
        assert!(call(&mut w, &ecl, &mut task, SYS_CREATE_BULLET, &args).is_ok());
        assert_eq!(task.stack[0], -1, "池满押 -1");
    }

    /// task_script ≥ 0 挂弹派任务：owner=(BULLET, 新弹 index/gen)，`parent`/`born_frame` 戳记
    /// 与 `OP_SPAWN` 同款；弹死后任务次帧被静默回收（owner 门禁，非 Fault）。
    #[test]
    fn sys_create_bullet_task_script_spawns_owner_bound_task_and_dies_with_bullet() {
        let ecl = EclImage {
            code: vec![
                crate::ecl::ops::OP_PUSHI as u32,
                999,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            subs: vec![0],
            content_hash: 0,
        };
        let mut w = World::new(1);
        w.body.frame = 5;
        let mut task = Task::default();
        let args = [APPEARANCE_SMALL as i32, 0, 0, 0, 0, 0, 0, 0]; // task_script=0（在册）
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
            APPEARANCE_LARGE as i32,
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
        let cfg = &TABLES_V0.appearances[APPEARANCE_LARGE as usize];
        for i in 0..8 {
            assert_eq!(w.body.bullets.radius[i], cfg.radius);
            assert_eq!(w.body.bullets.sprite[i], cfg.sprite);
        }
    }

    #[test]
    fn sys_spawn_enemy_creates_with_fields() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        // 正序：x,y,hp,drop_table,score
        let args = [Fx::from_int(5).raw(), Fx::from_int(6).raw(), 42, 1, 100];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let idx = task.stack[0];
        assert!(idx >= 0);
        let i = idx as usize;
        assert_eq!(w.body.enemies.x[i], Fx::from_int(5));
        assert_eq!(w.body.enemies.y[i], Fx::from_int(6));
        assert_eq!(w.body.enemies.hp[i], 42);
        assert_eq!(w.body.enemies.drop_table[i], 1);
        assert_eq!(w.body.enemies.score[i], 100);
    }

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

    /// 坏 syscall 号（不在 v1 号表内）→ Fault（`dispatch` 默认臂，复用 `FAULT_BAD_OP`）。
    #[test]
    fn dispatch_bad_syscall_number_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(call(&mut w, &ecl, &mut task, 9999, &[]), Err(FAULT_BAD_OP));
    }

    /// 栈下溢（参数不足）→ `Fault(FAULT_STACK)`：`SET_VAR` 需 2 参，空栈直接派发。
    #[test]
    fn dispatch_stack_underflow_faults() {
        let (mut w, ecl) = fresh();
        let mut task = Task::default();
        assert_eq!(
            call(&mut w, &ecl, &mut task, SYS_SET_VAR, &[]),
            Err(FAULT_STACK)
        );
    }
}
