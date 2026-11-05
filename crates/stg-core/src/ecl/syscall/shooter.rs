use super::*;

/// 取本任务的第 `id` 个 shooter（可变）。`id` 越界 → `None` + `contract_viol` + `BAD_ARGS`
/// （P4-b：no-op，不 Fault——见 600-660 号表注释）。`id` 是脚本给的任意 `i32`，负数与超界同处置。
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

pub(super) fn sys_sh_reset(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        *s = ShooterSlot::default();
    }
    Ok(())
}

pub(super) fn sys_sh_sprite(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_offset(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_offset_abs(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_offset_rad(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let r = pop(task)?;
    let ang = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.polar_ang = bam(ang);
        s.polar_r = Fx::from_raw(r);
    }
    Ok(())
}

pub(super) fn sys_sh_dist(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let d = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.dist = Fx::from_raw(d);
    }
    Ok(())
}

pub(super) fn sys_sh_angle(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let step = pop(task)?;
    let angle0 = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.angle0 = bam(angle0);
        s.angle_step = bam(step);
    }
    Ok(())
}

pub(super) fn sys_sh_speed(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let step = pop(task)?;
    let speed0 = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.speed0 = Fx::from_raw(speed0);
        s.speed_step = Fx::from_raw(step);
    }
    Ok(())
}

pub(super) fn sys_sh_count(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_aim(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_ring(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_xform(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    // 表层 `XformRef` 在 codegen 里降低成 `(off, cnt)` **两个**栈值。`cnt == 0` 即无
    // xform。**此处不校验区间**——`off + cnt*3 ≤ LOCALS` 在开火时查（与 `fire` 同
    // 口径：`fire` 也是在建弹那一刻才查 `LOCALS` 边界）。这里只做**存储收窄**的
    // 饱和钳，防负值/超宽裸 `as` 回绕成一个看似合法的小区间。
    let cnt = pop(task)?;
    let off = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.binding = Default::default();
        s.xform_off = off.clamp(0, u16::MAX as i32) as u16;
        s.xform_cnt = cnt.clamp(0, u8::MAX as i32) as u8;
    }
    Ok(())
}

pub(super) fn sys_sh_task(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
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

pub(super) fn sys_sh_req(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let req_id = pop(task)?;
    let id = pop(task)?;
    if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
        s.on_fire_req = req_id.clamp(0, u16::MAX as i32) as u16;
    }
    Ok(())
}

/// 开火（SYS 660）：按 spec §10 的七步用发射器槽 `id` 的参数造弹。1 参、**无返回值**。
///
/// **为什么开火循环住这儿而不是扩 `create_bullets_batch`**：P1——world 不知道"任务"存在，
/// 没法逐颗挂 `task_script`（`fire` 的挂任务也因此发生在 ECL 层）。代价是网格循环有了
/// 第二份实现，故 `shooter_fan_matches_batch_with_centering_compensation` 那条等价测试
/// 是**必需**而非顺带的。
///
/// 校验序照抄 `sys_create_bullet` 的"先验后建"：退化网格（P4-b no-op）→ appearance
/// （Fault）→ xform 区间（Fault）→ 挂弹任务号（Fault）——一切拒绝都发生在任何世界写之前。
pub(super) fn sys_sh_fire(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let id = pop(task)?;
    // 先取一份**拷贝**：后面要可变借用 `ctx.body`/`ctx.tasks` 建弹派任务，不能持着
    // `shooters` 的引用。顺带一层安全网——`tasks.spawn` 的"复用槽写满"会把新槽的
    // shooter 抹成默认，拿引用读到一半被抹是个隐蔽的自伤。
    let Some(sh) = shooter_mut(id, ctx.self_index, ctx).map(|s| *s) else {
        return Ok(()); // 越界 id：`shooter_mut` 已记 contract_viol + BAD_ARGS（P4-b）
    };

    // ① 退化网格（对齐 `create_bullets_batch` 的既有口径：不发 + contract_viol，不 Fault）。
    let total = sh.n_angle as u32 * sh.n_speed as u32;
    if sh.n_angle == 0 || sh.n_speed == 0 || total > crate::bullets::BulletPool::CAP as u32 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // ② 外观：越界/空格一律拒（同 `fire`——放行会造出"有判定但画面上什么都没有"的隐形弹，
    //    而校验和不关心贴图内容，金向量/冒烟都抓不到它）。T2 的 `sh_sprite` 收窄之所以
    //    必须保号越界性，就是为了让这一步还拒得掉。
    let tables = ctx.tables;
    let Some(cfg) = tables.appearances.get(sh.appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }
    let (sprite, radius) = (sh.appearance, cfg.radius);

    // ③ xform 解包（照抄 `sys_create_bullet` 的 LOCALS 边界校验与解包循环）。数据住
    //    **本任务的 locals**，由 codegen 在 sub 入口一次性 staging、`slots` 的调用图着色
    //    保证区间不被复用 ⇒ 存 `(off, cnt)`、开火时才读是安全的（跨帧亦然，有测试钉着）。
    //    存储侧已是 u8/u16，负值不可达；只剩长度与区间两条。
    let mut xform_buf = [XformSlot::default(); 16];
    let xform_len: usize = if sh.xform_cnt == 0 {
        0
    } else {
        let cnt = sh.xform_cnt as usize;
        if cnt > 16 {
            return Err(FAULT_BAD_OP);
        }
        let off = sh.xform_off as usize;
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

    // ④ 挂弹任务号先验（照抄 `sys_create_bullet`：坏号 → Fault，零副作用，弹未建）。
    let task_sub: Option<SubId> = if sh.task_script == SH_NO_TASK {
        None
    } else {
        let sub = ctx.ecl.sub_id(sh.task_script).ok_or(FAULT_BAD_OP)?;
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
    };

    // ⑤ 原点（spec §10 步 2）：直角偏移与极坐标偏移**相加**（ZUN 626 明写 stacks），
    //    不是覆盖——判别腿见 `rect_and_polar_offsets_stack_rather_than_override`。
    let (bx, by) = if sh.flags & SH_ABS_OFFSET != 0 {
        (Fx::ZERO, Fx::ZERO)
    } else {
        self_pos(task, ctx)
    };
    let (px, py) = polar_to_vec(sh.polar_r, sh.polar_ang);
    // ⚠️ **裸 `+` 的溢出归 P4-c 域**（引擎自身 bug 才该触发的那一档：debug 溢出 panic、
    // release 回绕），不是 P4-b 的"调用方违约要降级"。三个 `Fx` 相加没有钳制——
    // `sh_offset(0, 32767.0fx, 0fx)` 配一个 x 非零的 owner 就能在 debug 下 panic。
    // **这不是本刀新立的立场**：`world.rs::create_bullets_batch` 的文档已经声明过
    // "两轴累加器：角度 BAM 回绕、速度 Fx 裸加（溢出 P4-c 域）"，而 ⑦ 的 `cur_speed +
    // sh.speed_step` 继承的正是那个累加器。本处（偏移相加）只是那条声明**没覆盖到的
    // 同类一处**——本仓第一次把两个**大坐标**相加（`fire`/`batch` 是把 x/y 直通不加），
    // 故在此显式记一笔，免得后人以为漏了 P4-b 校验。
    // 要改立场（譬如改成 `saturating_add` 或收窄 `sh_offset` 的入参）请连同
    // `create_bullets_batch` 那条一起改——单改一处会让两个发射路径口径分叉，
    // 而 `shooter_fan_matches_batch_with_centering_compensation` 那条等价测试正押着它们。
    let origin_x = bx + sh.off_x + px;
    let origin_y = by + sh.off_y + py;

    // ⑥ 基准角（spec §10 步 3）——**在这一刻**解析 aim。"瞄谁"走引擎唯一口径
    //    `WorldBody::aim_target`（F8 统一，2026-09-03）：从**出弹口**看过去最近的可瞄
    //    自机，一个都没有则回退 `players[0]` 的最后坐标（发射这条路必须产出一个角度，
    //    没有"不瞄"这个选项——那是弹上 setter 那半边的处置）。基点是出弹口而非 owner
    //    位置，判别腿见 `aim_is_measured_from_the_fire_origin_not_from_the_owner`。
    //    角度算术一律在 `i32` 里做、最后才回绕成 `Angle`（`Angle` 底层 u16，直接加会在
    //    debug 触发 overflow-checks）。
    let base: i32 = if sh.flags & SH_AIMED != 0 {
        let p = ctx.body.aim_target(origin_x, origin_y);
        let a = crate::math::cordic::atan2(
            ctx.body.players[p].y - origin_y,
            ctx.body.players[p].x - origin_x,
        );
        a.raw() as i32 + sh.angle0.raw() as i32
    } else {
        sh.angle0.raw() as i32
    };

    // ⑦ 网格：角度外层、速度内层（照抄 `create_bullets_batch` 的序 = 池槽分配序，I4）。
    //    fan  以基准方向为**中心**对称展开（D-7）：angle_i = base + i·step − ((n−1)·step)/2
    //    ring 不居中、逐颗算 (i×65536)/n 把余数**均摊**（精确闭合；`i ≤ 254` 故 i32 不溢出）
    //         且 `angle_step` 转义成**逐层**偏移。
    let ring = sh.flags & SH_RING != 0;
    let n_angle = sh.n_angle as i32;
    // **符号扩展是必须的，不是风格问题**（复审 ①，Critical）：`angle_step` 存的是 `Angle`
    // （底层 u16），脚本写 `sh_angle(0, 0deg, -6deg)` 存进去的是 `65536 − 1092`。
    // `i·step`/`j·step` 在 mod 65536 下不受零扩展影响，**但下面的 `/2` 不与 mod 65536 交换**：
    //   `((n−1)(s + 65536))/2 = ((n−1)s)/2 + (n−1)·32768`
    // `n` 奇数 ⇒ 多出项是 65536 的整数倍、无害；**`n` 偶数 ⇒ 多出半圈**，整把扇形被搬到
    // `base` 的正对面（形状还对，故只看"相邻差 step"的测试看不见它）。
    // 读侧符号扩展也正是本仓家规：`create_bullets_batch` 的形参就是 `angle_step: i16`、
    // `Angle::add_delta` 也收 i16。钉死在 `fan_centering_handles_even_ways_with_negative_step`。
    let step = sh.angle_step.raw() as i16 as i32;
    // 奇数路时 (n−1) 为偶数、除 2 精确；偶数路截断半个 BAM 单位（1/65536 圈，确定且可忽略）。
    let fan_center = ((n_angle - 1) * step) / 2;
    let mut created: u32 = 0;
    'grid: for i in 0..n_angle {
        let mut cur_speed = sh.speed0;
        for j in 0..sh.n_speed as i32 {
            let raw = if ring {
                base + (i * 65536) / n_angle + j * step
            } else {
                base + i * step - fan_center
            };
            let angle = bam(raw);
            // `dist`：逐颗沿**各自**角度推出去，不是整环平移（判别腿
            // `dist_pushes_each_bullet_along_its_own_angle`）。dist=0 时 polar_to_vec
            // 恒返 (0,0)，故不必分支。
            let (dx, dy) = polar_to_vec(sh.dist, angle);
            let (vx, vy) = polar_to_vec(cur_speed, angle);
            let init = BulletInit {
                x: origin_x + dx,
                y: origin_y + dy,
                vx,
                vy,
                speed: cur_speed,
                angle,
                ang_vel: 0,
                accel: Fx::ZERO,
                ax: Fx::ZERO,
                ay: Fx::ZERO,
                sprite,
                radius,
                delay: 0,
                life: 0xFFFF,
                flags: 0,
                grazed_by: 0,
                transform_head: crate::xform::XFORM_NONE,
                xform_wait: 0,
                xform_next: 0,
                born_frame: ctx.body.frame,
            };
            let handle = if sh.binding.program_plus_one != 0 {
                ctx.body
                    .create_bullet_with_bound_xform(init, &sh.binding, ctx.ecl.xforms(), tables)
            } else if xform_len == 0 {
                ctx.body.create_bullet(init)
            } else {
                ctx.body
                    .create_bullet_with_xform(init, &xform_buf[..xform_len])
            };
            if handle == BulletHandle::NULL {
                // 短路（同 `batch` 的 `'grid`）：同相位无回收 ⇒ 后续必然同败。
                // **故意不做 `batch` 那样的"剩余批量补计"**——shooter 的失败因不止池满
                // 一种（坏 xform 内容 → `create_bullet_with_xform` 记 contract_viol +
                // BAD_ARGS），批量补 `pool_full[POOL_BULLET]` 会张冠李戴。本颗的计数已由
                // `create_bullet*` 自己按真实原因记过，就到此为止。
                break 'grid;
            }
            created += 1;

            if let Some(task_sub) = task_sub {
                // 号已在 ④ 校验过在册；池满 → 静默计数（P4-a），**弹保留**、不 Fault（同 `fire`）。
                let pc0 = ctx
                    .ecl
                    .sub_meta(task_sub)
                    .expect("已在 ④ 校验过")
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
            cur_speed = cur_speed + sh.speed_step;
        }
    }

    // ⑧ 开火请求（spec §8）：`args[3]` 是**实际**创建数而非请求数（池满时要能区分）。
    if sh.on_fire_req != 0 {
        ctx.body.emit_req(
            sh.on_fire_req,
            [
                origin_x.raw(),
                origin_y.raw(),
                sh.appearance as i32,
                created as i32,
                0,
                0,
            ],
        );
    }
    Ok(())
}
