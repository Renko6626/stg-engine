//! 2D 几何 / 向量原语。
//!
//! **单位纪律（重要）**：`Fx` 是屏幕空间标量（位置/速度/半径）。一旦乘法产出"离开屏幕空间"
//! 的量——平方距离、模平方、点积——它已不是坐标（量纲是像素²），必须走裸 i64、绝不塞回 `Fx`。
//! 否则 `Fx::mul` 的 `>>16 as i32` 会溢出：坐标差一超过约 181px，其平方就出 `Fx` 范围（±32768）。
//! 经验法则：`Fx::mul` 仅当至少一个操作数 ≤ ~1.0（三角值 / easing-t / 归一化权重）时安全。

use crate::math::trig::sincos;
use crate::math::{Angle, Fx};

/// 向量模平方 `dx² + dy²`，裸 i64（单位 raw² = 值²·2³²）。
///
/// 碰撞判定即用它与 `(r.raw() as i64).pow(2)` 直接比较（平方距离，不开根、不塞回 `Fx`）。
/// 满屏最大约 1.7e15，远在 i64 余量（9.2e18）内。
#[inline]
pub fn len_sq(dx: Fx, dy: Fx) -> i64 {
    let x = dx.raw() as i64;
    let y = dy.raw() as i64;
    x * x + y * y
}

/// 极坐标 → 笛卡尔速度：`(vx, vy) = (speed·cos θ, speed·sin θ)`。
///
/// 一次 `sincos` + 两次 `Fx::mul`（此处 mul 安全：三角值 ≤ 1.0）。所有极坐标 setter
/// （`set_speed/set_angle/turn/aim_player`）与 POLAR_FX 积分共用此原语（D3 双表示回填）。
#[inline]
pub fn polar_to_vec(speed: Fx, angle: Angle) -> (Fx, Fx) {
    let (sn, cs) = sincos(angle);
    (speed * cs, speed * sn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_sq_pythagorean() {
        // 3² + 4² = 5²（以 raw² 为单位）
        let got = len_sq(Fx::from_int(3), Fx::from_int(4));
        let want = (Fx::from_int(5).raw() as i64).pow(2);
        assert_eq!(got, want);
    }

    #[test]
    fn len_sq_zero() {
        assert_eq!(len_sq(Fx::ZERO, Fx::ZERO), 0);
    }

    #[test]
    fn len_sq_survives_large_distance() {
        // 400px：用 Fx::mul 早已溢出（其 i32 结果 >>16 越界），len_sq 走 i64 稳过。
        let d = Fx::from_int(400);
        let one = 400i64 * 65536; // = d.raw()
        assert_eq!(len_sq(d, d), 2 * one * one);
        assert!(len_sq(d, d) > i32::MAX as i64); // 证明确实超出 Fx::mul 的 i32 结果范围
    }

    #[test]
    fn polar_cardinal() {
        // θ=0 → (speed, 0)
        assert_eq!(
            polar_to_vec(Fx::from_int(10), Angle::ZERO),
            (Fx::from_int(10), Fx::ZERO)
        );
        // θ=π/2 → (0, speed)
        assert_eq!(
            polar_to_vec(Fx::from_int(10), Angle::QUARTER),
            (Fx::ZERO, Fx::from_int(10))
        );
    }

    #[test]
    fn polar_preserves_magnitude() {
        // 任意角，|v|² 应 ≈ speed²（定点近似，容差 0.2%）
        let speed = Fx::from_int(100);
        for &a in &[1234u16, 9000, 30000, 55000] {
            let (vx, vy) = polar_to_vec(speed, Angle(a));
            let got = len_sq(vx, vy);
            let want = (speed.raw() as i64).pow(2);
            let diff = (got - want).abs();
            assert!(diff * 500 < want, "a={a} |v|²={got} speed²={want}");
        }
    }
}
