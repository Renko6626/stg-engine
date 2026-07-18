//! ECL 表层语言类型趟（M1.9 T2）——三型（`int`/`fx`/`angle`）自底向上判型 + 值消费检查 +
//! 二元运算指令意图选择，产出 [`TypedInfo`]（供 `lang::slots` 与未来 T3 codegen 消费的带型
//! 影子 AST）。入口 [`check`]。
//!
//! ## 契约出入：`CompileError` 没有 `src_line`
//!
//! `check(&Program) -> Result<TypedInfo, Vec<CompileError>>`——签名只有 `&Program`，没有原始
//! 源码文本（`Program`/`Span` 均不携带它），故本趟构造的 [`CompileError`] 一律
//! `src_line: String::new()`（不经 `CompileError::at`，直接按公开字段构造）。真正呈现给
//! 用户的完整契约格式（文件:行:列 + 源行摘录 + `^`）由 `lang::mod::compile` 总入口负责——
//! 它手上有 `src`，回填 `src_line` 后再 `render()`。`lang::slots` 同款处理（见该模块文档）。
//!
//! ## 类型规则表（plan 核心接口块钉死；`binary_result` 是唯一权威实现，判型与常量折叠共用）
//!
//! | 运算 | 合法格 | 结果型 | 说明 |
//! |---|---|---|---|
//! | `+` `-` | `int+int`/`fx+fx`/`angle+angle`（**同型**，无 int/fx 隐式混算） | 同型 | `ADD`/`SUB`，angle 回绕靠消费点低 16 位截断天然发生，不需要专门指令 |
//! | `*` | `int*int`/`fx*fx`/`fx*int`/`int*fx` | `int`/`fx`/`fx`/`fx` | `fx*fx` 走 `MULF`（Q32.32 中间量 >>16），其余走整数 `MUL`（乘一个纯标量不需要移位） |
//! | `/` | `int/int`/`fx/fx`/`fx/int`（**`int/fx` 非法**——除数是 Q16.16 时直接整数除会错位 65536 倍，需先显式 cast） | `int`/`fx`/`fx` | `fx/fx` 走 `DIVF`（被除数先 `<<16`），其余走整数 `DIV` |
//! | `%` | `int%int` 仅此一格 | `int` | fx/angle 取模无意义，一律拒绝 |
//! | `==` `!=` `<` `<=` `>` `>=` | 同型（int/fx/angle 皆可） | `int`（0/1） | 异型比较拒绝 |
//! | `&&` `\|\|` | `int` 与 `int`（拍板：本语言无独立 bool 型，真值即 int；短路降低为跳转） | `int` | 混型/非 int 拒绝 |
//! | 一元 `-` | int/fx/angle 皆合法 | 同型 | `NEG`（角度回绕同上，天然） |
//! | 一元 `!` | 仅 int | `int` | 无 `NOT` op，T3 降低为等零比较 |
//! | `angle * /` 任意 | 全部非法 | — | plan 明文拍板 |
//!
//! 任何"非法格"（矩阵之外的类型组合）报错文案统一含英文单词 **cast**（"提示：用 `as` 做
//! 显式 cast"），供测试断言"非法格报错含提示 cast"钉死。
//!
//! ## cast 规则（4 条白名单，矩阵之外一律拒绝）
//!
//! `int as fx` = ×65536（[`CastIntent::IntToFx`]）；`fx as int` = ÷65536、**向零截断**
//! （[`CastIntent::FxToInt`]，注意与算术右移对负数的差异——`Fx::div`/`Fx::mul` 走的是
//! 算术右移/floor，这里的整数化走 `i32` 除法本身的向零截断，两者对负数不一致，作者须知）；
//! `int as angle`/`angle as int` = 位穿透（[`CastIntent::Bitcast`]，BAM 回绕靠消费点低 16 位
//! 天然发生，cast 本身不需要发任何指令）。`fx as angle`/`angle as fx`/同型 cast 均不在白名单，
//! 一律编译错误。
//!
//! ## 政策拍板（本刀落地，报告里逐条记录）
//!
//! - **值消费**：有返回值的调用（sub 调用恒无返回值；内建按 `Builtin.ret`）未消费 → 错；
//!   `_ =` 消费一个**无返回值**的调用 → 错（"无值可丢弃"）；裸表达式语句（非调用，如
//!   `1+2;`）同样必须消费。
//! - **变量遮蔽**：同一 sub 内（含跨嵌套块）变量/参数/`for` 归纳变量名一律不可重复——
//!   **拒绝**，不支持遮蔽（简单、确定性槽分配的前提）。
//! - **局部变量 vs 全局常量重名**：允许——局部名解析优先于同名 `const`（局部遮蔽全局，
//!   不报错；与"同一 sub 内变量不可重复"是两条不同的规则，不冲突）。
//! - **`sub` 调用只能作独立语句**：出现在表达式内部（嵌套位置）一律编译错误——v1 的 sub
//!   "无返回值"，用作值没有意义；同一限制也适用于返回 `None` 的内建。
//! - **`break`/`continue` 越界检查**：checklist 未明文要求，但零成本且能拦一整类真实错误
//!   （T3 若无此检查会对着不存在的循环发悬空 continue/break 目标），加做，超出 checklist
//!   字面范围但判定为有益增项，报告中记录。
//! - **`const` 折叠范围**：只支持字面量 + 更早声明的 `const` 引用 + 一元 `-` + 二元算术/比较/
//!   逻辑（复用 [`binary_result`] 同一张矩阵）；**不支持** `$` 引擎变量/`global()`/函数调用
//!   （非编译期可求值）。
//! - **`xformdef` 序列体不在本趟校验范围**：`ast.rs` 模块文档写"slots 里的 Expr 必须是常量
//!   表达式，T2 折叠校验"，但 Task 2 checklist 的实际测试点只考"被引用才占槽 / 3×cnt 对齐 /
//!   off+cnt 上界"——这些是 `lang::slots` 的槽分配职责，不是判型职责。Global Constraints 原文
//!   "序列折叠为 3 字/槽常量"字面归在 **T3 codegen** 段落（`sub 入口 staging（PUSHI+POPL 序列）`
//!   ——那正是 T3 才知道的字节码细节）。故本趟**不**递归校验 `XformDef.slots[].args` 的常量性/
//!   合法 op 名——留给 T3；本趟只处理 `fire(...)` 的 `xf` 参数标识符解析（是否是已声明
//!   `xformdef` 名或 `none`），见 `check_builtin_call_args`。此为有意收窄范围的选择，非疏漏，
//!   报告中列为 T3 交接注意事项。

use crate::lang::ast::{
    BinOp, Block, CompileError, ConstDef, EngVar, Expr, Program, Span, Stmt, SubDef, Ty, UnOp,
};
use crate::lang::builtins::{self, Builtin, ParamKind};
use std::collections::{BTreeMap, BTreeSet};

// ── 指令意图（T3 codegen 消费；Add/Sub/Mod/比较/逻辑在类型矩阵下零歧义，直接照 `BinOp`
// 选 op，`intent` 只对 `*`/`/` 真正做选择——但仍给全量变体，免去 T3 反查矩阵）───────────

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

// ── 带型影子 AST（T3 消费；每个 `TypedExpr` 恒携带且仅携带一个确定的 `Ty`——"无返回值的
// 调用"从不会出现在这层，见 `TypedCall`/`TypedStmt` 的专门分支）────────────────────────

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
    GlobalRead(Box<TypedExpr>),
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

// ── 类型矩阵唯一权威：判型（`type_binary`）与常量折叠（`fold_binary_const`）共用，
// 杜绝"两处写两套规则、迟早分叉"───────────────────────────────────────────────────

fn binary_result(op: BinOp, lt: Ty, rt: Ty) -> Result<(Ty, BinIntent), ()> {
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

fn op_symbol(op: BinOp) -> &'static str {
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
fn expr_span(e: &Expr) -> Option<Span> {
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

fn mulf_const(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

fn divf_const(a: i32, b: i32) -> i32 {
    (((a as i64) << 16) / b as i64) as i32
}

enum RefKind {
    Xform,
    Sub,
}

struct Checker<'p> {
    subs: BTreeMap<String, &'p SubDef>,
    xformdefs: BTreeSet<String>,
    consts: BTreeMap<String, (Ty, i32)>,
    errors: Vec<CompileError>,
    cur_sync_calls: Vec<String>,
    cur_xform_refs: Vec<String>,
}

impl<'p> Checker<'p> {
    fn err(&mut self, span: Span, msg: String) {
        self.errors.push(CompileError {
            line: span.line,
            col: span.col,
            msg,
            src_line: String::new(),
        });
    }

    /// 类型矩阵"非法格"统一报错口径：正文 + 固定的 cast 提示（测试断言含子串 `cast`）。
    fn push_type_mismatch(&mut self, span: Span, msg: String) {
        self.err(
            span,
            format!("{msg}（无隐式转换，提示：用 `as` 做显式 cast，例如 `x as fx`）"),
        );
    }

    fn push_sync_call(&mut self, name: String) {
        if !self.cur_sync_calls.contains(&name) {
            self.cur_sync_calls.push(name);
        }
    }

    fn push_xform_ref(&mut self, name: String) {
        if !self.cur_xform_refs.contains(&name) {
            self.cur_xform_refs.push(name);
        }
    }

    // ── 常量折叠（`const` 声明专用；见模块文档"const 折叠范围"）────────────────────

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
                use BinIntent::*;
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

    // ── 表达式判型（通用，值恒 `Some`——调用到 sub / `ret=None` 内建一律拒绝，见
    // `check_call` 的 `nested=true` 分支）─────────────────────────────────────────

    fn type_expr(&mut self, e: &Expr, locals: &BTreeMap<String, Ty>) -> Option<TypedExpr> {
        match e {
            Expr::IntLit(v) => Some(TypedExpr {
                ty: Ty::Int,
                kind: TypedExprKind::IntLit(*v),
            }),
            Expr::FxLit(v) => Some(TypedExpr {
                ty: Ty::Fx,
                kind: TypedExprKind::FxLit(*v),
            }),
            Expr::AngleLit(v) => Some(TypedExpr {
                ty: Ty::Angle,
                kind: TypedExprKind::AngleLit(*v),
            }),
            Expr::Var(name, span) => {
                if let Some(ty) = locals.get(name) {
                    Some(TypedExpr {
                        ty: *ty,
                        kind: TypedExprKind::LocalRef(name.clone()),
                    })
                } else if let Some((ty, val)) = self.consts.get(name) {
                    Some(TypedExpr {
                        ty: *ty,
                        kind: TypedExprKind::ConstRef(*val),
                    })
                } else {
                    self.err(*span, format!("未定义的变量 '{name}'"));
                    None
                }
            }
            Expr::EngineVar(ev, _span) => {
                let info = builtins::engine_var_info(*ev);
                Some(TypedExpr {
                    ty: info.ty,
                    kind: TypedExprKind::EngineVar(*ev),
                })
            }
            Expr::GlobalRead { slot, span } => {
                let slot_t = self.type_expr(slot, locals)?;
                if slot_t.ty != Ty::Int {
                    self.push_type_mismatch(
                        expr_span(slot).unwrap_or(*span),
                        format!("global(n) 的 n 必须是 int，实际 {:?}", slot_t.ty),
                    );
                    return None;
                }
                Some(TypedExpr {
                    ty: Ty::Int,
                    kind: TypedExprKind::GlobalRead(Box::new(slot_t)),
                })
            }
            Expr::Call { name, args, span } => {
                let (call, ret) = self.check_call(name, args, *span, locals, true)?;
                let ty = ret.expect("nested=true 分支已确保 ret 非 None，否则上面已 return None");
                Some(TypedExpr {
                    ty,
                    kind: TypedExprKind::Call(call),
                })
            }
            Expr::Binary { op, l, r, span } => {
                let lt = self.type_expr(l, locals)?;
                let rt = self.type_expr(r, locals)?;
                match binary_result(*op, lt.ty, rt.ty) {
                    Ok((ty, intent)) => Some(TypedExpr {
                        ty,
                        kind: TypedExprKind::Binary {
                            op: *op,
                            l: Box::new(lt),
                            r: Box::new(rt),
                            intent,
                        },
                    }),
                    Err(()) => {
                        self.push_type_mismatch(
                            *span,
                            format!("类型不匹配：{:?} {} {:?}", lt.ty, op_symbol(*op), rt.ty),
                        );
                        None
                    }
                }
            }
            Expr::Unary { op, e, span } => {
                let et = self.type_expr(e, locals)?;
                match op {
                    UnOp::Neg => Some(TypedExpr {
                        ty: et.ty,
                        kind: TypedExprKind::Unary {
                            op: *op,
                            e: Box::new(et),
                            intent: UnIntent::Neg,
                        },
                    }),
                    UnOp::Not => {
                        if et.ty == Ty::Int {
                            Some(TypedExpr {
                                ty: Ty::Int,
                                kind: TypedExprKind::Unary {
                                    op: *op,
                                    e: Box::new(et),
                                    intent: UnIntent::Not,
                                },
                            })
                        } else {
                            self.push_type_mismatch(
                                *span,
                                format!("'!' 仅支持 int 操作数，实际 {:?}", et.ty),
                            );
                            None
                        }
                    }
                }
            }
            Expr::Cast { e, to, span } => {
                let et = self.type_expr(e, locals)?;
                let intent = match (et.ty, *to) {
                    (Ty::Int, Ty::Fx) => CastIntent::IntToFx,
                    (Ty::Fx, Ty::Int) => CastIntent::FxToInt,
                    (Ty::Int, Ty::Angle) | (Ty::Angle, Ty::Int) => CastIntent::Bitcast,
                    _ => {
                        self.err(
                            *span,
                            format!(
                                "不支持的类型转换：{:?} as {to:?}\
                                 （合法：int as fx / fx as int / int as angle / angle as int）",
                                et.ty
                            ),
                        );
                        return None;
                    }
                };
                Some(TypedExpr {
                    ty: *to,
                    kind: TypedExprKind::Cast {
                        e: Box::new(et),
                        intent,
                    },
                })
            }
        }
    }

    // ── 调用解析（sub / 内建共用一套名字解析；`nested` 区分"表达式内部"（拒绝 `ret=None`）
    // 与"独立语句"（`ret=None` 合法）两种语境）──────────────────────────────────────

    fn check_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        locals: &BTreeMap<String, Ty>,
        nested: bool,
    ) -> Option<(TypedCall, Option<Ty>)> {
        if let Some(sub) = self.subs.get(name).copied() {
            let call_args = self.check_sub_call_args(&sub.params, args, span, locals)?;
            self.push_sync_call(name.to_string());
            if nested {
                self.err(
                    span,
                    format!(
                        "'{name}' 是 sub 调用，无返回值，不能用作表达式的值\
                         （sub 调用只能作为独立语句）"
                    ),
                );
                return None;
            }
            return Some((
                TypedCall {
                    name: name.to_string(),
                    target: CallTarget::Sub,
                    args: call_args,
                },
                None,
            ));
        }
        if let Some(b) = builtins::lookup(name) {
            let call_args = self.check_builtin_call_args(b, args, span, locals)?;
            let call = TypedCall {
                name: name.to_string(),
                target: CallTarget::Builtin(b),
                args: call_args,
            };
            if nested && b.ret.is_none() {
                self.err(span, format!("'{name}' 无返回值，不能用作表达式的值"));
                return None;
            }
            return Some((call, b.ret));
        }
        self.err(span, format!("未定义的函数/sub '{name}'"));
        None
    }

    fn check_sub_call_args(
        &mut self,
        params: &[(String, Ty)],
        args: &[Expr],
        span: Span,
        locals: &BTreeMap<String, Ty>,
    ) -> Option<Vec<CallArg>> {
        if params.len() != args.len() {
            self.err(
                span,
                format!("参数个数不匹配：期待 {}，实际 {}", params.len(), args.len()),
            );
            return None;
        }
        let mut out = Vec::with_capacity(args.len());
        let mut ok = true;
        for (i, (pname, pty)) in params.iter().enumerate() {
            match self.type_expr(&args[i], locals) {
                Some(t) if t.ty == *pty => out.push(CallArg::Val(t)),
                Some(t) => {
                    self.push_type_mismatch(
                        expr_span(&args[i]).unwrap_or(span),
                        format!(
                            "第 {} 个参数 '{pname}' 期待 {pty:?}，实际 {:?}",
                            i + 1,
                            t.ty
                        ),
                    );
                    ok = false;
                }
                None => ok = false,
            }
        }
        if ok { Some(out) } else { None }
    }

    fn check_builtin_call_args(
        &mut self,
        b: &'static Builtin,
        args: &[Expr],
        span: Span,
        locals: &BTreeMap<String, Ty>,
    ) -> Option<Vec<CallArg>> {
        if b.params.len() != args.len() {
            self.err(
                span,
                format!(
                    "'{}' 参数个数不匹配：期待 {}，实际 {}",
                    b.name,
                    b.params.len(),
                    args.len()
                ),
            );
            return None;
        }
        let mut out = Vec::with_capacity(args.len());
        let mut ok = true;
        for (i, pk) in b.params.iter().enumerate() {
            let a = &args[i];
            match pk {
                ParamKind::Val(pty) => match self.type_expr(a, locals) {
                    Some(t) if t.ty == *pty => out.push(CallArg::Val(t)),
                    Some(t) => {
                        self.push_type_mismatch(
                            expr_span(a).unwrap_or(span),
                            format!(
                                "'{}' 第 {} 个参数期待 {pty:?}，实际 {:?}",
                                b.name,
                                i + 1,
                                t.ty
                            ),
                        );
                        ok = false;
                    }
                    None => ok = false,
                },
                ParamKind::XformRef => match self.resolve_ident_ref(a, span, RefKind::Xform) {
                    Some(r) => {
                        if let Some(n) = &r {
                            self.push_xform_ref(n.clone());
                        }
                        out.push(CallArg::XformRef(r));
                    }
                    None => ok = false,
                },
                ParamKind::SubRef => match self.resolve_ident_ref(a, span, RefKind::Sub) {
                    Some(r) => out.push(CallArg::SubRef(r)),
                    None => ok = false,
                },
            }
        }
        if ok { Some(out) } else { None }
    }

    fn resolve_ident_ref(
        &mut self,
        a: &Expr,
        call_span: Span,
        kind: RefKind,
    ) -> Option<Option<String>> {
        match a {
            Expr::Var(name, vspan) => {
                if name == "none" {
                    return Some(None);
                }
                let known = match kind {
                    RefKind::Xform => self.xformdefs.contains(name),
                    RefKind::Sub => self.subs.contains_key(name),
                };
                if known {
                    Some(Some(name.clone()))
                } else {
                    let what = match kind {
                        RefKind::Xform => "xformdef",
                        RefKind::Sub => "sub",
                    };
                    self.err(*vspan, format!("未知的 {what} 名 '{name}'"));
                    None
                }
            }
            _ => {
                self.err(
                    call_span,
                    "期待标识符（xformdef 名 / sub 名或 'none'），不是求值表达式".into(),
                );
                None
            }
        }
    }

    // ── 语句判型 ─────────────────────────────────────────────────────────────────

    fn check_expr_stmt(
        &mut self,
        expr: &Expr,
        discarded: bool,
        span: Span,
        locals: &BTreeMap<String, Ty>,
    ) -> Option<TypedStmt> {
        if let Expr::Call {
            name,
            args,
            span: cspan,
        } = expr
        {
            let (call, ret) = self.check_call(name, args, *cspan, locals, false)?;
            return match ret {
                Some(ty) => {
                    if discarded {
                        Some(TypedStmt::ExprStmtDiscard {
                            expr: TypedExpr {
                                ty,
                                kind: TypedExprKind::Call(call),
                            },
                        })
                    } else {
                        self.err(
                            *cspan,
                            format!(
                                "调用 '{name}' 的返回值未消费\
                                 （加前缀 `_ = ` 显式丢弃，或参与更大的表达式）"
                            ),
                        );
                        None
                    }
                }
                None => {
                    if discarded {
                        self.err(
                            *cspan,
                            format!("'{name}' 无返回值，无值可丢弃（去掉 `_ = ` 前缀）"),
                        );
                        None
                    } else {
                        Some(TypedStmt::ExprStmtVoid { call })
                    }
                }
            };
        }
        // 非 `Call` 的裸表达式语句（如 `1 + 2;`）：恒有值，必须消费。
        let t = self.type_expr(expr, locals)?;
        if discarded {
            Some(TypedStmt::ExprStmtDiscard { expr: t })
        } else {
            self.err(span, "表达式的值未消费（加前缀 `_ = ` 显式丢弃）".into());
            None
        }
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt,
        locals: &mut BTreeMap<String, Ty>,
        in_loop: bool,
    ) -> Option<TypedStmt> {
        match stmt {
            Stmt::Var {
                name,
                ty,
                init,
                span,
            } => {
                let t = self.type_expr(init, locals)?;
                if t.ty != *ty {
                    self.push_type_mismatch(
                        expr_span(init).unwrap_or(*span),
                        format!(
                            "变量 '{name}' 声明类型 {ty:?}，初始化表达式类型却是 {:?}",
                            t.ty
                        ),
                    );
                    return None;
                }
                if locals.contains_key(name) {
                    self.err(
                        *span,
                        format!("变量名 '{name}' 重复声明（同一 sub 内需唯一，暂不支持遮蔽）"),
                    );
                    return None;
                }
                locals.insert(name.clone(), *ty);
                Some(TypedStmt::Var {
                    name: name.clone(),
                    ty: *ty,
                    init: t,
                })
            }
            Stmt::Assign { name, value, span } => {
                let t = self.type_expr(value, locals)?;
                match locals.get(name).copied() {
                    Some(lt) => {
                        if lt != t.ty {
                            self.push_type_mismatch(
                                expr_span(value).unwrap_or(*span),
                                format!("赋值给 '{name}'（类型 {lt:?}）的值类型却是 {:?}", t.ty),
                            );
                            return None;
                        }
                        Some(TypedStmt::Assign {
                            name: name.clone(),
                            value: t,
                        })
                    }
                    None => {
                        if self.consts.contains_key(name) {
                            self.err(*span, format!("不能给编译期常量 '{name}' 赋值"));
                        } else {
                            self.err(*span, format!("未定义的变量 '{name}'"));
                        }
                        None
                    }
                }
            }
            Stmt::If {
                cond,
                then_b,
                else_b,
                span,
            } => {
                let cond_t = self.type_expr(cond, locals);
                let ok_cond = match &cond_t {
                    Some(c) if c.ty == Ty::Int => true,
                    Some(c) => {
                        self.push_type_mismatch(
                            expr_span(cond).unwrap_or(*span),
                            format!("if 条件必须是 int，实际 {:?}", c.ty),
                        );
                        false
                    }
                    None => false,
                };
                let then_typed = self.check_block(then_b, locals, in_loop);
                let else_typed = else_b
                    .as_ref()
                    .map(|b| self.check_block(b, locals, in_loop));
                if ok_cond {
                    Some(TypedStmt::If {
                        cond: cond_t.unwrap(),
                        then_b: then_typed,
                        else_b: else_typed,
                    })
                } else {
                    None
                }
            }
            Stmt::While { cond, body, span } => {
                let cond_t = self.type_expr(cond, locals);
                let ok_cond = match &cond_t {
                    Some(c) if c.ty == Ty::Int => true,
                    Some(c) => {
                        self.push_type_mismatch(
                            expr_span(cond).unwrap_or(*span),
                            format!("while 条件必须是 int，实际 {:?}", c.ty),
                        );
                        false
                    }
                    None => false,
                };
                let body_typed = self.check_block(body, locals, true);
                if ok_cond {
                    Some(TypedStmt::While {
                        cond: cond_t.unwrap(),
                        body: body_typed,
                    })
                } else {
                    None
                }
            }
            Stmt::Loop { body, .. } => {
                let body_typed = self.check_block(body, locals, true);
                Some(TypedStmt::Loop { body: body_typed })
            }
            Stmt::For {
                var,
                from,
                to,
                body,
                span,
            } => {
                let from_t = self.type_expr(from, locals);
                let to_t = self.type_expr(to, locals);
                let ok_from = match &from_t {
                    Some(t) if t.ty == Ty::Int => true,
                    Some(t) => {
                        self.push_type_mismatch(
                            expr_span(from).unwrap_or(*span),
                            format!("for 循环起点必须是 int，实际 {:?}", t.ty),
                        );
                        false
                    }
                    None => false,
                };
                let ok_to = match &to_t {
                    Some(t) if t.ty == Ty::Int => true,
                    Some(t) => {
                        self.push_type_mismatch(
                            expr_span(to).unwrap_or(*span),
                            format!("for 循环终点必须是 int，实际 {:?}", t.ty),
                        );
                        false
                    }
                    None => false,
                };
                let dup = locals.contains_key(var);
                if dup {
                    self.err(
                        *span,
                        format!("变量名 '{var}' 重复声明（同一 sub 内需唯一，暂不支持遮蔽）"),
                    );
                } else {
                    locals.insert(var.clone(), Ty::Int);
                }
                let body_typed = self.check_block(body, locals, true);
                if ok_from && ok_to && !dup {
                    Some(TypedStmt::For {
                        var: var.clone(),
                        from: from_t.unwrap(),
                        to: to_t.unwrap(),
                        body: body_typed,
                    })
                } else {
                    None
                }
            }
            Stmt::Wait { frames, span } => match self.type_expr(frames, locals) {
                Some(t) if t.ty == Ty::Int => Some(TypedStmt::Wait { frames: t }),
                Some(t) => {
                    self.push_type_mismatch(
                        expr_span(frames).unwrap_or(*span),
                        format!("wait(n) 的 n 必须是 int，实际 {:?}", t.ty),
                    );
                    None
                }
                None => None,
            },
            Stmt::Spawn { name, args, span } => match self.subs.get(name).copied() {
                Some(target) => {
                    let call_args =
                        self.check_sub_call_args(&target.params, args, *span, locals)?;
                    Some(TypedStmt::Spawn {
                        call: TypedCall {
                            name: name.clone(),
                            target: CallTarget::Sub,
                            args: call_args,
                        },
                    })
                }
                None => {
                    self.err(
                        *span,
                        format!("未定义的 sub '{name}'（spawn 目标必须是已声明的 sub）"),
                    );
                    None
                }
            },
            Stmt::ExprStmt {
                expr,
                discarded,
                span,
            } => self.check_expr_stmt(expr, *discarded, *span, locals),
            Stmt::Return { .. } => Some(TypedStmt::Return),
            Stmt::Break { span } => {
                if in_loop {
                    Some(TypedStmt::Break)
                } else {
                    self.err(*span, "'break' 只能出现在循环内（while/loop/for）".into());
                    None
                }
            }
            Stmt::Continue { span } => {
                if in_loop {
                    Some(TypedStmt::Continue)
                } else {
                    self.err(
                        *span,
                        "'continue' 只能出现在循环内（while/loop/for）".into(),
                    );
                    None
                }
            }
        }
    }

    fn check_block(
        &mut self,
        block: &Block,
        locals: &mut BTreeMap<String, Ty>,
        in_loop: bool,
    ) -> Vec<TypedStmt> {
        let mut out = Vec::new();
        for stmt in block {
            if let Some(ts) = self.check_stmt(stmt, locals, in_loop) {
                out.push(ts);
            }
        }
        out
    }

    fn check_sub(&mut self, sub: &'p SubDef) -> TypedSub {
        self.cur_sync_calls = Vec::new();
        self.cur_xform_refs = Vec::new();
        let mut locals: BTreeMap<String, Ty> = BTreeMap::new();
        for (pname, pty) in &sub.params {
            if locals.contains_key(pname) {
                self.err(sub.span, format!("参数名 '{pname}' 重复"));
            } else {
                locals.insert(pname.clone(), *pty);
            }
        }
        let body = self.check_block(&sub.body, &mut locals, false);
        TypedSub {
            name: sub.name.clone(),
            is_async: sub.is_async,
            params: sub.params.clone(),
            body,
            sync_calls: std::mem::take(&mut self.cur_sync_calls),
            xform_refs: std::mem::take(&mut self.cur_xform_refs),
        }
    }
}

/// 类型趟入口：自底向上判型 + 值消费检查，产出 [`TypedInfo`]（见模块文档）。
pub fn check(prog: &Program) -> Result<TypedInfo, Vec<CompileError>> {
    let mut c = Checker {
        subs: BTreeMap::new(),
        xformdefs: BTreeSet::new(),
        consts: BTreeMap::new(),
        errors: Vec::new(),
        cur_sync_calls: Vec::new(),
        cur_xform_refs: Vec::new(),
    };

    for xf in &prog.xformdefs {
        c.xformdefs.insert(xf.name.clone());
    }

    for s in &prog.subs {
        if c.subs.contains_key(&s.name) {
            c.err(s.span, format!("sub '{}' 重复定义", s.name));
        } else {
            c.subs.insert(s.name.clone(), s);
        }
    }

    for cdef in &prog.consts {
        c.check_const_def(cdef);
    }

    let mut typed_subs = Vec::with_capacity(prog.subs.len());
    for s in &prog.subs {
        typed_subs.push(c.check_sub(s));
    }

    if c.errors.is_empty() {
        let consts = c
            .consts
            .iter()
            .map(|(k, (ty, v))| (k.clone(), *ty, *v))
            .collect();
        Ok(TypedInfo {
            subs: typed_subs,
            consts,
        })
    } else {
        Err(c.errors)
    }
}

impl<'p> Checker<'p> {
    fn check_const_def(&mut self, cdef: &ConstDef) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::parse as parse_program;

    /// 便于按源码片段快速构造 `Program`（复用 T1 前端；本趟测试不手搭 AST，除非要覆盖
    /// AST 层面到不了的边界）。
    fn prog(src: &str) -> Program {
        parse_program(src, "t.ecl").unwrap_or_else(|e| panic!("解析失败：{e:?}\n源码：\n{src}"))
    }

    fn ok(src: &str) -> TypedInfo {
        check(&prog(src)).unwrap_or_else(|e| panic!("判型失败：{e:?}\n源码：\n{src}"))
    }

    fn err(src: &str) -> Vec<CompileError> {
        check(&prog(src)).expect_err(&format!("期望判型失败，源码：\n{src}"))
    }

    fn main_body(ti: &TypedInfo) -> &[TypedStmt] {
        &ti.subs
            .iter()
            .find(|s| s.name == "main")
            .expect("应有 main")
            .body
    }

    /// 抽出 `main` 里唯一一条 `ExprStmtDiscard`/`Var` 的求值表达式，供矩阵测试断言型与
    /// intent（每条测试源码只放一条相关语句，简化定位）。
    fn only_var_init(ti: &TypedInfo) -> &TypedExpr {
        match &main_body(ti)[0] {
            TypedStmt::Var { init, .. } => init,
            other => panic!("期望 Var 语句，得到 {other:?}"),
        }
    }

    // ── 类型矩阵：`+`/`-` ────────────────────────────────────────────────────────

    #[test]
    fn add_int_int_is_legal_int_addi() {
        let ti = ok("sub main() { var x: int = 1 + 2; }");
        let e = only_var_init(&ti);
        assert_eq!(e.ty, Ty::Int);
        assert!(matches!(
            e.kind,
            TypedExprKind::Binary {
                intent: BinIntent::AddI,
                ..
            }
        ));
    }

    #[test]
    fn add_fx_fx_is_legal_fx_addi() {
        let ti = ok("sub main() { var x: fx = 1.0fx + 2.0fx; }");
        let e = only_var_init(&ti);
        assert_eq!(e.ty, Ty::Fx);
        assert!(matches!(
            e.kind,
            TypedExprKind::Binary {
                intent: BinIntent::AddI,
                ..
            }
        ));
    }

    #[test]
    fn add_angle_angle_is_legal_angle_addi() {
        let ti = ok("sub main() { var x: angle = 10deg + 20deg; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Angle);
    }

    #[test]
    fn sub_angle_angle_is_legal() {
        let ti = ok("sub main() { var x: angle = 90deg - 10deg; }");
        let e = only_var_init(&ti);
        assert_eq!(e.ty, Ty::Angle);
        assert!(matches!(
            e.kind,
            TypedExprKind::Binary {
                intent: BinIntent::SubI,
                ..
            }
        ));
    }

    #[test]
    fn add_fx_int_is_illegal_no_implicit_conversion() {
        let errors = err("sub main() { var x: fx = 1.0fx + 1; }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 类型矩阵：`*` ────────────────────────────────────────────────────────────

    #[test]
    fn mul_int_int_is_legal_muli() {
        let ti = ok("sub main() { var x: int = 3 * 4; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Int);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::MulI,
                ..
            }
        ));
    }

    #[test]
    fn mul_fx_fx_is_legal_mulf() {
        let ti = ok("sub main() { var x: fx = 1.0fx * 2.0fx; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Fx);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::MulF,
                ..
            }
        ));
    }

    #[test]
    fn mul_fx_int_is_legal_fx_muli() {
        let ti = ok("sub main() { var x: fx = 1.0fx * 3; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Fx);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::MulI,
                ..
            }
        ));
    }

    #[test]
    fn mul_int_fx_is_legal_fx_muli_symmetric() {
        let ti = ok("sub main() { var x: fx = 3 * 1.0fx; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Fx);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::MulI,
                ..
            }
        ));
    }

    #[test]
    fn mul_angle_int_is_illegal() {
        let errors = err("sub main() { var x: angle = 90deg * 2; }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 类型矩阵：`/`（asymmetric fx//int）───────────────────────────────────────

    #[test]
    fn div_int_int_is_legal_divi() {
        let ti = ok("sub main() { var x: int = 10 / 3; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Int);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::DivI,
                ..
            }
        ));
    }

    #[test]
    fn div_fx_fx_is_legal_divf() {
        let ti = ok("sub main() { var x: fx = 1.0fx / 2.0fx; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Fx);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::DivF,
                ..
            }
        ));
    }

    #[test]
    fn div_fx_int_is_legal_divi() {
        let ti = ok("sub main() { var x: fx = 1.0fx / 2; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Fx);
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Binary {
                intent: BinIntent::DivI,
                ..
            }
        ));
    }

    /// 判别式核心：`int/fx` 与 `fx/int` **不对称**——`fx/int` 合法，`int/fx` 非法（除数
    /// 是 Q16.16 时纯整数除会错位 65536 倍），矩阵里唯一的方向性陷阱。
    #[test]
    fn div_int_fx_is_illegal_asymmetric() {
        let errors = err("sub main() { var x: fx = 1 / 2.0fx; }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 类型矩阵：`%`（仅 int）────────────────────────────────────────────────────

    #[test]
    fn mod_int_int_is_legal() {
        let ti = ok("sub main() { var x: int = 7 % 3; }");
        assert_eq!(only_var_init(&ti).ty, Ty::Int);
    }

    #[test]
    fn mod_fx_fx_is_illegal() {
        let errors = err("sub main() { var x: fx = 1.0fx % 2.0fx; }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 类型矩阵：比较（同型合法，异型非法）───────────────────────────────────────

    #[test]
    fn comparison_same_type_yields_int_for_all_three_types() {
        for (src, _) in [
            ("sub main() { var x: int = (1 > 2); }", "int"),
            ("sub main() { var x: int = (1.0fx > 2.0fx); }", "fx"),
            ("sub main() { var x: int = (10deg > 20deg); }", "angle"),
        ] {
            let ti = ok(src);
            assert_eq!(only_var_init(&ti).ty, Ty::Int, "src={src}");
        }
    }

    #[test]
    fn comparison_mismatched_types_is_illegal() {
        let errors = err("sub main() { var x: int = (1 > 2.0fx); }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 类型矩阵：`&&`/`||`（拍板：仅 int）────────────────────────────────────────

    #[test]
    fn logic_and_or_int_int_is_legal() {
        let ti = ok(
            "sub main() { var a: int = 1; var b: int = 0; var x: int = a && b; var y: int = a || b; }",
        );
        // 第 3、4 条语句才是 And/Or；直接检查末条 Or。
        match &main_body(&ti)[3] {
            TypedStmt::Var { init, .. } => {
                assert_eq!(init.ty, Ty::Int);
                assert!(matches!(
                    init.kind,
                    TypedExprKind::Binary {
                        intent: BinIntent::LogicOr,
                        ..
                    }
                ));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn logic_and_fx_fx_is_illegal() {
        let errors = err("sub main() { var x: int = 1.0fx && 2.0fx; }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 一元运算 ─────────────────────────────────────────────────────────────────

    #[test]
    fn neg_is_legal_for_int_fx_angle() {
        for src in [
            "sub main() { var x: int = -1; }",
            "sub main() { var x: fx = -1.0fx; }",
            "sub main() { var x: angle = -90deg; }",
        ] {
            ok(src);
        }
    }

    #[test]
    fn not_is_legal_for_int_only() {
        ok("sub main() { var x: int = !1; }");
        let errors = err("sub main() { var x: int = !(1.0fx); }");
        assert!(errors.iter().any(|e| e.msg.contains("int")), "{errors:?}");
    }

    // ── cast 规则 ───────────────────────────────────────────────────────────────

    #[test]
    fn cast_int_to_fx_is_legal_with_int_to_fx_intent() {
        let ti = ok("sub main() { var x: fx = 1 as fx; }");
        let e = only_var_init(&ti);
        assert_eq!(e.ty, Ty::Fx);
        assert!(matches!(
            e.kind,
            TypedExprKind::Cast {
                intent: CastIntent::IntToFx,
                ..
            }
        ));
    }

    #[test]
    fn cast_fx_to_int_is_legal_with_fx_to_int_intent() {
        let ti = ok("sub main() { var x: int = 1.0fx as int; }");
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Cast {
                intent: CastIntent::FxToInt,
                ..
            }
        ));
    }

    #[test]
    fn cast_int_angle_bitcast_both_directions() {
        let ti = ok("sub main() { var x: angle = 1 as angle; }");
        assert!(matches!(
            only_var_init(&ti).kind,
            TypedExprKind::Cast {
                intent: CastIntent::Bitcast,
                ..
            }
        ));
        let ti2 = ok("sub main() { var x: int = 90deg as int; }");
        assert!(matches!(
            only_var_init(&ti2).kind,
            TypedExprKind::Cast {
                intent: CastIntent::Bitcast,
                ..
            }
        ));
    }

    #[test]
    fn cast_fx_to_angle_is_not_whitelisted() {
        err("sub main() { var x: angle = 1.0fx as angle; }");
    }

    #[test]
    fn cast_angle_to_fx_is_not_whitelisted() {
        err("sub main() { var x: fx = 90deg as fx; }");
    }

    // ── 值消费检查 ──────────────────────────────────────────────────────────────

    #[test]
    fn call_with_return_value_not_discarded_is_an_error() {
        let errors = err("sub main() { fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未消费")),
            "{errors:?}"
        );
    }

    #[test]
    fn call_with_return_value_discarded_is_ok() {
        ok("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
    }

    #[test]
    fn void_builtin_statement_without_discard_is_ok() {
        ok("sub main() { pulse_signal(0); }");
    }

    #[test]
    fn void_builtin_discarded_is_an_error_nothing_to_discard() {
        let errors = err("sub main() { _ = pulse_signal(0); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("无值可丢弃")),
            "{errors:?}"
        );
    }

    #[test]
    fn bare_expr_statement_not_discarded_is_an_error() {
        let errors = err("sub main() { var x: int = 1; x + 1; }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未消费")),
            "{errors:?}"
        );
    }

    #[test]
    fn bare_expr_statement_discarded_is_ok() {
        ok("sub main() { var x: int = 1; _ = x + 1; }");
    }

    // ── sub 调用：只能作独立语句 ───────────────────────────────────────────────

    #[test]
    fn sub_call_as_bare_statement_is_ok() {
        ok("sub helper() { } sub main() { helper(); }");
    }

    #[test]
    fn sub_call_discarded_is_an_error() {
        let errors = err("sub helper() { } sub main() { _ = helper(); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("无值可丢弃")),
            "{errors:?}"
        );
    }

    #[test]
    fn sub_call_in_expression_position_is_an_error() {
        let errors = err("sub helper() { } sub main() { var x: int = helper() + 1; }");
        assert!(
            errors.iter().any(|e| e.msg.contains("表达式的值")),
            "{errors:?}"
        );
    }

    // ── `$` 引擎变量判型 ────────────────────────────────────────────────────────

    #[test]
    fn engine_vars_type_per_plan_table() {
        let cases: &[(&str, Ty)] = &[
            ("$frame", Ty::Int),
            ("$player_x", Ty::Fx),
            ("$player_y", Ty::Fx),
            ("$self_x", Ty::Fx),
            ("$self_y", Ty::Fx),
            ("$self_hp", Ty::Int),
            ("$self_hp_max", Ty::Int),
            ("$self_age", Ty::Int),
        ];
        for (var, ty) in cases {
            let src = match ty {
                Ty::Int => format!("sub main() {{ var x: int = {var}; }}"),
                Ty::Fx => format!("sub main() {{ var x: fx = {var}; }}"),
                Ty::Angle => format!("sub main() {{ var x: angle = {var}; }}"),
            };
            let ti = ok(&src);
            assert_eq!(only_var_init(&ti).ty, *ty, "{var}");
        }
    }

    // ── `global`/`set_global` ──────────────────────────────────────────────────

    #[test]
    fn global_read_types_int_and_requires_int_slot() {
        let ti = ok("const S: int = 5; sub main() { var x: int = global(S); }");
        assert_eq!(only_var_init(&ti).ty, Ty::Int);
        let errors = err("sub main() { var x: int = global(1.0fx); }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    #[test]
    fn set_global_is_a_void_call_checked_like_a_builtin() {
        ok("sub main() { set_global(1, 2); }");
        let errors = err("sub main() { set_global(1, 2.0fx); }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    // ── 字面量折叠交互：`90deg + 10deg` 判型但不做常量折叠 ─────────────────────────

    #[test]
    fn angle_literal_addition_type_checks_but_stays_a_binary_node_not_folded() {
        let ti = ok("sub main() { var x: angle = 90deg + 10deg; }");
        let e = only_var_init(&ti);
        assert_eq!(e.ty, Ty::Angle);
        assert!(
            matches!(e.kind, TypedExprKind::Binary { .. }),
            "T2 判型层不折叠字面量二元运算，应保持 Binary 节点：{:?}",
            e.kind
        );
    }

    // ── 未定义名字 ──────────────────────────────────────────────────────────────

    #[test]
    fn undefined_variable_is_an_error() {
        let errors = err("sub main() { var x: int = y; }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未定义")),
            "{errors:?}"
        );
    }

    #[test]
    fn undefined_function_call_is_an_error() {
        let errors = err("sub main() { _ = no_such_fn(1); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未定义")),
            "{errors:?}"
        );
    }

    #[test]
    fn undefined_spawn_target_is_an_error() {
        let errors = err("sub main() { spawn no_such_sub(); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未定义")),
            "{errors:?}"
        );
    }

    // ── 变量遮蔽（拒绝，含跨嵌套块 / for 归纳变量 / 局部遮蔽常量允许）──────────────

    #[test]
    fn duplicate_var_name_in_same_sub_is_rejected() {
        err("sub main() { var x: int = 1; var x: int = 2; }");
    }

    #[test]
    fn duplicate_var_name_across_if_branches_is_still_rejected() {
        // pin：不做作用域分析，哪怕两个声明在互斥分支里也拒绝（简单、确定性槽分配）。
        err("sub main() { if 1 { var x: int = 1; } else { var x: int = 2; } }");
    }

    #[test]
    fn for_loop_var_participates_in_duplicate_check() {
        err("sub main() { var i: int = 0; for i in 0..5 { } }");
    }

    #[test]
    fn local_var_shadowing_a_const_name_is_allowed() {
        ok("const N: int = 1; sub main() { var N: int = 2; _ = N; }");
    }

    // ── spawn 参数校验 ──────────────────────────────────────────────────────────

    #[test]
    fn spawn_arity_mismatch_is_an_error() {
        let errors = err("sub helper(a: int) { } sub main() { spawn helper(); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("参数个数")),
            "{errors:?}"
        );
    }

    #[test]
    fn spawn_arg_type_mismatch_is_an_error() {
        let errors = err("sub helper(a: int) { } sub main() { spawn helper(1.0fx); }");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    #[test]
    fn spawn_arity_and_type_match_is_ok_and_recorded_but_not_a_sync_call() {
        let ti = ok("sub helper(a: int) { } sub main() { spawn helper(1); }");
        let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
        assert!(
            main.sync_calls.is_empty(),
            "spawn 不是同步调用边：{:?}",
            main.sync_calls
        );
    }

    // ── sync_calls 记录（供 slots.rs 建图）────────────────────────────────────────

    #[test]
    fn sync_call_records_target_name_deduped() {
        let ti = ok("sub helper() { } sub main() { helper(); helper(); }");
        let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
        assert_eq!(main.sync_calls, vec!["helper".to_string()]);
    }

    // ── fire 的 xf/task 标识符参数解析 ─────────────────────────────────────────

    #[test]
    fn fire_xf_none_and_task_none_is_ok() {
        ok("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, none); }");
    }

    #[test]
    fn fire_xf_known_xformdef_is_ok_and_recorded() {
        let ti = ok(
            "xformdef RING { turn(90deg); } sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, RING, none); }",
        );
        let main = ti.subs.iter().find(|s| s.name == "main").unwrap();
        assert_eq!(main.xform_refs, vec!["RING".to_string()]);
    }

    #[test]
    fn fire_xf_unknown_xformdef_is_an_error() {
        let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, NOPE, none); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未知的 xformdef")),
            "{errors:?}"
        );
    }

    #[test]
    fn fire_task_known_sub_is_ok() {
        ok(
            "sub bullet_task() { } sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, bullet_task); }",
        );
    }

    #[test]
    fn fire_task_unknown_sub_is_an_error() {
        let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, nope); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("未知的 sub")),
            "{errors:?}"
        );
    }

    #[test]
    fn fire_xf_non_identifier_expr_is_an_error() {
        let errors = err("sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, 1, none); }");
        assert!(
            errors.iter().any(|e| e.msg.contains("标识符")),
            "{errors:?}"
        );
    }

    // ── const 折叠 ──────────────────────────────────────────────────────────────

    #[test]
    fn const_literal_folds_directly() {
        let ti = ok("const N: int = 5;");
        assert_eq!(ti.consts, vec![("N".to_string(), Ty::Int, 5)]);
    }

    #[test]
    fn const_arithmetic_combo_folds() {
        let ti = ok("const N: int = 2 + 3 * 4;");
        assert_eq!(ti.consts, vec![("N".to_string(), Ty::Int, 14)]);
    }

    #[test]
    fn const_referencing_earlier_const_folds() {
        let ti = ok("const A: int = 10; const B: int = A + 1;");
        assert_eq!(
            ti.consts,
            vec![
                ("A".to_string(), Ty::Int, 10),
                ("B".to_string(), Ty::Int, 11)
            ]
        );
    }

    #[test]
    fn const_type_mismatch_is_an_error() {
        let errors = err("const N: fx = 1 + 2;");
        assert!(errors.iter().any(|e| e.msg.contains("cast")), "{errors:?}");
    }

    #[test]
    fn const_referencing_undeclared_name_is_an_error() {
        let errors = err("const N: int = M + 1;");
        assert!(
            errors.iter().any(|e| e.msg.contains("不是已声明的常量")),
            "{errors:?}"
        );
    }

    #[test]
    fn const_negative_angle_folds_with_wrapping() {
        let ti = ok("const A: angle = -90deg;");
        // -16384 の wrapping_neg 在 u16 域回绕 = 65536-16384 = 49152；本折叠走 i32
        // wrapping_neg（-16384i32 as raw），消费点低 16 位截断时同样得 49152——这里直接钉
        // i32 raw 值本身（-16384），符合"回绕天然发生在消费点"的设计（见模块文档 cast 段）。
        assert_eq!(ti.consts, vec![("A".to_string(), Ty::Angle, -16384)]);
    }

    // ── break/continue 越界检查（有益增项，超出 checklist 字面范围）───────────────

    #[test]
    fn break_outside_loop_is_an_error() {
        err("sub main() { break; }");
    }

    #[test]
    fn continue_outside_loop_is_an_error() {
        err("sub main() { continue; }");
    }

    #[test]
    fn break_continue_inside_loop_are_ok() {
        ok("sub main() { loop { break; } while 1 { continue; } for i in 0..5 { break; } }");
    }

    // ── 重复定义 ────────────────────────────────────────────────────────────────

    #[test]
    fn duplicate_sub_name_is_an_error() {
        err("sub main() { } sub main() { }");
    }

    #[test]
    fn duplicate_const_name_is_an_error() {
        err("const N: int = 1; const N: int = 2;");
    }
}
