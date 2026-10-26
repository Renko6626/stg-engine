use super::*;

/// `SYS_SPELL_TIMER`（130）：owner 非 ENEMY → 直接押 -1（同 `SYS_SELF_HP` 误用降级口径，
/// 不 Fault——读族误用是"确定性安全结果"而非脚本作者违约）；敌 → 押
/// `spell_frames_left_of`（世界侧核已处理"无绑定 → -1"）。
pub(super) fn sys_spell_timer(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    if task.owner_kind != OWNER_ENEMY {
        return push(task, -1);
    }
    let h = EnemyHandle {
        index: task.owner_index,
        generation: task.owner_gen,
    };
    let frames_left = ctx.body.spell_frames_left_of(h);
    push(task, frames_left)
}

pub(super) fn sys_spell_result(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let slot = pop(task)?;
    let v = match usize::try_from(slot) {
        Ok(s) if s < crate::boss::MAX_BOSSES => ctx.body.spell_last_result[s] as i32,
        _ => {
            ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
            ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
            0
        }
    };
    push(task, v)
}

/// boss 公告板整槽写（SYS 730）：`enemy` 字段取自 self owner（非 ENEMY → `EnemyHandle::NULL`，
/// 不 Fault——见模块文档"误用策略"boss_set 例外）；6 参逆序弹出：
/// `active, phase_left, timer_frames, spell_id, hp_ratio, slot`。无返回值。
pub(super) fn sys_boss_set(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

/// 符卡宣言（`SYS_SPELL_BEGIN`=740；符卡机构 spec 2026-07-24 §5）：7 参逆序弹出；
/// `self_enemy_handle`（非敌 misuse → Fault，同 `move_enemy_to` 误用策略）。
///
/// **四条控制器补充决策**（复审 T1-m3/T1-m1/Task2 复审落地，spec 未写全，本刀新增，
/// syscall 层专属——世界侧 `spell_begin_internal` 仍宽松接受负 threshold/双绑定供白盒测
/// 直调）：
/// - **拒负 threshold**：血线语义非负，`threshold < 0` → P4-b no-op + 计数，不建。
/// - **防双绑定**：该 boss 已绑到**另一** active 槽 → P4-b no-op + 计数（单卡/boss 是
///   设计用法，多卡序靠脚本顺序 begin 下一张，不允许并行两槽绑同一 boss）。
/// - **拒负 bonus0**（Task 2 复审 Important #2，spec §5 P4-b 拒收条件）：分数语义非负，
///   `bonus0 < 0` → P4-b no-op + 计数（必须在 `as u32` 转型前拒，否则变 `u32::MAX`）。
/// - **拒非正 time_limit**（Task 2 复审 Important #3，spec §5 P4-b 拒收条件）：
///   `time_limit <= 0` → P4-b no-op + 计数（必须在 `as u16` 转型前拒，否则负值抹符号
///   变成巨大正数，绕过 `spell_begin_internal` 里"`time_limit == 0`"那条守卫）。
///
/// `pattern`（`SubRef`，负值=none）**先查后建**（同 `sys_create_bullet`/`OP_SPAWN` 坏号
/// 口径）：越界/非 0 参 Async 号 → Fault，零副作用（`spell_begin_internal` 尚未调用）。
/// `spell_begin_internal` 返 `true` 且 `pattern` 非 none → 照 fire task-spawn 样板 spawn
/// 模式任务（owner=boss，`spell_bound = slot+1` + `spell_epoch` 读回本槽刚铸出的代际戳——
/// "生"，spec §2.1 + ABA 修复复审 Task 2）；返 `false` → 不 spawn（P4-b 已在 internal 计数）。
pub(super) fn sys_spell_begin(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let threshold = pop(task)?;
    let flags = pop(task)?;
    let bonus0 = pop(task)?;
    let time_limit = pop(task)?;
    let pattern_ref = pop(task)?;
    let spell_id = pop(task)?;
    let slot = pop(task)?;
    let h = self_enemy_handle(task)?;

    if threshold < 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // 复审 Task 2 补充决策（Important #2，spec §5 P4-b 拒收条件）：`bonus0 < 0` 必须在
    // `bonus0 as u32` 转型**之前**拒收——否则 `-1i32 as u32` 变 `u32::MAX`，等于凭空发
    // ~42.9 亿分。血线/分数语义都非负，同 `threshold < 0` 口径处置：no-op + 计数，不建。
    if bonus0 < 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // 复审 Task 2 补充决策（Important #3，spec §5 P4-b 拒收条件）：`time_limit <= 0` 必须在
    // `time_limit as u16` 转型**之前**拒收——`spell_begin_internal` 只查 `time_limit == 0`，
    // 而负值经 `as u16` 抹符号会变成一个巨大的正数（如 `-1 → 65535`），从而绕过那条守卫、
    // 带着离谱的时限成功 begin。血线语义要求正时限，同上口径处置。
    if time_limit <= 0 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    let already_bound_elsewhere = (0..crate::boss::MAX_BOSSES).any(|s| {
        ctx.body.spells[s].active != 0
            && ctx.body.spells[s].boss_index == h.index
            && ctx.body.spells[s].boss_gen == h.generation
    });
    if already_bound_elsewhere {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // pattern 号先查后建：坏号 Fault、none（负值）跳过 spawn，均在 internal 调用之前定案。
    let pattern_sub: Option<SubId> = if pattern_ref >= 0 {
        let raw = u16::try_from(pattern_ref).map_err(|_| FAULT_BAD_OP)?;
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

    let slot_idx = slot as usize; // 负/越界值 wrapping 成巨大 usize，internal 的越界门自然挡
    let began = ctx.body.spell_begin_internal(
        slot_idx,
        h,
        spell_id as u16,
        time_limit as u16,
        bonus0 as u32,
        flags as u8,
        threshold,
    );
    if began && let Some(pattern_sub) = pattern_sub {
        // 照 fire 的 task-spawn 样板（sys_create_bullet）：entry 已在上面校验过在册。
        let pc0 = ctx
            .ecl
            .sub_meta(pattern_sub)
            .expect("已在上面校验过")
            .code_entry();
        let owner = (OWNER_ENEMY, h.index, h.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(pattern_sub, pc0, owner, parent, ctx.frame) {
            Some(idx) => {
                // "生"：模式任务显式绑定本槽（spec §2.1），与 OP_SPAWN 的"继承"路径
                // 不同——这是模式树的根，绑定值凭空而来，不是继承自父任务。
                ctx.tasks.slots[idx as usize].spell_bound = (slot_idx + 1) as u8;
                // ABA 修复（复审 Task 2，Critical）：同批写入本槽刚铸出的代际戳（begin 内
                // 已在 `spell_begin_internal` 里推进过 `spell_seq[slot]` 并戳进
                // `spells[slot].epoch`，此处读回）——模式任务据此锁定"自己是这一代"，
                // 槽将来被复用给别的卡时，相位 2 调度门禁靠这个戳把它挡在门外。
                ctx.tasks.slots[idx as usize].spell_epoch = ctx.body.spells[slot_idx].epoch;
            }
            None => {
                ctx.body.diag.pool_full[crate::world::POOL_TASK] =
                    ctx.body.diag.pool_full[crate::world::POOL_TASK].wrapping_add(1);
            }
        }
    }
    Ok(())
}

/// 符卡逃生舱口（`SYS_SPELL_END`=741；符卡机构 spec 2026-07-24 §5）：无参；
/// `self_enemy_handle`（非敌 misuse → Fault）；转交 `spell_end_by_owner`（无绑定 →
/// no-op，重复调用安全，见该 API 文档）。
pub(super) fn sys_spell_end(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.spell_end_by_owner(h);
    Ok(())
}
