//! Vendored FNV-1a 64 校验和 —— 确定性契约的一部分（stg-world-design.md D11）。
//!
//! **为何 vendored（五行实现 commit 进仓库、绝不走外部依赖）**：校验和算法一旦被
//! 某次 crate 升级悄悄改了实现，全体回放与金向量当场作废。故算法字节冻结在此。
//! 换算法（如实测成 CI 瓶颈换 xxHash64）= 一次刻意评审 + bump `engine_ver`。
//!
//! **字节序契约**：支持平台限定小端（x86_64 / aarch64 全小端）；多字节整数一律按
//! 小端字节喂入，跨平台逐字节一致（D11）。大端平台不在支持矩阵。
//!
//! M0 起，`#[derive(Checksum)]`（stg-derive crate）在此算法之上自动生成"字段级、
//! 哈希全槽、防漏"的结构体校验和。本模块只提供最底层的 hasher 原语。

/// FNV-1a 64 位哈希器。
#[derive(Clone, Copy)]
pub struct Fnv1a64 {
    state: u64,
}

impl Fnv1a64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// 新建，初始化为 FNV offset basis。
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    /// 喂入一个字节：`state ^= b; state *= PRIME`（FNV-1a 的 xor-then-multiply）。
    #[inline]
    pub fn write_u8(&mut self, b: u8) {
        self.state ^= b as u64;
        self.state = self.state.wrapping_mul(Self::PRIME);
    }

    /// 喂入一段字节。
    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u8(b);
        }
    }

    /// 喂入一个 `u32`（小端字节序契约）。
    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 喂入一个 `u64`（小端字节序契约）。
    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 喂入一个 `i32`（小端字节序契约）—— Q16.16 定点数即以此喂入。
    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// 当前摘要。
    #[inline]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for Fnv1a64 {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// 参与快照校验和的类型（D11）。按字段声明序把自身喂入 hasher。
///
/// `#[derive(Checksum)]`（stg-derive）从结构体字段自动生成实现，杜绝"加字段忘哈希"；
/// 手写实现仅用于基本类型与数组（见下）。
pub trait Checksum {
    /// 把自身字节按契约（小端、字段声明序）喂入 hasher。
    fn hash_into(&self, h: &mut Fnv1a64);

    /// 便利收口：新建 hasher、喂入、出摘要。
    #[inline]
    fn checksum(&self) -> u64 {
        let mut h = Fnv1a64::new();
        self.hash_into(&mut h);
        h.finish()
    }
}

macro_rules! impl_le {
    ($($t:ty),*) => { $(
        impl Checksum for $t {
            #[inline]
            fn hash_into(&self, h: &mut Fnv1a64) {
                h.write_bytes(&self.to_le_bytes());
            }
        }
    )* };
}
impl_le!(i16, u16, i32, u32, i64, u64);

impl Checksum for u8 {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) {
        h.write_u8(*self);
    }
}
impl Checksum for i8 {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) {
        h.write_u8(*self as u8);
    }
}
impl Checksum for bool {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) {
        h.write_u8(*self as u8);
    }
}

/// 数组逐元素哈希 == SoA 整条哈希（同类型连续、无内部 padding，天然跳过字段间 padding）。
impl<T: Checksum, const N: usize> Checksum for [T; N] {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) {
        for e in self {
            e.hash_into(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 已发布的 FNV-1a 64 标准向量 —— 锁死 vendored 实现，防未来手滑改动。
    #[test]
    fn empty_is_offset_basis() {
        assert_eq!(Fnv1a64::new().finish(), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn known_vector_a() {
        let mut h = Fnv1a64::new();
        h.write_bytes(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn known_vector_foobar() {
        let mut h = Fnv1a64::new();
        h.write_bytes(b"foobar");
        assert_eq!(h.finish(), 0x85944171f73967e8);
    }

    #[test]
    fn little_endian_contract() {
        // write_u32(v) 必须等价于喂入 v 的小端字节，跨平台一致。
        let mut a = Fnv1a64::new();
        a.write_u32(0x0102_0304);
        let mut b = Fnv1a64::new();
        b.write_bytes(&[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn prim_matches_manual() {
        let mut h = Fnv1a64::new();
        0x0102_0304u32.hash_into(&mut h);
        assert_eq!(0x0102_0304u32.checksum(), h.finish());
    }

    #[test]
    fn array_equals_concat_bytes() {
        // [u32;2] 逐元素 == 直接喂 8 字节小端
        let a: [u32; 2] = [0x1122_3344, 0x5566_7788];
        let mut h = Fnv1a64::new();
        h.write_bytes(&0x1122_3344u32.to_le_bytes());
        h.write_bytes(&0x5566_7788u32.to_le_bytes());
        assert_eq!(a.checksum(), h.finish());
    }

    #[test]
    fn bool_and_i8() {
        assert_eq!(true.checksum(), 1u8.checksum());
        assert_eq!((-1i8).checksum(), 0xffu8.checksum());
    }
}
