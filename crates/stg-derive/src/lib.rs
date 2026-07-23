//! stg 引擎编译期过程宏：`#[derive(Checksum)]`（stg-world-design.md D11 防漏字段级校验和）。
//!
//! 从结构体字段自动生成 `stg_core::checksum::Checksum` 实现（按声明序 `hash_into`）+ debug 构建下
//! 的 inherent `checksum_fields()`（desync 逐字段定位）。`#[checksum(skip = "理由")]` 跳过字段
//! （理由字符串强制）。生成代码用 `::stg_core::checksum::` 绝对路径——stg-core 内经
//! `extern crate self as stg_core;` 解析（serde 同款），下游 crate 走真实 extern crate。

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    Data, DeriveInput, Ident, Index, LitInt, LitStr, Token, Type, braced, parse_macro_input,
};

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
        // 解析 #[checksum(skip = "理由")]：skip 需带理由字符串，否则编译错误。
        let mut skip = false;
        for attr in &f.attrs {
            if attr.path().is_ident("checksum") {
                let r = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                        let _reason: LitStr = meta.value()?.parse()?;
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

    // 泛型参数补 Checksum 约束（非泛型结构体此循环空转）。
    let mut generics = input.generics.clone();
    for tp in generics.type_params_mut() {
        tp.bounds
            .push(syn::parse_quote!(::stg_core::checksum::Checksum));
    }
    let (ig, tg, wc) = generics.split_for_impl();

    quote! {
        #[automatically_derived]
        impl #ig ::stg_core::checksum::Checksum for #name #tg #wc {
            fn hash_into(&self, __h: &mut ::stg_core::checksum::Fnv1a64) {
                #(#hash_stmts)*
            }
        }
        #[cfg(debug_assertions)]
        impl #ig #name #tg #wc {
            /// 逐字段校验和清单（desync 逐字段定位）。`#[derive(Checksum)]` 生成，仅 debug。
            pub fn checksum_fields(&self) -> ::std::vec::Vec<(&'static str, u64)> {
                let mut __v = ::std::vec::Vec::new();
                #(#report_stmts)*
                __v
            }
        }
    }
    .into()
}

/// `#[derive(SaveBytes)]`（存档格式 L1，与 `Checksum` 同源字段清单）——逐行镜像
/// `derive_checksum` 的结构体校验/字段遍历/`#[checksum(skip = "理由")]` 解析（同一 helper
/// attr，各自独立解析），生成 `stg_core::save::SaveBytes` 的 `write_bytes`/`read_bytes`。
#[proc_macro_derive(SaveBytes, attributes(checksum))]
pub fn derive_save_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => {
            return syn::Error::new_spanned(name, "SaveBytes 只支持 struct")
                .to_compile_error()
                .into();
        }
    };

    let mut write_stmts = Vec::new();
    let mut read_stmts = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        // 解析 #[checksum(skip = "理由")]：skip 需带理由字符串，否则编译错误。
        let mut skip = false;
        for attr in &f.attrs {
            if attr.path().is_ident("checksum") {
                let r = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("skip") {
                        skip = true;
                        let _reason: LitStr = meta.value()?.parse()?;
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
        let accessor = match &f.ident {
            Some(id) => quote! { #id },
            None => {
                let idx = Index::from(i);
                quote! { #idx }
            }
        };
        write_stmts.push(quote! {
            ::stg_core::save::SaveBytes::write_bytes(&self.#accessor, __out);
        });
        read_stmts.push(quote! {
            ::stg_core::save::SaveBytes::read_bytes(&mut self.#accessor, __r)?;
        });
    }

    // 泛型参数补 SaveBytes 约束（非泛型结构体此循环空转）。
    let mut generics = input.generics.clone();
    for tp in generics.type_params_mut() {
        tp.bounds
            .push(syn::parse_quote!(::stg_core::save::SaveBytes));
    }
    let (ig, tg, wc) = generics.split_for_impl();

    quote! {
        #[automatically_derived]
        impl #ig ::stg_core::save::SaveBytes for #name #tg #wc {
            fn write_bytes(&self, __out: &mut ::std::vec::Vec<u8>) {
                #(#write_stmts)*
            }
            fn read_bytes(
                &mut self,
                __r: &mut ::stg_core::save::SaveReader<'_>,
            ) -> ::core::result::Result<(), ::stg_core::save::LoadError> {
                #(#read_stmts)*
                Ok(())
            }
        }
    }
    .into()
}

// ───────────────────────── define_pool! （D2 池框架）─────────────────────────

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

/// 生成一个确定性实体池：`define_pool! { Bullet, cap = 8192, fields { x: Fx, ... } }`
/// → `BulletPool` / `BulletHandle` / `BulletInit`。存活掩码即分配器（最低空位优先，无独立
/// free-list）；每次 alloc `gen+1`；exhaustive Init；`#[derive(Checksum)]` 哈希全槽。见 M0-3 计划 / D2。
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

        /// 打包句柄（`index == 0xFFFF` 为 NULL）。
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
            pub(crate) fn alloc(&mut self, init: #init) -> ::core::option::Option<#handle> {
                let idx = self.first_free()?;
                self.alive[idx / 64] |= 1u64 << (idx % 64);
                self.generation[idx] = self.generation[idx].wrapping_add(1);
                #( self.#fnames[idx] = init.#fnames; )*
                ::core::option::Option::Some(#handle {
                    index: idx as u16,
                    generation: self.generation[idx],
                })
            }

            /// 句柄 → 活槽索引（悬垂 / 越界 / 零句柄 → None）。
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
            pub(crate) fn free(&mut self, h: #handle) -> bool {
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

            /// 只读裸切片访问器（通道 A，A9）——批量消费者拿它 + `alive_words()` 自扫存活。
            #(
                pub fn #fnames(&self) -> &[#ftypes] {
                    &self.#fnames
                }
            )*

            /// 存活位字切片（通道 A）——批量消费者按位扫活跃 index（`iter_alive` 的裸形态）。
            pub fn alive_words(&self) -> &[u64] {
                &self.alive
            }

            /// 安全逐字段快照拷贝（每条 SoA 数组 copy_from_slice = memcpy，原地无临时量）。
            pub fn copy_into(&self, dst: &mut Self) {
                #( dst.#fnames.copy_from_slice(&self.#fnames); )*
                dst.generation.copy_from_slice(&self.generation);
                dst.alive.copy_from_slice(&self.alive);
            }

            /// 按索引释放（清 alive 位，无句柄校验）——供相位 cleanup 用。
            pub(crate) fn free_index(&mut self, idx: usize) {
                self.alive[idx / 64] &= !(1u64 << (idx % 64));
            }
        }

        impl ::core::default::Default for #pool {
            fn default() -> Self {
                Self::new()
            }
        }

        const _: () = ::core::assert!(
            #cap <= 0xFFFE,
            "pool cap 必须 ≤ 0xFFFE（index u16 + 0xFFFF 哨兵）"
        );
    }
    .into()
}
