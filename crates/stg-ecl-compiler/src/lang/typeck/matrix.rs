//! `typeck` 子模块：类型矩阵唯一权威——判型（[`binary_result`]，被 `exprs::type_expr` 消费）
//! 与常量折叠（`consts::fold_binary_const`）共用同一张表，杜绝"两处写两套规则、迟早分叉"。

use super::intents::BinIntent;
use crate::lang::ast::{BinOp, Expr, Span, Ty};

pub(super) fn binary_result(op: BinOp, lt: Ty, rt: Ty) -> Result<(Ty, BinIntent), ()> {
    use BinIntent::*;
    use BinOp::*;
    use Ty::*;
    match op {
        Add => {
            if lt == rt {
                Ok((lt, AddI))
            } else {
                Err(())
            }
        }
        Sub => {
            if lt == rt {
                Ok((lt, SubI))
            } else {
                Err(())
            }
        }
        Mod => {
            if lt == Int && rt == Int {
                Ok((Int, ModI))
            } else {
                Err(())
            }
        }
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
            // (Int, Fx) 故意不合法：除数是 Q16.16 时纯整数除会错位 65536 倍，见模块文档。
            _ => Err(()),
        },
        Eq | Ne | Lt | Le | Gt | Ge => {
            if lt == rt {
                Ok((Int, Cmp))
            } else {
                Err(())
            }
        }
        And => {
            if lt == Int && rt == Int {
                Ok((Int, LogicAnd))
            } else {
                Err(())
            }
        }
        Or => {
            if lt == Int && rt == Int {
                Ok((Int, LogicOr))
            } else {
                Err(())
            }
        }
    }
}

pub(super) fn op_symbol(op: BinOp) -> &'static str {
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

/// 提取一个 `Expr` 自身的 `Span`（三种字面量变体没有 span 字段，见 T1 ast.rs——退化到
/// `None`，调用方用外层已知的 span 兜底）。
pub(super) fn expr_span(e: &Expr) -> Option<Span> {
    match e {
        Expr::IntLit(_) | Expr::FxLit(_) | Expr::AngleLit(_) => None,
        Expr::Var(_, s)
        | Expr::EngineVar(_, s)
        | Expr::GlobalRead { span: s, .. }
        | Expr::Call { span: s, .. }
        | Expr::Binary { span: s, .. }
        | Expr::Unary { span: s, .. }
        | Expr::Cast { span: s, .. } => Some(*s),
    }
}

pub(super) fn mulf_const(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

pub(super) fn divf_const(a: i32, b: i32) -> i32 {
    (((a as i64) << 16) / b as i64) as i32
}
