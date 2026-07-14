//! `Angle` —— BAM（Binary Angular Measurement）u16，一圈 = 65536（I2）。
//! u16 天然模 65536 回绕；**一切加减用 `wrapping_*`**，绝不用会在 debug 触发 overflow-checks 的 `+/-`。

/// BAM 角度。`repr(transparent)` ⇒ 与裸 `u16` 同布局。
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
