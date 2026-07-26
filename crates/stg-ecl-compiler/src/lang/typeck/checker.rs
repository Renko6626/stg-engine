//! `typeck` 子模块：判型器状态 + 错误上报辅助方法。各判型阶段的 `impl Checker` 分散在
//! `consts`/`exprs`/`stmts` 里，按"一类东西一个文件"拆分（见 `typeck` 模块文档）。

use crate::lang::ast::{CompileError, Span, SubDef, Ty};
use std::collections::{BTreeMap, BTreeSet};

/// 语句糖保留字：不是词法层关键字（`lang::lex` 没为它们开专属 `TokenKind`），但
/// `lang::parse` 按"名字恰为此文本且紧跟对应触发条件"在 `parse_stmt` 分派表里纯文本
/// 截胡、展开成固定子树——**先于任何 sub/const/var 名解析**。若脚本声明同名符号，
/// 该声明会被静默架空（调用点永远走糖展开，声明体永不可达，零诊断零 fault）——
/// C16 曾为 `global()` 修过同类事故（`global` 已改造成走 `check_call` 的普通 builtin
/// 表项，`sub 优先`解析顺序，不再是这种纯文本截胡）；`wait_spell` 复现同一类雷（Task 3
/// 复审修，spec 2026-07-24 §5）。三处声明入口（`typeck` 顶层 sub 登记 / `check_const_def`
/// / `Stmt::Var`）统一在声明处调用 [`Checker::check_not_reserved_sugar_name`] 拒绝，把
/// "同名碰撞"从静默行为错变成编译期错误，不需要在 parser 里逐个猜"这个名字是不是被
/// 用户重新声明过"。
pub(super) const RESERVED_SUGAR_NAMES: &[&str] = &["wait_spell", "mark"];

pub(super) struct Checker<'p, 't> {
    pub(super) subs: BTreeMap<String, &'p SubDef>,
    pub(super) xformdefs: BTreeSet<String>,
    pub(super) consts: BTreeMap<String, (Ty, i32)>,
    pub(super) engine_const_names: BTreeSet<String>,
    pub(super) errors: Vec<CompileError>,
    pub(super) cur_sync_calls: Vec<String>,
    pub(super) cur_xform_refs: Vec<String>,
    /// 绑定的世界表（`None` = 未绑定，跳过一切依赖表的判据）。本刀（颜色轴 T3）只铺管道
    /// 把表存进来，尚无判据消费它——下一刀（形/色组合判据）起读。
    #[allow(dead_code)]
    pub(crate) table: Option<&'t stg_core::tables::WorldTables>,
}

impl<'p, 't> Checker<'p, 't> {
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

    /// 声明名撞上 [`RESERVED_SUGAR_NAMES`] 时记错误并返回 `true`（调用方据此提前
    /// `return`/`continue`，不把声明登记进符号表）；未撞上返回 `false`，调用方照常
    /// 走后续检查（重复定义/引擎常量重名等）。
    pub(super) fn check_not_reserved_sugar_name(&mut self, span: Span, name: &str) -> bool {
        if RESERVED_SUGAR_NAMES.contains(&name) {
            self.err(
                span,
                format!(
                    "'{name}' 是保留字——语句糖 `{name}()` 专用，不能用作 \
                     sub/const/var 的标识符名"
                ),
            );
            true
        } else {
            false
        }
    }
}
