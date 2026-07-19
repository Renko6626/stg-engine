//! `typeck` 子模块：指令意图标签（T3 codegen 消费）。
//!
//! Add/Sub/Mod/比较/逻辑在类型矩阵下零歧义，直接照 `BinOp` 本身选 op，`intent` 只对 `*`/`/`
//! 真正做选择——但仍给全量变体，免去 T3 反查矩阵（矩阵本体见 `super::matrix::binary_result`）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinIntent {
    AddI,
    SubI,
    MulI,
    DivI,
    ModI,
    /// `fx*fx`：Q32.32 中间量 `>>16` 归一化（`OP_MULF`）。
    MulF,
    /// `fx/fx`：被除数先 `<<16`（`OP_DIVF`）。
    DivF,
    /// `==`/`!=`/`<`/`<=`/`>`/`>=` 中的一个——具体 op 由 `BinOp` 本身决定（六个各自对应同名
    /// op，`intent` 本身不做二次选择，只是"这是一次同型比较"的标签）。
    Cmp,
    /// 短路，不是单一 op——T3 降低为跳转模板（JZ 短路 + 常量臂，见 spec）。
    LogicAnd,
    LogicOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnIntent {
    /// `OP_NEG`（int/fx/angle 皆走这条，angle 回绕靠消费点低 16 位截断天然发生）。
    Neg,
    /// 无专用 `NOT` op——T3 降低为"与 0 比较"（`push 0; EQ` 一类模板），本趟只给标签。
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastIntent {
    /// `int as fx`：`×65536`（`PUSHI 65536; MUL`）。
    IntToFx,
    /// `fx as int`：`÷65536`，向零截断（`PUSHI 65536; DIV`）。
    FxToInt,
    /// `int as angle` / `angle as int`：位穿透，不发任何指令（BAM 回绕靠消费点低 16 位天然
    /// 发生）。
    Bitcast,
}
