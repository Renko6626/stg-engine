//! 敌人池（D5）——`define_pool!` 第 3 个实例。SoA ~94 B/敌，cap 256。
//! 本切片：全字段落池 + 直线积分（move_to 插值器/主控 AI 延后，字段惰性）。
//! `flags` 的 dying 位由 settle 置、cleanup 回收（敌人槽活到相位 8 供死亡脚本/表现层读）。

use crate::define_pool;
use crate::math::{Angle, Fx};

/// `flags` 的 dying 标记位（D5 预留位）：settle 命中致死置位，cleanup 回收。
pub const ENEMY_DYING: u8 = 1 << 0;

/// `vel_space`：极坐标插值空间（载体槽 = `(speed.raw(), angle.raw() as i32)`）。
pub const VEL_SPACE_POLAR: u8 = 0;
/// `vel_space`：笛卡尔插值空间（载体槽 = `(vx.raw(), vy.raw())`）。
pub const VEL_SPACE_CART: u8 = 1;

define_pool! {
    Enemy, cap = 256,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        // ── 双表示（敌人运动动词族刀 2026-07-31）：vx/vy 是积分真相，speed/angle 是
        //    作者视图。改任一侧后必须同步另一侧（正向 refresh_enemy_vel_from_polar /
        //    反向 backfill_enemy_polar）——"忘了回填"是弹那边被称作火药桶的同一个坑。
        speed: Fx, angle: Angle,
        // ── 速度插值器（一组，极坐标/笛卡尔共用；vel_space 决定四个载体槽怎么读）。
        //    裸 i32 是**载体**不是标量：polar 空间要装 (Fx, Angle)，cart 空间要装 (Fx, Fx)，
        //    把 Angle 塞进 Fx 字段是 newtype 破坏。同 xform 槽 args:[i32;2] 按 op 重解释。
        vel_from_0: i32, vel_from_1: i32,
        vel_to_0: i32, vel_to_1: i32,
        vel_t: u16, vel_dur: u16,
        vel_easing: u8, vel_active: u8, vel_space: u8,
        // ── 黏滞位：脚本**是否表达过**速度意图（四条速度动词任一置 1，move_to 武装归 0）。
        //    位置插值到点只在它为 0 时清速。**不能拿 vel_active 当判据**——速度插值若先于
        //    位置插值到期（常见写法），到点时 vel_active 已是 0，速度会被误清。
        vel_touched: u8,
        mv_from_x: Fx, mv_from_y: Fx, mv_to_x: Fx, mv_to_y: Fx,
        mv_t: u16, mv_dur: u16, mv_easing: u8, mv_active: u8,
        hp: i32, hp_max: i32,
        radius: Fx, hurtbox: Fx,
        invuln: u16, hit_flash: u8, flags: u8,
        sprite: u16, anm_state: u16,
        // `anm_state` 最后一次被写的帧号（表现契约 v2）：`spawn_enemy` 与 `set_anm_state`
        // 都盖；同状态重设也盖（= 重播，对应 ZUN interrupt 重触发）。表现层算
        // `state_age = frame_after_step − anm_state_frame`。进校验和。
        anm_state_frame: u32,
        // main_task（follow-ups B25 口径，D9 起有消费者）：存"任务槽号+1"（0=无），不带
        // generation。`ecl::vm::run_tasks` 的 D9 自燃判据读它——任务终止时若其槽号命中
        // 本字段即视为"owner 敌的主协程"，自然 `End` 就把 owner 标 `ENEMY_DYING`（ZUN ECL
        // 语义：主协程返回即自燃）。因不带 generation，**任意**终止路径（End/Fault/坏脚本号）
        // 命中即清零——防同槽被子任务复用后误判"主任务还在"（别名防护，见 vm.rs 测试
        // `main_task_slot_reuse_does_not_spuriously_self_destruct`）。敌死后任务被 owner-gate
        // 静默清杀那条路径本字段不清零（owner 已死，读它没有消费者），敌槽复用时会随
        // `EnemyInit`（exhaustive Init）重置为 0。
        main_task: u32, death_script: u16,
        // 掉落计数（敌人死亡效果刀 2026-07-30）：**逐道具类型的待掉落颗数**，敌身上的
        // 可变状态。此前是 `drop_table: u16`（生成时定死的表索引）——改成计数后脚本可以
        // 增量配置（`drop_clear`/`drop_add`），且"撒掉落"与"死亡"得以解耦（`drop_items`）。
        // `drop_table` 未消失，只是退化成 `spawn_enemy` 的**生成参数**：由
        // `tables::drop_counts` 在建敌时展开进本字段，此后无人读表号。
        // 撒的顺序是**类型升序**（I4），见 `world::settle::spill_drops`。
        drop_count: [u8; crate::items::ITEM_TYPE_COUNT],
        score: u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    /// 全字段 Init 助手（exhaustive；move_to/挂钩字段本切片惰性置零）。
    fn enemy_at(x: i32, y: i32, hp: i32) -> EnemyInit {
        EnemyInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            vx: Fx::ZERO,
            vy: Fx::ZERO,
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
            sprite: 0,
            anm_state: 0,
            anm_state_frame: 0,
            main_task: 0,
            death_script: 0,
            drop_count: [0; crate::items::ITEM_TYPE_COUNT],
            score: 100,
        }
    }

    #[test]
    fn enemy_pool_alloc_get_free() {
        let mut p = EnemyPool::new();
        let h = p.alloc(enemy_at(10, 20, 5)).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.hp[i], 5);
        assert_eq!(p.x[i], Fx::from_int(10));
        assert!(p.free(h));
        assert_eq!(p.get(h), None);
    }

    #[test]
    fn enemy_pool_new_deterministic() {
        assert_eq!(EnemyPool::new().checksum(), EnemyPool::new().checksum());
    }
}
