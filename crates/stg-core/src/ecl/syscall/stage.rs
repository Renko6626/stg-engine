use super::*;

pub(super) fn sys_add_score(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let d = pop(task)?;
    let s = &mut ctx.body.players[0].score;
    *s = if d >= 0 {
        s.saturating_add(d as u64)
    } else {
        s.saturating_sub(d.unsigned_abs() as u64)
    };
    Ok(())
}

pub(super) fn sys_add_lives(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let d = pop(task)?;
    let p = &mut ctx.body.players[0];
    p.lives = (p.lives as i32).saturating_add(d).clamp(0, u8::MAX as i32) as u8;
    Ok(())
}

pub(super) fn sys_add_bombs(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let d = pop(task)?;
    let p = &mut ctx.body.players[0];
    p.bombs = (p.bombs as i32)
        .saturating_add(d)
        .clamp(0, crate::player::STOP_STOCK_MAX as i32) as u8;
    Ok(())
}

pub(super) fn sys_add_power(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let d = pop(task)?;
    let p = &mut ctx.body.players[0];
    // 上钳 POWER_MAX(400) 而非 u16::MAX —— 见 SYS_ADD_POWER 号表注释。
    p.power = (p.power as i32)
        .saturating_add(d)
        .clamp(0, crate::items::POWER_MAX as i32) as u16;
    Ok(())
}

pub(super) fn sys_time_stop_player(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let n = pop(task)?;
    // D19 判例：越界 → P4-b 整条 no-op + 计数 + BAD_ARGS，不钳位、不 Fault。
    let Ok(frames) = u16::try_from(n) else {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    };
    ctx.body.freeze_left[1] = frames;
    Ok(())
}

pub(super) fn sys_set_player_hitbox(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let r = pop(task)?;
    ctx.body.set_player_hit_radius(0, Fx::from_raw(r));
    Ok(())
}

pub(super) fn sys_get_var(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let slot = pop(task)? as u16;
    let v = ctx.body.get_var(slot);
    push(task, v)
}

pub(super) fn sys_set_var(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_pulse_signal(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let ch = pop(task)?;
    ctx.body.pulse_signal(ch as usize);
    Ok(())
}

/// 5xx 族锚点写口的三种目标字段（`sys_anchor_u16` 判据）。
pub(super) enum AnchorKind {
    Bgm,
    Bg,
    BgPhase,
}

/// 三锚共用：弹 1 参收窄 u16（越界 = P4-b no-op+viol，同 `sys_emit_req` 口径），再走 world 写口。
pub(super) fn sys_anchor_u16(task: &mut Task, ctx: &mut VmCtx, kind: AnchorKind) -> Result<(), u8> {
    let v = pop(task)?;
    if !(0..=u16::MAX as i32).contains(&v) {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    match kind {
        AnchorKind::Bgm => ctx.body.set_bgm(v as u16),
        AnchorKind::Bg => ctx.body.set_bg(v as u16),
        AnchorKind::BgPhase => ctx.body.set_bg_phase(v as u16),
    }
    Ok(())
}

/// 道具掉落（SYS 220）：3 参逆序弹出。坏类型 → `drop_item` 自身 P4-b 处置（NULL + BAD_ARGS
/// 计数，不 Fault——世界既有 write API 契约，syscall 层不重复判定）。
pub(super) fn sys_drop_item(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

/// `SYS_EMIT_REQ`（720）：通道 B 推送。id 收窄 P4-b——栈值超出 `0..=65535` →
/// no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（作者违约 → 确定性安全结果）；
/// 值域内转交 `WorldBody::emit_req`（满缓冲处置 TRUNCATED + 计数在那边）。
pub(super) fn sys_emit_req(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let a5 = pop(task)?;
    let a4 = pop(task)?;
    let a3 = pop(task)?;
    let a2 = pop(task)?;
    let a1 = pop(task)?;
    let a0 = pop(task)?;
    let id = pop(task)?;
    if !(0..=u16::MAX as i32).contains(&id) {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }
    ctx.body.emit_req(id as u16, [a0, a1, a2, a3, a4, a5]);
    Ok(())
}

/// 定位一次性演出（[`SYS_FX_AT`]=721）：4 参逆序弹出，owner 无限制。布局钉死为
/// `REQ_FX_AT, [x raw, y raw, kind, param, 0, 0]`；缓冲满走 D12（丢弃 + `reqs_dropped`）。
pub(super) fn sys_fx_at(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let param = pop(task)?;
    let kind = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;
    ctx.body
        .emit_req(crate::consts::REQ_FX_AT, [x_raw, y_raw, kind, param, 0, 0]);
    Ok(())
}

/// 依附一次性演出（[`SYS_FX_ON`]=722）：2 参逆序弹出；self-only（非敌 → Fault）。
/// 布局钉死为 `REQ_FX_ATTACHED, [index, gen, kind, param, 0, 0]`——**裸 index/gen 两位**，
/// 不用 `crate::enemy::pack_handle` 的打包形态（那是脚本值域内的敌号；请求载荷给壳侧，两位分开
/// 免得壳侧再拆包）。
pub(super) fn sys_fx_on(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    let param = pop(task)?;
    let kind = pop(task)?;
    ctx.body.emit_req(
        crate::consts::REQ_FX_ATTACHED,
        [h.index as i32, h.generation as i32, kind, param, 0, 0],
    );
    Ok(())
}

/// 关卡结束（[`SYS_STAGE_CLEAR`]=723）：1 参弹出，发 `EVT_STAGE_CLEARED{data0 = stage}`。
/// 世界侧只记事实，不改任何状态；让出一帧由表层 codegen 追发的 `WAIT` 负责。
pub(super) fn sys_stage_clear(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let stage = pop(task)?;
    ctx.body.push_event(crate::events::Event {
        kind: crate::events::EVT_STAGE_CLEARED,
        data: [stage, 0],
        ..Default::default()
    });
    Ok(())
}
