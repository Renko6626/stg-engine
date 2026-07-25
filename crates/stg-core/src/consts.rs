//! 脚本可见引擎常量的单一注册表（C14）。
//!
//! `engine_consts!` 宏分两段登记：`structural`（① VM/ABI 事实，与表无关）与
//! `table_symbols`（② 数据表某些行的名字，join 校验对象）。每行同时生成
//! `pub const NAME: <rust_ty>`（Rust 侧照常用，类型保真）并汇入三份注入列表：
//! `ENGINE_STRUCTURAL`（①）、`TABLE_SYMBOLS`（②）、`ENGINE_CONSTS`（①⧺②，编译器
//! 注入用，callers 签名不变）。值只写一次、三头自动出，杜绝漂移。加新配置 = 加一行。
//! 叶子模块（crate 根），无依赖环。

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
    (
        structural { $( $sname:ident : $sty:ty as $sk:ident = $sval:expr ; )* }
        table_symbols { $( $tname:ident : $tty:ty as $tk:ident = $tval:expr ; )* }
    ) => {
        $( pub const $sname: $sty = $sval; )*
        $( pub const $tname: $tty = $tval; )*
        /// ① 引擎结构常量（VM/ABI，与表无关，版本漂移归 `engine_ver`）。
        pub const ENGINE_STRUCTURAL: &[EngineConst] = &[
            $( EngineConst::new(stringify!($sname), engine_consts!(@ty $sk), $sval as i32), )*
        ];
        /// ② 表符号词汇（数据表某些行的名字；join 校验对象；乙案将来搬进表符号段）。
        pub const TABLE_SYMBOLS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($tname), engine_consts!(@ty $tk), $tval as i32), )*
        ];
        /// 全部脚本可见引擎常量（编译器注入 = ①⧺②）。callers 用此名，签名不变。
        pub const ENGINE_CONSTS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($sname), engine_consts!(@ty $sk), $sval as i32), )*
            $( EngineConst::new(stringify!($tname), engine_consts!(@ty $tk), $tval as i32), )*
        ];
    };
    // v0 限制：`$val as i32` 要求 `$rust_ty` 为原生整数（现均 u16）。fx/angle 分支只标
    // `EclValueType`，不改求值——真登记 fx/angle 常量须 `$val` 已是 raw 整数（见 C14）。
    (@ty int)   => { EclValueType::Int };
    (@ty fx)    => { EclValueType::Fx };
    (@ty angle) => { EclValueType::Angle };
}

engine_consts! {
    structural {
        GVAR_RANK:           u16 as int = 0;
        GLOBALS_SYS_SEGMENT: u16 as int = 16;
        //  通道 B 引擎保留请求 id（分区与 args 约定见 `crate::reqs` 模块文档）
        REQ_ENEMY_DEATH:     u16 as int = 1;
        REQ_SPELL_DECLARE:   u16 as int = 2;
        REQ_SPELL_RESULT:    u16 as int = 3;
        //  整局流程刀（spec §4）：关卡结算边沿 + 表现锚点三族（5x syscall 写口专用）
        REQ_STAGE_CLEAR:     u16 as int = 4;
        REQ_BGM:             u16 as int = 5;
        REQ_BG:              u16 as int = 6;
        REQ_BG_PHASE:        u16 as int = 7;
        REQ_SCRIPT_BASE:     u16 as int = 64;
    }
    table_symbols {
        //  ② appearance 行名（值 = appearances 索引；join 校验 + FM2 防错序的锚）
        APPEARANCE_SMALL:  u16 as int = 0;
        APPEARANCE_MEDIUM: u16 as int = 1;
        APPEARANCE_LARGE:  u16 as int = 2;
        APPEARANCE_STAR:   u16 as int = 3;
    }
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

    #[test]
    fn consts_split_into_structural_and_table_symbols() {
        let has = |list: &[EngineConst], name: &str| list.iter().any(|c| c.name == name);
        assert!(
            has(ENGINE_STRUCTURAL, "GVAR_RANK") && has(ENGINE_STRUCTURAL, "GLOBALS_SYS_SEGMENT")
        );
        assert!(has(TABLE_SYMBOLS, "APPEARANCE_SMALL") && has(TABLE_SYMBOLS, "APPEARANCE_STAR"));
        assert!(!has(TABLE_SYMBOLS, "GVAR_RANK"), "结构常量不入 ②");
        assert!(!has(ENGINE_STRUCTURAL, "APPEARANCE_STAR"), "表符号不入 ①");
        // ENGINE_CONSTS = ①⧺② 且注入面不变
        assert_eq!(
            ENGINE_CONSTS.len(),
            ENGINE_STRUCTURAL.len() + TABLE_SYMBOLS.len()
        );
        for c in ENGINE_STRUCTURAL.iter().chain(TABLE_SYMBOLS) {
            assert!(
                ENGINE_CONSTS
                    .iter()
                    .any(|e| e.name == c.name && e.value == c.value)
            );
        }
        assert_eq!(APPEARANCE_STAR, 3u16); // pub const 原型不变
    }
}
