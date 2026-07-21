//! 脚本可见引擎常量的单一注册表（C14）。
//!
//! `engine_consts!` 宏对每行同时生成 ① `pub const NAME: <rust_ty>`（Rust 侧照常用，类型保真）
//! ② 汇入 `ENGINE_CONSTS: &[EngineConst]`（编译器注入进 `.ecl` 命名空间，脚本侧 i32）。
//! 值只写一次、两头自动出，杜绝漂移。加新配置 = 加一行。叶子模块（crate 根），无依赖环。

use crate::ecl::image::EclValueType;

/// 一条注入给 `.ecl` 编译器的命名常量（脚本侧统一 i32：Fx=raw、Angle=raw as i32）。
pub struct EngineConst {
    pub name: &'static str,
    pub ty: EclValueType,
    pub value: i32,
}

impl EngineConst {
    pub const fn new(name: &'static str, ty: EclValueType, value: i32) -> Self {
        Self { name, ty, value }
    }
}

macro_rules! engine_consts {
    ( $( $name:ident : $rust_ty:ty as $script:ident = $val:expr ; )* ) => {
        $( pub const $name: $rust_ty = $val; )*
        /// 全部脚本可见引擎常量（编译器注入用）。定义见本文件 `engine_consts!` 块。
        pub const ENGINE_CONSTS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($name), engine_consts!(@ty $script), $val as i32), )*
        ];
    };
    (@ty int)   => { EclValueType::Int };
    (@ty fx)    => { EclValueType::Fx };
    (@ty angle) => { EclValueType::Angle };
}

engine_consts! {
    //  名字                Rust 类型  脚本类型  值
    APPEARANCE_SMALL:       u16 as int = 0;
    APPEARANCE_MEDIUM:      u16 as int = 1;
    APPEARANCE_LARGE:       u16 as int = 2;
    APPEARANCE_STAR:        u16 as int = 3;
    GVAR_RANK:              u16 as int = 0;
    GLOBALS_SYS_SEGMENT:    u16 as int = 16;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::EclValueType;

    #[test]
    fn engine_consts_registry_exposes_named_ids() {
        // Rust 侧常量维持 u16 原型
        assert_eq!(APPEARANCE_STAR, 3u16);
        assert_eq!(GVAR_RANK, 0u16);
        assert_eq!(GLOBALS_SYS_SEGMENT, 16u16);
        // 注入列表把它们作为 i32/Int 携带
        let star = ENGINE_CONSTS
            .iter()
            .find(|c| c.name == "APPEARANCE_STAR")
            .unwrap();
        assert_eq!(star.ty, EclValueType::Int);
        assert_eq!(star.value, 3);
        // 名字唯一
        let n = ENGINE_CONSTS.len();
        let mut names: Vec<&str> = ENGINE_CONSTS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "引擎常量名必须唯一");
    }
}
