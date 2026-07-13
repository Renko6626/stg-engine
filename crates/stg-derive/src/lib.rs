//! stg 引擎编译期过程宏：`#[derive(Checksum)]`（stg-world-design.md D11 防漏字段级校验和）。
//!
//! 从结构体字段自动生成 `stg_core::checksum::Checksum` 实现（按声明序 `hash_into`）+ debug 构建下
//! 的 inherent `checksum_fields()`（desync 逐字段定位）。`#[checksum(skip = "理由")]` 跳过字段
//! （理由字符串强制）。生成代码用 `::stg_core::checksum::` 绝对路径——stg-core 内经
//! `extern crate self as stg_core;` 解析（serde 同款），下游 crate 走真实 extern crate。

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Index, LitStr, parse_macro_input};

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
