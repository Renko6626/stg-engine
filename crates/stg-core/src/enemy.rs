//! 敌人池（D5）——`define_pool!` 第 3 个实例。SoA ~70 B/敌，cap 256。
//! 本切片：全字段落池 + 直线积分（move_to 插值器/主控 AI 延后，字段惰性）。
//! `flags` 的 dying 位由 settle 置、cleanup 回收（敌人槽活到相位 8 供死亡脚本/表现层读）。

use crate::define_pool;
use crate::math::Fx;

/// `flags` 的 dying 标记位（D5 预留位）：settle 命中致死置位，cleanup 回收。
pub const ENEMY_DYING: u8 = 1 << 0;

define_pool! {
    Enemy, cap = 256,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        mv_from_x: Fx, mv_from_y: Fx, mv_to_x: Fx, mv_to_y: Fx,
        mv_t: u16, mv_dur: u16, mv_easing: u8, mv_active: u8,
        hp: i32, hp_max: i32,
        radius: Fx, hurtbox: Fx,
        invuln: u16, hit_flash: u8, flags: u8,
        sprite: u16, anm_state: u16,
        main_task: u32, death_script: u16, drop_table: u16,
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
            drop_table: 0,
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
