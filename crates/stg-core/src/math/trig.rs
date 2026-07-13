//! `sin/cos/sincos` —— 查 sin 四分之一波表（16385 × Q16.16），四象限对称重建（I2）。
//! 表字节由 `stg-harness bake-tables` 用 f64 生成并 commit；此处只按字节消费。
//!
//! 重建约定（契约）：`SIN_QUARTER[i] = round_ties_even(sin(i·(π/2)/16384)·65536)`，i ∈ 0..=16384。
//! ```text
//! sin(a): quad = a>>14 (0..3), idx = a & 0x3FFF (0..16383)
//!   quad 0: +SIN_QUARTER[idx]
//!   quad 1: +SIN_QUARTER[16384 - idx]
//!   quad 2: -SIN_QUARTER[idx]
//!   quad 3: -SIN_QUARTER[16384 - idx]
//! cos(a) = sin(a + π/2)
//! ```

use crate::math::codec::decode_i32;
use crate::math::{Angle, Fx};

/// `SIN_QUARTER[i] = round_ties_even(sin(i·(π/2)/16384)·65536)`，i ∈ 0..=16384。
pub(crate) static SIN_QUARTER: [i32; 16385] = decode_i32(include_bytes!("tables/sin_quarter.bin"));

/// 正弦：入 `Angle`（BAM）出 `Fx`（Q16.16）。
#[inline]
pub fn sin(a: Angle) -> Fx {
    let a = a.raw();
    let idx = (a & 0x3FFF) as usize;
    let v = match a >> 14 {
        0 => SIN_QUARTER[idx],
        1 => SIN_QUARTER[16384 - idx],
        2 => -SIN_QUARTER[idx],
        _ => -SIN_QUARTER[16384 - idx],
    };
    Fx::from_raw(v)
}

/// 余弦：`cos(a) = sin(a + π/2)`。
#[inline]
pub fn cos(a: Angle) -> Fx {
    sin(a.add(Angle::QUARTER))
}

/// 同时求 `(sin, cos)`。
#[inline]
pub fn sincos(a: Angle) -> (Fx, Fx) {
    (sin(a), cos(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinal_points_exact() {
        assert_eq!(sin(Angle::ZERO), Fx::ZERO);
        assert_eq!(sin(Angle::QUARTER), Fx::ONE); // sin(π/2) = 1
        assert_eq!(sin(Angle::HALF), Fx::ZERO); // sin(π) = 0
        assert_eq!(sin(Angle::THREE_QUARTER), -Fx::ONE); // sin(3π/2) = -1
        assert_eq!(cos(Angle::ZERO), Fx::ONE);
        assert_eq!(cos(Angle::QUARTER), Fx::ZERO);
        assert_eq!(cos(Angle::HALF), -Fx::ONE);
    }

    #[test]
    fn quadrant_symmetry() {
        // sin(π/4) 应 ≈ 0.7071 = 46341 raw（round-half-even 后确切值由表定）
        let s = sin(Angle(8192)).raw();
        assert!((46330..=46350).contains(&s), "sin(π/4) raw = {s}");
        // sin(x) 与 sin(π - x) 相等（quad0 vs quad1 对称）
        assert_eq!(sin(Angle(5000)), sin(Angle::HALF.sub(Angle(5000))));
        // sin(-x) = -sin(x)（用回绕表示 -x）
        assert_eq!(sin(Angle::ZERO.sub(Angle(5000))), -sin(Angle(5000)));
    }

    #[test]
    fn pythagorean_identity() {
        // sin²+cos² ≈ 1（定点近似，容差放宽到 ±8 raw）
        for &a in &[123u16, 7777, 20000, 40000, 60000] {
            let (s, c) = sincos(Angle(a));
            let sum = s * s + c * c;
            let diff = (sum.raw() - Fx::ONE.raw()).abs();
            assert!(diff <= 8, "a={a} sin²+cos²={} diff={diff}", sum.raw());
        }
    }
}
