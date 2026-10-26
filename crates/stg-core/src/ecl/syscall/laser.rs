use super::*;

/// 打包激光句柄 → 栈上 i32（编码同敌号：`((gen & 0x7FFF) << 16) | index`，NULL → -1）。
fn pack_laser_handle(h: LaserHandle) -> i32 {
    if h == LaserHandle::NULL {
        -1
    } else {
        (((h.generation & 0x7FFF) as i32) << 16) | h.index as i32
    }
}

/// 打包激光号 → 活槽句柄；`None` = 无效（负哨兵 / 越界 / 死槽 / **generation 不符**）。
/// 判据与 [`resolve_enemy_handle`] 逐条同款（两边都 `& 0x7FFF`）；重建时取池里的**完整
/// u16 代际**——相位 5 的挂靠比较用的是完整代际，低 15 位复刻会让高代际静默脱钩（I-3）。
fn resolve_laser_handle(packed: i32, ctx: &VmCtx) -> Option<LaserHandle> {
    if packed < 0 {
        return None;
    }
    let idx = (packed & 0xFFFF) as usize;
    let g = ((packed >> 16) & 0x7FFF) as u16;
    if idx < LaserPool::CAP
        && ctx.body.lasers.is_alive(idx)
        && (ctx.body.lasers.generation[idx] & 0x7FFF) == g
    {
        Some(LaserHandle {
            index: idx as u16,
            generation: ctx.body.lasers.generation[idx],
        })
    } else {
        None
    }
}

// ── 8xx 激光族 handler（Task 4）──────────────────────────────────────────────
//
// 统一口径：`resolve_laser_handle` 失败就传 `LaserHandle::NULL`，由世界侧写 API no-op +
// 计一次 `contract_viol`——计数只发生在一处（计划 Step 2）。参数一律逆序弹出。

/// 出 `[0, u16::MAX]` → 钳位并把 `bad` 置真（一次 `laser()` 多坏字段只计一次违约）。
fn clamp_u16_count(v: i32, bad: &mut bool) -> u16 {
    if (0..=u16::MAX as i32).contains(&v) {
        v as u16
    } else {
        *bad = true;
        v.clamp(0, u16::MAX as i32) as u16
    }
}

/// `laser()`（800）：9 参逆序弹。`color ∈ 0..=15` 否则 Fault（先验后建，同 `fire` 坏外观）；
/// `warn/active/fade` 出 `[0,65535]` 钳位并计一次违约；形态一字段 `start=0`、
/// `end=start_len=len`、`speed=omega=0`、`flags=0`，其余派生字段交给 `create_laser`。
pub(super) fn sys_laser_create(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let fade = pop(task)?;
    let active = pop(task)?;
    let warn = pop(task)?;
    let width = pop(task)?;
    let len = pop(task)?;
    let angle = pop(task)?;
    let y = pop(task)?;
    let x = pop(task)?;
    let color = pop(task)?;
    if !(0..=15).contains(&color) {
        return Err(FAULT_BAD_OP);
    }
    let mut bad = false;
    let warn = clamp_u16_count(warn, &mut bad);
    let active = clamp_u16_count(active, &mut bad);
    let fade = clamp_u16_count(fade, &mut bad);
    if bad {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
    }
    let init = LaserInit {
        ox: Fx::from_raw(x),
        oy: Fx::from_raw(y),
        angle: bam(angle),
        omega: 0,
        start: Fx::ZERO,
        end: Fx::from_raw(len),
        start_len: Fx::from_raw(len),
        speed: Fx::ZERO,
        width: Fx::from_raw(width),
        sprite: color as u16,
        warn,
        active,
        fade,
        timer: 0,
        state: 0,
        anchor_idx: ANCHOR_NONE,
        anchor_gen: 0,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        dx: Fx::ZERO,
        dy: Fx::ZERO,
        dang: 0,
        px: Fx::ZERO,
        py: Fx::ZERO,
        pang: Angle::ZERO,
        flags: 0,
        born_frame: 0,
    };
    let h = ctx.body.create_laser(init);
    push(task, pack_laser_handle(h))
}

/// `lz_speed(lz, speed, start_len)`（801）。
pub(super) fn sys_laser_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let start_len = pop(task)?;
    let speed = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body
        .laser_set_speed(h, Fx::from_raw(speed), Fx::from_raw(start_len));
    Ok(())
}

/// `lz_start(lz, s)`（802）。
pub(super) fn sys_laser_start(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let s = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_set_start(h, Fx::from_raw(s));
    Ok(())
}

/// `lz_omega(lz, a)`（803）：`a` 取低 16 位**按位回绕**为 `i16`（BAM 语义——反向扫射编码后
/// 原始值 > 32767，位穿透才能保持反向），不钳位、不计数。
pub(super) fn sys_laser_omega(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_set_omega(h, bam(a).raw() as i16);
    Ok(())
}

/// `lz_rotate(lz, a)`（804）：一次性转 `a`（回绕加）。
pub(super) fn sys_laser_rotate(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_rotate(h, bam(a));
    Ok(())
}

/// `lz_aim(lz, off)`（805）：指向自机 0 的角度 + `off`。
pub(super) fn sys_laser_aim(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let off = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_aim(h, bam(off));
    Ok(())
}

/// `lz_anchor(lz, enemy, ox, oy)`（806）。`enemy = -1` → `EnemyHandle::NULL`（解除）；
/// 其它敌号一律经 [`resolve_enemy_handle`] 解析，命中则用池里的**完整 u16 代际**重建句柄
/// （I-3）；解析失败 → 不挂靠 + 计一次 `contract_viol`（控制方裁定 ②）。
pub(super) fn sys_laser_anchor(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let oy = pop(task)?;
    let ox = pop(task)?;
    let enemy = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    let e = if enemy == -1 {
        EnemyHandle::NULL
    } else {
        match resolve_enemy_handle(enemy, ctx) {
            Some(idx) => EnemyHandle {
                index: idx as u16,
                generation: ctx.body.enemies.generation[idx],
            },
            None => {
                // 失效敌号（死了/代际不符/越界）：no-op + 计数，不落到世界侧。
                ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
                ctx.body.last_status = crate::world::STATUS_STALE_HANDLE;
                return Ok(());
            }
        }
    };
    ctx.body
        .laser_anchor(h, e, Fx::from_raw(ox), Fx::from_raw(oy));
    Ok(())
}

/// `lz_origin(lz, x, y)`（807）：直接设原点并解除挂靠。
pub(super) fn sys_laser_origin(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let y = pop(task)?;
    let x = pop(task)?;
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_origin(h, Fx::from_raw(x), Fx::from_raw(y));
    Ok(())
}

/// `lz_cancel(lz)`（808）。
pub(super) fn sys_laser_cancel(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let lz = pop(task)?;
    let h = resolve_laser_handle(lz, ctx).unwrap_or(LaserHandle::NULL);
    ctx.body.laser_cancel(h);
    Ok(())
}

/// `lz_alive(lz)`（809）：只读，不计数；押 0/1。
pub(super) fn sys_laser_alive(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let lz = pop(task)?;
    let alive = resolve_laser_handle(lz, ctx).is_some();
    push(task, if alive { 1 } else { 0 })
}

/// 激光几何读口（810–814）共用：只读、不计数；失效句柄押 0（同 `enemy_x` 读族口径）。
pub(super) fn sys_laser_read(task: &mut Task, ctx: &mut VmCtx, id: u16) -> Result<(), u8> {
    let lz = pop(task)?;
    let v = match resolve_laser_handle(lz, ctx) {
        None => 0,
        Some(h) => {
            let (l, i) = (&ctx.body.lasers, h.index as usize);
            match id {
                SYS_LASER_X => l.ox[i].raw(),
                SYS_LASER_Y => l.oy[i].raw(),
                SYS_LASER_ANGLE => i32::from(l.angle[i].raw()),
                SYS_LASER_NEAR => l.start[i].raw(),
                _ => l.end[i].raw(),
            }
        }
    };
    push(task, v)
}
