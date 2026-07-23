//! Vendored PCG32（XSH-RR，state+inc = 16B）—— I3 随机源，是 `World` 字段、随快照回滚。
//! 算法字节冻结（同 checksum 纪律）：外部 crate 悄改实现 = 全体回放作废。

/// PCG32 随机数发生器。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    const MUL: u64 = 6364136223846793005;

    /// 播种（`seq` 选流；标准 pcg32_srandom_r）。
    pub fn new(seed: u64, seq: u64) -> Self {
        let mut r = Pcg32 {
            state: 0,
            inc: (seq << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    /// 下一个 u32（推进 state，XSH-RR 输出）。
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(Self::MUL).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// `[0, n)` 上的有界随机（`% n`，确定；偏差处处相同，对 danmaku 可接受）。`n==0` 返回 0。
    #[inline]
    pub fn rand_range(&mut self, n: u32) -> u32 {
        if n == 0 { 0 } else { self.next_u32() % n }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PCG 官方 demo 向量：pcg32_srandom_r(seed=42, seq=54) 后 6 次输出。
    #[test]
    fn pcg_reference_vector() {
        let mut r = Pcg32::new(42, 54);
        let want = [
            0xa15c02b7u32,
            0x7b47f409,
            0xba1d3330,
            0x83d2f293,
            0xbfa4784b,
            0xcbed606e,
        ];
        for (k, &w) in want.iter().enumerate() {
            assert_eq!(r.next_u32(), w, "PCG32 第 {k} 个输出");
        }
    }

    #[test]
    fn deterministic_and_seq_independent_streams() {
        let mut a = Pcg32::new(1, 1);
        let mut b = Pcg32::new(1, 1);
        assert_eq!(a.next_u32(), b.next_u32()); // 同种子同流 → 同序列
        let mut c = Pcg32::new(1, 2);
        assert_ne!(Pcg32::new(1, 1).next_u32(), c.next_u32()); // 不同流不同序列（极大概率）
    }

    #[test]
    fn rand_range_bounds() {
        let mut r = Pcg32::new(7, 7);
        for _ in 0..1000 {
            assert!(r.rand_range(10) < 10);
        }
        assert_eq!(r.rand_range(0), 0);
    }
}
