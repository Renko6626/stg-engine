//! 作用区池（消弹区 / 伤敌区）——`define_pool!` 第 4 个实例。
//!
//! **通用哑原语**：bomb 只是首个租户；符卡切换清弹、阶段清场、ECL 死亡脚本清弹都是平等租户，
//! 一律走 `WorldBody::create_field` 写 API。
//!
//! **静止**：世界层零 follow 逻辑。跟随 = 上层每帧在目标位重铺 `life=1`（`life=1` 恰好活一帧
//! 且当帧生效：相位5 减到 0、相位6 alive 位仍在照常判定、相位9 才回收）；静止爆炸 = 铺一次 `life=N`。

use crate::define_pool;
use crate::math::Fx;

/// 能力位：启用碰撞矩阵行 6（Field × EnemyBullet → 消弹）。
pub const FIELD_CLEAR_BULLETS: u8 = 1 << 0;
/// 能力位：启用碰撞矩阵行 7（Field × EnemyBody → 按 `dmg_per_frame` 扣血）。
pub const FIELD_DAMAGE: u8 = 1 << 1;

/// 覆盖全场含越界边距的半径。
///
/// 场界 x∈[-192,192]、y∈[0,448]，边距 64 → 弹最远可在 (±256, −64..512)；field 置场心 (0,224)
/// 到最远角 = √(256² + 288²) = √148480 ≈ 385.3 px < 400。给脚本算好的常量，免得各自猜。
pub const FIELD_RADIUS_FULLSCREEN: Fx = Fx::from_int(400);

/// `create_field` 的半径钳制上限（P4-b）。
///
/// `Fx` 上限 32767.99998。若调用方传 32767 表达"无限大"，`field.radius + bullet.radius` 的
/// **Fx 加法会溢出** → debug panic / release 回绕成负数 → 平方后全场无条件判撞（debug/release 分歧）。
/// 钳到 1024 后 `1024 + 16 ≪ 32767`，Fx 加法永不溢出，行 6/7 得以与行 1-4 写法完全一致。
pub const FIELD_MAX_RADIUS: Fx = Fx::from_int(1024);

define_pool! {
    Field, cap = 16,
    fields {
        x: Fx, y: Fx, radius: Fx,
        dmg_per_frame: u16, life: u16,
        owner: u8, flags: u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksum::Checksum;

    fn field_at(x: i32, y: i32, radius: i32, flags: u8) -> FieldInit {
        FieldInit {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            radius: Fx::from_int(radius),
            dmg_per_frame: 0,
            life: 1,
            owner: 0,
            flags,
        }
    }

    #[test]
    fn field_pool_alloc_get_free() {
        let mut p = FieldPool::new();
        let h = p.alloc(field_at(0, 100, 20, FIELD_CLEAR_BULLETS)).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.radius[i], Fx::from_int(20));
        assert_eq!(p.flags[i], FIELD_CLEAR_BULLETS);
        assert!(p.free(h));
        assert_eq!(p.get(h), None);
    }

    #[test]
    fn field_pool_new_deterministic() {
        assert_eq!(FieldPool::new().checksum(), FieldPool::new().checksum());
    }

    #[test]
    fn fullscreen_radius_covers_farthest_corner() {
        // 场心 (0,224) 到最远角 (256, 512)：dx=256, dy=288 → dist ≈ 385.3 < 400
        let d2 = crate::math::geom::len_sq(Fx::from_int(256), Fx::from_int(288));
        let r = FIELD_RADIUS_FULLSCREEN.raw() as i64;
        assert!(d2 < r * r, "全屏半径必须覆盖最远角");
    }
}
