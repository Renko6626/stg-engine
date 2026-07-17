//! easing 曲线（D1）——每曲线 257 × Q16.16 归一化查表 + 相邻插值。用于 `move_to` / `STEP_*`（D4/D5）。
//! 表字节由 harness 用 f64 生成并 commit（多项式曲线是精确算术，天然跨平台一致）。
//!
//! **linear 亦烘入表**（第 0 行 = identity）：为统一表结构、`ease()` 零特例；与母文档 D1 原设想
//! "linear 直算不烘" 的差异见此——代价 +1KB、精确无损。

use crate::math::Fx;
use crate::math::codec::decode_i32;

/// 首版 8 条曲线（`repr(u8)` = 表的行索引）。所有 `f(t)` 定义域/值域均为 `[0,1]`。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Easing {
    /// `t` —— 匀速，无缓动（identity）。
    Linear = 0,
    /// `t²` —— 慢起步→加速，末端最快（起点速度 0）。ease-in。
    QuadIn = 1,
    /// `1−(1−t)²` —— 快起步→减速到停（终点速度 0）。ease-out。
    QuadOut = 2,
    /// `t<.5: 2t²`，否则 `1−(−2t+2)²/2` —— 慢-快-慢对称 S，两端速度 0。通用平滑。
    QuadInOut = 3,
    /// `t³` —— 同 QuadIn 但更陡（起步更慢、末端更冲）。强 ease-in。
    CubicIn = 4,
    /// `1−(1−t)³` —— 同 QuadOut 但更强的急起缓停。强 ease-out。
    CubicOut = 5,
    /// `t<.5: 4t³`，否则 `1−(−2t+2)³/2` —— 更强的对称 S（中段更陡）。
    CubicInOut = 6,
    /// `3t²−2t³` —— 经典 Hermite S，两端一阶导=0，最顺的起停（形近 QuadInOut 但更顺）。
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

/// id → 曲线（D4 STEP / D5 move_to 共用）。越界值 fallback Linear（create 期已拒 ≥8，
/// 此处兜底确定性——涂改段/字段的运行期防线）。
pub(crate) fn from_id(id: u8) -> Easing {
    match id {
        1 => Easing::QuadIn,
        2 => Easing::QuadOut,
        3 => Easing::QuadInOut,
        4 => Easing::CubicIn,
        5 => Easing::CubicOut,
        6 => Easing::CubicInOut,
        7 => Easing::Smoothstep,
        _ => Easing::Linear,
    }
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
