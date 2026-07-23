# 存档字节格式(L1)+ 风暴闸(L2)实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `World::save_bytes`/`load_bytes`(字段级规范字节 + 身份头 v1)+ harness `storm`
恢复重演逐位闸;F2 裁决记档。

**Architecture:** 权威 spec = `docs/superpowers/specs/2026-07-23-save-format-replay-gate-design.md`。
三刀:① `SaveBytes` trait/derive + 叶型(stg-derive+core 基础)→ ② 全 World 覆盖 + 头 + API
+ 判别测试 → ③ `storm` 子命令 + CI 短版 + 文档。

**Tech Stack:** Rust 1.92;**零新依赖**(SaveBytes 是 stg-derive 自家 derive)。

## Global Constraints

- `stg-core` 断层线纪律不变(禁浮点/时钟/宿主 RNG/无序容器);**零新增外部依赖**。
- **金向量逐位不变**(纯增量:新 trait/derive/API/子命令,不触 checksum 与演化路径);
  基线 Task 1 抓取,每任务 diff 全等。
- **skip 单一口径**:`#[checksum(skip)]` 字段写读两侧都省略;清零由 `load_bytes` 零构造
  契约承担(`read_bytes` trait 文档写明"目标须零初始化")。
- 头 v1 字节序钉死(小端,49 B):`b"STGW"(4) + file_ver u8=1 + ENGINE_VER u32 +
  tables_hash u64 + image_hash u64 + seed u64 + frame u32 + payload_len u32 + payload_fnv u64`。
- 错误全走 `LoadError`(P4 式 Err,不 panic);coherence"任一侧 0 = 未绑定跳过"同
  `start_main` 守卫口径。
- 每任务收尾:`cargo test --workspace` + `cargo fmt --all -- --check` +
  `cargo clippy --workspace --all-targets -- -D warnings`。
- commit 尾:`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`;
  `.superpowers/` 不入库(`git add -A ':!.superpowers'`)。
- 注释中文随邻居。

---

### Task 1: `SaveBytes` trait + derive + 叶型实现

**Files:**
- Create: `crates/stg-core/src/save.rs`(trait/SaveReader/LoadError/叶型 impl + tests)
- Modify: `crates/stg-core/src/lib.rs`(`pub mod save;`,按字母序插于 rng 与 shots 之间)
- Modify: `crates/stg-derive/src/lib.rs`(新 `#[proc_macro_derive(SaveBytes, attributes(checksum))]`)

**Interfaces:**
- Consumes: 既有 `derive_checksum`(stg-derive lib.rs:16-94)的字段遍历/skip 解析样式。
- Produces(T2 依赖):
  - `stg_core::save::{SaveBytes, SaveReader, LoadError}`,trait 签名:
    `fn write_bytes(&self, out: &mut Vec<u8>);`
    `fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError>;`
  - 叶型 impl:`u8 i8 u16 i16 u32 i32 u64 i64`、`[T: SaveBytes; N]`、`Fx`、`Angle`。
  - stg-derive 导出 `SaveBytes` derive(与 Checksum 同 attr 命名空间——两个 derive 共享
    `attributes(checksum)` 合法,各自独立解析)。

- [ ] **Step 1: 金向量基线**

```bash
mkdir -p .superpowers && cargo run -q -p stg-harness -- golden --out .superpowers/golden-pre-save.txt && wc -l .superpowers/golden-pre-save.txt
```

- [ ] **Step 2: core 侧 `save.rs`(trait + reader + 错误 + 叶型)**

```rust
//! save —— World 状态的字段级规范字节序列化(spec 2026-07-23 L1)。
//!
//! 纪律:小端、字段声明序、无 padding、无长度前缀(定长类型自描述);`#[checksum(skip)]`
//! 字段写读两侧都**省略**——清零语义由 `World::load_bytes` 的**零构造契约**承担(目标堆零
//! 构造后逐字段读回,skip 字段保持零 = "恢复出的 World 无陈旧输出",与 `copy_into` 同口径)。
//! `SaveBytes` derive(stg-derive)与 `Checksum` 共享同一字段清单与 skip 属性——新字段
//! 自动入档,防漏与校验和同源。

/// 读档失败(P4 式:一切坏输入 → Err,不 panic)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    BadMagic,
    BadFileVer { got: u8 },
    EngineVerMismatch { file: u32, engine: u32 },
    /// 载荷完整性 FNV 不符(文件损坏/被改)。
    HashMismatch { file: u64, computed: u64 },
    TablesMismatch { file: u64, given: u64 },
    ImageMismatch { file: u64, given: u64 },
    Truncated,
    TrailingBytes { left: usize },
}

/// 只进不退的读游标(tables.rs C11 `Reader` 同款纪律:越界即 `Truncated`)。
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
    /// 剩余未读字节数(载荷读毕必须为 0——规范字节无冗余)。
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}

/// derive 宏再导出(镜像 checksum.rs 的 `pub use stg_derive::Checksum;` 模式——trait 与
/// derive 宏异命名空间同名共存,attr 写 `crate::save::SaveBytes` 即可)。
pub use stg_derive::SaveBytes;

/// 字段级规范字节序列化。**`read_bytes` 契约:目标必须零初始化**(skip 字段不被触碰)。
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
```

(`Angle` 若内部字段非 `pub .0`,按 angle.rs 实况改用其 raw 构造器——机械对齐。)

- [ ] **Step 3: derive(stg-derive lib.rs)**

镜像 `derive_checksum`(16-94 行)逐行结构写 `derive_save_bytes`:同样的 struct 校验、
同样的 `#[checksum(skip = "理由")]` 解析(**skip 即两侧省略,无新增属性**)、同样的泛型
约束补齐(`::stg_core::save::SaveBytes`),生成:

```rust
#[automatically_derived]
impl #ig ::stg_core::save::SaveBytes for #name #tg #wc {
    fn write_bytes(&self, __out: &mut ::std::vec::Vec<u8>) {
        #( ::stg_core::save::SaveBytes::write_bytes(&self.#accessor, __out); )*
    }
    fn read_bytes(
        &mut self,
        __r: &mut ::stg_core::save::SaveReader<'_>,
    ) -> ::core::result::Result<(), ::stg_core::save::LoadError> {
        #( ::stg_core::save::SaveBytes::read_bytes(&mut self.#accessor, __r)?; )*
        Ok(())
    }
}
```

注册:`#[proc_macro_derive(SaveBytes, attributes(checksum))]`(与 Checksum 共享 helper
attr 合法;skip 解析代码可提私有 fn 复用,也可照抄——两个 derive 各自独立解析同一属性)。

- [ ] **Step 4: 判别测试(save.rs tests mod)**

```rust
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
```

- [ ] **Step 5: 全绿 + 金向量 + Commit**

```bash
cargo test -p stg-core save && cargo test --workspace
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t1.txt
diff .superpowers/golden-pre-save.txt .superpowers/golden-post-t1.txt && echo BYTE-IDENTICAL
git add -A ':!.superpowers'
git commit -m "feat(save): SaveBytes trait/derive + 叶型规范字节——与 Checksum 同源字段清单(刀 1/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: 全 World 覆盖 + 身份头 + `save_bytes`/`load_bytes`

**Files:**
- Modify: `crates/stg-core/src/{rng,player,boss,world}.rs`、`crates/stg-core/src/ecl/task.rs`、
  `crates/stg-core/src/step.rs`(derive 列表各加 `stg_derive::SaveBytes`;WorldBody/World 同)
- Modify: `crates/stg-derive/src/lib.rs:163`(define_pool! 的池 derive 列表加
  `::stg_core::save::SaveBytes`)
- Modify: `crates/stg-core/src/xform.rs`(镜像其手写 Checksum,手写 SaveBytes impl)
- Modify: `crates/stg-core/src/save.rs`(头常量 + 头写读)
- Modify: `crates/stg-core/src/step.rs`(`World::save_bytes`/`load_bytes` + tests)

**Interfaces:**
- Consumes: T1 全部;`crate::checksum::fnv1a64`(载荷完整性——以 checksum.rs 实际函数名
  为准,若只有 hasher 形态则流式喂)、`EclImage::content_hash()`、`WorldTables.content_hash`、
  `crate::ENGINE_VER`。
- Produces(T3 依赖):
  `World::save_bytes(&self, image: &EclImage) -> Vec<u8>`
  `World::load_bytes(bytes: &[u8], tables: &WorldTables, image: &EclImage) -> Result<Box<World>, LoadError>`

- [ ] **Step 1: 撒 derive + 宏接线 + xform 手写**

- 各类型 derive 列表加 SaveBytes(与 Checksum 并排,路径同款模式):`crate::save::SaveBytes`
  ——save.rs 的 `pub use stg_derive::SaveBytes;` 再导出使 trait/宏同名共存(镜像
  checksum.rs)。撒到:`Pcg32`(rng.rs:6)、`PlayerState`(player.rs:23)、`BossUiSlot`
  (boss.rs:17)、`DiagCounters`(world.rs)、`WorldBody`(world.rs)、`Task`(task.rs:26)、
  `TaskPool`(task.rs:77)、`World`(step.rs:17)。
- `define_pool!`(stg-derive lib.rs:163)池 struct derive 列表加一项,与其 Checksum 写法
  同构:`#[derive(Clone, ::stg_core::checksum::Checksum, ::stg_core::save::SaveBytes)]`。
- `XformSegPool`(xform.rs:142 旁):镜像手写 Checksum 的字段序,手写 SaveBytes(全字段,
  无 skip)。

- [ ] **Step 2: 头 + API(save.rs 常量,step.rs 方法)**

save.rs 追加:

```rust
pub(crate) const SAVE_MAGIC: [u8; 4] = *b"STGW";
pub(crate) const SAVE_FILE_VER: u8 = 1;
/// 头总长 = 魔数4 + file_ver1 + ENGINE_VER4 + 表哈希8 + 镜像哈希8 + seed8 + frame4
///          + payload_len4 + payload_fnv8(与 `save_bytes` 写出序逐项对应,=49)。
pub(crate) const SAVE_HEADER_LEN: usize = 4 + 1 + 4 + 8 + 8 + 8 + 4 + 4 + 8;
```

step.rs `impl World` 追加(邻 `copy_into`):

```rust
    /// 存档(spec L1):身份头 v1 + 字段级规范字节载荷。~2-3ms(大头是载荷 FNV),随地存档
    /// 零感知。`image` 只取 content_hash 入头(载荷不含镜像——静态数据不进 World,I7)。
    pub fn save_bytes(&self, image: &crate::ecl::image::EclImage) -> Vec<u8> {
        use crate::save::SaveBytes;
        let mut payload = Vec::with_capacity(1 << 20);
        SaveBytes::write_bytes(self, &mut payload);
        let fnv = crate::checksum::fnv1a64(&payload);
        let mut out = Vec::with_capacity(crate::save::SAVE_HEADER_LEN + payload.len());
        out.extend_from_slice(&crate::save::SAVE_MAGIC);
        out.push(crate::save::SAVE_FILE_VER);
        out.extend_from_slice(&crate::ENGINE_VER.to_le_bytes());
        out.extend_from_slice(&self.tables_hash.to_le_bytes());
        out.extend_from_slice(&image.content_hash().to_le_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.body.frame().to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&fnv.to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// 读档:头校验 + coherence(任一侧 0 = 未绑定跳过,同 `start_main` 守卫口径)→
    /// 堆零构造 + 逐字段读回。P4 式 Err 不 panic;skip 字段因零构造契约保持空。
    pub fn load_bytes(
        bytes: &[u8],
        tables: &crate::tables::WorldTables,
        image: &crate::ecl::image::EclImage,
    ) -> Result<Box<World>, crate::save::LoadError> {
        use crate::save::{LoadError, SaveBytes, SaveReader};
        let mut r = SaveReader::new(bytes);
        if r.take(4)? != crate::save::SAVE_MAGIC {
            return Err(LoadError::BadMagic);
        }
        let ver = r.take(1)?[0];
        if ver != crate::save::SAVE_FILE_VER {
            return Err(LoadError::BadFileVer { got: ver });
        }
        let eng = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
        if eng != crate::ENGINE_VER {
            return Err(LoadError::EngineVerMismatch { file: eng, engine: crate::ENGINE_VER });
        }
        let f_tables = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let f_image = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let _seed = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        let _frame = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
        let plen = u32::from_le_bytes(r.take(4)?.try_into().unwrap()) as usize;
        let f_fnv = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
        if f_tables != 0 && tables.content_hash != 0 && f_tables != tables.content_hash {
            return Err(LoadError::TablesMismatch { file: f_tables, given: tables.content_hash });
        }
        let i_hash = image.content_hash();
        if f_image != 0 && i_hash != 0 && f_image != i_hash {
            return Err(LoadError::ImageMismatch { file: f_image, given: i_hash });
        }
        let payload = r.take(plen)?;
        if r.remaining() != 0 {
            return Err(LoadError::TrailingBytes { left: r.remaining() });
        }
        let fnv = crate::checksum::fnv1a64(payload);
        if fnv != f_fnv {
            return Err(LoadError::HashMismatch { file: f_fnv, computed: fnv });
        }
        let mut w = World::new(0); // 堆零构造 + 播种——随后整个被字段读回覆盖(含 rng/seed)
        let mut pr = SaveReader::new(payload);
        SaveBytes::read_bytes(&mut *w, &mut pr)?;
        if pr.remaining() != 0 {
            return Err(LoadError::TrailingBytes { left: pr.remaining() });
        }
        Ok(w)
    }
```

(`fnv1a64(&[u8]) -> u64` 若 checksum.rs 无此便捷函数,以 hasher 流式喂或补一个
`pub(crate)` helper——机械对齐;`tables.content_hash` 字段/方法名以 tables.rs 实况为准。
注意 `World::new(0)` 会播种 rng,但随后 `read_bytes` 逐字段覆盖含 `body.rng`/`seed`,
终态与档案一致——测试 1 的 checksum 相等即为证。)

- [ ] **Step 3: 判别测试(step.rs tests mod 追加)**

```rust
    /// 深等价:跑一段有弹/敌/任务/道具的世界 → save → load → checksum 相等
    /// (校验和即全字段深比较,P6 白拿);外加二次 save 字节全等(规范自洽)。
    #[test]
    fn save_load_roundtrip_deep_equal_by_checksum() {
        let (mut w, image, _boss) = rainbow_for_test(); // 见下注
        for f in 0..240u32 {
            crate::step(&mut w, &crate::tables::TABLES_V0, &image, &crate::input::InputFrame::empty(f));
        }
        let bytes = w.save_bytes(&image);
        let w2 = World::load_bytes(&bytes, &crate::tables::TABLES_V0, &image).unwrap();
        assert_eq!(w2.checksum(), w.checksum(), "载入 == 从未离开");
        assert_eq!(w2.save_bytes(&image), bytes, "save→load→save 字节全等");
    }

    /// skip 字段不入档:save 前预污染源世界的三条输出缓冲 → load 后全空。
    #[test]
    fn save_omits_pure_output_buffers() {
        let (mut w, image, _boss) = rainbow_for_test();
        w.body.emit_req(7, [1; 6]);
        w.body.push_event(crate::events::Event { kind: 1, ..Default::default() });
        let bytes = w.save_bytes(&image);
        let w2 = World::load_bytes(&bytes, &crate::tables::TABLES_V0, &image).unwrap();
        assert!(w2.take_requests().is_empty(), "reqs 不入档");
        assert!(w2.frame_events().is_empty(), "events 不入档");
    }

    /// 头/载荷错误路径逐一判别(八条各得其 LoadError 变体)。
    #[test]
    fn load_rejects_each_corruption_distinctly() {
        use crate::save::LoadError;
        let (w, image, _boss) = rainbow_for_test();
        let good = w.save_bytes(&image);
        let t = &crate::tables::TABLES_V0;
        let mut b;
        b = good.clone(); b[0] ^= 0xFF;
        assert_eq!(World::load_bytes(&b, t, &image).unwrap_err(), LoadError::BadMagic);
        b = good.clone(); b[4] = 99;
        assert_eq!(World::load_bytes(&b, t, &image).unwrap_err(), LoadError::BadFileVer { got: 99 });
        b = good.clone(); b[5] ^= 0xFF; // ENGINE_VER 首字节
        assert!(matches!(World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::EngineVerMismatch { .. }));
        b = good.clone(); let last = b.len() - 1; b[last] ^= 0x01; // 载荷尾翻一位
        assert!(matches!(World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::HashMismatch { .. }));
        b = good.clone(); b.truncate(good.len() - 8);
        assert_eq!(World::load_bytes(&b, t, &image).unwrap_err(), LoadError::Truncated);
        b = good.clone(); b.push(0);
        assert!(matches!(World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::TrailingBytes { .. }));
        b = good.clone(); b[9] ^= 0xFF; // tables_hash 首字节
        assert!(matches!(World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::TablesMismatch { .. }));
        b = good.clone(); b[17] ^= 0xFF; // image_hash 首字节
        assert!(matches!(World::load_bytes(&b, t, &image).unwrap_err(),
            LoadError::ImageMismatch { .. }));
    }
```

(`rainbow_for_test()`:测试助手,搭"表绑定 + 有任务"的世界——用
`stg_ecl_compiler` 不可达(依赖方向),故在 core 测试里用 `ecl::image::test_image` 造带
`content_hash` 非零的小镜像 + `start_main` + 铺几颗弹/敌;照 binding.rs/step.rs 既有测试
样板拼装,要点:tables 用 `TABLES_V0`(content_hash 非零)、镜像 content_hash 设为
`TABLES_V0.content_hash` 使 coherence 过、跑若干帧让弹/敌/任务/rng 全非默认。实现者按
邻测实况拼,断言值是规格。)

(注意 `b[9]`/`b[17]` 是 tables/image 哈希在头里的偏移(4+1+4=9、9+8=17)——若测试因
TABLES_V0.content_hash 恰含被翻字节相同值而假绿,改翻整个 u64 的多个字节。)

- [ ] **Step 4: 全绿 + 金向量 + Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t2.txt
diff .superpowers/golden-pre-save.txt .superpowers/golden-post-t2.txt && echo BYTE-IDENTICAL
git add -A ':!.superpowers'
git commit -m "feat(save): 全 World SaveBytes 覆盖 + 身份头 v1 + save_bytes/load_bytes(刀 2/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `storm` 风暴闸 + F2 记档 + 文档

**Files:**
- Create: `crates/stg-harness/src/storm.rs`(子命令 + CI 短版测试)
- Modify: `crates/stg-harness/src/main.rs`(`mod storm;` + match 臂 + usage)
- Modify: `docs/follow-ups.md`(F2 改口记裁决)、`CLAUDE.md`(常用命令加 storm)、
  `docs/architecture.md`(M3 接缝行改口)

**Interfaces:**
- Consumes: T2 的 `save_bytes`/`load_bytes`;`crate::build_rainbow_world(seed)`(main.rs,
  viewer 刀产物)、`compile_rainbow_image` 所出镜像(带真 content_hash);
  `stg_core::rng::Pcg32`(宿主侧独立输入流,I3 表现层那颗)。
- Produces: `stg-harness storm [--frames 1200] [--saves 8] [--seed N]`,失败非零退出 +
  首分歧帧定位。

- [ ] **Step 1: storm 实现**

```rust
//! storm —— 恢复重演风暴闸(spec L2):伪随机输入跑全程,沿途多点双源存档(内存快照 +
//! 磁盘字节往返),逐点重演到终点,校验和流逐位对拍。"恢复 == 从未离开"的 CI 性质。

use std::process::ExitCode;
use stg_core::World;
use stg_core::input::{BTN_BOMB, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame};

pub(crate) fn cmd_storm(rest: &[String]) -> ExitCode {
    let mut frames: u32 = 1200;
    let mut saves: usize = 8;
    let mut seed: u64 = 0x5701;
    let mut i = 0;
    while i < rest.len() {
        match (rest[i].as_str(), rest.get(i + 1)) {
            ("--frames", Some(v)) => { frames = v.parse().expect("--frames 要 u32"); i += 2; }
            ("--saves", Some(v)) => { saves = v.parse().expect("--saves 要 usize"); i += 2; }
            ("--seed", Some(v)) => { seed = v.parse().expect("--seed 要 u64"); i += 2; }
            (a, _) => { eprintln!("storm: 未知参数 {a}"); return ExitCode::from(2); }
        }
    }
    match run_storm(frames, saves, seed) {
        Ok(()) => { eprintln!("storm: {frames} 帧 × {saves} 点 × 双源,全部逐位一致"); ExitCode::SUCCESS }
        Err(msg) => { eprintln!("storm: 失败——{msg}"); ExitCode::from(1) }
    }
}

/// 伪随机输入(宿主侧独立 Pcg32,I3):方向 3 帧一换,射击常按,低速/bomb 低频。
fn gen_inputs(frames: u32, seed: u64) -> Vec<u32> {
    let mut rng = stg_core::rng::Pcg32::new(seed ^ 0x1257_0AB1, 0x9E37_79B9_7F4A_7C15);
    let dirs = [0, BTN_LEFT, BTN_RIGHT, BTN_UP, BTN_DOWN, BTN_LEFT | BTN_UP, BTN_RIGHT | BTN_DOWN];
    let mut out = Vec::with_capacity(frames as usize);
    let mut cur = 0u32;
    for f in 0..frames {
        if f % 3 == 0 {
            cur = dirs[rng.rand_range(dirs.len() as u32) as usize] | BTN_SHOT;
            if rng.rand_range(10) == 0 { cur |= BTN_SLOW; }
            if rng.rand_range(120) == 0 { cur |= BTN_BOMB; }
        }
        out.push(cur);
    }
    out
}

pub(crate) fn run_storm(frames: u32, saves: usize, seed: u64) -> Result<(), String> {
    let (mut w, image, _boss) = crate::build_rainbow_world(seed);
    let inputs = gen_inputs(frames, seed);
    let save_at: Vec<u32> = (1..=saves as u32).map(|k| k * frames / (saves as u32 + 1)).collect();

    // 主跑:逐帧校验和流 + 沿途双源存档
    let mut stream = Vec::with_capacity(frames as usize);
    let mut snaps: Vec<(u32, Box<World>, Vec<u8>)> = Vec::new();
    for f in 0..frames {
        let mut input = InputFrame::empty(f);
        input.actions[0].buttons = inputs[f as usize];
        stg_core::step(&mut w, &stg_core::tables::TABLES_V0, &image, &input);
        stream.push(w.checksum());
        if save_at.contains(&f) {
            let mut snap = World::new(0);
            w.copy_into(&mut snap);
            let bytes = w.save_bytes(&image);
            // 规范自洽:load→save 字节全等 + checksum 等
            let loaded = World::load_bytes(&bytes, &stg_core::tables::TABLES_V0, &image)
                .map_err(|e| format!("帧 {f} 载档失败:{e:?}"))?;
            if loaded.checksum() != w.checksum() {
                return Err(format!("帧 {f}:load 后 checksum 与原世界不符"));
            }
            if loaded.save_bytes(&image) != bytes {
                return Err(format!("帧 {f}:save→load→save 字节不自洽"));
            }
            snaps.push((f, snap, bytes));
        }
    }

    // 逐点重演:内存源 + 磁盘源
    for (f0, snap, bytes) in &snaps {
        let disk = World::load_bytes(bytes, &stg_core::tables::TABLES_V0, &image).unwrap();
        for (label, src) in [("内存快照", snap.clone_boxed()), ("磁盘往返", disk)] {
            let mut rw = src;
            for f in (*f0 + 1)..frames {
                let mut input = InputFrame::empty(f);
                input.actions[0].buttons = inputs[f as usize];
                stg_core::step(&mut rw, &stg_core::tables::TABLES_V0, &image, &input);
                let expect = stream[f as usize];
                let got = rw.checksum();
                if got != expect {
                    return Err(format!(
                        "{label} 自帧 {f0} 重演,首分歧于帧 {f}:{got:016x} != {expect:016x}"
                    ));
                }
            }
        }
    }
    Ok(())
}
```

(`clone_boxed`:World 无 Clone——用 `let mut c = World::new(0); snap.copy_into(&mut c);`
替代,写一个本地 helper;上面代码里的 `snap.clone_boxed()` 按此实况替换,断言语义是规格。)

- [ ] **Step 2: CI 短版测试(storm.rs tests)**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn storm_short_gate() {
        super::run_storm(240, 3, 0xAB).expect("短风暴必须全逐位一致");
    }

    /// 变异检验:篡改重演输入一帧必须报分歧——证明对拍真在比,不是恒真。
    #[test]
    fn storm_detects_divergence_by_construction() {
        // 直接内联一个微型版:跑 60 帧记流,快照于 30,重演时第 40 帧改输入,断言分歧被抓。
        // (照 run_storm 结构手写 ~30 行,断言 Err 且信息含"首分歧";实现者照抄主体改一行。)
    }
}
```

- [ ] **Step 3: 接线 + 文档**

- main.rs:`mod storm;` + `Some("storm") => storm::cmd_storm(&args[2..]),` + usage 加
  `storm [--frames N] [--saves K] [--seed S]`。
- `CLAUDE.md` 常用命令块加:
  `cargo run --release -p stg-harness -- storm      # 恢复重演风暴闸(存档正确性)`
- `docs/follow-ups.md` F2 条目:标题改"F2. 校验和轻量化——**已裁决:保 FNV 冻结(2026-07-23)**",
  正文首行加裁决段(无在线逐帧消费者/换算法只省离线耐心/ENGINE_VER=1 含 FNV 身份;
  活口:M4 实测采样成本超预算再启),原分析正文保留作依据。
- `docs/architecture.md` M3 行:焊点补"L1 存档字节格式 ✅ + L2 storm 重演闸 ✅",还缺改
  "环形快照调度(已定为消费侧插件,随 godot 线)+ 延迟/扰动 harness"。

- [ ] **Step 4: 全绿 + 金向量 + Commit**

```bash
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo run -q -p stg-harness -- golden --out .superpowers/golden-post-t3.txt
diff .superpowers/golden-pre-save.txt .superpowers/golden-post-t3.txt && echo BYTE-IDENTICAL
cargo run --release -q -p stg-harness -- storm && echo STORM-OK
git add -A ':!.superpowers'
git commit -m "feat(harness): storm 恢复重演风暴闸 + F2 裁决记档(刀 3/3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## 合入前收尾(控制器步骤)

1. `PROGRESS.md`:史加一行 + 「现在」段重写(两线并行起点就绪:godot 线开 A1/A2+crate;
   RL 线开 stg-py;共享底座完工)。
2. bench-baseline 续表留 A2(本刀实测数已在 spec 记录)。
