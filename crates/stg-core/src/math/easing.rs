//! easing 曲线（D1）——每曲线 257 × Q16.16 归一化查表 + 相邻插值。用于 `move_to` / `STEP_*`（D4/D5）。
//! 表字节由harness离线用f64生成并提交；core只读固定字节，不执行浮点。
//!
//! **linear 亦烘入表**（第 0 行 = identity）：为统一表结构、`ease()` 零特例；与母文档 D1 原设想
//! "linear 直算不烘" 的差异见此——代价 +1KB、精确无损。

use crate::math::Fx;
use crate::math::codec::decode_i32;

/// `repr(u8)` = 烘焙表行索引；0–7保留原编码。输入时间钳在[0,1]，输出允许反向／超调。
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
    QuartIn = 8,
    QuartOut = 9,
    QuartInOut = 10,
    QuadOutIn = 11,
    CubicOutIn = 12,
    QuartOutIn = 13,
    /// 保持起点；精确终点由运动／STEP插值器在到时写入。
    Hold = 14,
    /// 首次推进即取终点；不改变插值器的启动时机。
    StepStart = 15,
    SineIn = 16,
    SineOut = 17,
    SineInOut = 18,
    SineOutIn = 19,
    /// Pxx为IN基准曲线的反向极值时刻；OUT是时间镜像。
    BackInP25 = 20,
    BackInP30 = 21,
    BackInP35 = 22,
    BackInP38 = 23,
    BackInP40 = 24,
    BackOutP25 = 25,
    BackOutP30 = 26,
    BackOutP35 = 27,
    BackOutP38 = 28,
    BackOutP40 = 29,
}

impl Easing {
    /// 按冻结编码排列；烘焙器与入口解码共用此集合。
    pub const ALL: [Self; 30] = [
        Self::Linear,
        Self::QuadIn,
        Self::QuadOut,
        Self::QuadInOut,
        Self::CubicIn,
        Self::CubicOut,
        Self::CubicInOut,
        Self::Smoothstep,
        Self::QuartIn,
        Self::QuartOut,
        Self::QuartInOut,
        Self::QuadOutIn,
        Self::CubicOutIn,
        Self::QuartOutIn,
        Self::Hold,
        Self::StepStart,
        Self::SineIn,
        Self::SineOut,
        Self::SineInOut,
        Self::SineOutIn,
        Self::BackInP25,
        Self::BackInP30,
        Self::BackInP35,
        Self::BackInP38,
        Self::BackInP40,
        Self::BackOutP25,
        Self::BackOutP30,
        Self::BackOutP35,
        Self::BackOutP38,
        Self::BackOutP40,
    ];

    pub const fn from_id(id: u8) -> Option<Self> {
        if (id as usize) < Self::ALL.len() {
            Some(Self::ALL[id as usize])
        } else {
            None
        }
    }
}

const CURVES: usize = Easing::ALL.len();
const SAMPLES: usize = 257;

/// `EASING[row*257 + col]`，col ∈ 0..=256 对应归一化t，值为Q16.16，可为负或超1。
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

/// id → 曲线（D4 STEP / D5 move_to 共用）。越界值 fallback Linear（写入口已拒无效值，
/// 此处兜底确定性——涂改段/字段的运行期防线）。
pub(crate) fn from_id(id: u8) -> Easing {
    Easing::from_id(id).unwrap_or(Easing::Linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_curves_match_independent_math_anchors() {
        // 固定序列化编号；期望来自独立分数／三角锚点，不调用烘焙器生成预期。
        for (id, t, expected) in [
            (8, 32768, 4096),
            (9, 32768, 61440),
            (10, 16384, 2048),
            (11, 16384, 24576),
            (12, 16384, 28672),
            (13, 16384, 30720),
            (14, 32768, 0),
            (15, 32768, 65536),
            (16, 32768, 19195),
            (17, 32768, 46341),
            (18, 16384, 9598),
            (19, 16384, 23170),
            (20, 16384, -8192),
            (21, 32768, -8192),
            (22, 32768, -21845),
            (23, 32768, -35499),
            (24, 32768, -49152),
            (25, 49152, 73728),
            (26, 32768, 73728),
            (27, 32768, 87381),
            (28, 32768, 101035),
            (29, 32768, 114688),
        ] {
            let actual = ease(from_id(id), Fx::from_raw(t)).raw();
            assert!(
                (actual - expected).abs() <= 2,
                "curve {id}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn endpoints_and_id_decoding_follow_each_curve_contract() {
        for c in Easing::ALL {
            assert_eq!(Easing::from_id(c as u8), Some(c));
            let start = if c == Easing::StepStart {
                Fx::ONE
            } else {
                Fx::ZERO
            };
            let end = if c == Easing::Hold { Fx::ZERO } else { Fx::ONE };
            assert_eq!(ease(c, Fx::ZERO), start, "{c:?} @0");
            assert_eq!(ease(c, Fx::ONE), end, "{c:?} @1");
        }
        assert_eq!(Easing::from_id(30), None);
        assert_eq!(Easing::from_id(255), None);
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
