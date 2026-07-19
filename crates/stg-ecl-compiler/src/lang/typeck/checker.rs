//! `typeck` 子模块：判型器状态 + 错误上报辅助方法。各判型阶段的 `impl Checker` 分散在
//! `consts`/`exprs`/`stmts` 里，按"一类东西一个文件"拆分（见 `typeck` 模块文档）。

use crate::lang::ast::{CompileError, Span, SubDef, Ty};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Checker<'p> {
    pub(super) subs: BTreeMap<String, &'p SubDef>,
    pub(super) xformdefs: BTreeSet<String>,
    pub(super) consts: BTreeMap<String, (Ty, i32)>,
    pub(super) errors: Vec<CompileError>,
    pub(super) cur_sync_calls: Vec<String>,
    pub(super) cur_xform_refs: Vec<String>,
}

impl<'p> Checker<'p> {
    pub(super) fn err(&mut self, span: Span, msg: String) {
        self.errors.push(CompileError {
            line: span.line,
            col: span.col,
            msg,
            src_line: String::new(),
        });
    }

    /// 类型矩阵"非法格"统一报错口径：正文 + 固定的 cast 提示（测试断言含子串 `cast`）。
    pub(super) fn push_type_mismatch(&mut self, span: Span, msg: String) {
        self.err(
            span,
            format!("{msg}（无隐式转换，提示：用 `as` 做显式 cast，例如 `x as fx`）"),
        );
    }

    pub(super) fn push_sync_call(&mut self, name: String) {
        if !self.cur_sync_calls.contains(&name) {
            self.cur_sync_calls.push(name);
        }
    }

    pub(super) fn push_xform_ref(&mut self, name: String) {
        if !self.cur_xform_refs.contains(&name) {
            self.cur_xform_refs.push(name);
        }
    }
}
