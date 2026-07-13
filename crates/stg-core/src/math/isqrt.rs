//! `isqrt` —— 整数平方根 `floor(sqrt(n))`。结果数学唯一 ⇒ 跨平台天然确定，无需烘焙表。
//! 碰撞判定按 I1 用平方距离比较，正常路径不需要开根；此函数供 CART_FX 极坐标回填（D3）等用。

/// `floor(sqrt(n))`。`isqrt(u64::MAX) == u32::MAX`，故 `as u32` 收窄安全。
#[inline]
pub fn isqrt(n: u64) -> u32 {
    n.isqrt() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_squares() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(65536), 256);
        assert_eq!(isqrt(1_000_000), 1000);
    }

    #[test]
    fn floors_non_squares() {
        assert_eq!(isqrt(2), 1);
        assert_eq!(isqrt(3), 1);
        assert_eq!(isqrt(8), 2); // 2.82 → 2
        assert_eq!(isqrt(99), 9); // 9.94 → 9
    }

    #[test]
    fn boundary_u64_max() {
        assert_eq!(isqrt(u64::MAX), u32::MAX); // 4294967295
    }
}
