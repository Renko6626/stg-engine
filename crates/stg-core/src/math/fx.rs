//! `Fx` —— Q16.16 定点标量（I1）。16 位整数 + 16 位小数，范围 ±32768，精度 ≈ 1.5e-5。

/// Q16.16 定点数。`repr(transparent)` ⇒ 与裸 `i32` 同布局（SoA / memcpy 快照 / 零拷贝无感）。
///
/// 乘除走 `Mul`/`Div` 重载（i64 中转 + 移位）——newtype 使裸 i32 乘法（"忘了移位差 65536 倍"
/// 的经典静默灾难）在类型上不可能发生（D1）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Fx(pub i32);

impl Fx {
    pub const ZERO: Fx = Fx(0);
    pub const ONE: Fx = Fx(1 << 16);

    /// 整数 → 定点（`n * 65536`）。
    #[inline]
    pub const fn from_int(n: i32) -> Fx {
        Fx(n << 16)
    }

    /// 定点 → 整数，向负无穷取整（算术右移）。
    #[inline]
    pub const fn to_int_floor(self) -> i32 {
        self.0 >> 16
    }

    /// 取原始 i32 位。
    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// 从原始 i32 位构造。
    #[inline]
    pub const fn from_raw(bits: i32) -> Fx {
        Fx(bits)
    }
}

impl core::ops::Add for Fx {
    type Output = Fx;
    #[inline]
    fn add(self, rhs: Fx) -> Fx {
        Fx(self.0 + rhs.0)
    }
}
impl core::ops::Sub for Fx {
    type Output = Fx;
    #[inline]
    fn sub(self, rhs: Fx) -> Fx {
        Fx(self.0 - rhs.0)
    }
}
impl core::ops::Neg for Fx {
    type Output = Fx;
    #[inline]
    fn neg(self) -> Fx {
        Fx(-self.0)
    }
}

impl core::ops::Mul for Fx {
    type Output = Fx;
    /// 定点乘。`Q16.16 × Q16.16` 的 raw 积天然是 **Q32.32**（i64 中间量）；本函数 `>>16`
    /// 把它归一化回 Q16.16（算术右移，向负无穷截断）。
    ///
    /// **代价**：结果须 ≤ ±32768，否则 `as i32` 溢出（debug panic / release wrap）——故仅在
    /// 至少一个操作数 ≤ ~1.0 时安全。平方距离 / 模平方 / 点积等两个大坐标相乘的量【不要】用它，
    /// 改用 [`crate::math::geom::len_sq`] 保留 Q32.32 于 i64（详见 CLAUDE.md 定点乘法规范）。
    #[inline]
    fn mul(self, rhs: Fx) -> Fx {
        let p = (self.0 as i64 * rhs.0 as i64) >> 16;
        debug_assert!(
            p >= i32::MIN as i64 && p <= i32::MAX as i64,
            "Fx::mul overflow"
        );
        Fx(p as i32)
    }
}
impl core::ops::Div for Fx {
    type Output = Fx;
    /// 定点除：被除数左移 16 再除，向零截断。
    #[inline]
    fn div(self, rhs: Fx) -> Fx {
        let q = ((self.0 as i64) << 16) / rhs.0 as i64;
        debug_assert!(
            q >= i32::MIN as i64 && q <= i32::MAX as i64,
            "Fx::div overflow"
        );
        Fx(q as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_conversion() {
        assert_eq!(Fx::ONE.0, 65536);
        assert_eq!(Fx::from_int(3).0, 3 * 65536);
        assert_eq!(Fx::from_int(-2).0, -2 * 65536);
        assert_eq!(Fx::from_int(7).to_int_floor(), 7);
        // 向负无穷取整：-0.5 → -1
        assert_eq!(Fx(-(1 << 15)).to_int_floor(), -1);
    }

    #[test]
    fn add_sub_neg() {
        assert_eq!((Fx::ONE + Fx::ONE).0, 2 * 65536);
        assert_eq!((Fx::from_int(5) - Fx::from_int(8)).0, -3 * 65536);
        assert_eq!((-Fx::ONE).0, -65536);
    }

    #[test]
    fn mul_basic() {
        // 2.0 * 3.0 = 6.0
        assert_eq!(Fx::from_int(2) * Fx::from_int(3), Fx::from_int(6));
        // 0.5 * 0.5 = 0.25
        let half = Fx(1 << 15);
        assert_eq!(half * half, Fx(1 << 14));
        // 乘 ONE 是恒等
        assert_eq!(Fx(12345) * Fx::ONE, Fx(12345));
    }

    #[test]
    fn mul_truncates_toward_neg_inf() {
        // 乘 ONE 恒等，覆盖负号路径
        assert_eq!(Fx(-1) * Fx::ONE, Fx(-1));
        // (1 raw) * (1 raw) = (1*1)>>16 = 0
        assert_eq!(Fx(1) * Fx(1), Fx(0));
        // (-1 raw) * (1 raw) = (-1)>>16 = -1（算术右移向 -inf，不是 0）
        assert_eq!(Fx(-1) * Fx(1), Fx(-1));
    }

    #[test]
    fn div_basic() {
        // 6.0 / 3.0 = 2.0
        assert_eq!(Fx::from_int(6) / Fx::from_int(3), Fx::from_int(2));
        // 1.0 / 2.0 = 0.5
        assert_eq!(Fx::ONE / Fx::from_int(2), Fx(1 << 15));
        // 向零截断：(-7 raw) / (2.0) = -3 raw（-3.5 → -3 向零）
        assert_eq!(Fx(-7) / Fx::from_int(2), Fx(-3));
    }
}
