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
        //  整局流程刀（spec §4）：关卡结算边沿 + 表现锚点三族（5xx syscall 写口专用）
        //  **已退役为兼容位**（壳子刀 2026-09-07）：关卡结束改走 `stage_clear(stage)` 内建 →
        //  `EVT_STAGE_CLEARED` 事实事件（通道 A）。号保留（引擎段 1..=63 冻结），内容包若还
        //  用 `emit_req(REQ_STAGE_CLEAR, …)` 只是一条无人认领的演出请求，不再驱动宿主流程。
        REQ_STAGE_CLEAR:     u16 as int = 4;
        REQ_BGM:             u16 as int = 5;
        REQ_BG:              u16 as int = 6;
        REQ_BG_PHASE:        u16 as int = 7;
        //  表现契约 v2（2026-09-07）：一次性演出两条（`fx_at`/`fx_on` 的钉死布局，即发即忘）
        REQ_FX_AT:           u16 as int = 8;
        REQ_FX_ATTACHED:     u16 as int = 9;
        REQ_SCRIPT_BASE:     u16 as int = 64;
        //  道具类型编号（`items.rs` 冻结编号；`drop_add` 的第 1 参）。**放①不放②**：
        //  ②段 join 校验（`tables.rs`）是用来抓"符号 vs 可加载表行"漂移的，而
        //  `item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT]` 是定长数组、类型数编译期冻结，
        //  没有可抓的漂移。
        ITEM_POWER:          u8 as int = crate::items::ITEM_POWER;
        ITEM_POINT:          u8 as int = crate::items::ITEM_POINT;
        ITEM_LIFE_PIECE:     u8 as int = crate::items::ITEM_LIFE_PIECE;
        ITEM_BOMB_PIECE:     u8 as int = crate::items::ITEM_BOMB_PIECE;
        ITEM_STAR:           u8 as int = crate::items::ITEM_STAR;
        //  难度档编号（难度档具名化刀，2026-07-31）：`GVAR_RANK` 槽的合法**取值**域 = `0..=4`
        //  （`GVAR_RANK` 自己是槽**号**，别混）。`new_game_at` 越界返
        //  `TaskStartError::RankOutOfRange`（拒绝而非钳位——rank 是回放/握手身份的一部分）。
        //  **放①不放②**：同道具类型号，这是**冻结的引擎编号**、不随可加载表漂移，②段的
        //  join 校验没有可抓的漂移。数值顺序即难度序，脚本可写
        //  `if global(GVAR_RANK) >= RANK_HARD { .. }`。
        //
        //  现代作品已基本弃用连续 rank（ZUN 那套「连续 rank + 档位」双轨不做）——就是四档
        //  确定性弹幕。**`RANK_EXTRA`(4) 是预留位、不是"第五档难度"**：Extra 在现代作品里是
        //  独立关卡走自己的脚本，通常不靠 rank 分支；留 4 号是以防将来有共享 sub 需要判它。
        RANK_EASY:           i32 as int = 0;
        RANK_NORMAL:         i32 as int = 1;
        RANK_HARD:           i32 as int = 2;
        RANK_LUNATIC:        i32 as int = 3;
        RANK_EXTRA:          i32 as int = 4;
        //  每任务的发射器槽数（shooter 刀 D-1 拍死 K=4）。它是 `sh_*` 族（syscall 600-660）
        //  槽号 `id` 的合法上界——越界走 P4-b（no-op + `contract_viol`，不 Fault）。
        //  **注入的理由**（C23 已还，2026-09-03）：它和 `GLOBALS_SYS_SEGMENT` 是同一类东西
        //  ——"脚本必须知道的边界值"——而在此之前只有它没进注入表，于是手册与所有 `.ecl`
        //  都只能把 `4` / `0..=3` 写成字面量，正是 C14 那条"跨语言常量引用缺失"要消灭的形态。
        //  **放①不放②**：K 是 VM/ABI 事实（槽存储在 `TaskPool` 里，定长），与可加载表无关。
        SHOOTERS_PER_TASK:   usize as int = crate::ecl::shooter::SHOOTERS_PER_TASK;
        //  时停固定时长（自机能力刀 spec §9.1）：脚本/手册引用此名而非硬编 180，
        //  数值单一来源 = `crate::player::TIMESTOP_FRAMES`。
        TIMESTOP_FRAMES:     u16 as int = crate::player::TIMESTOP_FRAMES;
        //  跳躍跨过的帧数 / 遡行落点深度（时间机制内核刀 2026-09-07）：脚本与手册引用此名，
        //  数值单一来源 = `crate::player::JUMP_FRAMES` / `crate::timeline::REWIND_DEPTH`。
        JUMP_FRAMES:         u16 as int = crate::player::JUMP_FRAMES;
        REWIND_DEPTH:        u32 as int = crate::timeline::REWIND_DEPTH;
    }
    //  ② 段目前**空**（颜色轴刀 2026-07-26）：弹型名/色名归**内容包**——由各内容包
    //  自己的 `.ecl` 用 `const` 声明（内建 demo 的一份见 `godot/ecl/game/bullets.ecl`），
    //  mod 作者与内建内容地位对等，引擎不再替某一份内容包注册词汇。段本身保留：
    //  机制（宏分段 + `validate` 的 join 校验）仍在，将来真有"引擎必须知道名字"的
    //  **可加载表行**时直接加行即可。
    //
    //  **道具类型符号不是那种行**（敌人死亡效果刀 T3 评估）：它们进的是 ① 段——
    //  `item_cfg` 是定长数组、类型数编译期冻结，②段的 join 校验没有可抓的漂移；
    //  而且当前 join 硬编码校验对象是 `appearances`，放 ② 只会得到一个拿
    //  `appearances.len()` 校验道具 id 的假检查。
    table_symbols {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::EclValueType;

    #[test]
    fn engine_consts_registry_exposes_named_ids() {
        // Rust 侧常量维持 u16 原型
        assert_eq!(REQ_BGM, 5u16);
        assert_eq!(GVAR_RANK, 0u16);
        assert_eq!(GLOBALS_SYS_SEGMENT, 16u16);
        // 注入列表把它们作为 i32/Int 携带
        let bgm = ENGINE_CONSTS.iter().find(|c| c.name == "REQ_BGM").unwrap();
        assert_eq!(bgm.ty, EclValueType::Int);
        assert_eq!(bgm.value, 5);
        // 名字唯一
        let n = ENGINE_CONSTS.len();
        let mut names: Vec<&str> = ENGINE_CONSTS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "引擎常量名必须唯一");
    }

    /// 难度档具名常量（难度档具名化刀）：值域 `0..=4` 逐个钉死 + 注入面按 ① 结构常量登记
    /// （**冻结的引擎编号**，不随可加载表漂移，故与道具类型号同段）。
    #[test]
    fn rank_consts_are_frozen_engine_ids_in_structural_segment() {
        // Rust 侧值逐档钉死（顺序即难度序，`>=` 比较是脚本的正规用法）
        assert_eq!(
            (RANK_EASY, RANK_NORMAL, RANK_HARD, RANK_LUNATIC),
            (0, 1, 2, 3)
        );
        assert_eq!(RANK_EXTRA, 4, "4 号是 Extra 预留位，不是第五档难度");
        // 五个名字全部经 ① 段注入，类型 Int，值与 Rust 侧一致
        for (name, want) in [
            ("RANK_EASY", 0),
            ("RANK_NORMAL", 1),
            ("RANK_HARD", 2),
            ("RANK_LUNATIC", 3),
            ("RANK_EXTRA", 4),
        ] {
            let c = ENGINE_STRUCTURAL
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name} 应在 ① 结构常量段"));
            assert_eq!(c.ty, EclValueType::Int, "{name} 是 int");
            assert_eq!(c.value, want, "{name} 值");
            assert!(
                ENGINE_CONSTS
                    .iter()
                    .any(|e| e.name == name && e.value == want),
                "{name} 应出现在注入表"
            );
        }
    }

    #[test]
    fn consts_split_into_structural_and_table_symbols() {
        let has = |list: &[EngineConst], name: &str| list.iter().any(|c| c.name == name);
        assert!(
            has(ENGINE_STRUCTURAL, "GVAR_RANK") && has(ENGINE_STRUCTURAL, "GLOBALS_SYS_SEGMENT")
        );
        assert!(
            TABLE_SYMBOLS.is_empty(),
            "② 段已清空——弹型名归内容包(颜色轴刀)"
        );
        assert!(!has(TABLE_SYMBOLS, "GVAR_RANK"), "结构常量不入 ②");
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
    }
}
