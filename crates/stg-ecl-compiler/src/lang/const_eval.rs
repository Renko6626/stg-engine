//! 编译期常量表达式求值。
//!
//! 此模块只处理纯表达式与已声明常量表；诊断由调用方按 [`ConstEvalError`] 原样写入，
//! 从而让类型趟保留既有的错误顺序与文本契约。

use crate::lang::ast::{BinOp, Expr, Span, Ty, UnOp};
use crate::lang::type_rules::{BinIntent, binary_result, cast_intent, op_symbol};
use std::collections::BTreeMap;

pub(super) struct ConstEvalError {
    pub span: Span,
    pub msg: String,
    pub needs_cast_hint: bool,
}

pub(super) fn evaluate(
    e: &Expr,
    consts: &BTreeMap<String, (Ty, i32)>,
) -> Result<(Ty, i32), ConstEvalError> {
    match e {
        Expr::IntLit(v) => Ok((Ty::Int, *v)),
        Expr::FxLit(v) => Ok((Ty::Fx, *v)),
        Expr::AngleLit(v) => Ok((Ty::Angle, *v as i32)),
        Expr::Var(name, span) => consts.get(name).copied().ok_or_else(|| {
            error(
                *span,
                format!(
                    "常量表达式引用了 '{name}'，但它不是已声明的常量\
                 （const 只能引用更早声明的 const，不能引用局部变量/参数）"
                ),
            )
        }),
        Expr::EngineVar(_, span) => Err(not_constant(*span, "`$` 引擎变量（非编译期可求值）")),
        Expr::Call { span, name, .. } => Err(error(
            *span,
            format!("常量表达式不能调用 '{name}'（非编译期可求值）"),
        )),
        Expr::Unary {
            op: UnOp::Neg, e, ..
        } => {
            let (ty, v) = evaluate(e, consts)?;
            Ok((ty, v.wrapping_neg()))
        }
        Expr::Unary {
            op: UnOp::Not,
            span,
            ..
        } => Err(error(*span, "常量表达式不支持 '!'")),
        Expr::Binary { op, l, r, span } => {
            let (lt, lv) = evaluate(l, consts)?;
            let (rt, rv) = evaluate(r, consts)?;
            evaluate_binary(*op, lt, lv, rt, rv, *span)
        }
        Expr::Cast { e, to, span } => {
            let (from, value) = evaluate(e, consts)?;
            let Some(intent) = cast_intent(from, *to) else {
                return Err(error(
                    *span,
                    format!(
                        "常量表达式：不支持的类型转换 {from:?} as {to:?}\
                         （合法：int as fx / fx as int / int as angle / angle as int）"
                    ),
                ));
            };
            let value = match intent {
                crate::lang::type_rules::CastIntent::IntToFx => value.wrapping_mul(65536),
                crate::lang::type_rules::CastIntent::FxToInt => value / 65536,
                crate::lang::type_rules::CastIntent::Bitcast => value,
            };
            Ok((*to, value))
        }
    }
}

fn not_constant(span: Span, subject: &str) -> ConstEvalError {
    error(span, format!("常量表达式不能引用 {subject}"))
}

fn evaluate_binary(
    op: BinOp,
    lt: Ty,
    lv: i32,
    rt: Ty,
    rv: i32,
    span: Span,
) -> Result<(Ty, i32), ConstEvalError> {
    let Ok((ty, intent)) = binary_result(op, lt, rt) else {
        let mut error = error(
            span,
            format!("常量表达式类型不匹配：{lt:?} {} {rt:?}", op_symbol(op)),
        );
        error.needs_cast_hint = true;
        return Err(error);
    };
    use BinIntent::*;
    let value = match intent {
        AddI => lv.wrapping_add(rv),
        SubI => lv.wrapping_sub(rv),
        MulI => lv.wrapping_mul(rv),
        DivI => checked_div(lv, rv, span)?,
        ModI => checked_rem(lv, rv, span)?,
        MulF => ((lv as i64 * rv as i64) >> 16) as i32,
        DivF => {
            if rv == 0 {
                return Err(div_zero(span));
            }
            (((lv as i64) << 16) / rv as i64) as i32
        }
        Cmp => match op {
            BinOp::Eq => (lv == rv) as i32,
            BinOp::Ne => (lv != rv) as i32,
            BinOp::Lt => (lv < rv) as i32,
            BinOp::Le => (lv <= rv) as i32,
            BinOp::Gt => (lv > rv) as i32,
            BinOp::Ge => (lv >= rv) as i32,
            _ => unreachable!("Cmp intent 只会来自六个比较符之一"),
        },
        LogicAnd => ((lv != 0) && (rv != 0)) as i32,
        LogicOr => ((lv != 0) || (rv != 0)) as i32,
    };
    Ok((ty, value))
}

fn checked_div(a: i32, b: i32, span: Span) -> Result<i32, ConstEvalError> {
    if b == 0 {
        Err(div_zero(span))
    } else {
        Ok(a.wrapping_div(b))
    }
}

fn checked_rem(a: i32, b: i32, span: Span) -> Result<i32, ConstEvalError> {
    if b == 0 {
        Err(div_zero(span))
    } else {
        Ok(a.wrapping_rem(b))
    }
}

fn div_zero(span: Span) -> ConstEvalError {
    error(span, "常量表达式除零")
}

fn error(span: Span, msg: impl Into<String>) -> ConstEvalError {
    ConstEvalError {
        span,
        msg: msg.into(),
        needs_cast_hint: false,
    }
}
