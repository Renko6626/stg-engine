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

/// `create_field` 的半径钳制上限（P4-b）——即 `world::MAX_ENTITY_RADIUS`。
///
/// `Fx` 上限 32767.99998。若调用方传 32767 表达"无限大"，`field.radius + bullet.radius` 的
/// **Fx 加法会溢出** → debug panic / release 回绕成负数 → 平方后全场无条件判撞（debug/release 分歧）。
/// 但只钳这一侧不构成完整证明——被动半径（弹/敌人/自机弹/自机 hit_radius/graze_radius）若无
/// 上限，同样能把和推过 `i32::MAX`。真正的证明分两条、强度不同：`create_bullet`/`create_enemy`/
/// `create_player_shot` 与本 API 共用同一常量，把池侧半径各自钳入 `[0, 1024]`——这一侧因池的
/// SoA 数组 `pub(crate)` 而是写 API **可强制**的；行 1/2/3 的被动操作数（自机半径）不经写 API，
/// 只由 `player.rs` 的编译期断言钉住上限，前提是没人绕过 `PlayerState::spawn` 直接写字段
/// （`WorldBody.players` 已收 `pub(crate)`——刀 A 2026-07-21，crate 外无绕行路径；
/// crate 内绕过 `spawn` 直写仍属纪律约束）。两侧都 ≤1024 时
/// 任意两半径之和 ≤ 2048 ≪ 32767，六行碰撞的 Fx 加法才不溢出。完整推导见 `world::MAX_ENTITY_RADIUS`。
pub const FIELD_MAX_RADIUS: Fx = crate::world::MAX_ENTITY_RADIUS;

define_pool! {
    Field, cap = 16,
    fields {
        x: Fx, y: Fx, radius: Fx,
        dmg_per_frame: u16, life: u16,
        owner: u8, flags: u8
    }
}

/// 全场消弹区的**唯一构造口**——置场心、`life=1`（活一帧且当帧生效）、纯消弹无伤害。
///
/// 两个调用方：符卡结算清弹（`world::WorldBody::settle_one_spell`）/ `clear_bullets()`
/// syscall（`ecl::syscall::SYS_CLEAR_BULLETS`）。**改这里等于同时改两处，这正是抽它的目的**
/// ——此前是两份逐字段相同的 `FieldInit` 字面量，将来谁给其中一处加个标志位
/// （比如 bomb 刀加 `FIELD_DAMAGE`），另一处不会跟着变，且没有任何测试会红。
///
/// 想要"带伤害的全屏区"（bomb）的人：**别改这个函数**，另起一个构造口——本函数的语义
/// 由它的两个现有调用方钉死。
pub(crate) fn fullscreen_clear_field() -> FieldInit {
    FieldInit {
        x: Fx::ZERO,
        y: Fx::from_int(crate::world::FIELD_HEIGHT / 2),
        radius: FIELD_RADIUS_FULLSCREEN,
        dmg_per_frame: 0,
        life: 1,
        owner: 0,
        flags: FIELD_CLEAR_BULLETS,
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
