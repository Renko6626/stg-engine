use super::*;

pub(super) fn sys_self_enemy(task: &mut Task, _ctx: &mut VmCtx) -> Result<(), u8> {
    // 非敌押 -1 而非 0：打包敌号 0 合法（spec §5.2），与族内其它变量「非敌读 0」有意不同。
    let v = if task.owner_kind == OWNER_ENEMY {
        crate::enemy::pack_handle(EnemyHandle {
            index: task.owner_index,
            generation: task.owner_gen,
        })
    } else {
        -1
    };
    push(task, v)
}

pub(super) fn sys_self_x(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (x, _) = self_pos(task, ctx);
    push(task, x.raw())
}

pub(super) fn sys_self_y(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (_, y) = self_pos(task, ctx);
    push(task, y.raw())
}

pub(super) fn sys_self_vx(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (vx, _, _, _) = self_vel(task, ctx);
    push(task, vx)
}

pub(super) fn sys_self_vy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (_, vy, _, _) = self_vel(task, ctx);
    push(task, vy)
}

pub(super) fn sys_self_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (_, _, sp, _) = self_vel(task, ctx);
    push(task, sp)
}

pub(super) fn sys_self_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (_, _, _, a) = self_vel(task, ctx);
    push(task, a)
}

pub(super) fn sys_self_hp(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let hp = self_hp(task, ctx);
    push(task, hp)
}

pub(super) fn sys_self_hp_max(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let hp_max = self_hp_max(task, ctx);
    push(task, hp_max)
}

pub(super) fn sys_self_age(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let age = ctx.frame.wrapping_sub(task.born_frame) as i32;
    push(task, age)
}

pub(super) fn sys_dist(task: &mut Task, _ctx: &mut VmCtx) -> Result<(), u8> {
    let dy = pop(task)?;
    let dx = pop(task)?;
    // Q32.32 → 开根 → Q16.16（见 SYS_DIST 号表注释的值域论证）。`.min()` 不是
    // 冗余：极端字面量（`dx = dy = i32::MAX`）能让开根结果超 i32 上限，裸 `as i32`
    // 会回绕成负距离——饱和降级换掉那个静默错值。
    let d2 = crate::math::geom::len_sq(Fx::from_raw(dx), Fx::from_raw(dy));
    let root = crate::math::isqrt::isqrt(d2 as u64).min(i32::MAX as u32);
    let d = Fx::from_raw(root as i32);
    push(task, d.raw())
}

/// `n<=0` → 押 0、不消耗世界 RNG 流（拍板：n==0 既定钉死，n<0 视同"无合法范围"同律扩展、
/// 同样不消耗——一次坏参不该让世界 RNG 流分叉）。
pub(super) fn sys_rand_range(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let n = pop(task)?;
    let v = if n <= 0 {
        0
    } else {
        ctx.body.rng.rand_range(n as u32) as i32
    };
    push(task, v)
}

/// 瞄准角查询（SYS 120）：0 参；self 位置（owner 未知/STAGE→原点）朝向**目标自机**的
/// `atan2`，押回 BAM raw（不消 RNG、不改世界，纯读）。
///
/// 目标自机走引擎唯一的"瞄谁"口径 [`crate::world::WorldBody::aim_target`]（F8 统一，
/// 2026-09-03）：从 self 位置看过去最近的可瞄自机，一个可瞄的都没有则回退 `players[0]`
/// 的最后坐标——查询必须产出一个角度。
/// 单人局（`players[1]` 恒 ABSENT）下这与旧的"恒 `players[0]`"逐位等同——改的是 co-op：
/// P1 已 game over 而 P2 还活着时，不再朝着尸体坐标算角。
pub(super) fn sys_aim_player_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let (sx, sy) = self_pos(task, ctx);
    let p = ctx.body.aim_target(sx, sy);
    let px = ctx.body.players[p].x;
    let py = ctx.body.players[p].y;
    let angle = crate::math::cordic::atan2(py - sy, px - sx);
    push(task, angle.raw() as i32)
}
