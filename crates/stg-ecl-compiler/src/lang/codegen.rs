//! ECL 表层语言 codegen 趟（M1.9 T3 Commit B）——把类型趟/槽分配趟的产物
//! （[`TypedInfo`]/[`SlotMap`]）连同原始 AST（[`Program`]，codegen 需要它拿
//! `xformdef` 的原始 `Expr` 序列做编译期常量折叠——T2 明确把这块留给本趟，见
//! `lang::typeck`/`lang::slots` 模块文档"xformdef 序列体不在本趟校验范围"）一起
//! 降低为 [`EclImage`]。入口 [`generate`]。
//!
//! ## 后端：复用 T3 前置刀（`ImageBuilder`/`SubBuilder`）的回填器，不另起炉灶
//!
//! `SubBuilder` 既有的结构化糖（`if_ge`/`loop_forever`/`repeat`）各自钉死一种控制流
//! 形状，装不下表层语言的 if/else 双分支、while 顶测、for 计数器、`&&`/`||` 短路——
//! 故给 `SubBuilder` 加了一层 `pub(crate)` raw 原语（`raw_jz`/`raw_jmp`/`here`/`patch`/
//! `raw_wait`/`raw_ret`/`raw_emit_op`，见 `lib.rs`），本模块用它们手搭全部降低模板，
//! 复用的是同一套 `jump_fixups` 回填机制（sub 内本地 target，`build()` 时随 sub 基址
//! 整体平移）——不是绕开既有机制另建一条路。
//!
//! ## sub 名 → `ScriptId`：声明序，一次性算好（不依赖 `ImageBuilder::add_sub` 的调用序）
//!
//! `call`/`spawn`/`fire` 的 `task` 引用都需要在**生成目标 sub 自己的字节码之前**就知道
//! 引用者的 `ScriptId`（互相调用是常态，A 调 B 时 B 可能还没生成）。`ImageBuilder::add_sub`
//! 的 `ScriptId` 分配规则是"调用序"（`self.subs.len()`），故只要本模块按 `Program.subs`
//! 的**声明序**依次生成每个 sub 并依次 `add_sub`，declaration-order 与 add-order 天然重合
//! ——`generate` 因此可以在生成任何 sub 体之前，先用纯声明序枚举出完整的
//! `name → ScriptId` 映射表，全程只读，不需要"先占位后回填"的两趟。
//!
//! ## call-style vs entry-style：`return;` 降低成 `OP_RET` 还是 `OP_END`
//!
//! `sub` 分两种运行形态（`lang::typeck` 的 async/同步途径强制分离已经保证一个 sub
//! 只属于其中一种，不会两者都是）：
//! - **call-style**（被至少一个别的 sub 同步 `CALL` 过，`TypedSub.sync_calls` 反向图非空）：
//!   必须以 `OP_RET` 收尾——调用方靠它把执行权弹回调用点。
//! - **entry-style**（从未被同步 `CALL` 过——`main`、`async sub`、只被 `spawn`/
//!   `fire(...,task)` 引用的 sub）：以 `OP_END` 收尾——它是任务的"整段协程体"，没有
//!   调用方等它弹回，`OP_RET`（`csp==0` 时）反而会 `Fault(FAULT_CALL_DEPTH)`。
//!
//! 判定用一次性算好的 `call_style: BTreeMap<String, bool>`（从 `TypedInfo.subs` 的
//! `sync_calls` 反向汇总——被任何一个 sub 同步调用过就是 call-style），`return;`
//! 语句与"函数体自然落空"两条路径都查这张表决定发 `OP_RET` 还是 `OP_END`。
//!
//! ## `&&`/`||` 短路：JZ + 常量臂（spec 降低模板）
//!
//! 两个操作数都规约为**精确的 `0`/`1`**（不是"随便一个非零值"）——`a && b`：
//! `emit(a); JZ L_false; emit(b); JZ L_false; PUSHI 1; JMP L_end; L_false: PUSHI 0; L_end:`；
//! `a || b` 镜像（先判 `a` 真即短路为 1，否则落到判 `b`）。两个分支各自落地字面量
//! `0`/`1`，不依赖操作数本身恰好就是 `0`/`1`（`int` 型只保证非零即真）。
//!
//! ## xformdef 参数常量折叠：T2 留给本趟的范围
//!
//! `XfSlotLit.args` 允许字面量 + 已声明 `const` 引用 + 一元 `-`（[`eval_const_arg`]，
//! 独立于 `lang::typeck` 的 `fold_const`——那个吃的是 `Checker` 内部状态，本趟只有
//! `TypedInfo.consts`（已折叠完的名字→值表），够用）；其它任何形状（`$` 变量、调用、
//! 二元运算……）一律编译错误，文案含"xformdef 参数必须是编译期常量"。
//!
//! ## 弹 setter 族 `handle:int` 首参：求值后丢弃（T3 落地拍板）
//!
//! `lang::builtins` 报告点名的悬而未决项——`set_speed`/`set_angle`/`turn`/`set_vel`/
//! `set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player` 九个 setter 的首参
//! `handle:int`，底层 syscall 实际操作的是 `self`（任务 owner），并不消费一个显式句柄。
//! 本趟落地策略：**求值后立即丢弃**（保留副作用——万一作者写了带副作用的表达式——但
//! 不把它压进 syscall 实际吃的参数序列），不是"Fault if handle != self"（更严格但当前
//! 无法在编译期证明等值，运行期也没有校验入口）——最小惊讶、不新增运行期检查。
use crate::lang::ast::{BinOp, CompileError, Expr, Program, Span, UnOp};
use crate::lang::builtins::{self, ParamKind};
use crate::lang::slots::{SlotMap, SubSlots};
use crate::lang::typeck::{
    BinIntent, CallArg, CallTarget, CastIntent, TypedCall, TypedExpr, TypedExprKind, TypedInfo,
    TypedStmt, TypedSub, UnIntent,
};
use crate::{ImageBuilder, ScriptId, SubBuilder};
use std::collections::BTreeMap;
use stg_core::ecl::image::EclImage;
use stg_core::xform::XformSlot;

// xformdef 操作名映射表已上移 `lang::xform_map`（slots 趟与本趟共用的单一权威，含物理
// 槽数与 STEP 族 scratch 语义）；`loop`/`END` 不开放的已知限制也记录在该模块文档。

/// `xformdef` 槽参数的编译期常量求值：字面量 + 已声明 `const` 引用 + 一元 `-`，
/// 其它一律拒绝（模块文档"xformdef 参数常量折叠"）。
fn eval_const_arg(e: &Expr, consts: &BTreeMap<String, i32>) -> Result<i32, String> {
    match e {
        Expr::IntLit(v) => Ok(*v),
        Expr::FxLit(v) => Ok(*v),
        Expr::AngleLit(v) => Ok(*v as i32),
        Expr::Var(name, _) => consts
            .get(name)
            .copied()
            .ok_or_else(|| format!("'{name}' 不是已声明的常量")),
        Expr::Unary {
            op: UnOp::Neg,
            e: inner,
            ..
        } => eval_const_arg(inner, consts).map(i32::wrapping_neg),
        _ => Err("只支持字面量 / const 引用 / 一元负号".to_string()),
    }
}

/// locals/xform 槽号窄化为 `u8`（`PUSHL`/`POPL`/`write_xform_locals` 操作数宽度，C19
/// 复审修复）。`lang::slots::allocate` 已经在编译期把 `base+width ≤ 64`
/// （`stg_core::ecl::task::LOCALS`）钉死（`slots.rs::locals_over_64_is_rejected` 单测
/// 覆盖），正常 `.ecl` 源码走不到这条断言——这里是给"假如分配器自己有 bug、吐出一个
/// 越界槽号"上的第二道防线：P4-c"引擎自身 bug"要求 debug 帧内断言就地 panic，release
/// 零成本；不断言 = 分配器一旦真越界，这里会静默截断槽号，吐出一段指向错误槽位的
/// 字节码而不是在 debug 构建炸出来。
fn narrow_slot(slot: usize) -> u8 {
    debug_assert!(
        slot <= u8::MAX as usize,
        "locals 槽号 {slot} 超出 u8 编码范围（lang::slots 分配器应已保证 ≤64）"
    );
    slot as u8
}

/// 弹 setter 族名单（模块文档"弹 setter 族 handle:int 首参"）。
fn is_self_bullet_setter(name: &str) -> bool {
    matches!(
        name,
        "set_speed"
            | "set_angle"
            | "turn"
            | "set_vel"
            | "set_ang_vel"
            | "set_accel"
            | "set_gravity"
            | "stop_fx"
            | "aim_at_player"
    )
}

/// 循环栈簿记（clox 惯用法）：`break`/`continue` 各自的跳转占位位置列表，循环结构生成
/// 完毕、`continue`/`break` 的真实目标（本地 code 位置）已知后统一回填。
#[derive(Default)]
struct LoopCtx {
    break_fixups: Vec<usize>,
    continue_fixups: Vec<usize>,
}

struct Gen<'p> {
    sm: &'p SlotMap,
    xformdefs: BTreeMap<String, &'p crate::lang::ast::XformDef>,
    name_to_id: BTreeMap<String, ScriptId>,
    sub_params: BTreeMap<String, Vec<String>>,
    call_style: BTreeMap<String, bool>,
    consts: BTreeMap<String, i32>,
    errors: Vec<CompileError>,
}

impl<'p> Gen<'p> {
    fn err(&mut self, span: Span, msg: String) {
        self.errors.push(CompileError {
            line: span.line,
            col: span.col,
            msg,
            src_line: String::new(),
        });
    }

    // ── sub 入口 xformdef staging（模块文档；一次，入口直排，先于 loop 回跳点）───

    fn gen_xformdef_staging(&mut self, b: &mut SubBuilder, slots: &SubSlots, sub: &TypedSub) {
        for xf_name in &sub.xform_refs {
            let Some(xfdef) = self.xformdefs.get(xf_name.as_str()).copied() else {
                continue; // 未知 xformdef 名：typeck 已经拦过，防御性跳过不重复报错
            };
            let (off, _cnt) = slots.xform_regions[xf_name];
            let mut built = Vec::with_capacity(xfdef.slots.len());
            for s in &xfdef.slots {
                match crate::lang::xform_map::lookup(&s.op_name) {
                    None | Some(crate::lang::xform_map::XformOp::Reserved) => {
                        // 未知/预留 op 名：slots 趟已报错（sizing 单一权威在那边），
                        // 此处防御性补位，不重复报错。
                        built.push(XformSlot::default());
                    }
                    Some(crate::lang::xform_map::XformOp::Op(op, arity, physical)) => {
                        if s.args.len() != arity {
                            self.err(
                                s.span,
                                format!(
                                    "xform 操作 '{}' 期待 {arity} 个参数，实际 {}",
                                    s.op_name,
                                    s.args.len()
                                ),
                            );
                            built.push(XformSlot::default());
                            continue;
                        }
                        let mut args = [0i32; 2];
                        let mut ok = true;
                        for (i, a) in s.args.iter().enumerate() {
                            match eval_const_arg(a, &self.consts) {
                                Ok(v) => args[i] = v,
                                Err(msg) => {
                                    self.err(
                                        s.span,
                                        format!("xformdef 参数必须是编译期常量：{msg}"),
                                    );
                                    ok = false;
                                }
                            }
                        }
                        if !ok {
                            built.push(XformSlot::default());
                            continue;
                        }
                        built.push(XformSlot {
                            wait: s.wait,
                            op,
                            _pad: 0,
                            args,
                        });
                        // STEP 族物理双槽：第二槽是引擎 scratch，表层作者不可见——
                        // 编译器自动补零槽（不补则运行期 scratch 写入覆写下一条 authored
                        // 槽，T3 复审 Important 的修法；区宽已由 slots 趟按物理数计）。
                        for _ in 1..physical {
                            built.push(XformSlot::default());
                        }
                    }
                }
            }
            b.write_xform_locals(narrow_slot(off), &built);
        }
    }

    // ── 语句 ─────────────────────────────────────────────────────────────

    fn gen_block(
        &mut self,
        b: &mut SubBuilder,
        loops: &mut Vec<LoopCtx>,
        slots: &SubSlots,
        sub: &TypedSub,
        body: &[TypedStmt],
    ) {
        for s in body {
            self.gen_stmt(b, loops, slots, sub, s);
        }
    }

    fn gen_stmt(
        &mut self,
        b: &mut SubBuilder,
        loops: &mut Vec<LoopCtx>,
        slots: &SubSlots,
        sub: &TypedSub,
        stmt: &TypedStmt,
    ) {
        match stmt {
            TypedStmt::Var { name, init, .. } => {
                self.gen_expr(b, slots, init);
                b.pop_l(narrow_slot(slots.locals[name]));
            }
            TypedStmt::Assign { name, value } => {
                self.gen_expr(b, slots, value);
                b.pop_l(narrow_slot(slots.locals[name]));
            }
            TypedStmt::If {
                cond,
                then_b,
                else_b,
            } => {
                self.gen_expr(b, slots, cond);
                let jz_pos = b.raw_jz();
                self.gen_block(b, loops, slots, sub, then_b);
                match else_b {
                    Some(else_body) => {
                        let jmp_end = b.raw_jmp();
                        let else_target = b.here();
                        b.patch(jz_pos, else_target);
                        self.gen_block(b, loops, slots, sub, else_body);
                        let end = b.here();
                        b.patch(jmp_end, end);
                    }
                    None => {
                        let end = b.here();
                        b.patch(jz_pos, end);
                    }
                }
            }
            TypedStmt::While { cond, body } => {
                let top = b.here();
                self.gen_expr(b, slots, cond);
                let jz_end = b.raw_jz();
                loops.push(LoopCtx::default());
                self.gen_block(b, loops, slots, sub, body);
                let lc = loops.pop().expect("刚 push 过");
                let jmp_top = b.raw_jmp();
                b.patch(jmp_top, top);
                let end = b.here();
                b.patch(jz_end, end);
                for p in lc.break_fixups {
                    b.patch(p, end);
                }
                for p in lc.continue_fixups {
                    b.patch(p, top);
                }
            }
            TypedStmt::Loop { body } => {
                let top = b.here();
                loops.push(LoopCtx::default());
                self.gen_block(b, loops, slots, sub, body);
                let lc = loops.pop().expect("刚 push 过");
                let jmp_top = b.raw_jmp();
                b.patch(jmp_top, top);
                let end = b.here();
                for p in lc.break_fixups {
                    b.patch(p, end);
                }
                for p in lc.continue_fixups {
                    b.patch(p, top);
                }
            }
            TypedStmt::For {
                var,
                from,
                to,
                body,
            } => {
                self.gen_expr(b, slots, from);
                let var_slot = narrow_slot(slots.locals[var]);
                b.pop_l(var_slot);
                let top = b.here();
                b.push_l(var_slot);
                self.gen_expr(b, slots, to);
                b.lt();
                let jz_end = b.raw_jz();
                loops.push(LoopCtx::default());
                self.gen_block(b, loops, slots, sub, body);
                let lc = loops.pop().expect("刚 push 过");
                // continue 指向自增段（spec 降低模板）。
                let cont = b.here();
                b.push_l(var_slot);
                b.push_i(1);
                b.add();
                b.pop_l(var_slot);
                let jmp_top = b.raw_jmp();
                b.patch(jmp_top, top);
                let end = b.here();
                b.patch(jz_end, end);
                for p in lc.break_fixups {
                    b.patch(p, end);
                }
                for p in lc.continue_fixups {
                    b.patch(p, cont);
                }
            }
            TypedStmt::Wait { frames } => {
                self.gen_expr(b, slots, frames);
                b.raw_wait();
            }
            TypedStmt::Spawn { call } => {
                for a in &call.args {
                    match a {
                        CallArg::Val(e) => self.gen_expr(b, slots, e),
                        CallArg::XformRef(_) | CallArg::SubRef(_) => {
                            unreachable!("spawn 目标是 sub，参数恒 Val（typeck 已保证）")
                        }
                    }
                }
                let id = self.name_to_id[&call.name];
                let argc = call.args.len();
                // 同 `narrow_slot`：argc 经由目标 sub 参数落在其自身 locals 区间，间接
                // 受同一条 ≤64 上限约束，正常源码走不到这条断言（C19 复审修复）。
                debug_assert!(
                    argc <= u8::MAX as usize,
                    "spawn 实参数 {argc} 超出 u8 编码范围"
                );
                b.spawn(id, argc as u8);
                b.pop(); // SPAWN 恒压任务句柄，语言无消费语法——自动丢弃（模块级说明同 typeck）
            }
            TypedStmt::ExprStmtDiscard { expr } => {
                self.gen_expr(b, slots, expr);
                b.pop();
            }
            TypedStmt::ExprStmtVoid { call } => {
                self.gen_call(b, slots, call);
            }
            TypedStmt::Return => {
                if *self.call_style.get(&sub.name).unwrap_or(&false) {
                    b.raw_ret();
                } else {
                    b.end();
                }
            }
            TypedStmt::Break => {
                let p = b.raw_jmp();
                loops
                    .last_mut()
                    .expect("typeck 已保证 break 只出现在循环内")
                    .break_fixups
                    .push(p);
            }
            TypedStmt::Continue => {
                let p = b.raw_jmp();
                loops
                    .last_mut()
                    .expect("typeck 已保证 continue 只出现在循环内")
                    .continue_fixups
                    .push(p);
            }
        }
    }

    // ── 表达式 ───────────────────────────────────────────────────────────

    fn gen_expr(&mut self, b: &mut SubBuilder, slots: &SubSlots, e: &TypedExpr) {
        match &e.kind {
            TypedExprKind::IntLit(v) => b.push_i(*v),
            TypedExprKind::FxLit(v) => b.push_i(*v),
            TypedExprKind::AngleLit(v) => b.push_i(*v as i32),
            TypedExprKind::ConstRef(v) => b.push_i(*v),
            TypedExprKind::LocalRef(name) => b.push_l(narrow_slot(slots.locals[name])),
            TypedExprKind::EngineVar(ev) => {
                let info = builtins::engine_var_info(*ev);
                b.sys(info.syscall);
            }
            TypedExprKind::Call(call) => self.gen_call(b, slots, call),
            TypedExprKind::Binary { op, l, r, intent } => {
                self.gen_binary(b, slots, *op, l, r, *intent)
            }
            TypedExprKind::Unary {
                e: inner, intent, ..
            } => {
                self.gen_expr(b, slots, inner);
                match intent {
                    UnIntent::Neg => b.neg(),
                    UnIntent::Not => {
                        b.push_i(0);
                        b.eq();
                    }
                }
            }
            TypedExprKind::Cast { e: inner, intent } => {
                self.gen_expr(b, slots, inner);
                match intent {
                    CastIntent::IntToFx => {
                        b.push_i(65536);
                        b.mul();
                    }
                    CastIntent::FxToInt => {
                        b.push_i(65536);
                        b.div();
                    }
                    CastIntent::Bitcast => {}
                }
            }
        }
    }

    fn gen_binary(
        &mut self,
        b: &mut SubBuilder,
        slots: &SubSlots,
        op: BinOp,
        l: &TypedExpr,
        r: &TypedExpr,
        intent: BinIntent,
    ) {
        match intent {
            BinIntent::LogicAnd => self.gen_logic_and(b, slots, l, r),
            BinIntent::LogicOr => self.gen_logic_or(b, slots, l, r),
            _ => {
                self.gen_expr(b, slots, l);
                self.gen_expr(b, slots, r);
                match intent {
                    BinIntent::AddI => b.add(),
                    BinIntent::SubI => b.sub(),
                    BinIntent::MulI => b.mul(),
                    BinIntent::DivI => b.div(),
                    BinIntent::ModI => b.rem(),
                    BinIntent::MulF => b.mulf(),
                    BinIntent::DivF => b.divf(),
                    BinIntent::Cmp => match op {
                        BinOp::Eq => b.eq(),
                        BinOp::Ne => b.ne(),
                        BinOp::Lt => b.lt(),
                        BinOp::Le => b.le(),
                        BinOp::Gt => b.gt(),
                        BinOp::Ge => b.ge(),
                        _ => unreachable!("Cmp intent 只来自六个比较符之一"),
                    },
                    BinIntent::LogicAnd | BinIntent::LogicOr => {
                        unreachable!("已在上面短路分支处理")
                    }
                }
            }
        }
    }

    /// `a && b`：模块文档"`&&`/`||` 短路"。
    fn gen_logic_and(
        &mut self,
        b: &mut SubBuilder,
        slots: &SubSlots,
        l: &TypedExpr,
        r: &TypedExpr,
    ) {
        self.gen_expr(b, slots, l);
        let jz1 = b.raw_jz();
        self.gen_expr(b, slots, r);
        let jz2 = b.raw_jz();
        b.push_i(1);
        let jmp_end = b.raw_jmp();
        let false_target = b.here();
        b.patch(jz1, false_target);
        b.patch(jz2, false_target);
        b.push_i(0);
        let end = b.here();
        b.patch(jmp_end, end);
    }

    /// `a || b`：模块文档"`&&`/`||` 短路"。
    fn gen_logic_or(&mut self, b: &mut SubBuilder, slots: &SubSlots, l: &TypedExpr, r: &TypedExpr) {
        self.gen_expr(b, slots, l);
        let jz_check_b = b.raw_jz();
        b.push_i(1);
        let jmp_end1 = b.raw_jmp();
        let check_b = b.here();
        b.patch(jz_check_b, check_b);
        self.gen_expr(b, slots, r);
        let jz_false = b.raw_jz();
        b.push_i(1);
        let jmp_end2 = b.raw_jmp();
        let false_target = b.here();
        b.patch(jz_false, false_target);
        b.push_i(0);
        let end = b.here();
        b.patch(jmp_end1, end);
        b.patch(jmp_end2, end);
    }

    // ── 调用 ─────────────────────────────────────────────────────────────

    /// sub 调用：**caller 写 callee 槽再 CALL**（plan Global Constraints 原文）——逐位
    /// 对齐目标 sub 的参数声明序，每位"求值、立即 `POPL` 进目标槽"（不是"全部求值压栈
    /// 再统一处理"——目标槽号各自独立、互不相邻，没有必要也没有栈序约定）。目标 sub 的
    /// 槽区间与调用方（`slots` 参数，调用方自己的 `SubSlots`）静态不相交（T2 调用图着色
    /// 保证），故对目标槽的写入不会踏进调用方自己正在使用的任何局部变量。
    fn gen_call(&mut self, b: &mut SubBuilder, slots: &SubSlots, call: &TypedCall) {
        match &call.target {
            CallTarget::Sub => {
                let target_name = call.name.clone();
                // 先算好目标槽号（拷贝出 Vec<u8>，与 `self` 的借用解耦）——循环体内
                // 紧接着要对 `self` 做可变借用（`gen_expr` 递归），不能让这份查表结果
                // 借着 `self.sm`/`self.sub_params` 的不可变借用穿越那次可变调用。
                let param_slots: Vec<u8> = {
                    let target_slots = &self.sm.subs[&target_name];
                    self.sub_params[&target_name]
                        .iter()
                        .map(|n| narrow_slot(target_slots.locals[n]))
                        .collect()
                };
                for (i, a) in call.args.iter().enumerate() {
                    match a {
                        CallArg::Val(e) => {
                            self.gen_expr(b, slots, e);
                            b.pop_l(param_slots[i]);
                        }
                        CallArg::XformRef(_) | CallArg::SubRef(_) => {
                            unreachable!("sub 调用参数恒 Val（typeck 已保证）")
                        }
                    }
                }
                let id = self.name_to_id[&target_name];
                b.call(id);
            }
            CallTarget::Builtin(bi) => self.gen_builtin_call(b, slots, bi, &call.args),
        }
    }

    fn gen_builtin_call(
        &mut self,
        b: &mut SubBuilder,
        slots: &SubSlots,
        bi: &'static builtins::Builtin,
        args: &[CallArg],
    ) {
        let discard_first_handle = is_self_bullet_setter(bi.name);
        for (i, (a, pk)) in args.iter().zip(bi.params.iter()).enumerate() {
            match (a, pk) {
                (CallArg::Val(e), ParamKind::Val(_)) => {
                    self.gen_expr(b, slots, e);
                    if i == 0 && discard_first_handle {
                        b.pop();
                    }
                }
                (CallArg::XformRef(name_opt), ParamKind::XformRef) => match name_opt {
                    Some(name) => {
                        let (off, cnt) = slots.xform_regions[name];
                        b.push_i(off as i32);
                        b.push_i(cnt as i32);
                    }
                    None => {
                        b.push_i(0);
                        b.push_i(0);
                    }
                },
                (CallArg::SubRef(name_opt), ParamKind::SubRef) => match name_opt {
                    Some(name) => {
                        let id = self.name_to_id[name];
                        b.push_i(id.0 as i32);
                    }
                    None => b.push_i(-1),
                },
                _ => unreachable!("typeck 已保证 CallArg 与 ParamKind 一一对应"),
            }
        }
        if bi.is_op {
            // `is_op` 直发路径的 `syscall` 字段实际装的是 VM op 码本身（`lang::builtins`
            // 模块文档"Builtin 字段形状"）——v1 只有 sin/cos 两个硬编码小常量，但断言
            // 挡住未来任何新增 `is_op:true` 条目手滑填了个超出 u8 的号（C19 复审修复）。
            debug_assert!(
                bi.syscall <= u8::MAX as u16,
                "'{}' 的 is_op 直发 op 码 {} 超出 u8 编码范围",
                bi.name,
                bi.syscall
            );
            b.raw_emit_op(bi.syscall as u8);
        } else {
            b.sys(bi.syscall);
        }
    }
}

/// codegen 趟入口：`TypedInfo`/`SlotMap`/原始 `Program`（xformdef 常量折叠用）→
/// `EclImage`。sub 名 → `ScriptId` 按 `Program.subs` 声明序分配（模块文档）。
pub fn generate(
    prog: &Program,
    ti: &TypedInfo,
    sm: &SlotMap,
) -> Result<EclImage, Vec<CompileError>> {
    let name_to_id: BTreeMap<String, ScriptId> = prog
        .subs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), ScriptId(i as u16)))
        .collect();
    let sub_params: BTreeMap<String, Vec<String>> = ti
        .subs
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.params.iter().map(|(n, _)| n.clone()).collect(),
            )
        })
        .collect();
    let mut call_style: BTreeMap<String, bool> =
        ti.subs.iter().map(|s| (s.name.clone(), false)).collect();
    for s in &ti.subs {
        for callee in &s.sync_calls {
            call_style.insert(callee.clone(), true);
        }
    }
    let consts: BTreeMap<String, i32> = ti.consts.iter().map(|(n, _, v)| (n.clone(), *v)).collect();
    let xformdefs: BTreeMap<String, &crate::lang::ast::XformDef> =
        prog.xformdefs.iter().map(|x| (x.name.clone(), x)).collect();

    let mut g = Gen {
        sm,
        xformdefs,
        name_to_id,
        sub_params,
        call_style,
        consts,
        errors: Vec::new(),
    };

    let mut ib = ImageBuilder::new();
    for sub in &ti.subs {
        let slots = &sm.subs[&sub.name];
        let mut b = SubBuilder::new();
        g.gen_xformdef_staging(&mut b, slots, sub);
        let mut loops: Vec<LoopCtx> = Vec::new();
        g.gen_block(&mut b, &mut loops, slots, sub, &sub.body);
        if *g.call_style.get(&sub.name).unwrap_or(&false) {
            b.raw_ret();
        } else {
            b.end();
        }
        let id = ib.add_sub(b);
        debug_assert_eq!(id, g.name_to_id[&sub.name], "add_sub 序必须与声明序一致");
    }

    if g.errors.is_empty() {
        Ok(ib.build())
    } else {
        Err(g.errors)
    }
}

#[cfg(test)]
mod tests {
    //! 端到端测试：`.ecl` 源码字符串 → `crate::lang::compile` → 真实 `World`/`step` 跑 N 帧 →
    //! 行为断言。观测口一律走 `stg-core` 公开面（`globals`/`iter_alive().count()`/
    //! `diag.task_faults`）——**P1 边界在编译器自己的测试里也生效**：连本 crate 的测试代码
    //! 都不摸 `BulletPool`/`TaskPool` 内部字段（那些是 `pub(crate)`，出了 `stg-core` 就是
    //! 私有，见 `stg-derive::define_pool!` 生成代码），只能靠脚本自己把要观测的量写进
    //! `globals`（`set_global`/`get_var`）或靠"存活与否"这类粗粒度公开信号推断行为——这不是
    //! 测试能力不足，是刻意的架构纪律（"调用方永不直接触碰池内存"）。
    //!
    //! **"xformdef 弹真转向"的观测替代方案**：brief 原文写"bullet angle sampling"，但
    //! `bullets.angle[i]` 是 `pub(crate)`，编译器测试读不到。改用**轨迹差分**：给弹一个
    //! 会在 N 帧内因"沿 x 轴出界"而被回收的初始位置/朝向，对照组不转向按时出界死亡，
    //! 实验组被 `TURN` 转向 90°（改沿 y 轴飞）后同一时窗内还没触达任何边界、依然存活——
    //! "存活与否"这个粗粒度但纯公开的信号足以反证 xformdef staging→fire xf 解析→
    //! `OP_TURN` 派发这条链路确实生效（弱于直接读角度值，但可达、且是本 crate 权限内
    //! 能拿到的最强证据，见模块文档"观测口"）。
    //!
    //! **`&&`/`||` 短路"副作用"的观测替代方案**：`set_global` 是 `ret:None`，语言类型系统
    //! 禁止把它嵌进表达式当子节点（"无返回值，不能用作表达式的值"）——brief 建议的
    //! "用 set_global 观测"字面上在这门语言里做不到（这是 T1/T2 已经钉死的语言约束，不是
    //! 本刀能绕开的）。改用**故意会 Fault 的右操作数**（`1/0`）：右操作数被跳过 ⇒
    //! 不 Fault；右操作数被求值 ⇒ Fault——`task_faults` 计数器同样是直接可读的公开字段，
    //! 精确反证"跳过"与"求值"两条路径分别对应哪种源码。
    use super::generate;
    use crate::lang::compile;
    use stg_core::ecl::task::OWNER_STAGE;
    use stg_core::input::InputFrame;
    use stg_core::step::{World, step};
    use stg_core::tables::TABLES_V0;

    const FREE: u16 = 20; // 自由段全局槽起点（≥ GLOBALS_SYS_SEGMENT）

    /// 编译 + 跑 N 帧，返回跑完后的 `World`（供调用方读 `globals`/`bullets`/`diag`）。
    fn run(src: &str, frames: u32) -> Box<World> {
        let image = compile(src, "e2e.ecl").unwrap_or_else(|e| panic!("编译失败：{e:?}"));
        let mut w = World::new(1);
        let main_id = *image.subs.first().expect("应至少有一个 sub");
        // sub 名→ScriptId 是声明序，源码里第一个声明的 sub 恒是 script 0——本模块全部
        // 测试脚本都遵循"main 是源码里第一个 sub"的约定（xformdef/const 不计入 sub 序）。
        let _ = main_id;
        w.spawn_task(&image, 0, (OWNER_STAGE, 0, 0))
            .expect("main 应能派生");
        for f in 0..frames {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
        }
        w
    }

    // ── if 双分支 ───────────────────────────────────────────────────────

    #[test]
    fn if_both_branches_are_reachable() {
        let src = "sub main() {\n\
                     var x: int = 0;\n\
                     if 1 == 1 { x = 10; } else { x = 20; }\n\
                     set_global(20, x);\n\
                     var y: int = 0;\n\
                     if 1 == 2 { y = 10; } else { y = 20; }\n\
                     set_global(21, y);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(w.body.globals[20], 10, "true 分支应落地 then");
        assert_eq!(w.body.globals[21], 20, "false 分支应落地 else");
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── while 计数 ──────────────────────────────────────────────────────

    #[test]
    fn while_loop_counts_correctly() {
        let src = "sub main() {\n\
                     var t: int = 0;\n\
                     while t < 5 { t = t + 1; }\n\
                     set_global(20, t);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(w.body.globals[20], 5);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── for 累加 + continue + break ─────────────────────────────────────

    #[test]
    fn for_loop_accumulates_with_continue_and_break() {
        // 0..10：跳过 i==3（continue），i==7 时中止（break，不计入）——
        // 0+1+2+4+5+6 = 18。
        let src = "sub main() {\n\
                     var sum: int = 0;\n\
                     for i in 0..10 {\n\
                       if i == 3 { continue; }\n\
                       if i == 7 { break; }\n\
                       sum = sum + i;\n\
                     }\n\
                     set_global(20, sum);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(w.body.globals[20], 18);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── `&&`/`||` 短路（观测替代方案见模块文档）─────────────────────────

    #[test]
    fn logic_and_short_circuits_and_skips_faulting_rhs() {
        let src = "sub main() {\n\
                     var flag: int = 0;\n\
                     if flag == 1 && 1 / flag == 1 { }\n\
                     set_global(20, 1);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(w.body.diag.task_faults, 0, "左假应短路，右操作数不该求值");
        assert_eq!(w.body.globals[20], 1, "短路后应正常继续执行到之后的语句");
    }

    #[test]
    fn logic_and_evaluates_rhs_when_lhs_true() {
        let src = "sub main() {\n\
                     var flag: int = 1;\n\
                     if flag == 1 && 1 / (flag - 1) == 1 { }\n\
                     set_global(20, 1);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(
            w.body.diag.task_faults, 1,
            "左真必须求值右操作数——除零应真的 Fault"
        );
    }

    #[test]
    fn logic_or_short_circuits_and_skips_faulting_rhs() {
        let src = "sub main() {\n\
                     var flag: int = 1;\n\
                     if flag == 1 || 1 / (flag - 1) == 1 { }\n\
                     set_global(20, 1);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(w.body.diag.task_faults, 0, "左真应短路，右操作数不该求值");
        assert_eq!(w.body.globals[20], 1);
    }

    #[test]
    fn logic_or_evaluates_rhs_when_lhs_false() {
        let src = "sub main() {\n\
                     var flag: int = 0;\n\
                     if flag == 1 || 1 / flag == 1 { }\n\
                     set_global(20, 1);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(
            w.body.diag.task_faults, 1,
            "左假必须求值右操作数——除零应真的 Fault"
        );
    }

    // ── sub 调用参数：声明序落位 ────────────────────────────────────────

    #[test]
    fn sub_call_args_land_in_declaration_order() {
        let src = "sub main() {\n\
                     helper(11, 22, 33);\n\
                   }\n\
                   sub helper(a: int, b: int, c: int) {\n\
                     set_global(20, a);\n\
                     set_global(21, b);\n\
                     set_global(22, c);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(
            [w.body.globals[20], w.body.globals[21], w.body.globals[22]],
            [11, 22, 33]
        );
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── spawn 带参（async sub）────────────────────────────────────────

    #[test]
    fn spawn_async_sub_with_args_lands_correctly() {
        let src = "sub main() {\n\
                     spawn child(11, 22);\n\
                     wait(1000);\n\
                   }\n\
                   async sub child(a: int, b: int) {\n\
                     set_global(20, a + b);\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（spawn child，child born_frame=1）；
        // 2=child 首跑（次帧首跑）。
        let w = run(src, 3);
        assert_eq!(w.body.globals[20], 33);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    /// `fire(...)` 的 `task` 参数（`CallArg::SubRef`）：与 `xf` 参数（`CallArg::XformRef`）
    /// 是独立的 codegen 分支——`xformdef_turn_changes_bullet_trajectory_survival` 只覆盖了
    /// `xf`，这里单独钉 `task`：挂在新弹上的 async sub 应经真实调度（次帧首跑）执行。
    #[test]
    fn fire_task_script_attaches_and_runs_async_sub_on_new_bullet() {
        let src = "sub main() {\n\
                     _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, none, on_bullet);\n\
                     wait(1000);\n\
                   }\n\
                   async sub on_bullet() {\n\
                     set_global(20, 1);\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（fire 挂任务，born_frame=1）；2=挂载任务首跑。
        let w = run(src, 3);
        assert_eq!(w.body.globals[20], 1);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── xformdef TURN：轨迹差分观测（模块文档）────────────────────────

    #[test]
    fn xformdef_turn_changes_bullet_trajectory_survival() {
        // 无转向：沿 +x 飞出 [-256,256] 边界（200 + 4*15 = 260 > 256）。
        let no_turn = "sub main() {\n\
                         _ = fire(0, 200fx, 100fx, 4.0fx, 0deg, none, none);\n\
                         wait(1000);\n\
                       }";
        let w1 = run(no_turn, 20);
        assert_eq!(
            w1.body.bullets.iter_alive().count(),
            0,
            "未转向应沿 x 轴飞出边界被回收"
        );

        // TURN +90°（16384 BAM，同帧生效——`wait=0`）：转向沿 +y（下）飞，同一时窗内
        // y ∈ [-64,512] 远未触边，应仍存活。
        let with_turn = "xformdef RING { turn(90deg); }\n\
                          sub main() {\n\
                            _ = fire(0, 200fx, 100fx, 4.0fx, 0deg, RING, none);\n\
                            wait(1000);\n\
                          }";
        let w2 = run(with_turn, 20);
        assert_eq!(
            w2.body.bullets.iter_alive().count(),
            1,
            "转向后应改沿 y 轴飞，同一时窗内不出界，应存活——\
             反证 xformdef staging → fire xf 解析 → OP_TURN 派发链路生效"
        );
        assert_eq!(w2.body.diag.task_faults, 0);
    }

    /// STEP 族 scratch 自动补槽的行为学判别（T3 复审 Important 修法）：
    /// `step_speed` 物理双槽——编译器不补 scratch 时，紧随其后的 `set_life(1)` 会落在
    /// scratch 槽位、被引擎运行期覆写而**永不执行**（弹永生）；补了则 set_life 照常
    /// 生效（弹快速回收）。本测试在"补"分支断言弹死——把足枪钉死成可红命题。
    #[test]
    fn step_op_auto_scratch_keeps_following_slot_alive() {
        let src = "xformdef S { step_speed(2.0fx, 4); set_life(1); }\n\
                    sub main() {\n\
                      _ = fire(0, 0fx, 100fx, 0.5fx, 0deg, S, none);\n\
                      wait(1000);\n\
                    }";
        let w = run(src, 20);
        assert_eq!(
            w.body.bullets.iter_alive().count(),
            0,
            "set_life(1) 必须在 STEP 的 scratch 槽之后照常执行——弹应已回收；\
             若此断言红 = scratch 未自动补，set_life 被引擎 scratch 覆写"
        );
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── cast 往返 ───────────────────────────────────────────────────────

    #[test]
    fn casts_round_trip() {
        let src = "sub main() {\n\
                     var i: int = 5;\n\
                     var f: fx = i as fx;\n\
                     var back: int = f as int;\n\
                     set_global(20, back);\n\
                     var a: angle = 1000 as angle;\n\
                     var a2: int = a as int;\n\
                     set_global(21, a2);\n\
                   }";
        let w = run(src, 2);
        assert_eq!(
            w.body.globals[20], 5,
            "int→fx→int 精确往返（整数无小数损失）"
        );
        assert_eq!(w.body.globals[21], 1000, "int↔angle 位穿透往返");
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── `$` 引擎变量读 ──────────────────────────────────────────────────

    #[test]
    fn engine_var_frame_reads_current_frame() {
        let src = "sub main() {\n\
                     var f: int = $frame;\n\
                     set_global(20, f);\n\
                   }";
        // 出生帧（0）跳过，次帧（1）首跑：$frame 应读到 1。
        let w = run(src, 2);
        assert_eq!(w.body.globals[FREE as usize], 1);
        assert_eq!(w.body.diag.task_faults, 0);
    }

    // ── 编译器确定性（全管线级别；`lang::mod` 已有一份，这里再钉一份跑真实
    // 端到端脚本的，双重实证）────────────────────────────────────────────

    #[test]
    fn compiling_end_to_end_script_twice_yields_identical_bytecode() {
        let src = "xformdef RING { turn(90deg); }\n\
                    async sub child(a: int) { set_global(20, a); }\n\
                    sub helper(x: int) { set_global(21, x); }\n\
                    sub main() {\n\
                      spawn child(1);\n\
                      helper(2);\n\
                      _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, RING, none);\n\
                      var i: int = 0;\n\
                      while i < 3 { i = i + 1; }\n\
                    }";
        let img1 = compile(src, "det.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        let img2 = compile(src, "det.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(img1.code, img2.code);
        assert_eq!(img1.subs, img2.subs);
    }

    // ── 错误路径：xformdef 参数非编译期常量 ─────────────────────────────

    #[test]
    fn xformdef_non_const_arg_is_a_compile_error() {
        let src = "xformdef BAD { turn($frame); }\n\
                    sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, BAD, none); }";
        let errors = match compile(src, "bad.ecl") {
            Err(e) => e,
            Ok(_) => panic!("期望编译失败（xformdef 参数非常量）"),
        };
        assert!(
            errors.iter().any(|e| e.msg.contains("编译期常量")),
            "{errors:?}"
        );
    }

    // ── C19 复审修复：locals 槽号窄化为 u8 前的 debug_assert 兜底 ─────────────

    /// `lang::slots` 已经把 locals 总量钉在 ≤64（`slots.rs::locals_over_64_is_rejected`），
    /// 正常 `.ecl` 源码走不到"槽号超出 u8 范围"这条路——这条测试钉的是第二道防线：万一
    /// 分配器自己出 bug、吐出一个越界槽号，codegen 必须在窄化为 `u8` 之前 debug 帧内断言
    /// 就地 panic（P4-c"引擎自身 bug"），而不是静默截断槽号、吐出一段指向错误槽位的
    /// 字节码。手工腐化一份合法编译产出的 `SlotMap`（模拟"分配器有 bug"）来触发它——同
    /// `phase_guard`/`sys_set_var_system_segment_guard` 一脉的"直写内部状态触发断言"模式。
    #[test]
    #[should_panic(expected = "超出 u8 编码范围")]
    fn narrow_slot_debug_asserts_when_slots_allocator_corrupted() {
        let prog = crate::lang::parse("sub main() { var x: int = 1; var y: int = x; }", "t.ecl")
            .expect("解析失败");
        let ti = crate::lang::typeck::check(&prog).expect("判型失败");
        let mut sm = crate::lang::slots::allocate(&prog, &ti).expect("槽分配失败");
        *sm.subs
            .get_mut("main")
            .unwrap()
            .locals
            .get_mut("x")
            .unwrap() = 999;
        let _ = generate(&prog, &ti, &sm);
    }
}
