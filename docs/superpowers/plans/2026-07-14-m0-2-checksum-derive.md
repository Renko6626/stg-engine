# M0-2: 校验和 derive (D11) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans（本轮由主 agent 自主一口气执行至合并）。

**Goal:** 在 `stg-core` 落地 `Checksum` trait + 基本类型/数组实现，在 `stg-derive` 落地 `#[derive(Checksum)]`（字段级、防漏、`#[checksum(skip="理由")]` 理由强制、debug 逐字段 `checksum_fields`），并 derive 到 `Fx`/`Angle`。为 M0-3 池、M0-4 World 的逐帧校验和奠基。

**Architecture:** `Checksum` trait 住 `stg_core::checksum`（与 vendored `Fnv1a64` 同处）：`hash_into(&self, &mut Fnv1a64)` 按字段声明序喂 hasher，`checksum()->u64` 收口。`#[derive(Checksum)]` 是 `stg-derive` proc-macro（syn/quote）：生成 trait impl + debug 的 inherent `checksum_fields()->Vec<(&str,u64)>`（desync 逐字段定位）。生成代码用 `::stg_core::checksum::` 绝对路径，`stg-core` 内经 `extern crate self as stg_core;` 解析（serde 同款）。**stg-core → stg-derive 是编译期 proc-macro 依赖**（syn/quote 不进运行时，也不在确定性防火墙禁用名单内）。

**Tech Stack:** Rust 1.92（edition 2024）、`syn = "2"`、`quote = "1"`、`proc-macro2 = "1"`。

## Global Constraints

- **FNV-1a 64 vendored**（`Fnv1a64`，已落地）；多字节整数**小端**喂入。
- **字段按声明序**哈希（单一真相源）；**哈希全槽、不用 alive 掩码**（数组逐元素 = SoA 整条哈希，天然跳过字段间 padding）。
- **`#[checksum(skip="理由")]` 的理由字符串强制**（无理由 = 编译错误）；skip 字段不哈希、不入 `checksum_fields`。
- **确定性防火墙**：`stg-core` 运行时不得依赖 `rand/getrandom/*time*/libm/godot`（proc-macro 的 syn/quote 是编译期、不在此列，CI `cargo tree` 断言仍过）。
- `checksum_fields` 仅 `#[cfg(debug_assertions)]`。
- 提交结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## File Structure

- 修改 `crates/stg-core/src/checksum.rs`：加 `Checksum` trait + 基本类型/数组 impl + `pub use stg_derive::Checksum;`（re-export derive）。
- 修改 `crates/stg-core/src/lib.rs`：加 `extern crate self as stg_core;`。
- 修改 `crates/stg-core/Cargo.toml`：`stg-derive.workspace = true`。
- 修改 `crates/stg-derive/Cargo.toml`：syn/quote/proc-macro2。
- 重写 `crates/stg-derive/src/lib.rs`：`#[proc_macro_derive(Checksum, attributes(checksum))]`。
- 修改 `crates/stg-core/src/math/fx.rs`、`angle.rs`：`#[derive(Checksum)]`。

---

### Task 1: `Checksum` trait + 基本类型/数组实现（stg-core）

- [ ] 在 `checksum.rs` 加 trait 与 impl：

```rust
/// 参与快照校验和的类型（D11）。按字段声明序把自身喂入 hasher。
pub trait Checksum {
    fn hash_into(&self, h: &mut Fnv1a64);
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
            fn hash_into(&self, h: &mut Fnv1a64) { h.write_bytes(&self.to_le_bytes()); }
        }
    )* };
}
impl_le!(i16, u16, i32, u32, i64, u64);

impl Checksum for u8 {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) { h.write_u8(*self); }
}
impl Checksum for i8 {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) { h.write_u8(*self as u8); }
}
impl Checksum for bool {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) { h.write_u8(*self as u8); }
}

/// 数组逐元素哈希 == SoA 整条哈希（同类型连续、无内部 padding）。
impl<T: Checksum, const N: usize> Checksum for [T; N] {
    #[inline]
    fn hash_into(&self, h: &mut Fnv1a64) {
        for e in self {
            e.hash_into(h);
        }
    }
}
```

- [ ] 测试（`checksum.rs` tests 内）：

```rust
#[test]
fn prim_matches_manual() {
    let mut h = Fnv1a64::new();
    0x0102_0304u32.hash_into(&mut h);
    assert_eq!(0x0102_0304u32.checksum(), h.finish());
}
#[test]
fn array_equals_concat_bytes() {
    // [u32;2] 逐元素 == 直接喂 8 字节小端
    let a: [u32; 2] = [0x11223344, 0x55667788];
    let mut h = Fnv1a64::new();
    h.write_bytes(&0x11223344u32.to_le_bytes());
    h.write_bytes(&0x55667788u32.to_le_bytes());
    assert_eq!(a.checksum(), h.finish());
}
#[test]
fn bool_and_i8() {
    assert_eq!(true.checksum(), 1u8.checksum());
    assert_eq!((-1i8).checksum(), 0xffu8.checksum());
}
```

- [ ] `cargo test -p stg-core checksum` 绿；`cargo commit`。

### Task 2: `#[derive(Checksum)]` proc-macro + 接线 + derive 到 Fx/Angle

- [ ] `crates/stg-derive/Cargo.toml` 加依赖：

```toml
[dependencies]
syn = "2"
quote = "1"
proc-macro2 = "1"
```

- [ ] 重写 `crates/stg-derive/src/lib.rs`：

```rust
//! stg 引擎编译期过程宏：`#[derive(Checksum)]`（D11 防漏字段级校验和）。
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Index, LitStr, parse_macro_input};

/// 从字段自动生成 `Checksum`（按声明序 `hash_into`）+ debug 的 `checksum_fields`。
/// `#[checksum(skip = "理由")]` 跳过字段（理由字符串强制）。
#[proc_macro_derive(Checksum, attributes(checksum))]
pub fn derive_checksum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => {
            return syn::Error::new_spanned(name, "Checksum 只支持 struct")
                .to_compile_error()
                .into();
        }
    };

    let mut hash_stmts = Vec::new();
    let mut report_stmts = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        // 解析 #[checksum(skip = "理由")]
        let mut skip = false;
        for attr in &f.attrs {
            if attr.path().is_ident("checksum") {
                let r = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                        let _reason: LitStr = meta.value()?.parse()?; // 理由强制
                        Ok(())
                    } else {
                        Err(meta.error("未知 checksum 属性（仅支持 skip = \"理由\"）"))
                    }
                });
                if let Err(e) = r {
                    return e.to_compile_error().into();
                }
            }
        }
        if skip {
            continue;
        }
        let (accessor, fname) = match &f.ident {
            Some(id) => (quote! { #id }, id.to_string()),
            None => {
                let idx = Index::from(i);
                (quote! { #idx }, i.to_string())
            }
        };
        hash_stmts.push(quote! {
            ::stg_core::checksum::Checksum::hash_into(&self.#accessor, __h);
        });
        report_stmts.push(quote! {
            __v.push((#fname, ::stg_core::checksum::Checksum::checksum(&self.#accessor)));
        });
    }

    // 泛型参数加 Checksum 约束
    let mut generics = input.generics.clone();
    for tp in generics.type_params_mut() {
        tp.bounds
            .push(syn::parse_quote!(::stg_core::checksum::Checksum));
    }
    let (ig, tg, wc) = generics.split_for_impl();

    quote! {
        impl #ig ::stg_core::checksum::Checksum for #name #tg #wc {
            fn hash_into(&self, __h: &mut ::stg_core::checksum::Fnv1a64) {
                #(#hash_stmts)*
            }
        }
        #[cfg(debug_assertions)]
        impl #ig #name #tg #wc {
            /// 逐字段校验和清单（desync 逐字段定位）。derive 生成。
            pub fn checksum_fields(&self) -> ::std::vec::Vec<(&'static str, u64)> {
                let mut __v = ::std::vec::Vec::new();
                #(#report_stmts)*
                __v
            }
        }
    }
    .into()
}
```

- [ ] `crates/stg-core/Cargo.toml` `[dependencies]` 加 `stg-derive.workspace = true`。
- [ ] `crates/stg-core/src/lib.rs` 顶部加 `extern crate self as stg_core;`（使生成的 `::stg_core::` 路径在本 crate 内解析）。
- [ ] `checksum.rs` 末尾 `pub use stg_derive::Checksum;`（re-export derive；与 trait 同名不冲突——类型 vs 宏命名空间）。
- [ ] `fx.rs`：`Fx` 的 `#[derive(...)]` 加 `Checksum`（`use crate::checksum::Checksum;`）；`angle.rs` 同理给 `Angle`。
- [ ] 集成测试（`checksum.rs` tests）：derive 出的结构体 checksum == 手写按序哈希；`Fx`/`Angle` 可校验：

```rust
#[test]
fn derived_matches_manual_field_order() {
    #[derive(Checksum)]
    struct Foo { a: u32, b: u16, c: [i32; 3] }
    let foo = Foo { a: 1, b: 2, c: [3, 4, 5] };
    let mut h = Fnv1a64::new();
    1u32.hash_into(&mut h);
    2u16.hash_into(&mut h);
    [3i32, 4, 5].hash_into(&mut h);
    assert_eq!(foo.checksum(), h.finish());
}
```

- [ ] `cargo test -p stg-core`、`cargo run -p stg-harness -- verify-tables` 绿；commit。

### Task 3: `#[checksum(skip)]` 与 `checksum_fields` 验证

- [ ] 测试 skip 跳过 + checksum_fields 逐字段：

```rust
#[test]
fn skip_field_excluded() {
    #[derive(Checksum)]
    struct WithSkip {
        a: u32,
        #[checksum(skip = "纯输出缓冲，重演再生")]
        _scratch: u64,
        b: u16,
    }
    let x = WithSkip { a: 1, _scratch: 999, b: 2 };
    let y = WithSkip { a: 1, _scratch: 12345, b: 2 };
    assert_eq!(x.checksum(), y.checksum()); // _scratch 不参与
}
#[test]
fn checksum_fields_lists_nonskipped() {
    #[derive(Checksum)]
    struct Bar { x: u32, y: u16 }
    let b = Bar { x: 7, y: 9 };
    let rep = b.checksum_fields();
    assert_eq!(rep.len(), 2);
    assert_eq!(rep[0].0, "x");
    assert_eq!(rep[1].0, "y");
    assert_eq!(rep[0].1, 7u32.checksum());
}
```

- [ ] `cargo test -p stg-core` 绿；commit。

### Task 4: 收口 + CI + 合并

- [ ] `cargo test --workspace` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `verify-tables` 全绿。
- [ ] 确认 CI 依赖防火墙仍过（`cargo tree -p stg-core` 出现 syn/quote 但不含禁用名单）。
- [ ] 推分支 → PR → 三平台 CI 绿 → ff 合并 main → 清理分支。

## Self-Review 要点

- D11 覆盖：字段级 ✓、全槽（数组逐元素）✓、防漏（derive 自动纳入）✓、skip 理由强制 ✓、checksum_fields 逐字段（debug）✓；小端 ✓。
- **未覆盖（明确留后）**：`checksum()` 顶层入口方法名与"World 尺寸/字段数变更即红"CI 守卫属 **M0-4**（World 存在后）；`checksum_report` 的**嵌套 dotted-path** 递归为后续增强（M0-2 只做顶层逐字段，够定位到"哪个池/字段"，再手动下钻）。
- 依赖方向：stg-derive **不**依赖 stg-core（proc-macro 生成的路径是 token，不产生 crate 依赖），无环。
