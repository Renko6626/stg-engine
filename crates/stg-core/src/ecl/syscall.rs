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
use crate::ecl::image::{SubId, SubKind};
use crate::ecl::shooter::{
    SH_ABS_OFFSET, SH_AIMED, SH_NO_TASK, SH_RING, SHOOTERS_PER_TASK, ShooterSlot,
};
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
/// 符卡计时读族（符卡机构 spec 2026-07-24 §5）：owner 绑定的 active 槽 → `frames_left`；
/// 无绑定 → `-1`（`wait_spell()` 语法糖的判据，同 `SYS_SELF_HP` 误用降级口径：owner
/// 非 ENEMY 直接押 -1，不 Fault）。
pub const SYS_SPELL_TIMER: u16 = 11;
/// 查敌读口(A5 补遗):活敌返 hp,其余 -1。P4-b:句柄是池 index,悬垂/复用不可辨,
/// 越界/死槽一律 -1 不 Fault——stage 编排等 boss 死的轮询原语。
pub const SYS_ENEMY_HP: u16 = 12;

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
/// 通道 B 渲染请求推送（M2 前置刀；D12/spec §2.5）。无 owner 类别限制——宣言/音效/震屏
/// 常由 STAGE 任务发。
pub const SYS_EMIT_REQ: u16 = 27;
/// 符卡宣言（符卡机构 spec 2026-07-24 §5）：owner 必须 ENEMY（misuse → Fault）；7 参
/// 正序压栈 `slot, spell_id, pattern:SubRef, time_limit, bonus0, flags, hp_threshold`
/// （`pattern` 同 `fire` task 参同款 `SubRef`，负值=none）。
pub const SYS_SPELL_BEGIN: u16 = 28;
/// 符卡逃生舱口（符卡机构 spec 2026-07-24 §5）：无参；owner 绑定槽走 HP 路径结算，
/// 无绑定 → no-op（重复调用安全）。
pub const SYS_SPELL_END: u16 = 29;

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

// 5x：写——账面/表现声明族（整局流程刀 spec §4；owner 类别无限制，STAGE 任务常发）。
/// 1 参：`delta`（允许负，饱和钳 `[0, u64::MAX]`，P4-b）。
pub const SYS_ADD_SCORE: u16 = 50;
/// 1 参：`id`（`0..=65535` 收窄，越界 no-op+viol）。写 `bgm_id` + 发 `REQ_BGM`。
pub const SYS_BGM: u16 = 51;
/// 同上，写 `bg_id` + `REQ_BG`。
pub const SYS_BG: u16 = 52;
/// 1 参：`n`。写 `bg_phase` + 自动盖 `bg_phase_frame` = 当前帧 + 发 `REQ_BG_PHASE`。
pub const SYS_BG_PHASE: u16 = 53;
/// 全场清弹（B19；0 参、无返回）。铺一个覆盖全场、`life=1` 的 `FIELD_CLEAR_BULLETS`
/// 作用区——**复用现成的消弹区机制**，故"每颗被消的弹原位转一颗星星"（M0-15）与
/// `EVT_FIELD_CLEARED` 都是白送的，引擎侧零新机制（同 `settle_one_spell` 的全屏清弹样板）。
///
/// 关底转场（`REQ_STAGE_CLEAR` 挂牌前）是首个真实消费者。**不给护盾帧**——那是 bomb
/// 那刀的职责（bomb = `FIELD_CLEAR_BULLETS | FIELD_DAMAGE` + 自机无敌）。
/// P4-a：field 池满 → `create_field` 自身的降级（NULL + 计数），本 syscall 不 Fault。
pub const SYS_CLEAR_BULLETS: u16 = 54;
/// 残机增量（B20；1 参 `delta`、无返回）。双边钳 `[0, u8::MAX]`（P4-b：`delta` 是脚本给的
/// 任意 `i32`，先 `saturating_add` 再 `clamp`，不回绕不 panic）。
///
/// **增量形态（`add_*`）是人类裁定**，不是漏了 `set_*`：绝对赋值的唯一确定场景（开局装备）
/// 已被 [`crate::player::Loadout`]（`World::new_game_at` 的装备参）收编，运行中脚本要的
/// 都是"奖命 +1 / 中弹 −1"这类记账。
/// 别把这族"补全"成 `set_lives`/`set_bombs`/`set_power` 四件套——多一条写路径就多一处
/// 与 `Loadout` 抢开局初值的歧义。
pub const SYS_ADD_LIVES: u16 = 55;
/// bomb 增量（B20）。语义同 [`SYS_ADD_LIVES`]，钳 `[0, u8::MAX]`；增量形态同为人类裁定。
pub const SYS_ADD_BOMBS: u16 = 56;
/// 火力增量（B20）。语义同 [`SYS_ADD_LIVES`]，但上钳是 [`crate::items::POWER_MAX`]（400，
/// = 显示 4.00）**而非 `u16::MAX`**——越过它 `power_tier` 索引就 OOB（见
/// `world::WorldBody::set_player_power` 文档）。增量形态同为人类裁定。
pub const SYS_ADD_POWER: u16 = 57;
/// 清空自身待掉落计数（58；0 参、无返回。敌人死亡效果刀，参照 ZUN ECL 的 `dropClear` 506）。
/// self owner 必须是 ENEMY，否则 Fault（misuse 策略，同 `move_enemy_to`）。
pub const SYS_DROP_CLEAR: u16 = 58;
/// 给自身待掉落计数增量加 `n` 颗 `type`（59；2 参、无返回。参照 ZUN `dropExtra` 507）。
/// **只增不减**是人类裁定——要清空用 `drop_clear()`。坏类型/负 n 的处置见
/// [`crate::world::WorldBody::add_enemy_drop`]。
pub const SYS_DROP_ADD: u16 = 59;
/// 立刻把自身待掉落计数撒出去（60；0 参、无返回。参照 ZUN `dropItems` 509）。
/// **吐完不清空**（人类裁定 D-3，照 ZUN 字面）——故 `drop_items(); die();` 掉**双份**，
/// 作者自负。这条语义有测试钉死（`drop_items_does_not_clear_counts_...`），别"顺手修好"。
pub const SYS_DROP_ITEMS: u16 = 60;
/// 就地阵亡：跑完整死亡效果（61；0 参、无返回。参照 ZUN `die` 561）。
/// **表层 `die()` 降低成本 syscall + `OP_KILL_SELF` 两条指令**（见 codegen），故调用它的
/// 任务立即终止（人类裁定 D-4）。ZUN 的 561 还经 `setDeath`(556) 间接一层——那半留给
/// `death_script` 通电那一刀，届时与 ZUN 完全同构。
pub const SYS_DIE: u16 = 61;

// ── Shooter：预存发射参数集（62-76；shooter 刀 2026-07-31，参照 ZUN et* 族 600-641）──
//
// 每任务 4 个编号槽（[`crate::ecl::shooter::SHOOTERS_PER_TASK`]）。`id ≥ SHOOTERS_PER_TASK`
// 一律 no-op + `contract_viol`（P4-b）——不 Fault，因为"槽号写错"是常见笔误而非结构性违约，
// 降级比杀任务更有用。判据集中在 [`shooter_mut`] 一处，14 个派发臂共用。
//
// 下面 14 条**都只是写字段，无副作用**：不查 appearance 是否在册、不查 xform 区间、不查
// sub 号在册——那些校验统一在**开火那一刻**做（同 `fire` 的"先验后建"口径：设参数时弹还
// 不存在，没有可拒绝的对象）。`sh_fire`(76) 的语义见其自身文档。
pub const SYS_SH_RESET: u16 = 62;
pub const SYS_SH_SPRITE: u16 = 63;
pub const SYS_SH_OFFSET: u16 = 64;
pub const SYS_SH_OFFSET_ABS: u16 = 65;
pub const SYS_SH_OFFSET_RAD: u16 = 66;
pub const SYS_SH_DIST: u16 = 67;
pub const SYS_SH_ANGLE: u16 = 68;
pub const SYS_SH_SPEED: u16 = 69;
pub const SYS_SH_COUNT: u16 = 70;
pub const SYS_SH_AIM: u16 = 71;
pub const SYS_SH_RING: u16 = 72;
pub const SYS_SH_XFORM: u16 = 73;
pub const SYS_SH_TASK: u16 = 74;
pub const SYS_SH_REQ: u16 = 75;

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

/// `SYS_SPELL_TIMER`（11）：owner 非 ENEMY → 直接押 -1（同 `SYS_SELF_HP` 误用降级口径，
/// 不 Fault——读族误用是"确定性安全结果"而非脚本作者违约）；敌 → 押
/// `spell_frames_left_of`（世界侧核已处理"无绑定 → -1"）。
fn sys_spell_timer(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

/// `SYS_ENEMY_HP`（12；A5 补遗）：1 参 `handle`（池 index，直读，不比对 generation——
/// 句柄复用不可辨，同 `SYS_SPELL_TIMER`/`SYS_SELF_HP` 的读族误用降级口径，不 Fault）。
/// 活敌返当前 hp；越界/死槽/负值一律 -1——stage 编排"等 boss 死"的轮询原语。
fn sys_enemy_hp(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let handle = pop(task)?;
    let idx = handle as usize;
    let alive = handle >= 0 && idx < crate::enemy::EnemyPool::CAP && ctx.body.enemies.is_alive(idx);
    push(task, if alive { ctx.body.enemies.hp[idx] } else { -1 })
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

/// 取本任务的第 `id` 个 shooter（可变）。`id` 越界 → `None` + `contract_viol` + `BAD_ARGS`
/// （P4-b：no-op，不 Fault——见 62-75 号表注释）。`id` 是脚本给的任意 `i32`，负数与超界同处置。
///
/// **必须在参数全部 `pop` 完之后再调**：栈效应与成功路径一致（同 `sys_emit_req` 的"先弹后验"
/// 口径），否则越界腿会给下一条指令留下垃圾栈。
fn shooter_mut<'a>(
    id: i32,
    task_index: u16,
    ctx: &'a mut VmCtx<'_>,
) -> Option<&'a mut ShooterSlot> {
    if id < 0 || id as usize >= SHOOTERS_PER_TASK {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return None;
    }
    Some(&mut ctx.tasks.shooters[task_index as usize][id as usize])
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
        SYS_SPELL_TIMER => sys_spell_timer(task, ctx),
        SYS_ENEMY_HP => sys_enemy_hp(task, ctx),
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
        SYS_EMIT_REQ => sys_emit_req(task, ctx),
        SYS_SPELL_BEGIN => sys_spell_begin(task, ctx),
        SYS_SPELL_END => sys_spell_end(task, ctx),
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
        SYS_ADD_SCORE => {
            let d = pop(task)?;
            let s = &mut ctx.body.players[0].score;
            *s = if d >= 0 {
                s.saturating_add(d as u64)
            } else {
                s.saturating_sub(d.unsigned_abs() as u64)
            };
            Ok(())
        }
        SYS_BGM => sys_anchor_u16(task, ctx, AnchorKind::Bgm),
        SYS_BG => sys_anchor_u16(task, ctx, AnchorKind::Bg),
        SYS_BG_PHASE => sys_anchor_u16(task, ctx, AnchorKind::BgPhase),
        SYS_CLEAR_BULLETS => {
            ctx.body
                .create_field(crate::field::fullscreen_clear_field());
            Ok(())
        }
        // B20 三件套：`d` 是脚本 push 上来的**任意 i32**，故一律先 `saturating_add`
        // 再 `clamp` —— 裸 `+`（如 `255i32 + i32::MAX`）在 debug 下溢出 panic，而
        // "调用方给坏参数"是 P4-b（确定性安全结果），不是 P4-c（引擎自身 bug 就地炸）。
        SYS_ADD_LIVES => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.lives = (p.lives as i32).saturating_add(d).clamp(0, u8::MAX as i32) as u8;
            Ok(())
        }
        SYS_ADD_BOMBS => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.bombs = (p.bombs as i32).saturating_add(d).clamp(0, u8::MAX as i32) as u8;
            Ok(())
        }
        SYS_ADD_POWER => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            // 上钳 POWER_MAX(400) 而非 u16::MAX —— 见 SYS_ADD_POWER 号表注释。
            p.power = (p.power as i32)
                .saturating_add(d)
                .clamp(0, crate::items::POWER_MAX as i32) as u16;
            Ok(())
        }
        // 敌人死亡效果四件（58-61）：一律经 `WorldBody` 的 handle 写 API（P1：调用方
        // 永不直接摸池内存），self owner 必须是敌（misuse → Fault，同 `move_enemy_to`）。
        SYS_DROP_CLEAR => {
            let h = self_enemy_handle(task)?;
            ctx.body.clear_enemy_drops(h);
            Ok(())
        }
        SYS_DROP_ADD => {
            let h = self_enemy_handle(task)?;
            // 逆序弹出（模块文档"参数传递约定"）：`drop_add(type, n)` 故先 `n` 后 `type`。
            let n = pop(task)?;
            let item_type = pop(task)?;
            ctx.body.add_enemy_drop(h, item_type, n);
            Ok(())
        }
        SYS_DROP_ITEMS => {
            let h = self_enemy_handle(task)?;
            ctx.body.spill_enemy_drops(h, ctx.tables);
            Ok(())
        }
        SYS_DIE => {
            let h = self_enemy_handle(task)?;
            ctx.body.kill_enemy_by_handle(h, ctx.tables);
            Ok(())
        }
        // ── Shooter 配置面 62-75（参数**逆序弹出**，照 `sys_move_enemy_to`；`id` 是首参、
        //    故最后弹）。每条都是"弹完全部参数 → 取槽（越界即 no-op）→ 写字段"三段式。
        SYS_SH_RESET => {
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                *s = ShooterSlot::default();
            }
            Ok(())
        }
        SYS_SH_SPRITE => {
            // 表层是 (id, shape, color) 三参，但 shape/color 在**编译期**已折叠成单个
            // appearance 值（`builtins::fold_start("sh_sprite") == 1`），故这里只弹两个。
            // appearance 在册与否留到开火时查（同 `fire` 的先验后建）。
            //
            // **收窄必须保号越界性**：`try_from` 失败（负值 / 超 u16）一律落到 `u16::MAX`
            // ——它远超表长，T3 开火时照样查不到 ⇒ 越界值进来、越界值出去。
            // 曾经写成 `clamp(0, u16::MAX)`，把负值钳成 **0**，而 `appearances[0]` 是在册
            // 且 valid 的格子 ⇒ 越界值被偷偷洗成合法值，开火时再也拒不掉，和上面那句
            // "留到开火时查"的承诺直接矛盾（复审 ①）。`fire` 侧同样的脚本值是 Fault
            // （`appearances.get(负 as usize)` → `None`），两条路不该有这种差别。
            // 可达性是真的：typeck 的形色判据只在两参**都是编译期常量**时施加，
            // `sh_sprite(0, 16, c)` 里 `c` 是变量时判据跳过，`c = -20` 就折叠成 -4。
            let appearance = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.appearance = u16::try_from(appearance).unwrap_or(u16::MAX);
            }
            Ok(())
        }
        SYS_SH_OFFSET => {
            let y = pop(task)?;
            let x = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.off_x = Fx::from_raw(x);
                s.off_y = Fx::from_raw(y);
                s.flags &= !SH_ABS_OFFSET; // 相对模式：清标志（与 sh_offset_abs 互为反向）
            }
            Ok(())
        }
        SYS_SH_OFFSET_ABS => {
            let y = pop(task)?;
            let x = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.off_x = Fx::from_raw(x);
                s.off_y = Fx::from_raw(y);
                s.flags |= SH_ABS_OFFSET;
            }
            Ok(())
        }
        SYS_SH_OFFSET_RAD => {
            let r = pop(task)?;
            let ang = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.polar_ang = bam(ang);
                s.polar_r = Fx::from_raw(r);
            }
            Ok(())
        }
        SYS_SH_DIST => {
            let d = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.dist = Fx::from_raw(d);
            }
            Ok(())
        }
        SYS_SH_ANGLE => {
            let step = pop(task)?;
            let angle0 = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.angle0 = bam(angle0);
                s.angle_step = bam(step);
            }
            Ok(())
        }
        SYS_SH_SPEED => {
            let step = pop(task)?;
            let speed0 = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.speed0 = Fx::from_raw(speed0);
                s.speed_step = Fx::from_raw(step);
            }
            Ok(())
        }
        SYS_SH_COUNT => {
            let n_speed = pop(task)?;
            let n_angle = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                // P4-b：脚本给的任意 i32，先钳进 [0,255] 再存（裸 `as u8` 会让负数回绕）。
                s.n_angle = n_angle.clamp(0, u8::MAX as i32) as u8;
                s.n_speed = n_speed.clamp(0, u8::MAX as i32) as u8;
            }
            Ok(())
        }
        SYS_SH_AIM => {
            let on = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                if on != 0 {
                    s.flags |= SH_AIMED;
                } else {
                    s.flags &= !SH_AIMED;
                }
            }
            Ok(())
        }
        SYS_SH_RING => {
            let on = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                if on != 0 {
                    s.flags |= SH_RING;
                } else {
                    s.flags &= !SH_RING;
                }
            }
            Ok(())
        }
        SYS_SH_XFORM => {
            // 表层 `XformRef` 在 codegen 里降低成 `(off, cnt)` **两个**栈值。`cnt == 0` 即无
            // xform。**此处不校验区间**——`off + cnt*3 ≤ LOCALS` 在开火时查（与 `fire` 同
            // 口径：`fire` 也是在建弹那一刻才查 `LOCALS` 边界）。这里只做**存储收窄**的
            // 饱和钳，防负值/超宽裸 `as` 回绕成一个看似合法的小区间。
            let cnt = pop(task)?;
            let off = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.xform_off = off.clamp(0, u16::MAX as i32) as u16;
                s.xform_cnt = cnt.clamp(0, u8::MAX as i32) as u8;
            }
            Ok(())
        }
        SYS_SH_TASK => {
            // 表层 `SubRef` 降低成一个栈值（sub 号或 -1）。**此处不校验号是否在册**——与
            // `fire` 的"先验后建"不同，因为设的时候还没建弹；校验在开火时做。
            // 超 `u16` 的号同样降级成"不挂"（P4-b，确定性安全结果）。
            let sub = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.task_script = if sub < 0 {
                    SH_NO_TASK
                } else {
                    u16::try_from(sub).unwrap_or(SH_NO_TASK)
                };
            }
            Ok(())
        }
        SYS_SH_REQ => {
            let req_id = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.on_fire_req = req_id.clamp(0, u16::MAX as i32) as u16;
            }
            Ok(())
        }
        _ => Err(FAULT_BAD_OP),
    }
}

/// 5x 族锚点写口的三种目标字段（`sys_anchor_u16` 判据）。
enum AnchorKind {
    Bgm,
    Bg,
    BgPhase,
}

/// 三锚共用：弹 1 参收窄 u16（越界 = P4-b no-op+viol，同 `sys_emit_req` 口径），再走 world 写口。
fn sys_anchor_u16(task: &mut Task, ctx: &mut VmCtx, kind: AnchorKind) -> Result<(), u8> {
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
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

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

/// 敌人创建（SYS 22；A5 乙案，append-only：旧 5 参前缀不动，尾追 sprite/task）：
/// 7 参逆序弹出。半径/受击盒用固定默认值（同 `world::test_support::spawn_enemy` 惯例）；
/// `task` 号先验后建（镜像 `sys_create_bullet` 的 task 路径：坏号/非 0 参 Async → Fault，
/// 零副作用，敌未建）；`main_task` 在敌句柄产出**之后**回填（任务的 owner 三元组需要敌
/// index/gen，敌必须先于任务存在）；`death_script` 仍恒 0（脚本面缺口留 follow-ups）。
fn sys_spawn_enemy(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let task_script = pop(task)?;
    let sprite = pop(task)?;
    let score = pop(task)?;
    let drop_table = pop(task)?;
    let hp = pop(task)?;
    let y_raw = pop(task)?;
    let x_raw = pop(task)?;

    // task 号先验后建（镜像 sys_create_bullet：坏号 FAULT_BAD_OP，敌未建；Async+零参白名单）。
    let task_sub: Option<SubId> = if task_script >= 0 {
        let raw = u16::try_from(task_script).map_err(|_| FAULT_BAD_OP)?;
        let sub = ctx.ecl.sub_id(raw).ok_or(FAULT_BAD_OP)?;
        let meta = ctx.ecl.sub_meta(sub).ok_or(FAULT_BAD_OP)?;
        if meta.kind() != SubKind::Async || ctx.ecl.param_types(sub).is_none_or(|p| !p.is_empty()) {
            return Err(FAULT_BAD_OP);
        }
        Some(sub)
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
        main_task: 0, // 任务 spawn 后回填（敌句柄先于任务存在）
        death_script: 0,
        drop_count,
        score: score as u16,
    };
    let handle = ctx.body.create_enemy(init);
    if handle == EnemyHandle::NULL {
        return push(task, -1);
    }
    push(task, handle.index as i32)?;

    if let Some(sub) = task_sub {
        // entry 已在上面校验过在册；池满 → 静默计数（P4-a），敌已建、句柄已押，不 Fault。
        let pc0 = ctx.ecl.sub_meta(sub).expect("已在上面校验过").code_entry();
        let owner = (OWNER_ENEMY, handle.index, handle.generation);
        let parent = ctx.self_index + 1;
        match ctx.tasks.spawn(sub, pc0, owner, parent, ctx.frame) {
            Some(slot) => {
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

/// `SYS_EMIT_REQ`（27）：通道 B 推送。id 收窄 P4-b——栈值超出 `0..=65535` →
/// no-op + `contract_viol` + `BAD_ARGS`，**不 Fault**（作者违约 → 确定性安全结果）；
/// 值域内转交 `WorldBody::emit_req`（满缓冲处置 TRUNCATED + 计数在那边）。
fn sys_emit_req(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

/// 符卡宣言（`SYS_SPELL_BEGIN`=28；符卡机构 spec 2026-07-24 §5）：7 参逆序弹出；
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
fn sys_spell_begin(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

/// 符卡逃生舱口（`SYS_SPELL_END`=29；符卡机构 spec 2026-07-24 §5）：无参；
/// `self_enemy_handle`（非敌 misuse → Fault）；转交 `spell_end_by_owner`（无绑定 →
/// no-op，重复调用安全，见该 API 文档）。
fn sys_spell_end(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let h = self_enemy_handle(task)?;
    ctx.body.spell_end_by_owner(h);
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
    use crate::ecl::image::{EclImage, EntryInit, SubInit, SubKind, test_image};
    use crate::ecl::shooter::{SH_ABS_OFFSET, SH_AIMED, SH_RING, SHOOTERS_PER_TASK, ShooterSlot};
    use crate::ecl::task::{OWNER_ENEMY, OWNER_STAGE, Task};
    use crate::step::World;
    use crate::tables::TABLES_V0;

    // 外观表行号（颜色轴刀 T4 起 ② 段清空、旧的 `APPEARANCE_*` 引擎常量退场——弹型名
    // 归内容包的 `.ecl` const，引擎侧测试直接写行号）。id = 弹型 × color_stride(16) + 色号：
    // 0 = 0 号形第 0 色，1 = 0 号形第 1 色，2 = 0 号形第 2 色。三者只用于"随便挑一个
    // 合法格"，与具体形/色语义无关。
    const ROW_A: i32 = 0;
    const ROW_B: i32 = 1;
    const ROW_C: i32 = 2;

    /// 派发测试助手：把 `args`（脚本**声明顺序**，正序）压栈，直连 `dispatch`（不经
    /// `OP_SYS`/`exec`——聚焦 syscall 语义本身，`OP_SYS` 派发链路已由 `vm.rs` 测试覆盖）。
    fn call(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
    ) -> Result<(), u8> {
        call_with_tables(w, ecl, task, no, args, &TABLES_V0)
    }

    /// 绑定**指定表**的 `call`——空格判据一类"内建表提供不了靶子"的测试用它挂合成表
    /// （当前图集 12 行全满 16 色，没有空格；数据如实反映美术，不为测试在内建表里造假）。
    fn call_with_tables(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        tables: &crate::tables::WorldTables,
    ) -> Result<(), u8> {
        call_full(w, ecl, task, no, args, tables, 0)
    }

    /// 指定 `self_index` 的 `call`——shooter 族专用（复审 ③）：shooter 存储按**任务索引**
    /// 键，而 `call` 把 `self_index` 写死 0，于是"派发臂用的是 `ctx.self_index` 还是字面量
    /// 0"在测试里不可辨（等价变异）。`vm.rs` 已有 `self_index: 7` 的先例。
    fn call_at(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        self_index: u16,
    ) -> Result<(), u8> {
        call_full(w, ecl, task, no, args, &TABLES_V0, self_index)
    }

    fn call_full(
        w: &mut World,
        ecl: &EclImage,
        task: &mut Task,
        no: u16,
        args: &[i32],
        tables: &crate::tables::WorldTables,
        self_index: u16,
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
            tables,
            self_index,
            frame,
        };
        dispatch(no, task, &mut ctx)
    }

    /// 合成一张"某格是空格"的表（其余与内建表逐位相同）。
    fn tables_with_hole(hole: usize) -> crate::tables::WorldTables {
        let mut t = crate::tables::build_tables_v0();
        let mut rows = t.appearances.to_vec();
        rows[hole].valid = false;
        t.appearances = rows.into_boxed_slice();
        t
    }

    fn fresh() -> (Box<World>, EclImage) {
        (World::new(1), EclImage::empty())
    }

    /// 单个 0 参 Async sub（raw=1）的镜像——`spell_begin` 的 `pattern:SubRef` 测试专用
    /// （代码体本身无关紧要，只需能被 `run_tasks` 安全 wait）。
    fn async_pattern_image() -> EclImage {
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                1,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("pattern", 1)],
            Some(0),
        )
    }

    /// 单个 **CallOnly**（非 Async）sub（raw=1）的镜像——task 号支路③专用（B25）：
    /// 号在册（`sub_id`/`sub_meta` 均命中）但 `kind() != SubKind::Async`，用来钉死
    /// "在册但不是零参 Async" 这条校验，而不是被支路②（号不在册）顺手拦下。
    /// CallOnly sub 不需要 entry 表项（`MissingAsyncEntry` 只查 Async sub）。
    fn wrong_kind_task_image() -> EclImage {
        test_image(
            vec![crate::ecl::ops::OP_RET as u32],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::CallOnly, vec![]),
            ],
            vec![],
            Some(0),
        )
    }

    /// 单个**带参** `Async` sub（raw=1，一个 `Fx` 参）的镜像——支路③另一半专用（终审 M-4）：
    /// `kind() != Async || !param_types().is_empty()` 这个判据是**或**，`wrong_kind_task_image`
    /// 只打了左半（kind 不对）；这个 helper 打右半（kind 对但带参）——号在册、`sub_meta`
    /// 命中、`kind()==Async`，唯独 `param_types` 非空。若把 `!param_types().is_empty()`
    /// 那半判据删掉，这里必须能把变异逮住（否则"带参 Async sub 不许当 task 挂载"这条
    /// 约束就是无网状态）。先例：`image.rs:606` 的 `SubInit::new(0, SubKind::Async,
    /// vec![EclValueType::Fx])`。
    fn async_with_param_task_image() -> EclImage {
        test_image(
            vec![
                crate::ecl::ops::OP_PUSHI as u32,
                1,
                crate::ecl::ops::OP_WAIT as u32,
            ],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![crate::ecl::image::EclValueType::Fx]),
            ],
            vec![EntryInit::new("pattern", 1)],
            Some(0),
        )
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
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
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
            assert_eq!(w.body.bullets.sprite[i], cfg.sprite);
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
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let idx = task.stack[0];
        assert!(idx >= 0);
        let i = idx as usize;
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
            let args = [Fx::ZERO.raw(), Fx::from_int(80).raw(), 10, bad, 100, 0, -1];
            assert!(
                call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok(),
                "越界表号不得 Fault（P4-b 降级，不是违约方的锅）"
            );
            let idx = task.stack[0];
            assert!(idx >= 0, "敌照建（表号 {bad}）");
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
                w.body.enemies.drop_count[idx as usize],
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
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = task.stack[0] as u16;
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
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = task.stack[0];
        assert!(eidx >= 0);
        assert_eq!(
            w.body.enemies.main_task[eidx as usize], 0,
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
        let args = [0, 0, 100, 0, 0, 0, 65537]; // 65537 > u16::MAX，但截断后=1（在册）
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
        let args = [0, 0, 100, 0, 0, 0, 1]; // task_script=1，在册但 CallOnly
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
        let args = [0, 0, 100, 0, 0, 0, 1]; // task_script=1，在册、Async，但带参
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
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = task.stack[0];
        assert!(eidx >= 0, "任务池满不应阻止敌建成");
        assert_eq!(
            w.body.diag.pool_full[crate::world::POOL_TASK],
            1,
            "任务池满应计一次 P4-a 降级"
        );
        assert_eq!(
            w.body.enemies.main_task[eidx as usize], 0,
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
        ];
        assert!(call(&mut w, &ecl, &mut task, SYS_SPAWN_ENEMY, &args).is_ok());
        let eidx = task.stack[0] as u16;
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

    /// `enemy_hp`（SYS 12）：活敌返当前 hp（判别值 77，非默认）；死敌/越界句柄均返 -1
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
        assert!(call(&mut w, &ecl, &mut task, SYS_ENEMY_HP, &[eh.index as i32]).is_ok());
        assert_eq!(task.stack[0], 77, "活敌返当前 hp（判别值，非 hp_max）");

        w.body.enemies.free(eh);
        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ENEMY_HP, &[eh.index as i32]).is_ok());
        assert_eq!(task.stack[0], -1, "死敌返 -1");

        task.sp = 0;
        assert!(call(&mut w, &ecl, &mut task, SYS_ENEMY_HP, &[9999]).is_ok());
        assert_eq!(task.stack[0], -1, "越界句柄返 -1（P4-b 不 Fault）");
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

    // ── SYS_SPELL_BEGIN/END/TIMER（Task 2；符卡机构 spec 2026-07-24 §5）─────────

    fn enemy_task(w: &mut World, hp: i32) -> (EnemyHandle, Task) {
        let boss = crate::world::test_support::spawn_enemy(w, 0, 80, hp);
        let task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: boss.index,
            owner_gen: boss.generation,
            ..Task::default()
        };
        (boss, task)
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

    // ── SYS_ADD_SCORE/SYS_BGM/SYS_BG/SYS_BG_PHASE（整局流程刀 Task 2；5x 族）────────

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

        assert_eq!(w.body.events_len, 1, "field 消弹应聚合发一条事件");
        assert_eq!(w.body.events[0].kind, EVT_FIELD_CLEARED);
        assert_eq!(w.body.events[0].data[0], 3, "data[0] == 弹数");
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

    /// B20：`add_bombs(d)` 同构（独立字段——判别腿：写错字段会让 lives 动而 bombs 不动）。
    #[test]
    fn add_bombs_clamps_both_ends() {
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

        w.body.players[0].bombs = u8::MAX;
        assert!(call(&mut w, &ecl, &mut task, SYS_ADD_BOMBS, &[1]).is_ok());
        assert_eq!(w.body.players[0].bombs, u8::MAX, "上钳 u8::MAX");
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
            assert_eq!(v, u8::MAX, "syscall {no}：i32::MAX 应钳到 u8::MAX");

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

    // ── 敌人死亡效果四 syscall（58-61；敌人死亡效果刀 T3）────────────────────

    /// 造一只 owner 敌 + 指向它的任务（掉落计数**空**——要灌表 1 用 `load_drop_table_1`）。
    fn enemy_owner_task(w: &mut World, hp: i32, score: u16) -> (EnemyHandle, Task) {
        let eh = crate::world::test_support::spawn_enemy(w, 0, 0, hp);
        w.body.enemies.score[eh.index as usize] = score;
        let task = Task {
            owner_kind: OWNER_ENEMY,
            owner_index: eh.index,
            owner_gen: eh.generation,
            ..Task::default()
        };
        (eh, task)
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

    // ── Shooter 配置面（syscall 62-75；shooter 刀 T2 2026-07-31）──────────────
    //
    // 四条测试全是**表驱动**的：14 个 setter 各写一段复制粘贴既臃肿、又保证将来加第 15 个
    // 时漏掉——而漏掉的那个恰恰是最可能忘写边界检查/写串字段的那个。表在一处，加一行即入网。

    /// 一个 setter 的完整描述：syscall 号 + `id` 之后的实参（正序）+ "它应当造成的改变"。
    /// `mutate` 是**期望的**改变（作用在 `ShooterSlot::default()` 上），与实际派发结果整槽比对
    /// ⇒ 既验"该改的改了"，又验"不该改的一个字节都没动"。
    struct ShCase {
        no: u16,
        /// `id` 之后的实参，脚本声明顺序（正序压栈）。
        rest: &'static [i32],
        mutate: fn(&mut ShooterSlot),
    }

    /// 14 个 setter 的单一权威表（新增 setter 就加一行——四条测试全部自动覆盖它）。
    fn shooter_cases() -> Vec<ShCase> {
        vec![
            // `sh_reset` 作用在默认槽上是恒等——它自己的判别腿在
            // `sh_reset_restores_every_field_to_default`（先弄脏再重置）。
            ShCase {
                no: SYS_SH_RESET,
                rest: &[],
                mutate: |_s| {},
            },
            ShCase {
                no: SYS_SH_SPRITE,
                rest: &[19], // 编译期已折叠成单个 appearance（同 fire/batch）
                mutate: |s| s.appearance = 19,
            },
            ShCase {
                no: SYS_SH_OFFSET,
                rest: &[0x0001_0000, 0x0002_0000],
                mutate: |s| {
                    s.off_x = Fx::from_raw(0x0001_0000);
                    s.off_y = Fx::from_raw(0x0002_0000);
                },
            },
            ShCase {
                no: SYS_SH_OFFSET_ABS,
                rest: &[0x0003_0000, 0x0004_0000],
                mutate: |s| {
                    s.off_x = Fx::from_raw(0x0003_0000);
                    s.off_y = Fx::from_raw(0x0004_0000);
                    s.flags |= SH_ABS_OFFSET;
                },
            },
            ShCase {
                no: SYS_SH_OFFSET_RAD,
                rest: &[0x2000, 0x0005_0000],
                mutate: |s| {
                    s.polar_ang = Angle(0x2000);
                    s.polar_r = Fx::from_raw(0x0005_0000);
                },
            },
            ShCase {
                no: SYS_SH_DIST,
                rest: &[0x0006_0000],
                mutate: |s| s.dist = Fx::from_raw(0x0006_0000),
            },
            ShCase {
                no: SYS_SH_ANGLE,
                rest: &[0x1234, 0x0100],
                mutate: |s| {
                    s.angle0 = Angle(0x1234);
                    s.angle_step = Angle(0x0100);
                },
            },
            ShCase {
                no: SYS_SH_SPEED,
                rest: &[0x0002_8000, -0x0000_4000],
                mutate: |s| {
                    s.speed0 = Fx::from_raw(0x0002_8000);
                    s.speed_step = Fx::from_raw(-0x0000_4000);
                },
            },
            ShCase {
                no: SYS_SH_COUNT,
                rest: &[7, 3],
                mutate: |s| {
                    s.n_angle = 7;
                    s.n_speed = 3;
                },
            },
            ShCase {
                no: SYS_SH_AIM,
                rest: &[1],
                mutate: |s| s.flags |= SH_AIMED,
            },
            ShCase {
                no: SYS_SH_RING,
                rest: &[1],
                mutate: |s| s.flags |= SH_RING,
            },
            ShCase {
                no: SYS_SH_XFORM,
                rest: &[9, 2], // codegen 把 XformRef 降低成 (off, cnt) 两个栈值
                mutate: |s| {
                    s.xform_off = 9;
                    s.xform_cnt = 2;
                },
            },
            ShCase {
                no: SYS_SH_TASK,
                rest: &[3],
                mutate: |s| s.task_script = 3,
            },
            ShCase {
                no: SYS_SH_REQ,
                rest: &[42],
                mutate: |s| s.on_fire_req = 42,
            },
        ]
    }

    /// `id` 打头的完整实参（正序）。
    fn sh_args(id: i32, rest: &[i32]) -> Vec<i32> {
        let mut v = vec![id];
        v.extend_from_slice(rest);
        v
    }

    /// `fresh()` + 把 0 号任务的 shooter 槽置成 `spawn` 会给的初值。
    ///
    /// ⚠️ **必须显式做这一步**：`World::new` 走 `alloc_zeroed`（见 `step::World::new`），
    /// 而 `ShooterSlot::default()` **非全零**（`n_angle/n_speed=1`、`task_script=0xFFFF`）——
    /// 生产路径上是 `TaskPool::spawn` 负责重置，而本模块的 `call` 助手用的是手拼 `Task`
    /// （从没经过 `spawn`）。不补这一步，测试比对的基线就是"全零槽"而非真实初值，
    /// `sh_reset` 一类断言会以看不出所以然的方式红。坑档见 `TaskPool::new` 文档。
    fn fresh_with_shooters() -> (Box<World>, EclImage) {
        let (mut w, ecl) = fresh();
        w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        (w, ecl)
    }

    /// 每个 setter 只改自己那一维，其余字段纹丝不动。
    /// 这条守的是"14 个派发臂没有互相串写"——串写在只测单个 setter 的测试里看不出来。
    #[test]
    fn each_shooter_setter_touches_only_its_own_field() {
        for c in shooter_cases() {
            let (mut w, ecl) = fresh_with_shooters();
            let mut task = Task::default();
            assert!(
                call(&mut w, &ecl, &mut task, c.no, &sh_args(1, c.rest)).is_ok(),
                "syscall {} 应正常返回",
                c.no
            );
            let mut want = ShooterSlot::default();
            (c.mutate)(&mut want);
            assert_eq!(
                w.tasks.shooters[0][1], want,
                "syscall {} 改动的字段集不符（多改/少改/串写）",
                c.no
            );
            for k in [0usize, 2, 3] {
                assert_eq!(
                    w.tasks.shooters[0][k],
                    ShooterSlot::default(),
                    "syscall {} 串写到了槽 {k}",
                    c.no
                );
            }
            assert_eq!(task.sp, 0, "syscall {} 未把实参全部弹栈", c.no);
        }
    }

    /// `sh_offset` 与 `sh_offset_abs` 写的是同一对字段，只是解释方式不同——**后写的赢**。
    /// 判别腿：先 abs 后 rel，`SH_ABS_OFFSET` 必须被**清掉**（不是只置不清）。
    #[test]
    fn offset_abs_flag_is_set_and_cleared_by_the_two_setters() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let ten = Fx::from_int(10).raw();
        let twenty = Fx::from_int(20).raw();
        let thirty = Fx::from_int(30).raw();
        let forty = Fx::from_int(40).raw();

        assert!(
            call(
                &mut w,
                &ecl,
                &mut task,
                SYS_SH_OFFSET_ABS,
                &[0, ten, twenty]
            )
            .is_ok()
        );
        let s = w.tasks.shooters[0][0];
        assert_ne!(s.flags & SH_ABS_OFFSET, 0, "abs 设完标志应置位");
        assert_eq!((s.off_x.raw(), s.off_y.raw()), (ten, twenty));

        assert!(call(&mut w, &ecl, &mut task, SYS_SH_OFFSET, &[0, thirty, forty]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(
            s.flags & SH_ABS_OFFSET,
            0,
            "相对模式必须把 SH_ABS_OFFSET **清掉**（只置不清 = 一旦 abs 过就再也回不去）"
        );
        assert_eq!((s.off_x.raw(), s.off_y.raw()), (thirty, forty), "后写的赢");
    }

    /// `sh_reset` 恢复**全部**默认（不只是清几个字段）。
    #[test]
    fn sh_reset_restores_every_field_to_default() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        // 14 个 setter 全调一遍把槽 0 弄脏（`sh_reset` 自己那行是恒等，无所谓）。
        for c in shooter_cases() {
            assert!(call(&mut w, &ecl, &mut task, c.no, &sh_args(0, c.rest)).is_ok());
        }
        assert_ne!(
            w.tasks.shooters[0][0],
            ShooterSlot::default(),
            "前置条件：整套 setter 跑完槽必须确实脏了（否则本测试是假绿）"
        );
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_RESET, &[0]).is_ok());
        assert_eq!(
            w.tasks.shooters[0][0],
            ShooterSlot::default(),
            "sh_reset 必须恢复整槽默认，不是清几个字段"
        );
    }

    /// P4-b：`id ≥ SHOOTERS_PER_TASK` → no-op + contract_viol + STATUS_BAD_ARGS。
    /// **14 个 setter 各一腿**（表驱动）——只测其中一个的话，漏写边界检查的那几个不会红。
    #[test]
    fn every_shooter_setter_rejects_out_of_range_id() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let mut expect_viol = w.body.diag.contract_viol;
        for c in shooter_cases() {
            for bad_id in [SHOOTERS_PER_TASK as i32, -1] {
                w.body.last_status = crate::world::STATUS_OK;
                assert!(
                    call(&mut w, &ecl, &mut task, c.no, &sh_args(bad_id, c.rest)).is_ok(),
                    "syscall {} 越界 id 是 no-op 而非 Fault",
                    c.no
                );
                expect_viol += 1;
                assert_eq!(
                    w.body.diag.contract_viol, expect_viol,
                    "syscall {} 的 id={bad_id} 腿应记一次 contract_viol",
                    c.no
                );
                assert_eq!(
                    w.body.last_status,
                    crate::world::STATUS_BAD_ARGS,
                    "syscall {} 的 id={bad_id} 腿应置 STATUS_BAD_ARGS",
                    c.no
                );
                // no-op 的判别腿：只查 contract_viol 证不了"没写脏"。
                for k in 0..SHOOTERS_PER_TASK {
                    assert_eq!(
                        w.tasks.shooters[0][k],
                        ShooterSlot::default(),
                        "syscall {} 的 id={bad_id} 腿写脏了槽 {k}",
                        c.no
                    );
                }
                assert_eq!(task.sp, 0, "syscall {} 越界腿也应把实参全部弹栈", c.no);
            }
        }
    }

    /// 复审 ②：`sh_aim`/`sh_ring` 的**清位**方向。
    ///
    /// `shooter_cases()` 那张表每个 setter 只喂一组入参，两条旗标 setter 喂的都是 `on = 1`
    /// ⇒ 把两个派发臂里的 `else { s.flags &= !BIT }` **整个删掉，表驱动那三条测试全绿**。
    /// 这跟 `sh_offset` 的"只置不清"是同一个 bug 类，只是表结构塞不下"同一 setter 两种
    /// 入参"，故另开一条定向腿。
    #[test]
    fn aim_and_ring_flags_are_cleared_by_passing_zero() {
        for (no, bit, who) in [
            (SYS_SH_AIM, SH_AIMED, "sh_aim"),
            (SYS_SH_RING, SH_RING, "sh_ring"),
        ] {
            let (mut w, ecl) = fresh_with_shooters();
            let mut task = Task::default();
            assert!(call(&mut w, &ecl, &mut task, no, &[0, 1]).is_ok());
            assert_ne!(w.tasks.shooters[0][0].flags & bit, 0, "{who}(id, 1) 应置位");
            assert!(call(&mut w, &ecl, &mut task, no, &[0, 0]).is_ok());
            assert_eq!(
                w.tasks.shooters[0][0].flags & bit,
                0,
                "{who}(id, 0) 必须把位**清掉**（只置不清 = 开了就再也关不上）"
            );
            // 判别腿：清的是自己那一位，没顺手把另一位也抹了。
            assert_eq!(
                w.tasks.shooters[0][0],
                ShooterSlot::default(),
                "{who} 一置一清之后整槽应回到默认"
            );
        }
    }

    /// 复审 ③：shooter 存储按**任务索引**键——派发臂必须用 `ctx.self_index`，不是字面量 0。
    /// 助手把 `self_index` 写死 0 时这条是等价变异（改成 `0` 全绿）。
    #[test]
    fn setters_write_the_slots_of_the_calling_task_not_task_zero() {
        let (mut w, ecl) = fresh();
        w.tasks.shooters[0] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        w.tasks.shooters[7] = [ShooterSlot::default(); SHOOTERS_PER_TASK];
        let mut task = Task::default();
        assert!(call_at(&mut w, &ecl, &mut task, SYS_SH_DIST, &[2, 0x0006_0000], 7).is_ok());
        assert_eq!(
            w.tasks.shooters[7][2].dist,
            Fx::from_raw(0x0006_0000),
            "写的应是 self_index=7 的槽 2"
        );
        for k in 0..SHOOTERS_PER_TASK {
            assert_eq!(
                w.tasks.shooters[0][k],
                ShooterSlot::default(),
                "0 号任务的槽 {k} 不该被碰（派发臂若写死 0 就会红在这里）"
            );
        }
    }

    /// 复审 ①：`sh_sprite` 的收窄**必须保号越界性**。
    ///
    /// 负折叠值（`sh_sprite(0, 16, c)` 里 `c` 是变量、typeck 判据跳过 ⇒ `c = -20` 折成 -4）
    /// 若被钳成 **0**，就落在 `appearances[0]` 这个**在册且 valid** 的格子上 ⇒ T3 开火时
    /// 再也拒不掉。`fire` 侧同样的值是 `FAULT_BAD_OP`，两条路不该有这种差别。
    #[test]
    fn sh_sprite_narrowing_keeps_out_of_range_values_out_of_range() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        let n_rows = TABLES_V0.appearances.len();
        assert!(
            TABLES_V0.appearances[0].valid,
            "前置条件：0 号格在册且 valid——正因为如此，把负值钳成 0 才是事故"
        );

        for bad in [-4, -1, i32::MIN] {
            assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, bad]).is_ok());
            let got = w.tasks.shooters[0][0].appearance;
            assert_ne!(got, 0, "负 appearance {bad} 不得被洗成合法的 0 号格");
            assert!(
                got as usize >= n_rows,
                "负 appearance {bad} 收窄后仍须落在表外（表 {n_rows} 行，实际存了 {got}）"
            );
        }
        // 上沿：超 u16 同样保持越界。
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 100_000]).is_ok());
        assert!(w.tasks.shooters[0][0].appearance as usize >= n_rows);
        // 合法值照常原样存（防"一律存 u16::MAX"这种把测试骗绿的实现）。
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_SPRITE, &[0, 19]).is_ok());
        assert_eq!(w.tasks.shooters[0][0].appearance, 19);
    }

    /// `sh_req` 的收窄（复审 ① 顺带点名的覆盖空缺）。这里钳到 **0 是对的**——`0` 在
    /// `ShooterSlot::on_fire_req` 上有明确语义（"不发请求"），不像 appearance 的 0 是个
    /// 在册格子；负 req id 降级成"不发"是 P4-b 的确定性安全结果。
    #[test]
    fn sh_req_narrowing_degrades_bad_ids_to_no_request() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, -7]).is_ok());
        assert_eq!(w.tasks.shooters[0][0].on_fire_req, 0, "负 id → 不发请求");
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_REQ, &[0, 100_000]).is_ok());
        assert_eq!(
            w.tasks.shooters[0][0].on_fire_req,
            u16::MAX,
            "超 u16 上钳，不回绕（裸 `as u16` 会得 34464）"
        );
    }

    /// P4-b：`sh_count` 的参数是脚本给的任意 i32，先钳后存，不回绕不 panic。
    #[test]
    fn sh_count_clamps_both_ends() {
        let (mut w, ecl) = fresh_with_shooters();
        let mut task = Task::default();
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, -5, i32::MAX]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(s.n_angle, 0, "负数下钳到 0（裸 `as u8` 会回绕成 251）");
        assert_eq!(s.n_speed, 255, "i32::MAX 上钳到 255");
        assert!(call(&mut w, &ecl, &mut task, SYS_SH_COUNT, &[0, 256, -256]).is_ok());
        let s = w.tasks.shooters[0][0];
        assert_eq!(s.n_angle, 255, "256 上钳到 255（裸 `as u8` 会回绕成 0）");
        assert_eq!(s.n_speed, 0, "-256 下钳到 0");
    }
}
