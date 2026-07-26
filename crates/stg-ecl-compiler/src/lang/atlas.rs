//! 图集坐标（弹型 × 颜色）的编译期判据——**typeck 与 codegen 共用的单一权威**。
//!
//! 三个入口写的是同一个数域（`id = 弹型 × color_stride + 色号`，表索引 ≡ 图集格号 ≡ 池
//! `sprite` 值，identity）：
//! - `fire(shape, color, …)` / `batch(shape, color, …)`——判据在 `lang::typeck::exprs`
//!   （实参可能是运行期表达式，只对编译期常量对施加）；
//! - xformdef 的 `set_sprite(shape, color)`——判据在 `lang::codegen` 的 `OpFold2` staging
//!   （xformdef 槽参数**恒是**编译期常量，故这一路总能判）。
//!
//! 三条判据当初只挂在 `fire`/`batch` 上，`set_sprite` 漏网——它照样能造出 spec 点名要挡的
//! "有判定但看不见的弹"（`set_sprite(BULLET_HEART, COLOR_WHITE)` = 144+12 是图集空格），
//! 也照样会被写反（两参同型，`set_sprite(COLOR_BLUE, BULLET_AMULET)` 静默换成错误的格）。
//! 收进本模块之后，"哪些位置写图集坐标"与"什么算合法图集坐标"分开演化，加入口不必抄判据。
//!
//! ## 顺序是契约：色号 → 弹型 → 空格
//!
//! **色号必须先查**。写反的 `fire(COLOR_BLUE, BULLET_AMULET, …)` = `fire(8, 112, …)` 折叠成
//! `8 + 112 = 120`，恰是第 7 形第 8 色——一个**完全合法**的格：折叠之后再查 id 永远抓不住
//! 写反。只有"先分别校验两参"才有救；而写反时**色号位上装的是弹型值**（远大于 stride），
//! 所以先查色号才报得出真凶。顺序一反，写反会被报成"弹型 8 不是合法弹型"，指错地方。
//!
//! ## 阈值全从表读
//!
//! `color_stride` 是 `WorldTables` 的**数据**，不是引擎常量——引擎里不得出现"每形 16 色"
//! 这个数（spec §2 硬约束一）。mod 表换成 8 色时，同一份判据自动按 8 走。

use stg_core::tables::WorldTables;

/// 出错该赖在哪个参上——决定调用方挑哪个 span 报错。xformdef 那一路只有整槽一个 span，
/// 忽略本字段即可。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blame {
    Shape,
    Color,
}

/// 一条图集坐标判据的否决。`msg` 在本模块生成——措辞是契约（测试按"色号"/"弹型"/"空格"
/// 字样判别是哪条判据开的火），调用方只负责挂 span，不得各自改写文案。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShapeColorError {
    pub(crate) blame: Blame,
    pub(crate) msg: String,
}

/// 形/色两参判据（spec §6.3）。`who` = 报错里出现的调用名（`fire`/`batch`/`set_sprite`）。
///
/// **必须在折叠之前对两个参分别施加**（见模块文档"顺序是契约"）。`stride <= 0` 视为坏表，
/// 直接放行——那是 `WorldTables::validate` 的职责，此处不重复报错。
pub(crate) fn check_shape_color(
    table: &WorldTables,
    who: &str,
    shape: i32,
    color: i32,
) -> Result<(), ShapeColorError> {
    let stride = i32::from(table.color_stride);
    if stride <= 0 {
        return Ok(()); // 坏表：validate 的职责，此处不重复报错
    }
    if !(0..stride).contains(&color) {
        return Err(ShapeColorError {
            blame: Blame::Color,
            msg: format!(
                "'{who}' 的色号 {color} 越界：当前表每种弹型 {stride} 色，合法范围 0..{}",
                stride - 1
            ),
        });
    }
    let shapes = (table.appearances.len() as i32) / stride;
    if shape < 0 || shape % stride != 0 || shape / stride >= shapes {
        return Err(ShapeColorError {
            blame: Blame::Shape,
            msg: format!(
                "'{who}' 的弹型 {shape} 不是合法弹型：必须是 {stride} 的倍数且小于 {}",
                shapes * stride
            ),
        });
    }
    let id = (shape + color) as usize;
    if !table.appearances[id].valid {
        return Err(ShapeColorError {
            blame: Blame::Color,
            msg: format!(
                "弹型 {shape} 没有 {color} 号颜色（图集空格）——放行会造出有判定但看不见的弹"
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stg_core::tables::TABLES_V0;

    fn err_of(shape: i32, color: i32) -> ShapeColorError {
        check_shape_color(&TABLES_V0, "fire", shape, color).expect_err("应被判据否决")
    }

    #[test]
    fn accepts_a_valid_cell() {
        assert_eq!(check_shape_color(&TABLES_V0, "fire", 16, 3), Ok(()));
    }

    /// 顺序钉死（谓词层）：写反的 `(8, 112)` 折叠后是合法格 120，只有"色号先查"抓得住。
    #[test]
    fn color_is_checked_before_shape_so_swapped_args_blame_the_color() {
        assert!(
            TABLES_V0.appearances[120].valid,
            "前提：8+112=120 必须是合法格，否则本测试没有判别力"
        );
        let e = err_of(8, 112);
        assert_eq!(e.blame, Blame::Color);
        assert!(e.msg.contains("色号"), "实际: {}", e.msg);
    }

    #[test]
    fn shape_must_sit_on_a_stride_boundary() {
        let e = err_of(5, 0);
        assert_eq!(e.blame, Blame::Shape);
        assert!(e.msg.contains("弹型"), "实际: {}", e.msg);
    }

    #[test]
    fn blank_atlas_cell_is_rejected() {
        let e = err_of(144, 12); // 第 9 形（掩码 0x0FFF）第 12 色
        assert_eq!(e.blame, Blame::Color);
        assert!(e.msg.contains("空格"), "实际: {}", e.msg);
    }

    /// 坏表（stride=0）不在本谓词报错——`validate` 的职责，此处放行避免双轨。
    #[test]
    fn zero_stride_table_is_left_to_validate() {
        let mut t = stg_core::tables::build_tables_v0();
        t.color_stride = 0;
        assert_eq!(check_shape_color(&t, "fire", 999, 999), Ok(()));
    }
}
