//! save —— World 状态的字段级规范字节序列化（spec 2026-07-23 L1）。
//!
//! 纪律：小端、字段声明序、无 padding、无长度前缀（定长类型自描述）；`#[checksum(skip)]`
//! 字段写读两侧都**省略**——清零语义由 `World::load_bytes` 的**零构造契约**承担（目标堆零
//! 构造后逐字段读回，skip 字段保持零 = "恢复出的 World 无陈旧输出"，与 `copy_into` 同口径）。
//! `SaveBytes` derive（stg-derive）与 `Checksum` 共享同一字段清单与 skip 属性——新字段
//! 自动入档，防漏与校验和同源。

/// 读档失败（P4 式：一切坏输入 → Err，不 panic）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    BadMagic,
    BadFileVer {
        got: u8,
    },
    EngineVerMismatch {
        file: u32,
        engine: u32,
    },
    /// 载荷完整性 FNV 不符（文件损坏/被改）。
    HashMismatch {
        file: u64,
        computed: u64,
    },
    TablesMismatch {
        file: u64,
        given: u64,
    },
    ImageMismatch {
        file: u64,
        given: u64,
    },
    Truncated,
    TrailingBytes {
        left: usize,
    },
}

/// 只进不退的读游标（tables.rs C11 `Reader` 同款纪律：越界即 `Truncated`）。
pub struct SaveReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SaveReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        let end = self.pos.checked_add(n).ok_or(LoadError::Truncated)?;
        if end > self.buf.len() {
            return Err(LoadError::Truncated);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    /// 剩余未读字节数（载荷读毕必须为 0——规范字节无冗余）。
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}

/// 身份头魔数（"STGW" ascii）。
pub(crate) const SAVE_MAGIC: [u8; 4] = *b"STGW";
/// 存档文件格式版本（与 `ENGINE_VER` 独立演化——格式不变而引擎内部改版时后者才动）。
pub(crate) const SAVE_FILE_VER: u8 = 1;
/// 头总长(魔数4+版1+ENGINE_VER4+表8+镜像8+seed8+frame4+len4+fnv8)。
pub(crate) const SAVE_HEADER_LEN: usize = 45;

/// derive 宏再导出（镜像 checksum.rs 的 `pub use stg_derive::Checksum;` 模式——trait 与
/// derive 宏异命名空间同名共存，attr 写 `crate::save::SaveBytes` 即可）。
pub use stg_derive::SaveBytes;

/// 字段级规范字节序列化。**`read_bytes` 契约：目标必须零初始化**（skip 字段不被触碰）。
pub trait SaveBytes {
    fn write_bytes(&self, out: &mut Vec<u8>);
    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError>;
}

macro_rules! save_int {
    ($($t:ty),+) => {$(
        impl SaveBytes for $t {
            fn write_bytes(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
                let b = r.take(core::mem::size_of::<$t>())?;
                *self = <$t>::from_le_bytes(b.try_into().unwrap());
                Ok(())
            }
        }
    )+};
}
save_int!(u8, i8, u16, i16, u32, i32, u64, i64);

impl<T: SaveBytes, const N: usize> SaveBytes for [T; N] {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        for e in self {
            e.write_bytes(out);
        }
    }
    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
        for e in self {
            e.read_bytes(r)?;
        }
        Ok(())
    }
}

impl SaveBytes for crate::math::Fx {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        self.raw().write_bytes(out);
    }
    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
        let mut v = 0i32;
        v.read_bytes(r)?;
        *self = crate::math::Fx::from_raw(v);
        Ok(())
    }
}

impl SaveBytes for crate::math::Angle {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        self.0.write_bytes(out);
    }
    fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError> {
        let mut v = 0u16;
        v.read_bytes(r)?;
        *self = crate::math::Angle(v);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, PartialEq, Debug, crate::checksum::Checksum, crate::save::SaveBytes)]
    struct Probe {
        a: u32,
        b: i16,
        #[checksum(skip = "测试:纯输出字段,不入档")]
        junk: u64,
        c: [u8; 3],
        d: crate::math::Fx,
    }

    #[test]
    fn derive_roundtrip_is_field_exact_and_skip_omitted() {
        let src = Probe {
            a: 0xA1B2_C3D4,
            b: -7,
            junk: 0xDEAD_BEEF,
            c: [1, 2, 3],
            d: crate::math::Fx::from_raw(98304),
        };
        let mut bytes = Vec::new();
        src.write_bytes(&mut bytes);
        assert_eq!(
            bytes.len(),
            4 + 2 + 3 + 4,
            "skip 字段不占一个字节(4+2+[u8;3]+Fx)"
        );
        let mut dst = Probe::default(); // 零初始化契约
        let mut r = SaveReader::new(&bytes);
        dst.read_bytes(&mut r).unwrap();
        assert_eq!(r.remaining(), 0, "规范字节恰好耗尽");
        assert_eq!(dst.a, src.a);
        assert_eq!(dst.b, src.b);
        assert_eq!(dst.c, src.c);
        assert_eq!(dst.d, src.d);
        assert_eq!(dst.junk, 0, "skip 字段保持零(不入档不回读)");
    }

    #[test]
    fn reader_truncation_and_le_order() {
        // 端序判别:0x0102 的 u16 写出必须是 [0x02, 0x01]
        let mut out = Vec::new();
        0x0102u16.write_bytes(&mut out);
        assert_eq!(out, [0x02, 0x01], "小端");
        let mut v = 0u32;
        let mut r = SaveReader::new(&out); // 只有 2 字节,读 u32 必须 Truncated
        assert_eq!(v.read_bytes(&mut r), Err(LoadError::Truncated));
    }
}
