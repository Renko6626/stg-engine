//! `typeck` 子模块：`const` 声明的编译期常量折叠（见 `typeck` 模块文档"const 折叠范围"）。
//!
//! 求值本体委托 `crate::lang::const_eval::evaluate`（C14 收编：判型与常量折叠共用同一份
//! 求值器，杜绝两处各写一套规则、迟早分叉）——本文件只做"把求值器的错误转本趟诊断"的薄
//! 封装，`check_const_def` 逻辑不变。

use super::checker::Checker;
use crate::lang::ast::ConstDef;
use crate::lang::const_eval::{self, ConstEvalError};

impl<'p> Checker<'p> {
    /// 常量折叠：委托 `lang::const_eval` 唯一求值器，把其错误转本趟诊断（保留既有文案）。
    pub(super) fn fold_const(
        &mut self,
        e: &crate::lang::ast::Expr,
    ) -> Option<(crate::lang::ast::Ty, i32)> {
        match const_eval::evaluate(e, &self.consts) {
            Ok(pair) => Some(pair),
            Err(ConstEvalError {
                span,
                msg,
                needs_cast_hint,
            }) => {
                if needs_cast_hint {
                    self.push_type_mismatch(span, msg);
                } else {
                    self.err(span, msg);
                }
                None
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
