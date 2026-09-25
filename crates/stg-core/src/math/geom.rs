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

/// `seg_box_dist_sq` 的「远到不可能相交」阈值（raw）。点与原点任一分量差超过它即视为无穷远。
///
/// 12000 px 的取法：判定能用到的最大量是 `end ≤ 8832`（640 + 2×4096）、`half ≤ 1024`、
/// `r ≤ 1024`，都远小于 12000，所以**可能相交的点绝不会被误判成无穷远**；而分量差一超过
/// 12000 px，`dx·c + dy·s` 的两个 Fx 乘积相加（各约 12000 px）就有溢出 i32 的风险。
const FAR_RAW: i64 = 12_000 << 16;

/// 点到「旋转线段盒」的平方距离（Q32.32，i64，不开根）。盒 = 从 (ox,oy) 沿 angle 的射线上
/// `[start, end]` 一段，横向半高 `half`。把点转进盒的局部系后钳位，求到钳位点的距离。
/// 激光判定（碰撞行 9/10）用：`seg_box_dist_sq(..) <= r.raw()²` 即相交。
///
/// **无前置条件（对任意输入都安全）**：任一分量与原点之差超过 `FAR_RAW`（12000 px）时
/// 返回 `i64::MAX`——远于 12000 px 的点视为无穷远，绝不与判定半径相交；这同时保证进入 Fx
/// 路径的坐标差不超过 12000 px，`dx·c + dy·s` / `dy·c − dx·s` 的加法不溢出。
#[allow(clippy::too_many_arguments)] // 判定原语的天然参数面（点 + 线段盒），签名即 spec §4.2 契约
pub fn seg_box_dist_sq(
    px: Fx,
    py: Fx,
    ox: Fx,
    oy: Fx,
    angle: Angle,
    start: Fx,
    end: Fx,
    half: Fx,
) -> i64 {
    // 先用 i64 求坐标差（`px − ox` 本身在 Fx 里就可能溢出），超远直接判无穷远。
    let dx = px.raw() as i64 - ox.raw() as i64;
    let dy = py.raw() as i64 - oy.raw() as i64;
    if dx.abs() > FAR_RAW || dy.abs() > FAR_RAW {
        return i64::MAX;
    }
    let (dx, dy) = (Fx::from_raw(dx as i32), Fx::from_raw(dy as i32));
    let (s, c) = sincos(angle);
    let along = dx * c + dy * s;
    let perp = dy * c - dx * s;
    let qa = if along < start {
        start
    } else if along > end {
        end
    } else {
        along
    };
    let qp = if perp < -half {
        -half
    } else if perp > half {
        half
    } else {
        perp
    };
    len_sq(along - qa, perp - qp)
}

#[cfg(test)]
mod seg_box_tests {
    use super::*;
    const R: i64 = 65536; // 1 px 的 raw
    fn fx(v: i32) -> Fx {
        Fx::from_int(v)
    }
    fn d(px: i32, py: i32, a: u16, st: i32, en: i32, half: i32) -> i64 {
        seg_box_dist_sq(
            fx(px),
            fx(py),
            fx(0),
            fx(0),
            Angle(a),
            fx(st),
            fx(en),
            fx(half),
        )
    }
    #[test]
    fn inside_box_is_zero() {
        assert_eq!(d(50, 3, 0, 0, 100, 4), 0);
    }
    // 半高 4：点在 y=6 → 距 2 px。若误用 width/4 或 half*2，结果不是 4 px²。
    #[test]
    fn perp_distance_uses_half() {
        assert_eq!(d(50, 6, 0, 0, 100, 4), 4 * R * R);
    }
    // 近端留空 start=64：点在 x=10 → 到 x=64 距 54。若钳位下界误写成 0，结果是 0。
    #[test]
    fn clamps_to_start_not_zero() {
        assert_eq!(d(10, 0, 0, 64, 500, 4), 54 * 54 * R * R);
    }
    #[test]
    fn beyond_end() {
        assert_eq!(d(110, 0, 0, 0, 100, 4), 100 * R * R);
    }
    // angle = 90°（BAM 16384，指向 +y，屏幕向下）：点 (0,50) 在盒内，点 (50,0) 在侧面 46 px 外。
    // along/perp 写反会让前两条互换。点 (0,−50) 在原点**后方** 50 px：sin 符号写反时它会被算进盒内（得 0）。
    #[test]
    fn rotated_quarter_turn() {
        assert_eq!(d(0, 50, 16384, 0, 100, 4), 0);
        assert_eq!(d(50, 0, 16384, 0, 100, 4), 46 * 46 * R * R);
        assert_eq!(d(0, -50, 16384, 0, 100, 4), 50 * 50 * R * R);
    }
    // Review Focus 4：原点在屏外很远、长 640，点在远端附近，结果精确且不溢出。
    #[test]
    fn far_origin_no_overflow() {
        let v = seg_box_dist_sq(
            fx(0),
            fx(440),
            fx(0),
            fx(-200),
            Angle(16384),
            fx(0),
            fx(640),
            fx(3),
        );
        assert_eq!(v, 0);
        let v = seg_box_dist_sq(
            fx(700),
            fx(440),
            fx(0),
            fx(-200),
            Angle(16384),
            fx(0),
            fx(640),
            fx(3),
        );
        assert_eq!(v, 697 * 697 * R * R);
    }
    // F1（终审）：任一分量与原点差超过 12000 px 视为无穷远，直接返回 i64::MAX，绝不进 Fx 加法。
    // (32767,32767) vs (−4096,−4096)：dx = dy = 36863 px，远超阈值；旧实现 `px − ox` 的
    // Fx 减法先溢出 i32（dev panic）。
    #[test]
    fn far_point_is_infinite_distance() {
        let v = seg_box_dist_sq(
            fx(32767),
            fx(32767),
            fx(-4096),
            fx(-4096),
            Angle(0),
            fx(0),
            fx(640),
            fx(4),
        );
        assert_eq!(v, i64::MAX, "远点视为无穷远，不 panic");
    }
}
