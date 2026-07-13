//! easing 曲线（D1）——每曲线 257 × Q16.16 归一化查表 + 相邻插值。用于 `move_to` / `STEP_*`（D4/D5）。
//! 表字节由 harness 用 f64 生成并 commit（多项式曲线是精确算术，天然跨平台一致）。

use crate::math::Fx;
use crate::math::codec::decode_i32;

/// 首版 8 条曲线（`repr(u8)` = 表的行索引）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Easing {
    Linear = 0,
    QuadIn = 1,
    QuadOut = 2,
    QuadInOut = 3,
    CubicIn = 4,
    CubicOut = 5,
    CubicInOut = 6,
    Smoothstep = 7,
}

const CURVES: usize = 8;
const SAMPLES: usize = 257;

/// `EASING[row*257 + col]`，col ∈ 0..=256 对应归一化 t，值为 Q16.16 [0,1]。
static EASING: [i32; CURVES * SAMPLES] = decode_i32(include_bytes!("tables/easing.bin"));

/// 缓动：`t` 归一化 Q16.16（钳制到 [0,1]），返回缓动后的归一化值。相邻档线性插值。
pub fn ease(curve: Easing, t: Fx) -> Fx {
    let raw = t.raw().clamp(0, Fx::ONE.raw()); // [0, 65536]
    let scaled = (raw as i64) * 256; // [0, 256*65536]
    let idx = (scaled >> 16) as usize; // 0..=256
    let row = (curve as usize) * SAMPLES;
    if idx >= 256 {
        return Fx::from_raw(EASING[row + 256]);
    }
    let a = EASING[row + idx] as i64;
    let b = EASING[row + idx + 1] as i64;
    let frac = scaled & 0xFFFF; // [0,65536)
    let v = a + (((b - a) * frac) >> 16);
    Fx::from_raw(v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_zero_and_one() {
        for c in [
            Easing::Linear,
            Easing::QuadIn,
            Easing::CubicInOut,
            Easing::Smoothstep,
        ] {
            assert_eq!(ease(c, Fx::ZERO), Fx::ZERO, "{c:?} @0");
            assert_eq!(ease(c, Fx::ONE), Fx::ONE, "{c:?} @1");
        }
    }

    #[test]
    fn linear_is_identity() {
        let half = Fx(1 << 15);
        let d = (ease(Easing::Linear, half).raw() - half.raw()).abs();
        assert!(d <= 2, "linear(0.5) diff={d}");
    }

    #[test]
    fn quad_in_below_linear_midpoint() {
        // QuadIn(0.5)=0.25 < 0.5
        let half = Fx(1 << 15);
        let q = ease(Easing::QuadIn, half).raw();
        assert!((15000..=18000).contains(&q), "quadIn(0.5) raw={q}"); // ≈16384
    }

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(ease(Easing::Linear, Fx(-100)), Fx::ZERO);
        assert_eq!(ease(Easing::Linear, Fx(Fx::ONE.raw() + 100)), Fx::ONE);
    }
}
