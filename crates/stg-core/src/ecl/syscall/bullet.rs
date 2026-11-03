use super::*;

pub(super) fn sys_set_bullet_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let speed = pop(task)?;
    ctx.body.set_bullet_speed(h, Fx::from_raw(speed));
    Ok(())
}

pub(super) fn sys_set_bullet_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let a = pop(task)?;
    ctx.body.set_bullet_angle(h, bam(a));
    Ok(())
}

pub(super) fn sys_turn_bullet(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let d = pop(task)?;
    ctx.body.turn_bullet(h, bam(d));
    Ok(())
}

pub(super) fn sys_set_bullet_vel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let vy = pop(task)?;
    let vx = pop(task)?;
    ctx.body
        .set_bullet_vel(h, Fx::from_raw(vx), Fx::from_raw(vy));
    Ok(())
}

pub(super) fn sys_set_bullet_ang_vel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let w = pop(task)?;
    ctx.body.set_bullet_ang_vel(h, w as i16);
    Ok(())
}

pub(super) fn sys_set_bullet_accel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let a = pop(task)?;
    ctx.body.set_bullet_accel(h, Fx::from_raw(a));
    Ok(())
}

pub(super) fn sys_set_bullet_gravity(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let ay = pop(task)?;
    let ax = pop(task)?;
    ctx.body
        .set_bullet_gravity(h, Fx::from_raw(ax), Fx::from_raw(ay));
    Ok(())
}

pub(super) fn sys_stop_bullet_fx(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    ctx.body.stop_bullet_fx(h);
    Ok(())
}

pub(super) fn sys_aim_bullet_at_player(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_bullet_handle(task)?;
    let d = pop(task)?;
    ctx.body.aim_bullet_at_player(h, bam(d));
    Ok(())
}

pub(super) fn sys_clear_bullets_at(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let stars = pop(task)?;
    let r = pop(task)?;
    let y = pop(task)?;
    let x = pop(task)?;
    ctx.body.create_field(crate::field::clear_field_at(
        Fx::from_raw(x),
        Fx::from_raw(y),
        Fx::from_raw(r),
        stars != 0,
    ));
    Ok(())
}

pub(super) fn sys_clear_bullets(_task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    ctx.body
        .create_field(crate::field::fullscreen_clear_field());
    Ok(())
}

/// 丙方案 create_bullet（SYS 200）：8 参逆序弹出；appearance 越界/xform 区间越界 → Fault；
/// task_script ≥0 时脚本号必须在册（否则 Fault，同 `OP_SPAWN` 口径）——**先查后建**：
/// 一切校验在任何世界写之前完成，避免"弹已建、任务绑定失败"的半成品状态。
/// 创建失败（appearance/xform 校验过关后，池满或 xform 内容仍被 `create_bullet_with_xform`
/// 拒绝）→ 押 -1，不派生任务。
pub(super) fn sys_create_bullet(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    create_bullet_impl(task, ctx, false)
}
pub(super) fn sys_create_bullet_program(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    create_bullet_impl(task, ctx, true)
}
fn create_bullet_impl(task: &mut Task, ctx: &mut VmCtx, program: bool) -> Result<(), u8> {
    let task_script = pop(task)?;
    let xform_cnt = pop(task)?;
    let xform_off = pop(task)?;
    let program_id = if program { Some(pop(task)?) } else { None };
    let angle_raw = pop(task)?;
    let speed_raw = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    let appearance = pop(task)?;

    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

    // xform 区间越界（LOCALS 边界）→ Fault；内容层面的坏 xform（>16 槽/未知 op/...）留给
    // `create_bullet_with_xform` 自己的 P4-b 处置（NULL + BAD_ARGS，见下方"押 -1"分支）。
    let mut xform_buf = [XformSlot::default(); 16];
    let binding = if let Some(id) = program_id {
        match bind_program(task, ctx, id, xform_off, xform_cnt)? {
            Some(b) => Some(b),
            None => return push(task, -1),
        }
    } else {
        None
    };
    let xform_len: usize = if program || xform_cnt == 0 {
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
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async
            || ctx
                .ecl
                .param_types(sub)
                .is_none_or(|params| !params.is_empty())
        {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
    } else {
        None
    };

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
        born_frame: ctx.body.frame,
    };

    let handle = if let Some(binding) = binding {
        ctx.body
            .create_bullet_with_bound_xform(init, &binding, ctx.ecl.xforms(), ctx.tables)
    } else if xform_len == 0 {
        ctx.body.create_bullet(init)
    } else {
        ctx.body
            .create_bullet_with_xform(init, &xform_buf[..xform_len])
    };

    if handle == BulletHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)?;

    if let Some(task_sub) = task_sub {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），弹已建、句柄已押，不 Fault。
        let pc0 = ctx
            .ecl
            .sub_meta(task_sub)
            .expect("已在上面校验过")
            .code_entry();
        let owner = (OWNER_BULLET, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        if ctx
            .tasks
            .spawn(task_sub, pc0, owner, parent, ctx.frame)
            .is_none()
        {
            ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
        }
    }
    Ok(())
}

/// 批量环（SYS 201）：9 参逆序弹出，无 xform（性能语义原语，dumb 弹批量铺环）。
/// appearance 越界 → Fault；池满/坏轴由 `create_bullets_batch` 自身 P4 处置（压实发数，
/// 可能为 0——not 视为 Fault，与单发 create_bullet 的"NULL→-1"口径不同因为返回语义本就是数量）。
pub(super) fn sys_create_bullets_batch(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

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
        born_frame: ctx.body.frame,
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
