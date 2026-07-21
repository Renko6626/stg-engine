//! ECL 三型系统的纯规则表。
//!
//! 本模块不持有符号表、不构造诊断，也不依赖编译阶段状态；类型检查与常量折叠共用它，
//! 以避免两条路径各自维护一份运算矩阵。

use crate::lang::ast::{BinOp, Ty};

/// 二元表达式的后端降低意图。
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
    /// 同型比较；具体 opcode 由 [`BinOp`] 决定。
    Cmp,
    /// 短路逻辑由 codegen 降低为跳转模板。
    LogicAnd,
    LogicOr,
}

/// 一元表达式的后端降低意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnIntent {
    Neg,
    /// 无专用 `NOT` op，降低为与零比较。
    Not,
}

/// 显式 cast 的后端降低意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastIntent {
    /// `int as fx`：`×65536`。
    IntToFx,
    /// `fx as int`：`÷65536`，向零截断。
    FxToInt,
    /// `int as angle` / `angle as int`：位穿透。
    Bitcast,
}

/// 类型矩阵的唯一权威。`Ok` 同时给出结果型与后端降低意图。
pub fn binary_result(op: BinOp, lt: Ty, rt: Ty) -> Result<(Ty, BinIntent), ()> {
    use BinIntent::*;
    use BinOp::*;
    use Ty::*;
    match op {
        Add if lt == rt => Ok((lt, AddI)),
        Sub if lt == rt => Ok((lt, SubI)),
        Mod if lt == Int && rt == Int => Ok((Int, ModI)),
        Mul => match (lt, rt) {
            (Int, Int) => Ok((Int, MulI)),
            (Fx, Fx) => Ok((Fx, MulF)),
            (Fx, Int) | (Int, Fx) => Ok((Fx, MulI)),
            _ => Err(()),
        },
        Div => match (lt, rt) {
            (Int, Int) => Ok((Int, DivI)),
            (Fx, Fx) => Ok((Fx, DivF)),
            (Fx, Int) => Ok((Fx, DivI)),
            _ => Err(()),
        },
        Eq | Ne | Lt | Le | Gt | Ge if lt == rt => Ok((Int, Cmp)),
        And if lt == Int && rt == Int => Ok((Int, LogicAnd)),
        Or if lt == Int && rt == Int => Ok((Int, LogicOr)),
        _ => Err(()),
    }
}

/// 用户可见的二元操作符文本，供诊断复用。
pub fn op_symbol(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Eq => "==",
        Ne => "!=",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        And => "&&",
        Or => "||",
    }
}

/// cast 白名单。调用方负责以原有错误文案报告 `None`。
pub fn cast_intent(from: Ty, to: Ty) -> Option<CastIntent> {
    match (from, to) {
        (Ty::Int, Ty::Fx) => Some(CastIntent::IntToFx),
        (Ty::Fx, Ty::Int) => Some(CastIntent::FxToInt),
        (Ty::Int, Ty::Angle) | (Ty::Angle, Ty::Int) => Some(CastIntent::Bitcast),
        _ => None,
    }
}
