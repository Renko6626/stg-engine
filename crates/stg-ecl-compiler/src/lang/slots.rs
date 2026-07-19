//! ECL 表层语言槽分配趟（M1.9 T2）——调用图 DFS 着色，给每个 `sub` 分配一段静态、互不
//! 相撞的 `locals` 槽区间，供 T3 codegen 把 `var`/参数/`for` 归纳变量直译成 `PUSHL`/`POPL`
//! 槽号、把 `fire(...)` 引用的 `xformdef` 折算成 locals 内的连续区间。入口 [`allocate`]。
//!
//! ## 契约出入：`CompileError` 没有 `src_line`
//!
//! 同 `lang::typeck` 模块文档——本趟只有 `&Program`/`&TypedInfo`，没有原始源码文本，构造的
//! [`CompileError`] 一律 `src_line: String::new()`；`lang::mod::compile` 总入口负责回填。
//!
//! ## 算法（唯一权威说明，其它地方不重复展开）
//!
//! **每个 `sub` 只有一段静态槽区间**——不按"调用它的入口"分别着色（那会给同一个 sub 产生
//! 多套冲突的槽号）。核心洞察："`sub` 调用不开新窗口，locals 全任务共享"
//! （`docs/ecl-ops.md`），故一个 `sub` 被 `CALL` 进入时，它的槽必须让在**所有**同步调用它
//! 的 caller 已经用掉的槽段之后，不论调用方是谁；`spawn`（含 `fire(..., task, ...)` 的
//! `task` 引用）则相反——它们启动的是**全新任务**，`Task.locals` 是该任务私有、`spawn`
//! 时清零重开的数组（`TaskPool::spawn` 全零初始化，见 `stg-core/src/ecl/task.rs`），跟
//! "谁 spawn 了它"完全无关，故这类边**不参与**同步调用图，天然让被 spawn 的 sub 的候选
//! 基址里总有一个 `0`。
//!
//! 具体算法：
//!
//! 1. **建同步调用图**（有向边 caller → callee，仅取 `TypedSub.sync_calls`——`spawn`/
//!    `fire` 的 `task` 引用不产生边，见上）。
//! 2. **环检测**：DFS 找环（v1 禁递归，直接/间接皆报错，错误文案含完整路径）；找到即
//!    短路返回（环存在时后续宽度/基址计算没有意义）。
//! 3. **每 sub 的"自身宽度"**（与基址无关，独立算）：`params.len() + 去重递归收集到的
//!    var/for 归纳变量个数`（顺序 = 参数声明序 + 源码遍历序，唯一性已由 `lang::typeck` 的
//!    "同一 sub 内变量名不可重复"规则保证，见该模块文档）**加上** `fire(...)` 引用到的每个
//!    xformdef 各自的 `3 × slots.len()`（顺序 = `TypedSub.xform_refs` 的去重源码序）。
//! 4. **基址**（`base_of`，DAG 上的最长路径，用调用图**反向边**做记忆化递归）：
//!    `base(v) = max({ base(caller) + width(caller) : caller 同步调用 v } ∪ {0})`——
//!    集合为空（没有同步 caller，含"仅被 spawn/fire task 引用"或"完全孤立"两种情况）时
//!    自然退化成 `0`；`spawn`/`fire task` 引用天生不出现在这个集合里，故它们对基址计算
//!    **零贡献但也零阻碍**（"新任务根"语义是这条公式的自然推论，不需要额外分支——
//!    见模块文档"为什么不需要显式 entry 集合"）。**健全性依据 = async/同步途径强制分离**
//!    （`lang::typeck` 三腿规则，T2 复审 Critical 修复）：`async sub` 只许被 spawn/fire-task
//!    引用（无同步 caller ⇒ 本公式恒给基址 0，与 `OP_SPAWN` 拷实参进子任务 `locals[0..argc)`
//!    对齐）；普通 `sub` 只许被同步 CALL（基址随链上浮）。"同一 sub 双途径"在类型检查层
//!    即打回，本层永不可见——早期版本注释声称该场景"已验证正确"是**错的**（那正是
//!    实参/参数槽错位的 Critical），以本条为准。
//! 5. **落位**：`sub` 的槽区间 = `[base, base+width)`；区内先参数（声明序）、再变量（源码
//!    遍历序），最后 xformdef 引用区（`TypedSub.xform_refs` 源码序，每个 `(off, cnt)`
//!    对齐 3 字/槽）——这是"xformdef 区放在变量之后"的选定placement（pin，见下）。
//! 6. **容量校验**：`base+width` 必须 `≤ stg_core::ecl::task::LOCALS`（64）；单个引用到的
//!    xformdef 的 `slots.len()` 必须 `≤ 16`（`docs/xform-ops.md` 硬顶）；每个 sub 的求值栈
//!    深上界（表达式树后序模拟 + 调用实参压栈，见 [`body_depth`]）必须
//!    `≤ stg_core::ecl::task::EVAL_DEPTH`（32）。
//!
//! ## Pin：xformdef 区放置在"本 sub 变量之后"，且按**引用它的 sub**分配（不是"入口任务"）
//!
//! 计划的原始措辞（"xformdef regions allocated in the ENTRY task's space"）出自"按入口
//! 分别着色"的旧构想，已被"每 sub 一段全局静态区间"的分辨率取代（见上）。给定这个前提，
//! "入口任务的空间"不再是一个有意义的独立位置——`fire(..., NAME, ...)` 这行代码住在
//! **某个具体的 sub 体内**（不管这个 sub 最终被哪个/哪几个入口经同步调用链间接跑到），
//! T3 的 staging 序列（`sub 入口对每个被引用 xformdef 发 PUSHI+POPL`，
//! `docs/superpowers/plans/2026-07-18-m19-ecl-language.md` Task 3 Commit B）天然就发在**那个
//! sub 自己的入口**——故 xformdef 区归属该 sub 的静态槽区间是唯一自洽的选择：本模块把它
//! 放在该 sub 的 `params+vars` 区之后（"变量之后"，而非"变量之前"或"整个 locals 顶端
//! 倒着分配"——顶端倒着分配会跟 `SubBuilder::repeat` 的计数器槽（同样从 `LOCALS-1` 往下借）
//! 未来撞车风险更高，选"变量之后顺着排"更简单、也更符合"先看得见的用户槽、后看不见的
//! 编译器生成槽"的直觉顺序）。
//!
//! ## 为什么不需要显式 entry 集合
//!
//! task brief 原始描述要求先界定"entry 集合"（spawn 目标 ∪ fire task 引用 ∪ 没有同步
//! caller 的 sub）。展开 `base_of` 的递归定义后可以证明这个集合在数值上是多余的：
//! `max` 的空集合默认值天然是 `0`，而 spawn/fire-task 引用本来就不产生同步调用边——所以
//! "S 是 spawn 目标"这个事实不管 S 是否也有同步 caller 都不会改变 `max` 的结果（`0` 在
//! `max` 里恒被非负的同步基址支配，若同步 caller 集合为空则本来就默认 `0`）。故本实现
//! 干脆不单独构造 entry 集合，直接让公式自己算——`spawn 目标视为新任务根` 是这条公式的
//! 推论，不是需要额外编码的规则（Task 2 checklist 对应测试见本模块测试
//! `spawn_target_reroots_to_base_zero_independent_of_caller`）。

use crate::lang::ast::{CompileError, Program, Span};
use crate::lang::typeck::{CallArg, TypedExpr, TypedExprKind, TypedInfo, TypedStmt};
use std::collections::BTreeMap;
use stg_core::ecl::task::{EVAL_DEPTH, LOCALS};

/// 一个 `sub` 的槽分配结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubSlots {
    /// 本 sub 静态槽区间的起点（含）。
    pub base: usize,
    /// 参数 / `var` / `for` 归纳变量名 → 绝对槽号（直接可用作 `PUSHL`/`POPL` 操作数，
    /// 不需要调用方再加 `base`）。
    pub locals: BTreeMap<String, usize>,
    /// `fire(...)` 引用到的 xformdef 名 → `(off, cnt)`（`off` 绝对槽号，`cnt` = 该 xformdef
    /// 的 op 槽数；3 字/槽，区间宽度 = `cnt*3`）。未被任何 `fire` 引用的 xformdef 不出现
    /// （也不出现在任何 sub 里）。
    pub xform_regions: BTreeMap<String, (usize, usize)>,
    /// 本 sub 槽区间总宽度（`locals.len() + Σ cnt*3`）——`base+width` = 下一个可用槽。
    pub width: usize,
    /// 本 sub 求值栈深上界（表达式树后序模拟 + 调用实参压栈峰值，见 [`body_depth`]）。
    pub max_stack: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMap {
    /// sub 名 → 槽分配结果（与 `TypedInfo.subs` 同名字集合）。
    pub subs: BTreeMap<String, SubSlots>,
}

fn err(span: Span, msg: String) -> CompileError {
    CompileError {
        line: span.line,
        col: span.col,
        msg,
        src_line: String::new(),
    }
}

// ── 环检测（DFS，v1 禁递归；直接/间接皆报错，错误文案含完整路径）───────────────────

fn detect_cycle(ti: &TypedInfo) -> Result<(), Vec<String>> {
    let edges: BTreeMap<String, Vec<String>> = ti
        .subs
        .iter()
        .map(|s| (s.name.clone(), s.sync_calls.clone()))
        .collect();
    let mut state: BTreeMap<String, u8> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    for sub in &ti.subs {
        if state.get(&sub.name).copied().unwrap_or(0) == 0 {
            visit_cycle(&sub.name, &edges, &mut state, &mut stack)?;
        }
    }
    Ok(())
}

/// 0=未访问 / 1=在当前 DFS 栈上（灰）/ 2=已完成（黑）——经典三色标记找环。
fn visit_cycle(
    name: &str,
    edges: &BTreeMap<String, Vec<String>>,
    state: &mut BTreeMap<String, u8>,
    stack: &mut Vec<String>,
) -> Result<(), Vec<String>> {
    match state.get(name).copied().unwrap_or(0) {
        1 => {
            let start = stack.iter().position(|n| n == name).unwrap_or(0);
            let mut path: Vec<String> = stack[start..].to_vec();
            path.push(name.to_string());
            return Err(path);
        }
        2 => return Ok(()),
        _ => {}
    }
    state.insert(name.to_string(), 1);
    stack.push(name.to_string());
    if let Some(callees) = edges.get(name) {
        for c in callees {
            visit_cycle(c, edges, state, stack)?;
        }
    }
    stack.pop();
    state.insert(name.to_string(), 2);
    Ok(())
}

// ── 基址（DAG 最长路径，记忆化递归；见模块文档算法第 4 步）─────────────────────────

fn base_of(
    name: &str,
    callers_of: &BTreeMap<String, Vec<String>>,
    widths: &BTreeMap<String, usize>,
    memo: &mut BTreeMap<String, usize>,
) -> usize {
    if let Some(&b) = memo.get(name) {
        return b;
    }
    let mut best = 0usize;
    if let Some(callers) = callers_of.get(name) {
        for caller in callers {
            let cb = base_of(caller, callers_of, widths, memo);
            let cw = *widths.get(caller).unwrap_or(&0);
            best = best.max(cb + cw);
        }
    }
    memo.insert(name.to_string(), best);
    best
}

// ── 每 sub 的局部名序（参数 + 递归收集的 var/for 归纳变量，源码序）─────────────────

fn collect_locals(body: &[TypedStmt], out: &mut Vec<String>) {
    for s in body {
        match s {
            TypedStmt::Var { name, .. } => out.push(name.clone()),
            TypedStmt::For { var, body, .. } => {
                out.push(var.clone());
                collect_locals(body, out);
            }
            TypedStmt::If { then_b, else_b, .. } => {
                collect_locals(then_b, out);
                if let Some(e) = else_b {
                    collect_locals(e, out);
                }
            }
            TypedStmt::While { body, .. } | TypedStmt::Loop { body } => collect_locals(body, out),
            TypedStmt::Assign { .. }
            | TypedStmt::Wait { .. }
            | TypedStmt::Spawn { .. }
            | TypedStmt::ExprStmtDiscard { .. }
            | TypedStmt::ExprStmtVoid { .. }
            | TypedStmt::Return
            | TypedStmt::Break
            | TypedStmt::Continue => {}
        }
    }
}

// ── 求值栈深（表达式树后序模拟 + 调用实参压栈峰值）─────────────────────────────────
//
// 静态上界，刻意保守（宁可拒绝极端但罕见的深表达式，也不能在 T3 尚未定案的降低模板下
// 少算——少算会让"编译通过"却在运行期真撞 `Fault(2)`，那样静态检查就失去意义）。

fn expr_depth(e: &TypedExpr) -> usize {
    match &e.kind {
        TypedExprKind::IntLit(_)
        | TypedExprKind::FxLit(_)
        | TypedExprKind::AngleLit(_)
        | TypedExprKind::ConstRef(_)
        | TypedExprKind::LocalRef(_)
        | TypedExprKind::EngineVar(_) => 1,
        TypedExprKind::Call(call) => call_args_depth(&call.args).max(1),
        TypedExprKind::Binary { l, r, .. } => expr_depth(l).max(1 + expr_depth(r)),
        TypedExprKind::Unary { e, .. } => expr_depth(e),
        TypedExprKind::Cast { e, intent } => {
            let inner = expr_depth(e);
            match intent {
                crate::lang::typeck::CastIntent::Bitcast => inner,
                // `int as fx`/`fx as int`：`PUSHI 65536` 之后瞬时驻留 2（cast 结果 + 常量），
                // 见 typeck 模块文档。
                _ => inner.max(2),
            }
        }
    }
}

/// 调用实参的求值峰值：逐位累加"已驻留字数" + 当前位自身峰值。`XformRef`/`SubRef` 不是
/// 求值表达式，但仍会各自发出若干条 `PUSHI` 立即数（`fire` 的 `xf` 展开成
/// `(off, cnt)` 两个立即数，`task` 展开成 1 个脚本号立即数——见 Global Constraints
/// "fire(..., NAME, ...) 处自动填 (off, cnt)"），故按"1 个立即数 = 1 字深"计入，
/// `XformRef` 计 2、`SubRef` 计 1——这是对 T3（尚未实现）降低形状的合理预判，T3 落地时若
/// 形状有出入需回头校正本函数，见本刀报告"contract notes for T3"。
fn call_args_depth(args: &[CallArg]) -> usize {
    let mut peak = 0usize;
    let mut resident = 0usize;
    for a in args {
        let (d, words) = match a {
            CallArg::Val(t) => (expr_depth(t), 1),
            CallArg::XformRef(_) => (2, 2),
            CallArg::SubRef(_) => (1, 1),
        };
        peak = peak.max(resident + d);
        resident += words;
    }
    peak
}

fn stmt_depth(s: &TypedStmt) -> usize {
    match s {
        TypedStmt::Var { init, .. } => expr_depth(init),
        TypedStmt::Assign { value, .. } => expr_depth(value),
        TypedStmt::If {
            cond,
            then_b,
            else_b,
        } => {
            let c = expr_depth(cond);
            let t = body_depth(then_b);
            let e = else_b.as_ref().map(|b| body_depth(b)).unwrap_or(0);
            c.max(t).max(e)
        }
        TypedStmt::While { cond, body } => expr_depth(cond).max(body_depth(body)),
        TypedStmt::Loop { body } => body_depth(body),
        TypedStmt::For { from, to, body, .. } => {
            // 保守估计：循环体内比较序列形如 `PUSHL var; <to 表达式>; LT`——`var` 已驻留 1，
            // 再叠 `to` 自身峰值；`from` 只在入口求值一次，独立取 max 即可。
            expr_depth(from)
                .max(1 + expr_depth(to))
                .max(body_depth(body))
        }
        TypedStmt::Wait { frames } => expr_depth(frames),
        // `SPAWN` 恒压 1 个任务号（T3 自动补 POP，见模块文档"为什么不需要 discard 语法"），
        // 故与"用作值的调用"同一峰值口径：`max(参数峰值, 1)`。
        TypedStmt::Spawn { call } => call_args_depth(&call.args).max(1),
        TypedStmt::ExprStmtDiscard { expr } => expr_depth(expr),
        // 无返回值：dispatch 之后不残留任何值，不需要 `max(..,1)`。
        TypedStmt::ExprStmtVoid { call } => call_args_depth(&call.args),
        TypedStmt::Return | TypedStmt::Break | TypedStmt::Continue => 0,
    }
}

fn body_depth(body: &[TypedStmt]) -> usize {
    body.iter().map(stmt_depth).max().unwrap_or(0)
}

/// 槽分配趟入口：调用图 DFS 着色 + 递归环检测 + 容量静态校验，产出 [`SlotMap`]（见模块
/// 文档"算法"）。
pub fn allocate(prog: &Program, ti: &TypedInfo) -> Result<SlotMap, Vec<CompileError>> {
    // 区宽按**物理**槽数计（STEP 族双槽含引擎 scratch，`lang::xform_map` 单一权威）；
    // 未知 op 名在此趟报错（早于 codegen——sizing 正确性依赖名字可解析）。
    let mut xform_name_errors = Vec::new();
    let xformdef_len: BTreeMap<String, (usize, Span)> = prog
        .xformdefs
        .iter()
        .map(|x| {
            for s in &x.slots {
                if crate::lang::xform_map::lookup(&s.op_name).is_none() {
                    xform_name_errors
                        .push(err(s.span, format!("未知的 xform 操作名 '{}'", s.op_name)));
                }
            }
            (
                x.name.clone(),
                (crate::lang::xform_map::physical_len(&x.slots), x.span),
            )
        })
        .collect();
    let sub_span: BTreeMap<String, Span> =
        prog.subs.iter().map(|s| (s.name.clone(), s.span)).collect();

    if let Err(path) = detect_cycle(ti) {
        let span = path
            .first()
            .and_then(|n| sub_span.get(n).copied())
            .unwrap_or_default();
        let msg = format!("检测到递归调用环（v1 禁递归）：{}", path.join(" -> "));
        return Err(vec![err(span, msg)]);
    }

    // 同步调用链深静态检查（M1.9 终审 Important——让 ecl-lang.md"调用深 ≤8 编译期检查"
    // 成为真话）：链上每次 CALL 压一层调用栈，链的**边数**即运行时 csp 峰值，超
    // `CALL_DEPTH`(8) 原本要到运行期才 FAULT_CALL_DEPTH——现在编译期拒绝并给出最深链。
    {
        use stg_core::ecl::task::CALL_DEPTH;
        let edges: BTreeMap<&str, &[String]> = ti
            .subs
            .iter()
            .map(|s| (s.name.as_str(), s.sync_calls.as_slice()))
            .collect();
        fn depth_of<'a>(
            name: &'a str,
            edges: &BTreeMap<&'a str, &'a [String]>,
            memo: &mut BTreeMap<&'a str, usize>,
        ) -> usize {
            if let Some(&d) = memo.get(name) {
                return d;
            }
            let d = edges
                .get(name)
                .map(|cs| {
                    cs.iter()
                        .map(|c| {
                            if edges.contains_key(c.as_str()) {
                                1 + depth_of(c.as_str(), edges, memo)
                            } else {
                                0
                            }
                        })
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            memo.insert(name, d);
            d
        }
        let mut memo: BTreeMap<&str, usize> = BTreeMap::new();
        for sub in &ti.subs {
            let d = depth_of(sub.name.as_str(), &edges, &mut memo);
            if d > CALL_DEPTH {
                let span = sub_span.get(&sub.name).copied().unwrap_or_default();
                return Err(vec![err(
                    span,
                    format!(
                        "从 '{}' 起的同步调用链深 {d} 超出调用栈上限 {CALL_DEPTH}\
                         （每层 CALL 压一层栈；请拆平调用链或改 spawn）",
                        sub.name
                    ),
                )]);
            }
        }
    }

    let mut errors = xform_name_errors;

    // 每 sub 的局部名序 + xformdef 引用区宽度（与 base 无关，独立算）。
    struct Layout {
        locals_order: Vec<String>,
        xform_order: Vec<(String, usize)>, // (xformdef 名, cnt)
        width: usize,
    }
    let mut layouts: BTreeMap<String, Layout> = BTreeMap::new();
    for sub in &ti.subs {
        let mut names: Vec<String> = sub.params.iter().map(|(n, _)| n.clone()).collect();
        let mut var_names = Vec::new();
        collect_locals(&sub.body, &mut var_names);
        names.extend(var_names);

        let mut xform_order = Vec::new();
        let mut xform_width = 0usize;
        for xf_name in &sub.xform_refs {
            let (cnt, xf_span) = xformdef_len
                .get(xf_name)
                .copied()
                .unwrap_or((0, Span::default()));
            if cnt > 16 {
                errors.push(err(
                    xf_span,
                    format!(
                        "xformdef '{xf_name}' 物理槽数 {cnt} 超出上限 16 槽（STEP 族每条占 2 槽）"
                    ),
                ));
            }
            xform_order.push((xf_name.clone(), cnt));
            xform_width += cnt * 3;
        }

        let width = names.len() + xform_width;
        layouts.insert(
            sub.name.clone(),
            Layout {
                locals_order: names,
                xform_order,
                width,
            },
        );
    }

    // 同步调用反向图 + 记忆化最长路径基址。
    let mut callers_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sub in &ti.subs {
        for callee in &sub.sync_calls {
            callers_of
                .entry(callee.clone())
                .or_default()
                .push(sub.name.clone());
        }
    }
    let widths: BTreeMap<String, usize> =
        layouts.iter().map(|(k, v)| (k.clone(), v.width)).collect();
    let mut base_memo: BTreeMap<String, usize> = BTreeMap::new();
    for sub in &ti.subs {
        base_of(&sub.name, &callers_of, &widths, &mut base_memo);
    }

    // 落位 + 容量校验。
    let mut subs_out: BTreeMap<String, SubSlots> = BTreeMap::new();
    for sub in &ti.subs {
        let base = base_memo.get(&sub.name).copied().unwrap_or(0);
        let layout = &layouts[&sub.name];

        let mut locals = BTreeMap::new();
        for (i, name) in layout.locals_order.iter().enumerate() {
            locals.insert(name.clone(), base + i);
        }

        let mut xform_regions = BTreeMap::new();
        let mut off = base + layout.locals_order.len();
        for (name, cnt) in &layout.xform_order {
            xform_regions.insert(name.clone(), (off, *cnt));
            off += cnt * 3;
        }

        let end = base + layout.width;
        if end > LOCALS {
            let span = sub_span.get(&sub.name).copied().unwrap_or_default();
            errors.push(err(
                span,
                format!(
                    "sub '{}' 的 locals 总量 {end} 超出上限 {LOCALS}（base={base}，宽度={}）",
                    sub.name, layout.width
                ),
            ));
        }

        let max_stack = body_depth(&sub.body);
        if max_stack > EVAL_DEPTH {
            let span = sub_span.get(&sub.name).copied().unwrap_or_default();
            errors.push(err(
                span,
                format!(
                    "sub '{}' 的求值栈深 {max_stack} 超出上限 {EVAL_DEPTH}",
                    sub.name
                ),
            ));
        }

        subs_out.insert(
            sub.name.clone(),
            SubSlots {
                base,
                locals,
                xform_regions,
                width: layout.width,
                max_stack,
            },
        );
    }

    if errors.is_empty() {
        Ok(SlotMap { subs: subs_out })
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::{parse as parse_program, typeck};

    fn build(src: &str) -> (Program, TypedInfo) {
        let prog = parse_program(src, "t.ecl").unwrap_or_else(|e| panic!("解析失败：{e:?}\n{src}"));
        let ti = typeck::check(&prog).unwrap_or_else(|e| panic!("判型失败：{e:?}\n{src}"));
        (prog, ti)
    }

    fn ok(src: &str) -> SlotMap {
        let (prog, ti) = build(src);
        allocate(&prog, &ti).unwrap_or_else(|e| panic!("槽分配失败：{e:?}\n{src}"))
    }

    fn err_of(src: &str) -> Vec<CompileError> {
        let (prog, ti) = build(src);
        allocate(&prog, &ti).expect_err(&format!("期望槽分配失败，源码：\n{src}"))
    }

    // ── A 调 B：不相交 ──────────────────────────────────────────────────────────

    #[test]
    fn a_calls_b_disjoint_ranges_b_starts_at_a_end() {
        let sm = ok("sub b() { var y: int = 1; } sub a() { var x: int = 1; b(); }");
        let a = &sm.subs["a"];
        let b = &sm.subs["b"];
        assert_eq!(a.base, 0);
        assert_eq!(a.width, 1);
        assert_eq!(
            b.base,
            a.base + a.width,
            "B 的槽起点 ≥（此处恰等于）A 的槽用量"
        );
        assert_eq!(b.locals["y"], b.base);
    }

    // ── 姊妹 sub 槽可重叠 ───────────────────────────────────────────────────────

    #[test]
    fn sibling_subs_may_overlap() {
        // b、c 互不调用、也互不被同一父 sync-call，各自槽起点独立回落到 0（重叠）。
        let sm = ok("sub b() { var x: int = 1; } sub c() { var y: int = 1; }");
        assert_eq!(sm.subs["b"].base, 0);
        assert_eq!(sm.subs["c"].base, 0);
    }

    // ── 递归环 ──────────────────────────────────────────────────────────────────

    #[test]
    fn direct_recursion_is_rejected_with_path() {
        let errors = err_of("sub a() { a(); }");
        assert!(errors[0].msg.contains('a'), "{errors:?}");
        assert!(errors[0].msg.contains("递归"), "{errors:?}");
    }

    #[test]
    fn indirect_recursion_a_b_a_is_rejected_with_full_path() {
        let errors = err_of("sub a() { b(); } sub b() { a(); }");
        let msg = &errors[0].msg;
        assert!(msg.contains('a') && msg.contains('b'), "{msg}");
    }

    // ── spawn 目标视为新任务根 ──────────────────────────────────────────────────

    #[test]
    fn spawn_target_reroots_to_base_zero_independent_of_caller() {
        let sm = ok("async sub b() { var y: int = 1; } \
             sub a() { var x0: int=0; var x1: int=0; var x2: int=0; var x3: int=0; spawn b(); }");
        assert!(sm.subs["a"].width >= 4, "a 应该有非零宽度撑开对照");
        assert_eq!(
            sm.subs["b"].base, 0,
            "spawn 目标应重新从 0 着色，不受 caller 约束"
        );
    }

    /// fire 的 `task` 引用同理是"新任务根"，不产生同步调用边。
    #[test]
    fn fire_task_ref_also_reroots_to_base_zero() {
        let sm = ok("async sub on_hit() { var y: int = 1; } \
             sub main() { var x0:int=0; var x1:int=0; var x2:int=0; var x3:int=0; \
                          _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, on_hit); }");
        assert!(sm.subs["main"].width >= 4);
        assert_eq!(sm.subs["on_hit"].base, 0);
    }

    // ── 菱形：D 被 B 和 C 共同调用，取 MAX ─────────────────────────────────────

    #[test]
    fn diamond_callee_gets_max_of_both_callers_end() {
        let sm = ok("sub d() { var dv: int = 1; } \
             sub b() { var b0: int=0; d(); } \
             sub c() { var c0:int=0; var c1:int=0; var c2:int=0; d(); } \
             sub a() { b(); c(); }");
        let a = &sm.subs["a"];
        let b = &sm.subs["b"];
        let c = &sm.subs["c"];
        let d = &sm.subs["d"];
        // b、c 是姊妹（同被 a 同步调用），基址相同——即便各自宽度不同。
        assert_eq!(b.base, a.base + a.width);
        assert_eq!(c.base, a.base + a.width, "姊妹 sub 基址相同（可重叠）");
        assert_eq!(b.width, 1);
        assert_eq!(c.width, 3);
        let expect_d_base = (b.base + b.width).max(c.base + c.width);
        assert_eq!(d.base, expect_d_base, "D 应取两个 caller 端点的 MAX");
        assert_eq!(
            d.base,
            c.base + c.width,
            "本例中 c 分支更宽，MAX 应由 c 分支决定"
        );
    }

    // ── 容量：locals > 64 ───────────────────────────────────────────────────────

    #[test]
    fn locals_over_64_is_rejected() {
        let mut src = String::from("sub main() { ");
        for i in 0..65 {
            src.push_str(&format!("var v{i}: int = 0; "));
        }
        src.push('}');
        let errors = err_of(&src);
        assert!(
            errors.iter().any(|e| e.msg.contains("locals 总量")),
            "{errors:?}"
        );
    }

    #[test]
    fn locals_at_exactly_64_is_accepted() {
        let mut src = String::from("sub main() { ");
        for i in 0..64 {
            src.push_str(&format!("var v{i}: int = 0; "));
        }
        src.push('}');
        let sm = ok(&src);
        assert_eq!(sm.subs["main"].width, 64);
    }

    // ── 容量：求值栈深 > 32 ─────────────────────────────────────────────────────

    #[test]
    fn eval_stack_depth_over_32_is_rejected() {
        // 33 个 int 参数的调用：call_args_depth = 33（每位驻留 1 字，峰值随位数线性增长）。
        let params: Vec<String> = (0..33).map(|i| format!("p{i}: int")).collect();
        let args: Vec<String> = (0..33).map(|_| "1".to_string()).collect();
        let src = format!(
            "sub helper({}) {{ }} sub main() {{ helper({}); }}",
            params.join(", "),
            args.join(", ")
        );
        let errors = err_of(&src);
        assert!(
            errors.iter().any(|e| e.msg.contains("求值栈深")),
            "{errors:?}"
        );
    }

    #[test]
    fn eval_stack_depth_at_exactly_32_is_accepted() {
        let params: Vec<String> = (0..32).map(|i| format!("p{i}: int")).collect();
        let args: Vec<String> = (0..32).map(|_| "1".to_string()).collect();
        let src = format!(
            "sub helper({}) {{ }} sub main() {{ helper({}); }}",
            params.join(", "),
            args.join(", ")
        );
        let sm = ok(&src);
        assert_eq!(sm.subs["main"].max_stack, 32);
    }

    /// checklist 字面构造："33 深表达式"（右结合嵌套加法，非调用实参）——`1+(1+(1+...+1))`
    /// 32 层嵌套，`expr_depth` 每层 +1，峰值 33 > 32。与上面按调用实参构造的版本是同一条
    /// 静态检查的两种独立触发路径，都必须报错。
    #[test]
    fn eval_stack_depth_over_32_via_nested_expression_is_rejected() {
        let mut expr = "1".to_string();
        for _ in 0..32 {
            expr = format!("1+({expr})");
        }
        let src = format!("sub main() {{ var x: int = {expr}; }}");
        let errors = err_of(&src);
        assert!(
            errors.iter().any(|e| e.msg.contains("求值栈深")),
            "{errors:?}"
        );
    }

    // ── xformdef 区分配 ────────────────────────────────────────────────────────

    #[test]
    fn unreferenced_xformdef_is_absent_from_every_sub() {
        let sm = ok("xformdef RING { turn(90deg); } sub main() { }");
        assert!(sm.subs["main"].xform_regions.is_empty());
    }

    #[test]
    fn referenced_xformdef_is_allocated_after_vars_with_3x_alignment() {
        let sm = ok("xformdef RING { turn(90deg); set_ang_vel(128); } \
             sub main() { var x: int = 1; _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, RING, none); }");
        let main = &sm.subs["main"];
        assert_eq!(main.locals["x"], 0, "变量先占槽");
        let (off, cnt) = main.xform_regions["RING"];
        assert_eq!(cnt, 2, "RING 有 2 条 op");
        assert_eq!(
            off,
            main.base + main.locals.len(),
            "xformdef 区紧跟在变量之后"
        );
        assert_eq!(main.width, main.locals.len() + cnt * 3, "3×cnt 对齐");
    }

    /// 同步调用链深编译期检查（M1.9 终审 Important 修法）：8 边链恰过、9 边链拒——
    /// 与运行时 CALL_DEPTH=8 的 csp 峰值语义严格对齐（链边数 = csp 峰值）。
    #[test]
    fn sync_call_chain_depth_eight_ok_nine_rejected() {
        let chain = |n: usize| -> String {
            let mut s = String::new();
            for i in 0..n {
                if i + 1 < n {
                    s.push_str(&format!("sub s{i}() {{ s{}(); }} ", i + 1));
                } else {
                    s.push_str(&format!("sub s{i}() {{ }} "));
                }
            }
            s
        };
        // 9 个 sub = 8 条边：恰在上限内。
        let _ = ok(&chain(9));
        // 10 个 sub = 9 条边：编译期拒绝。
        let errors = err_of(&chain(10));
        assert!(
            errors.iter().any(|e| e.msg.contains("调用链深")),
            "{errors:?}"
        );
    }

    /// STEP 族物理双槽计宽（T3 复审 Important 修法的 sizing 腿）：
    /// authored 2 条（step_speed + turn）→ 物理 3 槽（scratch 计入区宽）。
    #[test]
    fn step_op_counts_two_physical_slots_in_region_width() {
        let sm = ok("xformdef S { step_speed(2.0fx, 4); turn(90deg); } \
             sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, S, none); }");
        let main = &sm.subs["main"];
        let (_off, cnt) = main.xform_regions["S"];
        assert_eq!(cnt, 3, "step_speed 物理 2 槽 + turn 1 槽");
        assert_eq!(
            main.width,
            main.locals.len() + 3 * 3,
            "区宽按物理槽数 ×3 字"
        );
    }

    /// 未知 xform 操作名在 slots 趟即报错（sizing 单一权威所在层）。
    #[test]
    fn unknown_xform_op_name_errors_in_slots_pass() {
        let errors = err_of(
            "xformdef S { frobnicate(1); } \
             sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, S, none); }",
        );
        assert!(
            errors.iter().any(|e| e.msg.contains("未知的 xform 操作名")),
            "{errors:?}"
        );
    }

    #[test]
    fn xformdef_over_16_slots_is_rejected() {
        let mut xf_body = String::new();
        for _ in 0..17 {
            xf_body.push_str("turn(1deg); ");
        }
        let src = format!(
            "xformdef BIG {{ {xf_body} }} sub main() {{ _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, BIG, none); }}"
        );
        let errors = err_of(&src);
        assert!(
            errors.iter().any(|e| e.msg.contains("超出上限 16 槽")),
            "{errors:?}"
        );
    }

    // ── 端到端：菱形 + xformdef + spawn 混合场景下的整体 sanity（大杂烩回归）───────

    #[test]
    fn mixed_scenario_all_subs_get_a_slot_map_entry() {
        let sm = ok("xformdef RING { turn(90deg); } \
             async sub patrol() { loop { wait(1); } } \
             async sub timer_ui(spell: int) { var t: int = 600; } \
             sub main() { spawn patrol(); spawn timer_ui(1); \
                          var base: angle = 0deg; \
                          _ = fire(0, 0fx, 0fx, 1.0fx, base, RING, none); }");
        for name in ["patrol", "timer_ui", "main"] {
            assert!(sm.subs.contains_key(name), "缺少 sub '{name}' 的槽分配");
        }
    }
}
