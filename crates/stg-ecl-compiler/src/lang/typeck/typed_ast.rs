//! `typeck` 子模块：带型影子 AST（T3 codegen 消费）。
//!
//! 每个 [`TypedExpr`] 恒携带且仅携带一个确定的 `Ty`——"无返回值的调用"从不会出现在这层，
//! 见 [`TypedStmt::ExprStmtVoid`]/[`TypedStmt::Spawn`] 里的 [`TypedCall`]。

use super::intents::{BinIntent, CastIntent, UnIntent};
use crate::lang::ast::{BinOp, EngVar, Ty, UnOp};
use crate::lang::builtins::Builtin;

/// 一次调用的目标：`sub`（v1 恒无返回值）或内建函数表条目（返回型见 `Builtin.ret`）。
#[derive(Debug, Clone, PartialEq)]
pub enum CallTarget {
    Sub,
    Builtin(&'static Builtin),
}

/// 一个实参位的判型结果——对齐 `Builtin.params`（或 sub 的 `params`，恒 `Val`）逐位一一
/// 对应；`XformRef`/`SubRef` 不产生求值指令（`fire` 的 `xf`/`task`，编译期标识符解析）。
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Val(TypedExpr),
    /// `Some(name)` = 引用了该 xformdef；`None` = 字面量 `none`。
    XformRef(Option<String>),
    /// `Some(name)` = 引用了该 sub；`None` = 字面量 `none`。
    SubRef(Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedCall {
    pub name: String,
    pub target: CallTarget,
    /// 与源码实参**位置一一对应**（含 `XformRef`/`SubRef` 位——不是"只收 `Val`位后压缩"）。
    pub args: Vec<CallArg>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExprKind {
    IntLit(i32),
    FxLit(i32),
    AngleLit(u16),
    /// 编译期常量的内联替身：折叠出的原始值（`Ty` 见外层 `TypedExpr.ty`）——**不占用任何
    /// locals 槽**，T3 直接 `PUSHI` 这个值，`lang::slots` 也据此不把 `const` 计入宽度。
    ConstRef(i32),
    /// 局部（参数 / `var` / `for` 归纳变量）引用——槽号留 `lang::slots::allocate` 决定，本层
    /// 只留名字。
    LocalRef(String),
    EngineVar(EngVar),
    /// 调用，恒有返回值（`ret: Some(_)`）——`ret: None` 的调用只会出现在
    /// `TypedStmt::ExprStmtVoid`/`TypedStmt::Spawn` 里的 [`TypedCall`]，不会包进这里，
    /// 维持"每个 `TypedExpr` 恒有一个确定 `Ty`"的不变量。
    Call(TypedCall),
    Binary {
        op: BinOp,
        l: Box<TypedExpr>,
        r: Box<TypedExpr>,
        intent: BinIntent,
    },
    Unary {
        op: UnOp,
        e: Box<TypedExpr>,
        intent: UnIntent,
    },
    Cast {
        e: Box<TypedExpr>,
        intent: CastIntent,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedExpr {
    pub ty: Ty,
    pub kind: TypedExprKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedStmt {
    Var {
        name: String,
        ty: Ty,
        init: TypedExpr,
    },
    Assign {
        name: String,
        value: TypedExpr,
    },
    If {
        cond: TypedExpr,
        then_b: Vec<TypedStmt>,
        else_b: Option<Vec<TypedStmt>>,
    },
    While {
        cond: TypedExpr,
        body: Vec<TypedStmt>,
    },
    Loop {
        body: Vec<TypedStmt>,
    },
    For {
        var: String,
        from: TypedExpr,
        to: TypedExpr,
        body: Vec<TypedStmt>,
    },
    Wait {
        frames: TypedExpr,
    },
    Spawn {
        call: TypedCall,
    },
    /// 有返回值且被显式丢弃（`_ = expr;`）——T3 emit 后补一条 `POP`。
    ExprStmtDiscard {
        expr: TypedExpr,
    },
    /// 无返回值的语句调用（内建 `ret=None` 或 sub 调用）——T3 不补 `POP`。
    ExprStmtVoid {
        call: TypedCall,
    },
    Return,
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedSub {
    pub name: String,
    pub is_async: bool,
    pub params: Vec<(String, Ty)>,
    pub body: Vec<TypedStmt>,
    /// 本 sub 内所有同步调用（`Expr::Call`/`Stmt::ExprStmt` 解析到 sub 的那些）目标名，去重、
    /// 源码序——`lang::slots` 建同步调用图的边表，直接消费，不必重新遍历 AST 识别调用目标
    /// 是 sub 还是内建。**不含** `spawn`/`fire` 的 `task` 引用（两者都是"新任务根"，不产生
    /// 同步调用边，见 `lang::slots` 模块文档"为什么不需要显式 entry 集合"）。
    pub sync_calls: Vec<String>,
    /// 本 sub 内所有 `fire(..., xf, ...)` 引用到的 xformdef 名，去重、源码序（`none` 不计入）。
    pub xform_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedInfo {
    /// 与 `Program.subs` 同序同长。
    pub subs: Vec<TypedSub>,
    /// 折叠后的常量：名 / 声明型 / 原始值（源码序）。
    pub consts: Vec<(String, Ty, i32)>,
}
