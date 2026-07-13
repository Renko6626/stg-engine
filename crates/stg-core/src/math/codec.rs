//! `const fn` 字节→数组解码器——烘焙表机制共享。表以原始【小端】字节 commit，
//! 编译期解码为定长数组（零运行时开销）。字节序契约见 CLAUDE.md / checksum.rs。

/// 把 `4*N` 字节小端解码为 `[i32; N]`。字节数不匹配则编译期 panic（const eval）。
pub const fn decode_i32<const N: usize>(bytes: &[u8]) -> [i32; N] {
    assert!(bytes.len() == N * 4, "decode_i32: 字节数与 N 不符");
    let mut out = [0i32; N];
    let mut i = 0;
    while i < N {
        let b = i * 4;
        out[i] = i32::from_le_bytes([bytes[b], bytes[b + 1], bytes[b + 2], bytes[b + 3]]);
        i += 1;
    }
    out
}

/// 把 `2*N` 字节小端解码为 `[u16; N]`。
pub const fn decode_u16<const N: usize>(bytes: &[u8]) -> [u16; N] {
    assert!(bytes.len() == N * 2, "decode_u16: 字节数与 N 不符");
    let mut out = [0u16; N];
    let mut i = 0;
    while i < N {
        let b = i * 2;
        out[i] = u16::from_le_bytes([bytes[b], bytes[b + 1]]);
        i += 1;
    }
    out
}
