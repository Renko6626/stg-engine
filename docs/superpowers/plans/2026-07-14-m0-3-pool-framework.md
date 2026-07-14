# M0-3: 池框架 `define_pool!` (D2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans（本轮分组检查点执行）。

**Goal:** 在 `stg-derive` 落地函数式 proc-macro `define_pool!`，一次声明生成一个确定性实体池（SoA 数组 + 打包句柄 + 掩码分配器 + 全字段 Init + 升序 alive 迭代 + Checksum + 零初始化），并实例化 `BulletPool`（D3 全 16 字段）。为 M0-4 World/step 奠基。

**Architecture:** `define_pool! { Bullet, cap=8192, fields{...} }` 在模块位置展开成 `BulletPool`/`BulletHandle`/`BulletInit` 一组 item。**存活位掩码即分配器**（最低空位优先，无独立 free-list）；句柄 `{u16 index, u16 gen}`，**每次 alloc `gen+1`**，`valid = alive && gen 匹配`；**exhaustive Init**（漏字段编译不过）；Checksum 走 M0-2 的 `#[derive(Checksum)]`（哈希全字段数组 + `generation[]` + `alive[]`）。生成代码用 `::stg_core::checksum::` 绝对路径（extern-crate-self 解析）。

**Tech Stack:** Rust 1.92（edition 2024）、`syn = { version="2", features=["full"] }`、`quote`、`proc-macro2`。

## Global Constraints（6 条 grill 决策 + 既有不变量）

- **宏机制**：函数式 proc-macro（stg-derive），非 macro_rules。
- **分配器**：`alive: [u64; cap.div_ceil(64)]` 即分配器；alloc = 最低空位（`trailing_zeros`，末字按 `cap%64` 掩码防幽灵位）；free = 清 bit。**无独立 free-list**。
- **句柄**：`{index:u16, generation:u16}`；alloc 时 `generation[idx] += 1`（首次 0→1，活槽 gen≥1）；`valid(h) = idx<cap && alive[idx] && generation[idx]==h.generation`；`NULL = {0xFFFF, 0}`；越界/悬垂/零句柄 → `None`；u16 gen，ABA 接受。
- **Init**：exhaustive（含每个声明字段，不含 gen/alive）；alloc 逐字段拷入。
- **写满槽**：靠 exhaustive Init 编译强制 + 宏全覆写单测；**不写** §2.4 运行时"遗留值"断言。
- **new()**：全零（`generation=[0;cap]`、`alive=[0;NW]`、字段 `core::array::from_fn(Default::default)`）。字段类型须 `Copy + Default + Checksum`（Fx/Angle 补 `Default`）。
- **Checksum**：`#[derive(Checksum)]` 于池结构体，哈希全字段数组 + `generation[]` + `alive[]`（全状态、无 skip、声明序）。
- I1/I4/I7：Fx/Angle/整数字段；升序遍历；固定容量、无堆容器、POD（`#[repr(C)]`）。
- 提交结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- **PoolView 不在本计划**（推迟 M2）；其余实体池不在本计划（M0-4）。

## File Structure

- 改 `crates/stg-derive/Cargo.toml`：`syn` 加 `features=["full"]`。
- 改 `crates/stg-derive/src/lib.rs`：加 `#[proc_macro] define_pool` + `PoolDef`/`FieldDef` 解析器。
- 改 `crates/stg-core/src/lib.rs`：`pub use stg_derive::define_pool;` + `pub mod bullets;`。
- 改 `crates/stg-core/src/math/fx.rs`、`angle.rs`：derive 列表加 `Default`。
- 新建 `crates/stg-core/src/bullets.rs`：`define_pool!{Bullet,...}` + 测试。
- 改 `stg-world-design.md`（D2/§2.4 回写）、`CLAUDE.md`（proc-macro 修订）。

---

### Task 1: `define_pool!` proc-macro（解析 + 全代码生成）

**Files:** 改 `crates/stg-derive/Cargo.toml`、`crates/stg-derive/src/lib.rs`；改 `stg-core/src/lib.rs`（re-export + `pub mod bullets;` 占位空文件先建）；改 `fx.rs`/`angle.rs`（加 Default）。

- [ ] **Step 1: Cargo.toml syn 加 full**

```toml
syn = { version = "2", features = ["full"] }
quote = "1"
proc-macro2 = "1"
```

- [ ] **Step 2: 在 `stg-derive/src/lib.rs` 追加解析器 + 宏**（保留已有 `derive_checksum`）

```rust
use quote::format_ident;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, Type, braced};

struct FieldDef {
    name: Ident,
    ty: Type,
}
impl Parse for FieldDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty = input.parse()?;
        Ok(FieldDef { name, ty })
    }
}

struct PoolDef {
    name: Ident,
    cap: LitInt,
    fields: Vec<FieldDef>,
}
impl Parse for PoolDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let cap_kw: Ident = input.parse()?;
        if cap_kw != "cap" {
            return Err(syn::Error::new(cap_kw.span(), "expected `cap`"));
        }
        input.parse::<Token![=]>()?;
        let cap: LitInt = input.parse()?;
        input.parse::<Token![,]>()?;
        let fields_kw: Ident = input.parse()?;
        if fields_kw != "fields" {
            return Err(syn::Error::new(fields_kw.span(), "expected `fields`"));
        }
        let content;
        braced!(content in input);
        let punct = content.parse_terminated(FieldDef::parse, Token![,])?;
        Ok(PoolDef {
            name,
            cap,
            fields: punct.into_iter().collect(),
        })
    }
}

/// 生成一个确定性实体池：`define_pool! { Bullet, cap = 8192, fields { x: Fx, ... } }`。
/// 展开 `BulletPool` / `BulletHandle` / `BulletInit` + 掩码分配器 + Checksum（见 M0-3 计划 / D2）。
#[proc_macro]
pub fn define_pool(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as PoolDef);
    let cap_val: usize = match def.cap.base10_parse() {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let nw = cap_val.div_ceil(64);
    let cap = &def.cap;

    let pool = format_ident!("{}Pool", def.name);
    let handle = format_ident!("{}Handle", def.name);
    let init = format_ident!("{}Init", def.name);
    let fnames: Vec<&Ident> = def.fields.iter().map(|f| &f.name).collect();
    let ftypes: Vec<&Type> = def.fields.iter().map(|f| &f.ty).collect();

    quote! {
        #[repr(C)]
        #[derive(Clone, ::stg_core::checksum::Checksum)]
        pub struct #pool {
            #( pub(crate) #fnames: [#ftypes; #cap], )*
            pub(crate) generation: [u16; #cap],
            pub(crate) alive: [u64; #nw],
        }

        /// 打包句柄（`index==0xFFFF` 为 NULL）。
        #[repr(C)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug, ::stg_core::checksum::Checksum)]
        pub struct #handle {
            pub index: u16,
            pub generation: u16,
        }
        impl #handle {
            pub const NULL: #handle = #handle { index: 0xFFFF, generation: 0 };
        }

        /// 全字段初始化结构体（exhaustive；漏字段编译不过）。
        #[derive(Clone, Copy)]
        pub struct #init {
            #( pub #fnames: #ftypes, )*
        }

        impl #pool {
            pub const CAP: usize = #cap;
            const NW: usize = #nw;

            /// 全零初始化。
            pub fn new() -> Self {
                Self {
                    #( #fnames: ::core::array::from_fn(|_| ::core::default::Default::default()), )*
                    generation: [0u16; #cap],
                    alive: [0u64; #nw],
                }
            }

            /// 最低空位（末字按 cap%64 掩码，杜绝幽灵位）。
            fn first_free(&self) -> ::core::option::Option<usize> {
                for w in 0..Self::NW {
                    let valid: u64 = if w == Self::NW - 1 && Self::CAP % 64 != 0 {
                        (1u64 << (Self::CAP % 64)) - 1
                    } else {
                        !0u64
                    };
                    let free = !self.alive[w] & valid;
                    if free != 0 {
                        return ::core::option::Option::Some(w * 64 + free.trailing_zeros() as usize);
                    }
                }
                ::core::option::Option::None
            }

            /// 分配：写满全字段 + gen+1；池满返回 None。
            pub fn alloc(&mut self, init: #init) -> ::core::option::Option<#handle> {
                let idx = self.first_free()?;
                self.alive[idx / 64] |= 1u64 << (idx % 64);
                self.generation[idx] = self.generation[idx].wrapping_add(1);
                #( self.#fnames[idx] = init.#fnames; )*
                ::core::option::Option::Some(#handle {
                    index: idx as u16,
                    generation: self.generation[idx],
                })
            }

            /// 句柄 → 活槽索引（悬垂/越界/零句柄 → None）。
            pub fn get(&self, h: #handle) -> ::core::option::Option<usize> {
                let idx = h.index as usize;
                if idx < Self::CAP
                    && (self.alive[idx / 64] >> (idx % 64)) & 1 != 0
                    && self.generation[idx] == h.generation
                {
                    ::core::option::Option::Some(idx)
                } else {
                    ::core::option::Option::None
                }
            }

            /// 释放（清 alive 位）；句柄无效则 no-op 返回 false。
            pub fn free(&mut self, h: #handle) -> bool {
                match self.get(h) {
                    ::core::option::Option::Some(idx) => {
                        self.alive[idx / 64] &= !(1u64 << (idx % 64));
                        true
                    }
                    ::core::option::Option::None => false,
                }
            }

            pub fn is_alive(&self, idx: usize) -> bool {
                idx < Self::CAP && (self.alive[idx / 64] >> (idx % 64)) & 1 != 0
            }

            /// 升序 alive 索引迭代（只读；相位内可变遍历用拷贝 alive 字 + 索引访问，见 M0-4）。
            pub fn iter_alive(&self) -> impl ::core::iter::Iterator<Item = usize> + '_ {
                (0..Self::NW).flat_map(move |w| {
                    let mut bits = self.alive[w];
                    ::core::iter::from_fn(move || {
                        if bits == 0 {
                            return ::core::option::Option::None;
                        }
                        let b = bits.trailing_zeros() as usize;
                        bits &= bits - 1;
                        ::core::option::Option::Some(w * 64 + b)
                    })
                })
            }
        }

        impl ::core::default::Default for #pool {
            fn default() -> Self {
                Self::new()
            }
        }

        const _: () = ::core::assert!(#cap <= 0xFFFE, "pool cap 必须 ≤ 0xFFFE（index u16 + 0xFFFF 哨兵）");
    }
    .into()
}
```

- [ ] **Step 3: Fx/Angle 加 Default**

`fx.rs`：derive 列表加 `Default`（`Fx::default()==Fx(0)`）。`angle.rs`：同加 `Default`（`Angle(0)`）。

- [ ] **Step 4: stg-core 接线**

`lib.rs`：`pub use stg_derive::define_pool;`（re-export 函数式宏）+ `pub mod bullets;`。先建空 `bullets.rs`（`// M0-3 Task 3 填 define_pool!{Bullet,...}`）使编译通过。

- [ ] **Step 5: 编译通过**

Run: `cargo build -p stg-core`
Expected: 通过（bullets.rs 暂空）。

- [ ] **Step 6: Commit**

```bash
git add crates/stg-derive/Cargo.toml crates/stg-derive/src/lib.rs \
        crates/stg-core/src/lib.rs crates/stg-core/src/bullets.rs \
        crates/stg-core/src/math/fx.rs crates/stg-core/src/math/angle.rs Cargo.lock
git commit -m "feat(pool): define_pool! proc-macro（掩码分配器/句柄/Init/迭代/Checksum）"
```

### Task 2: 泛型行为测试（小测试池，压全宏逻辑）

**Files:** 改 `crates/stg-core/src/bullets.rs`（tests）。

- [ ] **Step 1: 写测试**（`bullets.rs` 内 `#[cfg(test)]`；小池 `cap=130` 故意非 64 倍数以验末字掩码）

```rust
#[cfg(test)]
mod tests {
    use crate::define_pool;

    define_pool! { Tp, cap = 130, fields { a: u32, b: u16 } }

    #[test]
    fn alloc_get_free_roundtrip() {
        let mut p = TpPool::new();
        let h = p.alloc(TpInit { a: 7, b: 9 }).unwrap();
        let i = p.get(h).unwrap();
        assert_eq!(p.a[i], 7);
        assert_eq!(p.b[i], 9);
        assert!(p.free(h));
        assert_eq!(p.get(h), None); // 释放后失效
        assert!(!p.free(h)); // 二次 free no-op
    }

    #[test]
    fn zeroed_handle_invalid() {
        let p = TpPool::new();
        assert_eq!(p.get(TpHandle { index: 0, generation: 0 }), None);
        assert_eq!(p.get(TpHandle::NULL), None);
    }

    #[test]
    fn generation_defeats_stale_handle() {
        let mut p = TpPool::new();
        let h1 = p.alloc(TpInit { a: 1, b: 1 }).unwrap();
        assert!(p.free(h1));
        let h2 = p.alloc(TpInit { a: 2, b: 2 }).unwrap();
        assert_eq!(h2.index, h1.index); // 最低空位 → 复用同槽
        assert_eq!(p.get(h1), None); // 旧句柄 gen 失配
        assert_eq!(p.get(h2).map(|i| p.a[i]), Some(2));
    }

    #[test]
    fn full_pool_returns_none_and_tail_mask() {
        let mut p = TpPool::new();
        for k in 0..130u32 {
            assert!(p.alloc(TpInit { a: k, b: 0 }).is_some(), "第 {k} 个应成功");
        }
        assert_eq!(p.alloc(TpInit { a: 0, b: 0 }), None); // 满
        // 幽灵位（130..192）绝不被分配 → 全部 index < 130
        assert!(p.iter_alive().all(|i| i < 130));
        assert_eq!(p.iter_alive().count(), 130);
    }

    #[test]
    fn iter_alive_ascending() {
        let mut p = TpPool::new();
        let _h0 = p.alloc(TpInit { a: 0, b: 0 }).unwrap();
        let h1 = p.alloc(TpInit { a: 1, b: 0 }).unwrap();
        let _h2 = p.alloc(TpInit { a: 2, b: 0 }).unwrap();
        p.free(h1); // 释放中间
        let got: Vec<usize> = p.iter_alive().collect();
        assert_eq!(got, vec![0, 2]); // 升序、跳过死槽
        // 再分配 → 最低空位 = 1
        let h = p.alloc(TpInit { a: 9, b: 0 }).unwrap();
        assert_eq!(h.index, 1);
    }

    #[test]
    fn realloc_overwrites_all_fields() {
        let mut p = TpPool::new();
        let h = p.alloc(TpInit { a: 111, b: 222 }).unwrap();
        p.free(h);
        let h2 = p.alloc(TpInit { a: 333, b: 444 }).unwrap();
        let i = p.get(h2).unwrap();
        assert_eq!((p.a[i], p.b[i]), (333, 444)); // 无陈旧遗留
    }
}
```

- [ ] **Step 2: 跑测试**

Run: `cargo test -p stg-core bullets::tests`
Expected: 6 passed。

- [ ] **Step 3: fmt + clippy**

Run: `cargo fmt --all` / `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 干净（生成代码若触发 lint 就地修：如位运算用 `!= 0`、循环用 for、`idx as u16` 属 pedantic 不拦）。

- [ ] **Step 4: Commit**

```bash
git add crates/stg-core/src/bullets.rs
git commit -m "test(pool): 掩码分配器/句柄/迭代/末字掩码/全覆写 泛型测试"
```

### Task 3: 实例化 `BulletPool`（D3 全 16 字段）+ Checksum 测试

**Files:** 改 `crates/stg-core/src/bullets.rs`。

- [ ] **Step 1: 实例化 BulletPool**（`bullets.rs` 顶部，tests 之外）

```rust
//! 弹池（D3）——`define_pool!` 的首个真实实例。运动/双表示【逻辑】归 M0-4，本模块只落存储/分配/校验。

use crate::define_pool;
use crate::math::{Angle, Fx};

define_pool! {
    Bullet, cap = 8192,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        speed: Fx, angle: Angle,
        ang_vel: i16, accel: Fx,
        ax: Fx, ay: Fx,
        sprite: u16, radius: Fx,
        delay: u8, life: u16, flags: u8, grazed_by: u8,
        transform_head: u16, xform_wait: u16, xform_next: u8
    }
}
```

- [ ] **Step 2: BulletPool Checksum 测试**（tests 内）

```rust
#[test]
fn bullet_pool_checksum_reacts_to_state() {
    use crate::checksum::Checksum;
    use crate::math::{Angle, Fx};
    let mut p = BulletPool::new();
    let empty = p.checksum();
    let bi = BulletInit {
        x: Fx::from_int(10), y: Fx::from_int(20),
        vx: Fx::ZERO, vy: Fx::ZERO, speed: Fx::ZERO, angle: Angle::ZERO,
        ang_vel: 0, accel: Fx::ZERO, ax: Fx::ZERO, ay: Fx::ZERO,
        sprite: 0, radius: Fx::from_int(2),
        delay: 0, life: 0xFFFF, flags: 0, grazed_by: 0,
        transform_head: 0xFFFF, xform_wait: 0, xform_next: 0,
    };
    let h = p.alloc(bi).unwrap();
    assert_ne!(p.checksum(), empty); // 分配改变指纹
    // 改一个字段 → 指纹变
    let before = p.checksum();
    let i = p.get(h).unwrap();
    p.x[i] = Fx::from_int(11);
    assert_ne!(p.checksum(), before);
}

#[test]
fn bullet_pool_new_is_zero_and_deterministic() {
    let a = BulletPool::new();
    let b = BulletPool::new();
    use crate::checksum::Checksum;
    assert_eq!(a.checksum(), b.checksum()); // 两个新池指纹相同（全零确定）
}
```

- [ ] **Step 3: 跑测试 + clippy**

Run: `cargo test -p stg-core` / `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 全 passed；clippy 干净。（BulletPool ~450KB，`new()` 返回值在栈上，M0-3 单池 OK；M0-4 的 World 走 Box。）

- [ ] **Step 4: Commit**

```bash
git add crates/stg-core/src/bullets.rs
git commit -m "feat(pool): 实例化 BulletPool（D3 16 字段）+ Checksum 测试"
```

### Task 4: 设计文档回写 + CLAUDE 修订 + 收口合并

**Files:** 改 `stg-world-design.md`（D2/§2.4）、`design_doc.md`（§2.4）、`CLAUDE.md`。

- [ ] **Step 1: 回写偏离**（grill 已作为评审拍板）
  - `stg-world-design.md` D2：`define_pool!` = proc-macro；"LIFO free-list" → "存活掩码即分配器（最低空位优先，无独立 free-list）"；点 4 的运行时"遗留值"断言 → 删除，改"exhaustive Init 编译强制 + 全覆写单测"。
  - `stg-world-design.md` §2.4 / `design_doc.md` §2.4：同步"掩码即分配器""每次 alloc gen+1""删遗留值断言"。
  - `CLAUDE.md`：把 crate 结构里 `define_pool!` 的"macro_rules"表述改为"stg-derive proc-macro"；池纪律要点补"掩码分配器/gen 语义/exhaustive Init"。

- [ ] **Step 2: 全量绿灯**

Run: `cargo test --workspace` / `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo run -p stg-harness -- verify-tables`
Expected: 全绿。

- [ ] **Step 3: 推分支 → PR → 三平台 CI 绿 → ff 合并 main → 清理分支。**

## Self-Review

- D2 六样产出覆盖：SoA ✓、句柄 ✓、分配/释放（掩码，替代 LIFO）✓、Init（exhaustive）✓、迭代器 ✓、Checksum ✓；**PoolView 明确推迟 M2**。
- §2.4：掩码分配器（偏离 LIFO，已回写）✓、gen 悬垂检测（每次 alloc+1，回写）✓、memcpy-POD（repr(C) + 整数数组）✓、写满槽（exhaustive Init 编译强制，删运行时断言，回写）✓。
- I4 升序遍历 ✓（iter_alive）；I7 无堆容器/固定容量/POD ✓。
- 边界：cap 非 64 倍数经末字掩码（cap=130 测试覆盖）；零句柄/NULL/满池/ABA-复用 均有测试。
- 留后：World 的 Box 堆分配 + 就地初始化（避 1.3MB 栈临时量）、snapshot `copy_into`、PoolView、其余实体池 —— 全 M0-4/M2。
