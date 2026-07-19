//! `typeck` 子模块：块作用域 / definite-assignment 状态机（复审 Critical 修复）。

use crate::lang::ast::Ty;
use std::collections::{BTreeMap, BTreeSet};

/// 一个 sub 内的局部变量状态，两层语义分开管：
/// - `names`：**扁平命名空间**，跨嵌套块永久占用、不支持遮蔽（既有拍板，供确定性槽分配
///   与重名检测——`lang::slots` 消费这份全集，见 `typeck` 模块文档"变量遮蔽"）。
/// - `visible`：**当前控制流位置保证已初始化**的子集，block-scoped——进 if/while/for/loop
///   前拍快照、退出后还原。一个变量只在声明它的块及其嵌套块内可读；块外读取即便同名在
///   `names` 里"合法存在"，也必须拒绝，否则读到的是该 locals 槽此前遗留的值（VM locals
///   是任务级持久内存，不会在进块时清零——复审 Critical 修复，见 `typeck::tests` "块作用域"节）。
#[derive(Default)]
pub(super) struct LocalScope {
    names: BTreeMap<String, Ty>,
    visible: BTreeSet<String>,
}

impl LocalScope {
    pub(super) fn declare(&mut self, name: &str, ty: Ty) {
        self.names.insert(name.to_string(), ty);
        self.visible.insert(name.to_string());
    }

    pub(super) fn is_declared(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub(super) fn type_of(&self, name: &str) -> Option<Ty> {
        self.names.get(name).copied()
    }

    pub(super) fn is_visible(&self, name: &str) -> bool {
        self.visible.contains(name)
    }

    /// 进 if/while/for/loop 的嵌套块前拍快照。
    pub(super) fn snapshot(&self) -> BTreeSet<String> {
        self.visible.clone()
    }

    /// 出嵌套块后还原——块内新声明的名字退出可见集（但仍留在 `names` 里占位，防同名
    /// 在别的分支/之后再声明）。
    pub(super) fn restore(&mut self, snapshot: BTreeSet<String>) {
        self.visible = snapshot;
    }
}
