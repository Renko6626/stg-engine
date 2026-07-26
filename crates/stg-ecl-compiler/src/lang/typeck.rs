//! ECL 表层语言类型趟（M1.9 T2）——三型（`int`/`fx`/`angle`）自底向上判型 + 值消费检查 +
//! 二元运算指令意图选择，产出 [`TypedInfo`]（供 `lang::slots` 与未来 T3 codegen 消费的带型
//! 影子 AST）。入口 [`check`]。
//!
//! ## 子模块划分（一类东西一个文件）
//!
//! - `crate::lang::type_rules`（跨趟共享）—— 指令意图标签（`BinIntent`/`UnIntent`/
//!   `CastIntent`）+ 类型矩阵唯一权威：`binary_result`/`op_symbol`/`cast_intent`，判型与
//!   `crate::lang::const_eval` 常量折叠共用同一张表（C14 收编，原 `matrix`/`intents` 两个
//!   typeck 内部子模块已并入）。`expr_span` 同刀迁至 `crate::lang::ast`（AST 遍历助手，非
//!   类型规则）。
//! - [`typed_ast`] —— 带型影子 AST 数据结构（`TypedExpr`/`TypedStmt`/`TypedSub`/…）。
//! - `scope`（内部）—— [`LocalScope`]：块作用域 / definite-assignment 状态机。
//! - `checker`（内部）—— `Checker` 状态 + 错误上报辅助方法。
//! - `consts`（内部）—— `const` 声明的编译期常量折叠。
//! - `exprs`（内部）—— 表达式判型 + 调用解析。
//! - `stmts`（内部）—— 语句判型 + sub 体总入口。
//!
//! `Checker` 定义在 `checker` 子模块，但它的行为（`impl Checker`）按判型阶段分散在
//! `consts`/`exprs`/`stmts` 三个文件里——Rust 允许同一类型的 `impl` 块跨文件出现，只要都在
//! 同一 crate 内；这里用它把"状态"和"每一类判型逻辑"物理分开，不代表 `Checker` 本身被拆成
//! 多个类型。
//!
//! ## 契约出入：`CompileError` 没有 `src_line`
//!
//! `check(&Program) -> Result<TypedInfo, Vec<CompileError>>`——签名只有 `&Program`，没有原始
//! 源码文本（`Program`/`Span` 均不携带它），故本趟构造的 `CompileError` 一律
//! `src_line: String::new()`（不经 `CompileError::at`，直接按公开字段构造）。真正呈现给
//! 用户的完整契约格式（文件:行:列 + 源行摘录 + `^`）由 `lang::mod::compile` 总入口负责——
//! 它手上有 `src`，回填 `src_line` 后再 `render()`。`lang::slots` 同款处理（见该模块文档）。
//!
//! ## 类型规则表（plan 核心接口块钉死；`crate::lang::type_rules::binary_result` 是唯一权威
//! 实现，判型与常量折叠共用）
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
//!   逻辑（求值委托 `crate::lang::const_eval::evaluate`，复用 `type_rules::binary_result`
//!   同一张矩阵）；**不支持** `$` 引擎变量/`global()`/函数调用（非编译期可求值）。
//! - **`xformdef` 序列体不在本趟校验范围**：`ast.rs` 模块文档写"slots 里的 Expr 必须是常量
//!   表达式，T2 折叠校验"，但 Task 2 checklist 的实际测试点只考"被引用才占槽 / 3×cnt 对齐 /
//!   off+cnt 上界"——这些是 `lang::slots` 的槽分配职责，不是判型职责。Global Constraints 原文
//!   "序列折叠为 3 字/槽常量"字面归在 **T3 codegen** 段落（`sub 入口 staging（PUSHI+POPL 序列）`
//!   ——那正是 T3 才知道的字节码细节）。故本趟**不**递归校验 `XformDef.slots[].args` 的常量性/
//!   合法 op 名——留给 T3；本趟只处理 `fire(...)` 的 `xf` 参数标识符解析（是否是已声明
//!   `xformdef` 名或 `none`），见 `exprs::check_builtin_call_args`。此为有意收窄范围的选择，
//!   非疏漏，报告中列为 T3 交接注意事项。

use crate::lang::ast::{Block, CompileError, Program, Span, Stmt, Ty};
use checker::Checker;
use std::collections::{BTreeMap, BTreeSet};
use stg_core::consts::EngineConst;
use stg_core::ecl::image::EclValueType;

mod checker;
mod consts;
mod exprs;
mod scope;
mod stmts;
mod typed_ast;

pub use crate::lang::type_rules::{BinIntent, CastIntent, UnIntent};
pub use typed_ast::{
    CallArg, CallTarget, TypedCall, TypedExpr, TypedExprKind, TypedInfo, TypedStmt, TypedSub,
};

/// 类型趟入口：自底向上判型 + 值消费检查，产出 [`TypedInfo`]（见模块文档）。
///
/// `engine_consts`（C14 Task 3）——引擎常量注册表（见 `stg_core::consts::ENGINE_CONSTS`），
/// 在处理脚本 `const` **之前**预填进 `c.consts`/`c.engine_const_names`，等效于"第 1 行前
/// 声明的 const"：脚本 const 重名会撞进 `check_const_def` 的引擎重名分支。末尾 `TypedInfo.consts`
/// 会把它们一并收进去——供 codegen 的 xformdef 槽参数求值器引用。
///
/// `table`（颜色轴 T3）——绑定的世界表，原样存进 `Checker.table`；`None` = 未绑定，跳过
/// 一切依赖表的判据（当前仅存，尚无判据消费；供 T4 的形/色组合判据使用）。调用方注入的
/// `BULLET_COLOR_STRIDE` 等表派生常量走 `engine_consts`，不经此参——`table` 只用于判型阶段
/// 直接查表的判据，不是常量注入的通道。
pub fn check(
    prog: &Program,
    engine_consts: &[EngineConst],
    table: Option<&stg_core::tables::WorldTables>,
) -> Result<TypedInfo, Vec<CompileError>> {
    let mut c = Checker {
        subs: BTreeMap::new(),
        xformdefs: BTreeSet::new(),
        consts: BTreeMap::new(),
        engine_const_names: BTreeSet::new(),
        errors: Vec::new(),
        cur_sync_calls: Vec::new(),
        cur_xform_refs: Vec::new(),
        table,
    };

    for ec in engine_consts {
        c.consts
            .insert(ec.name.to_string(), (eclty_to_ty(ec.ty), ec.value));
        c.engine_const_names.insert(ec.name.to_string());
    }

    for xf in &prog.xformdefs {
        c.xformdefs.insert(xf.name.clone());
    }

    for s in &prog.subs {
        if c.check_not_reserved_sugar_name(s.span, &s.name) {
            continue;
        }
        if c.subs.contains_key(&s.name) {
            c.err(s.span, format!("sub '{}' 重复定义", s.name));
        } else {
            c.subs.insert(s.name.clone(), s);
        }
    }

    for cdef in &prog.consts {
        c.check_const_def(cdef);
    }

    c.errors.extend(validate_marks(prog, &c.consts));

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

/// `EclValueType`（`stg_core::ecl::image`，引擎常量注册表的脚本侧类型标签）→ 本趟的 [`Ty`]。
/// 两者定义独立（`stg-core` 不依赖本 crate 的 `Ty`），字段一一对应，逐条穷尽匹配（新增变体
/// 编译期报错，防漏）。
fn eclty_to_ty(t: EclValueType) -> Ty {
    match t {
        EclValueType::Int => Ty::Int,
        EclValueType::Fx => Ty::Fx,
        EclValueType::Angle => Ty::Angle,
    }
}

// ── mark 语句结构校验（Task 4；整局流程刀 spec §2）─────────────────────────────────
//
// 独立于 `check_stmt` 的逐语句判型（那里只管求值 id / typecheck 补偿块，见
// `stmts::check_stmt` 的 `Stmt::Mark` 臂文档）——本趟管三条位置/顺序规则 + id 合法性：
// 1. `mark` 只能出现在 `main` 顶层（非 main 任意位置、main 内嵌套块位置皆拒）；
// 2. `main` 顶层的 `var` 声明不得先于 `mark`（任务帧零初始化，中段跳入后局部为 0，先声明
//    的 var 在跳入路径上不会被正常执行到，语义不自洽）；
// 3. `id` 必须是编译期常量、正整数、程序内不重复（`const_eval::evaluate` 求值，`consts`
//    是 `check()` 入口在 sub 登记 + const 折叠之后传入的完整常量表，含引擎常量）。
//
// `check()` 入口在 const 折叠之后、`check_sub` 之前调用，错误并入 `c.errors`。

/// mark 语句结构合法性校验：见上方"mark 语句结构校验"小节。
fn validate_marks(prog: &Program, consts: &BTreeMap<String, (Ty, i32)>) -> Vec<CompileError> {
    let mut errors = Vec::new();

    // 规则 1（前半）：非 main 的任何 sub，出现在任意深度的 mark 一律拒绝。
    for sub in &prog.subs {
        if sub.name == "main" {
            continue;
        }
        let mut found = Vec::new();
        collect_marks_recursive(&sub.body, &mut found);
        for span in found {
            errors.push(mark_err(span, "mark 只能出现在 main 顶层"));
        }
    }

    let Some(main) = prog.subs.iter().find(|s| s.name == "main") else {
        return errors;
    };

    // 规则 1（后半）+ 2 + 3：main 顶层线性扫描。顶层直接出现的 `Stmt::Mark` 走完整 id
    // 合法性校验；任何嵌套在 if/while/for/loop 块或 mark 自己补偿块内的 mark 都算"非顶层"
    // （规则 1 后半），走 `push_nested_mark_errors`。
    let mut var_seen = false;
    let mut seen_ids: BTreeMap<i32, Span> = BTreeMap::new();
    for stmt in &main.body {
        match stmt {
            Stmt::Var { .. } => {
                var_seen = true;
            }
            Stmt::Mark { id, block, span } => {
                if var_seen {
                    errors.push(mark_err(
                        *span,
                        "main 顶层 var 声明不得先于 mark（任务帧零初始化，跳入后局部为 0）",
                    ));
                }
                match crate::lang::const_eval::evaluate(id, consts) {
                    Ok((ty, val)) => {
                        if ty != Ty::Int {
                            errors.push(mark_err(*span, "mark 编号必须是 int 型编译期常量"));
                        } else if val <= 0 {
                            errors.push(mark_err(*span, "mark 编号必须是正整数"));
                        } else if let std::collections::btree_map::Entry::Vacant(e) =
                            seen_ids.entry(val)
                        {
                            e.insert(*span);
                        } else {
                            errors.push(mark_err(*span, format!("mark 编号 {val} 重复")));
                        }
                    }
                    Err(_) => {
                        errors.push(mark_err(*span, "mark 编号必须是编译期常量"));
                    }
                }
                if let Some(b) = block {
                    push_nested_mark_errors(b, &mut errors);
                }
            }
            Stmt::If { then_b, else_b, .. } => {
                push_nested_mark_errors(then_b, &mut errors);
                if let Some(e) = else_b {
                    push_nested_mark_errors(e, &mut errors);
                }
            }
            Stmt::While { body, .. } | Stmt::Loop { body, .. } | Stmt::For { body, .. } => {
                push_nested_mark_errors(body, &mut errors);
            }
            _ => {}
        }
    }

    errors
}

/// 递归收集 `block` 内全部 `Stmt::Mark` 的 span（"mark 只能在 main 顶层语句位（不进
/// if/while/for/loop 块）"错误），逐一并入 `errors`。
fn push_nested_mark_errors(block: &Block, errors: &mut Vec<CompileError>) {
    let mut found = Vec::new();
    collect_marks_recursive(block, &mut found);
    for span in found {
        errors.push(mark_err(
            span,
            "mark 只能在 main 顶层语句位（不进 if/while/for/loop 块）",
        ));
    }
}

/// 递归收集一段语句序列（含任意深度嵌套块，含 mark 自身补偿块）内全部 `Stmt::Mark` 的
/// span——供"非 main"与"main 内嵌套"两种"非法位置"扫描共用。
fn collect_marks_recursive(stmts: &[Stmt], out: &mut Vec<Span>) {
    for s in stmts {
        match s {
            Stmt::Mark { block, span, .. } => {
                out.push(*span);
                if let Some(b) = block {
                    collect_marks_recursive(b, out);
                }
            }
            Stmt::If { then_b, else_b, .. } => {
                collect_marks_recursive(then_b, out);
                if let Some(e) = else_b {
                    collect_marks_recursive(e, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Loop { body, .. } | Stmt::For { body, .. } => {
                collect_marks_recursive(body, out);
            }
            _ => {}
        }
    }
}

fn mark_err(span: Span, msg: impl Into<String>) -> CompileError {
    CompileError {
        line: span.line,
        col: span.col,
        msg: msg.into(),
        src_line: String::new(),
    }
}

#[cfg(test)]
mod tests;
