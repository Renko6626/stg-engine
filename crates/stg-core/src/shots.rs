//! 自机弹池（D7 PlayerShot 层）——`define_pool!` 第 2 个实例。行为由角色模块驱动（A8）。
//! "池即层"：本池 = PlayerShot，与 EnemyBullet 的 `bullets` 池天然分阵营（碰撞矩阵按池配对）。

use crate::define_pool;
use crate::math::Fx;

define_pool! {
    Shot, cap = 1024,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        damage: u16, radius: Fx, sprite: u16, owner: u8, flags: u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_pool_alloc_get_free() {
        let mut p = ShotPool::new();
        let h = p
            .alloc(ShotInit {
                x: Fx::ZERO,
                y: Fx::ZERO,
                vx: Fx::ZERO,
                vy: -Fx::from_int(12),
                damage: 1,
                radius: Fx::from_int(4),
                sprite: 0,
                owner: 0,
                flags: 0,
            })
            .unwrap();
        assert_eq!(p.get(h).map(|i| p.damage[i]), Some(1));
        assert!(p.free(h));
        assert_eq!(p.get(h), None);
    }
}
