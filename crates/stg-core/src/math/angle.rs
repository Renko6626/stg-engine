//! `Angle` —— BAM（Binary Angular Measurement）u16，一圈 = 65536（I2）。
//! u16 天然模 65536 回绕；**一切加减用 `wrapping_*`**，绝不用会在 debug 触发 overflow-checks 的 `+/-`。

/// BAM 角度。`repr(transparent)` ⇒ 与裸 `u16` 同布局。
///
/// ⚠️ **派生的 `Ord`/`PartialOrd` 是 raw 上的线性序，不是环形序**（C20②）：`Angle(65535)`
/// 与 `Angle(0)` 线性上最远、环上只差 1 BAM。`a < b` 只能用来做"确定性排序/去重"这类
/// **需要一个全序但不关心它的几何意义**的事；表达"更接近某个方向"必须走差值
/// （`a.sub(b)` 后判 `raw() <= Angle::HALF.raw()` 之类），别拿 `<` 硬套。
#[repr(transparent)]
#[derive(
    Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, crate::checksum::Checksum,
)]
pub struct Angle(pub u16);

impl Angle {
    pub const ZERO: Angle = Angle(0);
    pub const QUARTER: Angle = Angle(16384); // π/2
    pub const HALF: Angle = Angle(32768); // π
    pub const THREE_QUARTER: Angle = Angle(49152); // 3π/2

    /// 一整圈的 BAM 计数（**不是** `Angle` 值——整圈回绕到 `ZERO`，`Angle` 表示不了它）。
    /// 断层线以上做 BAM↔弧度/角度换算要除以它（`Fx::ONE` 有对称物，此前 `Angle` 没有，
    /// 于是外接层只能硬编 `65536`，C20①）。
    pub const FULL_TURN: u32 = 65536;

    /// 回绕加。
    #[inline]
    pub const fn add(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_add(rhs.0))
    }

    /// 回绕减。
    #[inline]
    pub const fn sub(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_sub(rhs.0))
    }

    /// 施加有符号增量（`ang_vel: i16`，BAM/帧）。负增量即反向回绕。
    #[inline]
    pub const fn add_delta(self, delta: i16) -> Angle {
        Angle(self.0.wrapping_add(delta as u16))
    }

    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
}

impl core::ops::Add for Angle {
    type Output = Angle;
    #[inline]
    fn add(self, rhs: Angle) -> Angle {
        Angle::add(self, rhs)
    }
}
impl core::ops::Sub for Angle {
    type Output = Angle;
    #[inline]
    fn sub(self, rhs: Angle) -> Angle {
        Angle::sub(self, rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_full_circle() {
        // 3π/2 + π/2 = 2π ≡ 0
        assert_eq!(Angle::THREE_QUARTER.add(Angle::QUARTER), Angle::ZERO);
        // 0 - π/2 = 3π/2（回绕，不 panic）
        assert_eq!(Angle::ZERO.sub(Angle::QUARTER), Angle::THREE_QUARTER);
    }

    #[test]
    fn add_delta_signed() {
        // 从 0 施加 -1 → 65535
        assert_eq!(Angle::ZERO.add_delta(-1), Angle(65535));
        // 从 QUARTER 施加 +100
        assert_eq!(Angle::QUARTER.add_delta(100), Angle(16484));
        // 从 0 施加 i16::MIN(-32768) → 32768（回绕到 HALF）
        assert_eq!(Angle::ZERO.add_delta(i16::MIN), Angle::HALF);
    }

    #[test]
    fn ops_sugar() {
        assert_eq!(Angle::QUARTER + Angle::QUARTER, Angle::HALF);
        assert_eq!(Angle::HALF - Angle::QUARTER, Angle::QUARTER);
    }
}
