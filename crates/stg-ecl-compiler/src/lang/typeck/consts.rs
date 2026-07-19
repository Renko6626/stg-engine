//! `typeck` 子模块：`const` 声明的编译期常量折叠（见 `typeck` 模块文档"const 折叠范围"）。
//!
//! 只支持字面量 + 更早声明的 `const` 引用 + 一元 `-` + 二元算术/比较/逻辑（复用
//! `matrix::binary_result` 同一张矩阵）；**不支持** `$` 引擎变量/`global()`/函数调用
//! （非编译期可求值）。

use super::checker::Checker;
use super::matrix::{binary_result, divf_const, mulf_const, op_symbol};
use crate::lang::ast::{BinOp, ConstDef, Expr, Span, Ty, UnOp};

impl<'p> Checker<'p> {
    fn fold_const(&mut self, e: &Expr) -> Option<(Ty, i32)> {
        match e {
            Expr::IntLit(v) => Some((Ty::Int, *v)),
            Expr::FxLit(v) => Some((Ty::Fx, *v)),
            Expr::AngleLit(v) => Some((Ty::Angle, *v as i32)),
            Expr::Var(name, span) => match self.consts.get(name) {
                Some((ty, val)) => Some((*ty, *val)),
                None => {
                    self.err(
                        *span,
                        format!(
                            "常量表达式引用了 '{name}'，但它不是已声明的常量\
                             （const 只能引用更早声明的 const，不能引用局部变量/参数）"
                        ),
                    );
                    None
                }
            },
            Expr::EngineVar(_, span) => {
                self.err(
                    *span,
                    "常量表达式不能引用 `$` 引擎变量（非编译期可求值）".into(),
                );
                None
            }
            Expr::GlobalRead { span, .. } => {
                self.err(
                    *span,
                    "常量表达式不能引用 `global()`（非编译期可求值）".into(),
                );
                None
            }
            Expr::Call { span, name, .. } => {
                self.err(
                    *span,
                    format!("常量表达式不能调用 '{name}'（非编译期可求值）"),
                );
                None
            }
            Expr::Unary {
                op: UnOp::Neg, e, ..
            } => {
                let (ty, v) = self.fold_const(e)?;
                Some((ty, v.wrapping_neg()))
            }
            Expr::Unary {
                op: UnOp::Not,
                span,
                ..
            } => {
                self.err(*span, "常量表达式不支持 '!'".into());
                None
            }
            Expr::Binary { op, l, r, span } => {
                let (lt, lv) = self.fold_const(l)?;
                let (rt, rv) = self.fold_const(r)?;
                self.fold_binary_const(*op, lt, lv, rt, rv, *span)
            }
            Expr::Cast { e, to, span } => {
                let (ty, v) = self.fold_const(e)?;
                match (ty, *to) {
                    (Ty::Int, Ty::Fx) => Some((Ty::Fx, v.wrapping_mul(65536))),
                    (Ty::Fx, Ty::Int) => Some((Ty::Int, v / 65536)),
                    (Ty::Int, Ty::Angle) | (Ty::Angle, Ty::Int) => Some((*to, v)),
                    _ => {
                        self.err(
                            *span,
                            format!(
                                "常量表达式：不支持的类型转换 {ty:?} as {to:?}\
                                 （合法：int as fx / fx as int / int as angle / angle as int）"
                            ),
                        );
                        None
                    }
                }
            }
        }
    }

    fn fold_binary_const(
        &mut self,
        op: BinOp,
        lt: Ty,
        lv: i32,
        rt: Ty,
        rv: i32,
        span: Span,
    ) -> Option<(Ty, i32)> {
        match binary_result(op, lt, rt) {
            Err(()) => {
                self.push_type_mismatch(
                    span,
                    format!("常量表达式类型不匹配：{lt:?} {} {rt:?}", op_symbol(op)),
                );
                None
            }
            Ok((ty, intent)) => {
                use super::intents::BinIntent::*;
                let v = match intent {
                    AddI => lv.wrapping_add(rv),
                    SubI => lv.wrapping_sub(rv),
                    MulI => lv.wrapping_mul(rv),
                    DivI => {
                        if rv == 0 {
                            self.err(span, "常量表达式除零".into());
                            return None;
                        }
                        lv.wrapping_div(rv)
                    }
                    ModI => {
                        if rv == 0 {
                            self.err(span, "常量表达式除零".into());
                            return None;
                        }
                        lv.wrapping_rem(rv)
                    }
                    MulF => mulf_const(lv, rv),
                    DivF => {
                        if rv == 0 {
                            self.err(span, "常量表达式除零".into());
                            return None;
                        }
                        divf_const(lv, rv)
                    }
                    Cmp => {
                        let b = match op {
                            BinOp::Eq => lv == rv,
                            BinOp::Ne => lv != rv,
                            BinOp::Lt => lv < rv,
                            BinOp::Le => lv <= rv,
                            BinOp::Gt => lv > rv,
                            BinOp::Ge => lv >= rv,
                            _ => unreachable!("Cmp intent 只会来自六个比较符之一"),
                        };
                        b as i32
                    }
                    LogicAnd => ((lv != 0) && (rv != 0)) as i32,
                    LogicOr => ((lv != 0) || (rv != 0)) as i32,
                };
                Some((ty, v))
            }
        }
    }
}

impl<'p> Checker<'p> {
    pub(super) fn check_const_def(&mut self, cdef: &ConstDef) {
        if self.consts.contains_key(&cdef.name) {
            self.err(cdef.span, format!("常量 '{}' 重复定义", cdef.name));
            return;
        }
        if let Some((ty, val)) = self.fold_const(&cdef.value) {
            if ty != cdef.ty {
                self.push_type_mismatch(
                    cdef.span,
                    format!(
                        "常量 '{}' 声明类型 {:?}，初始化表达式类型却是 {ty:?}",
                        cdef.name, cdef.ty
                    ),
                );
            } else {
                self.consts.insert(cdef.name.clone(), (ty, val));
            }
        }
    }
}
