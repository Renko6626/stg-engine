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
//! ## sub 名 → `BuilderSubRef`：先声明、后定义
//!
//! `call`/`spawn`/`fire` 的 `task` 引用都需要在**生成目标 sub 自己的字节码之前**就知道
//! 引用者（互相调用是常态，A 调 B 时 B 可能还没生成）。`generate` 因此先声明全部 sub，
//! 得到稳定的 `name → BuilderSubRef` 映射，再生成并定义各个 body。最终 `build()` 按名字
//! 排序并把引用重写为 canonical `SubId`，产物不依赖源码声明顺序。
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
//! `XfSlotLit.args` 求值走 `crate::lang::const_eval::evaluate`（C14 收编：与
//! `lang::typeck::consts::fold_const` 共用同一份求值器唯一实现，之前各自维护一份的
//! `eval_const_arg`/`fold_const` 已合一）——字面量 + 已声明 `const` 引用 + 一元 `-` + 二元
//! 算术/比较/逻辑 + cast 皆合法（求值器的能力矩阵，非本趟单独收窄）；`$` 变量/调用等非
//! 编译期可求值的形状仍一律编译错误，文案包一层前缀含"编译期常量"（`const_eval` 本身的
//! 报错文案含"非编译期可求值"，见下方求值处）。`self.consts` 由 `generate` 从
//! `TypedInfo.consts` 构造（保留 `Ty`，供 `evaluate` 的带型常量表签名使用）。
//!
//! ## 弹 setter 族 `handle:int` 首参：求值后丢弃（T3 落地拍板）
//!
//! `lang::builtins` 报告点名的悬而未决项——`set_speed`/`set_angle`/`turn`/`set_vel`/
//! `set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player` 九个 setter 的首参
//! `handle:int`，底层 syscall 实际操作的是 `self`（任务 owner），并不消费一个显式句柄。
//! 本趟落地策略：**求值后立即丢弃**（保留副作用——万一作者写了带副作用的表达式——但
//! 不把它压进 syscall 实际吃的参数序列），不是"Fault if handle != self"（更严格但当前
//! 无法在编译期证明等值，运行期也没有校验入口）——最小惊讶、不新增运行期检查。
use crate::lang::ast::{BinOp, CompileError, Program, Span, Ty};
use crate::lang::builtins::{self, ParamKind};
use crate::lang::const_eval;
use crate::lang::slots::{SlotMap, SubSlots};
use crate::lang::typeck::{
    BinIntent, CallArg, CallTarget, CastIntent, TypedCall, TypedExpr, TypedExprKind, TypedInfo,
    TypedStmt, TypedSub, UnIntent, const_val,
};
use crate::{BuilderSubRef, ImageBuilder, SubBuilder};
use std::collections::{BTreeMap, BTreeSet};
use stg_core::ecl::image::{EclImage, EclValueType, ImageBuildError, SubKind};
use stg_core::ecl::syscall;
use stg_core::xform;
use stg_core::xform::XformSlot;

// xformdef 操作名映射表已上移 `lang::xform_map`（slots 趟与本趟共用的单一权威，含物理
// 槽数与 STEP 族 scratch 语义）；`loop`/`END` 不开放的已知限制也记录在该模块文档。

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

/// 多物理槽 op 的 scratch 补零（`physical - 1` 个空槽）。
///
/// STEP 族物理双槽：第二槽是引擎 scratch，表层作者不可见——编译器自动补零槽（不补则
/// 运行期 scratch 写入会覆写下一条 authored 槽，T3 复审 Important 的修法；区宽已由
/// slots 趟经 `xform_map::physical_len` 按物理数计）。`Op`/`OpFold2`/`OpWithStride`
/// 三个分支（T7 之后的调用点，见下方 `gen_xformdef_staging` 的三处 `push_scratch_slots`
/// 调用）都要走——`physical_len` 对三者一视同仁地把 `physical` 计进区宽，漏补任何一边
/// 就会重现那个坑。
fn push_scratch_slots(built: &mut Vec<XformSlot>, physical: usize) {
    for _ in 1..physical {
        built.push(XformSlot::default());
    }
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

/// 降低后要追发 `OP_KILL_SELF` 的内建（当前仅 `die`）。名字键控，与
/// [`is_self_bullet_setter`] 同款——集中在一处，免得散落在 `gen_builtin_call` 里。
fn emits_kill_self_after(name: &str) -> bool {
    name == "die"
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
    name_to_ref: BTreeMap<String, BuilderSubRef>,
    sub_params: BTreeMap<String, Vec<String>>,
    consts: BTreeMap<String, (Ty, i32)>,
    /// mark 自动补偿表（Task 5）：`mark(id)` → 该落点应注入的三类锚点值。整个 codegen 趟
    /// 期间不变（同 `xformdefs`/`consts` 一样是"全程序"范围数据，故与它们同样落在 `Gen`
    /// 自己身上，而不是随 `gen_block`/`gen_stmt` 调用链层层穿参——这批字段本就不是
    /// "当前生成到哪个 sub"这种逐帧变化的状态）。
    comp: BTreeMap<i32, AnchorComp>,
    /// 绑定的世界表（`None` = 未绑定，跳过一切依赖表的判据）——与 `typeck::Checker.table`
    /// 同款穿线。消费者：`set_sprite(shape, color)` 的图集三判据（`lang::atlas`）；
    /// xformdef 槽参数恒是编译期常量，故这一路总能判（不像 `fire` 还要先问是不是常量）。
    table: Option<&'p stg_core::tables::WorldTables>,
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

    /// 一条 xformdef 槽的实参：arity 校验 + 逐参编译期常量折叠。
    ///
    /// 出错时已把 `CompileError` 记进 `self.errors`，返回 `None`——调用方补一个默认槽
    /// 继续收集后续错误即可。`Op` 与 `OpFold2` 两个分支共用本函数：错误路径（span 归属、
    /// 措辞、恢复方式）只有一份，改一处不会静默分叉出两种行为。
    fn eval_slot_args(
        &mut self,
        s: &crate::lang::ast::XfSlotLit,
        arity: usize,
    ) -> Option<[i32; 2]> {
        debug_assert!(
            arity <= 2,
            "xform 槽物理上只有 2 个参数字（`XformSlot.args`），'{}' 声明了 {arity} 个",
            s.op_name
        );
        if s.args.len() != arity {
            self.err(
                s.span,
                format!(
                    "xform 操作 '{}' 期待 {arity} 个参数，实际 {}",
                    s.op_name,
                    s.args.len()
                ),
            );
            return None;
        }
        let mut args = [0i32; 2];
        let mut ok = true;
        for (i, a) in s.args.iter().enumerate() {
            match const_eval::evaluate(a, &self.consts) {
                Ok((_ty, v)) => args[i] = v,
                Err(e) => {
                    self.err(
                        s.span,
                        format!("xformdef 槽参数必须是编译期常量：{}", e.msg),
                    );
                    ok = false;
                }
            }
        }
        ok.then_some(args)
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
                        let Some(args) = self.eval_slot_args(s, arity) else {
                            built.push(XformSlot::default());
                            continue;
                        };
                        built.push(XformSlot {
                            wait: s.wait,
                            op,
                            _pad: 0,
                            args,
                        });
                        push_scratch_slots(&mut built, physical);
                    }
                    // 颜色轴糖：表层两个常量参折叠进 `args[0]`（核心只读 args[0]）。
                    Some(crate::lang::xform_map::XformOp::OpFold2(op, physical)) => {
                        let Some(vals) = self.eval_slot_args(s, 2) else {
                            built.push(XformSlot::default());
                            continue;
                        };
                        // 图集三判据（与 fire/batch 同一份谓词，`lang::atlas`）——xformdef
                        // 槽参数恒是编译期常量，故这一路无条件判得动。漏掉它，
                        // `set_sprite(BULLET_HEART, COLOR_WHITE)` 会把弹换成图集空格
                        // （有判定但看不见），写反两参也会静默换成错误的格。
                        if let Some(t) = self.table
                            && let Err(e) = crate::lang::atlas::check_shape_color(
                                t, &s.op_name, vals[0], vals[1],
                            )
                        {
                            self.err(s.span, e.msg);
                            built.push(XformSlot::default());
                            continue;
                        }
                        built.push(XformSlot {
                            wait: s.wait,
                            op,
                            _pad: 0,
                            // 未绑定表时判据被跳过，作者可以塞进任意大的数——`wrapping_add`
                            // 保证编译器不会在 debug 构建里因整数溢出 abort（编译器该以
                            // 诊断拒绝，不该崩）。绑定表时判据已把两参都钳在表内，不会绕回。
                            args: [vals[0].wrapping_add(vals[1]), 0],
                        });
                        push_scratch_slots(&mut built, physical);
                    }
                    // 部分设（颜色轴刀 T7）：表层收 1 个常量参，`args[1]` 由本趟写入绑定表的
                    // `color_stride`——引擎只做两个操作数的取模，不需要认识"颜色"这回事。
                    // 单轴判据只查值本身合法，**不查空格**（人类裁定，见 `lang::atlas` 文档）。
                    Some(crate::lang::xform_map::XformOp::OpWithStride(op, physical)) => {
                        let Some(vals) = self.eval_slot_args(s, 1) else {
                            built.push(XformSlot::default());
                            continue;
                        };
                        // stride 必须来自绑定的表——没有表就无从得知，报错而不是猜一个默认值
                        let Some(table) = self.table else {
                            self.err(
                                s.span,
                                format!(
                                    "xform 操作 '{}' 需要绑定的外观表才能确定色轴宽度",
                                    s.op_name
                                ),
                            );
                            built.push(XformSlot::default());
                            continue;
                        };
                        // 显式按 op 身份分派（复审 Minor 5 修法）：**不用 `else` 兜底**——
                        // `xform_map` 模块文档点过 `spawn_pattern` 落 `_` 兜底安静失效那颗
                        // 雷，本刀目前只有两个 `OpWithStride` 成员，但将来再加一个会静默
                        // 拿到形状判据（错判据比拒收更糟：编译通过但语义错）。两道保险：
                        // debug 就地断言炸出来，release 退化成一条明确的编译错误（不是
                        // 猜一个判据）。
                        let check = if op == xform::OP_SET_COLOR {
                            crate::lang::atlas::check_color_only(table, vals[0])
                        } else if op == xform::OP_SET_SHAPE {
                            crate::lang::atlas::check_shape_only(table, vals[0])
                        } else {
                            debug_assert!(
                                false,
                                "OpWithStride 新成员 op={op} ('{}') 未接判据分派——\
                                 补一支显式分支，别让它静默套用错误判据",
                                s.op_name
                            );
                            Err(crate::lang::atlas::ShapeColorError {
                                blame: crate::lang::atlas::Blame::Shape,
                                msg: format!(
                                    "编译器内部错误：xform 操作 '{}' 的单轴判据未接线\
                                     （op={op}）",
                                    s.op_name
                                ),
                            })
                        };
                        if let Err(e) = check {
                            self.err(s.span, e.msg);
                            built.push(XformSlot::default());
                            continue;
                        }
                        built.push(XformSlot {
                            wait: s.wait,
                            op,
                            _pad: 0,
                            args: [vals[0], i32::from(table.color_stride)],
                        });
                        push_scratch_slots(&mut built, physical);
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
                let id = self.name_to_ref[&call.name];
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
                if sub.name == "main" || sub.is_async {
                    b.end();
                } else {
                    b.raw_ret();
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
            TypedStmt::Mark { id, body } => {
                // 落点垫片降低（Task 4；整局流程刀 spec §2.1）：
                // `JMP after; landing: <Task5 注入><作者块>; after:`——正常流一跳跨过垫片，
                // 中段启动经 `EclImage::resolve_mark(id)` 直接跳进 `landing` 执行补偿块。
                let skip = b.raw_jmp();
                b.mark_here(*id); // 落点 = 垫片首指令（紧跟在 JMP 之后）
                // Task 5：自动补偿注入，直发 `push_i(值); sys(SYS_*)`（不经 `gen_expr`——
                // 值已在 `scan_mark_compensation` 折叠为编译期常量，无需再走表达式求值）。
                // 顺序固定 bgm→bg→bg_phase，且严格在作者块之前（作者手写的同类调用若
                // 存在，`comp` 里对应类已被 `scan_mark_compensation` 抑制为 `None`）。
                if let Some(c) = self.comp.get(id) {
                    for (v, sysno) in [
                        (c.bgm, syscall::SYS_BGM),
                        (c.bg, syscall::SYS_BG),
                        (c.bg_phase, syscall::SYS_BG_PHASE),
                    ] {
                        if let Some(v) = v {
                            b.push_i(v);
                            b.sys(sysno);
                        }
                    }
                }
                self.gen_block(b, loops, slots, sub, body);
                let after = b.here();
                b.patch(skip, after);
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
                let id = self.name_to_ref[&target_name];
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
        // 颜色轴糖：表层 (shape, color) 两参 → 字节码单个 appearance 值。折叠掉两位，
        // 故循环改成索引推进式（`zip(...).enumerate()` 一位一步走不了这个合并）。
        // 折叠**起点不总是 0**：`fire`/`batch` 折前两参，`sh_sprite(id, shape, color)` 折的
        // 是第 2、3 参。起点由 `builtins::fold_start` 单一权威给出（曾硬编码 `i == 0`）。
        let fold_start = builtins::fold_start(bi.name);
        let mut i = 0usize;
        while i < args.len() {
            if fold_start == Some(i) {
                match (const_val(&args[i]), const_val(&args[i + 1])) {
                    // 常量对：折成单个字面量——与手写单参字节码逐字节相同（零运行期开销）。
                    // `wrapping_add`：未绑定表时判据被跳过（`fire(i32::MAX, 1, …)` 可编），
                    // 编译器不该因整数溢出在 debug 构建里 abort——诊断拒绝可以，崩不行。
                    // 绑定表时判据已把两参钳在表内，绕不回来。
                    (Some(s), Some(c)) => b.push_i(s.wrapping_add(c)),
                    _ => {
                        let (CallArg::Val(se), CallArg::Val(ce)) = (&args[i], &args[i + 1]) else {
                            unreachable!("typeck 已保证两参糖的那两位是 Val")
                        };
                        self.gen_expr(b, slots, se);
                        self.gen_expr(b, slots, ce);
                        b.add();
                    }
                }
                i += 2;
                continue;
            }
            let (a, pk) = (&args[i], &bi.params[i]);
            match (a, pk) {
                (CallArg::Val(e), ParamKind::Val(_) | ParamKind::RawVal) => {
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
                        b.push_task_ref(Some(self.name_to_ref[name]));
                    }
                    None => b.push_task_ref(None),
                },
                _ => unreachable!("typeck 已保证 CallArg 与 ParamKind 一一对应"),
            }
            i += 1;
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
            if emits_kill_self_after(bi.name) {
                // `die()` 降低成两条指令：`SYS(SYS_DIE)` 跑死亡效果，`OP_KILL_SELF` 终止本
                // 任务（人类裁定 D-4）。这样做是因为 `syscall::dispatch` 的签名是
                // `Result<(), u8>`，没有"结束本任务"的返回通道——与其给六十个 match arm
                // 换返回类型，不如在这里发第二条指令（零 VM 改动、零 op 表改动）。
                b.raw_emit_op(stg_core::ecl::ops::OP_KILL_SELF);
            }
        }
    }
}

// ── mark 自动补偿扫描（Task 5；整局流程刀 spec §5）──────────────────────────
//
// 三类锚点 builtin 名字与下标的唯一绑定处：改名字只需要改 `anchor_kind`。

const ANCHOR_BGM: usize = 0;
const ANCHOR_BG: usize = 1;
const ANCHOR_BG_PHASE: usize = 2;

fn anchor_kind(name: &str) -> Option<usize> {
    match name {
        "bgm" => Some(ANCHOR_BGM),
        "bg" => Some(ANCHOR_BG),
        "bg_phase" => Some(ANCHOR_BG_PHASE),
        _ => None,
    }
}

/// `mark(id)` 自动补偿的一次快照（spec §5）：`bgm`/`bg`/`bg_phase` 各自"其前最近"的
/// 常量声明值——`None` 表示扫描期间未见过该类声明，或已被作者块顶层同名手写调用
/// 逐类抑制（三类互不影响）。
#[derive(Debug, Clone, Copy, Default)]
struct AnchorComp {
    bgm: Option<i32>,
    bg: Option<i32>,
    bg_phase: Option<i32>,
}

/// 裸调用的唯一常量实参值（spec §5"只捕获常量参"）：typed AST 的 `ConstRef`（`const`
/// 引用折叠出的原始值）或 `IntLit`（整型字面量）——`LocalRef`/`Binary`/`Cast`/`Call`…
/// 等一律视为"变量参"跳过，垫片处（`mark` 落点）是跳进来的落地指令，不可能重新求值
/// 一个依赖运行期状态的表达式。判定本体住 `typeck::const_val`（颜色轴 T4 起与形/色
/// 判据共用同一份实现），本函数只是"取首参"这层薄壳。
fn anchor_const_arg(args: &[CallArg]) -> Option<i32> {
    const_val(args.first()?)
}

/// mark 补偿扫描（spec §5）：沿 `main` 的**同步调用链顶层线性**展开——顶层语句按源码序
/// 走，遇到对另一个 sub 的同步调用（`TypedStmt::ExprStmtVoid` 且 `CallTarget::Sub`）就
/// 递归进该 sub 的顶层继续走（`visited` 集合防环，同一 sub 全程只走一次）；**不下潜**
/// if/while/for/loop 块体（对应语句变体落在 catch-all 分支，直接跳过，不递归其 `body`）；
/// 也不跟 `spawn`/`fire`——它们分别是 `TypedStmt::Spawn`（新任务根，不是同步调用边）与
/// 需要显式消费返回值的 `TypedStmt::ExprStmtDiscard`（`fire` 恒有返回句柄），两者都不落
/// 在本函数唯一匹配的两个语句变体（`ExprStmtVoid`/`Mark`）里，天然被排除。
///
/// 沿途记录三类锚点 builtin（`bgm`/`bg`/`bg_phase`）裸调用的"目前最新常量值 + 单调位置"
/// （位置只用来判断 `bg_phase` 是否早于最近一次 `bg`——`bg` 未声明时隐式默认位置视为
/// 0，对应 spec"bg 缺省视位置 0"）。每遇到一个 `TypedStmt::Mark`，对当前"最新值"三元组
/// 拍一次快照：`bgm`/`bg` 直接取当前最新值；`bg_phase` 仅当其记录位置严格晚于 `bg` 的
/// 记录位置才取（否则是"旧背景的段号"，弃，不注入）；再用该 `mark` 块顶层手写的锚点
/// 调用名逐类抑制（作者已经手写的那一类不再自动注入，三类各自独立判断，不影响其余两
/// 类）。`TypedStmt::Mark` 只可能出现在 `main` 顶层（typeck 已经拒绝其余位置），故本函数
/// 对每个 sub 的顶层语句一视同仁地扫描，不需要额外区分"当前是不是在 main 里"。
fn scan_mark_compensation(ti: &TypedInfo) -> BTreeMap<i32, AnchorComp> {
    let by_name: BTreeMap<&str, &TypedSub> = ti.subs.iter().map(|s| (s.name.as_str(), s)).collect();
    let mut out = BTreeMap::new();
    let Some(&main) = by_name.get("main") else {
        return out;
    };
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    visited.insert("main");
    // latest[ANCHOR_*] = (值, 单调位置)；位置只在 pos+=1 时前进，仅由锚点声明本身推进。
    let mut latest: [Option<(i32, u32)>; 3] = [None; 3];
    let mut pos: u32 = 0;
    walk_mark_scan(
        main,
        &by_name,
        &mut visited,
        &mut pos,
        &mut latest,
        &mut out,
    );
    out
}

fn walk_mark_scan<'a>(
    sub: &'a TypedSub,
    by_name: &BTreeMap<&'a str, &'a TypedSub>,
    visited: &mut BTreeSet<&'a str>,
    pos: &mut u32,
    latest: &mut [Option<(i32, u32)>; 3],
    out: &mut BTreeMap<i32, AnchorComp>,
) {
    for stmt in &sub.body {
        match stmt {
            TypedStmt::ExprStmtVoid { call } => match &call.target {
                CallTarget::Builtin(_) => {
                    if let Some(k) = anchor_kind(&call.name)
                        && let Some(v) = anchor_const_arg(&call.args)
                    {
                        *pos += 1;
                        latest[k] = Some((v, *pos));
                    }
                }
                CallTarget::Sub => {
                    if let Some(&callee) = by_name.get(call.name.as_str())
                        && visited.insert(callee.name.as_str())
                    {
                        walk_mark_scan(callee, by_name, visited, pos, latest, out);
                    }
                }
            },
            TypedStmt::Mark { id, body } => {
                let bg_pos = latest[ANCHOR_BG].map_or(0, |(_, p)| p);
                let mut comp = AnchorComp {
                    bgm: latest[ANCHOR_BGM].map(|(v, _)| v),
                    bg: latest[ANCHOR_BG].map(|(v, _)| v),
                    bg_phase: latest[ANCHOR_BG_PHASE]
                        .filter(|&(_, p)| p > bg_pos)
                        .map(|(v, _)| v),
                };
                // 作者块顶层手写了同名锚点 builtin 裸调用 → 该类不注入（逐类独立判断，
                // 只看块顶层——同 scan 本身"不下潜"的纪律一致，块内嵌套控制流不查）。
                for s in body {
                    if let TypedStmt::ExprStmtVoid { call } = s
                        && matches!(call.target, CallTarget::Builtin(_))
                    {
                        match anchor_kind(&call.name) {
                            Some(ANCHOR_BGM) => comp.bgm = None,
                            Some(ANCHOR_BG) => comp.bg = None,
                            Some(ANCHOR_BG_PHASE) => comp.bg_phase = None,
                            _ => {}
                        }
                    }
                }
                out.insert(*id, comp);
            }
            _ => {} // if/while/for/loop/wait/spawn/… 不下潜（spec §5"顶层线性"）。
        }
    }
}

/// codegen 趟入口：`TypedInfo`/`SlotMap`/原始 `Program`（xformdef 常量折叠用）→
/// `EclImage`。所有 sub 先声明再生成；Builder 按名字确定最终 ABI。
///
/// `table`（颜色轴 T4 复审修）——绑定的世界表，同 `typeck::check` 的同名参数：`None` =
/// 未绑定，跳过一切依赖表的判据。本趟唯一消费者是 xformdef `set_sprite(shape, color)`
/// 的图集三判据（`lang::atlas`）；`fire`/`batch` 的同款判据在 typeck 就已施加过，本趟
/// 只负责折叠。
pub fn generate(
    prog: &Program,
    ti: &TypedInfo,
    sm: &SlotMap,
    content_hash: u64,
    table: Option<&stg_core::tables::WorldTables>,
) -> Result<EclImage, Vec<CompileError>> {
    let mut ib = ImageBuilder::new();
    let mut name_to_ref = BTreeMap::new();
    for sub in &ti.subs {
        let kind = if sub.name == "main" {
            SubKind::Root
        } else if sub.is_async {
            SubKind::Async
        } else {
            SubKind::CallOnly
        };
        let params: Vec<EclValueType> = sub
            .params
            .iter()
            .map(|(_, ty)| match ty {
                crate::lang::ast::Ty::Int => EclValueType::Int,
                crate::lang::ast::Ty::Fx => EclValueType::Fx,
                crate::lang::ast::Ty::Angle => EclValueType::Angle,
            })
            .collect();
        let span = prog
            .subs
            .iter()
            .find(|source| source.name == sub.name)
            .map_or(Span { line: 1, col: 1 }, |source| source.span);
        let id = ib
            .declare_sub(&sub.name, kind, &params)
            .map_err(|error| image_error_at(span, error))?;
        name_to_ref.insert(sub.name.clone(), id);
    }
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
    let consts: BTreeMap<String, (Ty, i32)> = ti
        .consts
        .iter()
        .map(|(n, t, v)| (n.clone(), (*t, *v)))
        .collect();
    let xformdefs: BTreeMap<String, &crate::lang::ast::XformDef> =
        prog.xformdefs.iter().map(|x| (x.name.clone(), x)).collect();
    let comp = scan_mark_compensation(ti);

    let mut g = Gen {
        sm,
        xformdefs,
        name_to_ref,
        sub_params,
        consts,
        comp,
        table,
        errors: Vec::new(),
    };

    for sub in &ti.subs {
        let slots = &sm.subs[&sub.name];
        let mut b = SubBuilder::new();
        g.gen_xformdef_staging(&mut b, slots, sub);
        let mut loops: Vec<LoopCtx> = Vec::new();
        g.gen_block(&mut b, &mut loops, slots, sub, &sub.body);
        if sub.name == "main" || sub.is_async {
            b.end();
        } else {
            b.raw_ret();
        }
        ib.define_sub(g.name_to_ref[&sub.name], b)
            .map_err(|error| image_error_at(Span { line: 1, col: 1 }, error))?;
    }

    if g.errors.is_empty() {
        ib.build(content_hash)
            .map_err(|error| image_error_at(Span { line: 1, col: 1 }, error))
    } else {
        Err(g.errors)
    }
}

fn image_error_at(span: Span, error: ImageBuildError) -> Vec<CompileError> {
    vec![CompileError {
        line: span.line,
        col: span.col,
        msg: format!("image build error: {error:?}"),
        src_line: String::new(),
    }]
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
    use stg_core::input::InputFrame;
    use stg_core::step::{World, step};
    use stg_core::tables::TABLES_V0;

    const FREE: u16 = 20; // 自由段全局槽起点（≥ GLOBALS_SYS_SEGMENT）

    /// 编译 + 跑 N 帧，返回跑完后的 `World`（供调用方读 `globals`/`bullets`/`diag`）。
    fn run(src: &str, frames: u32) -> Box<World> {
        let image = compile(src, "e2e.ecl").unwrap_or_else(|e| panic!("编译失败：{e:?}"));
        let mut w = World::new(1);
        w.start_main(&image).expect("main 应能派生");
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
        assert_eq!(w.body.view().globals()[20], 10, "true 分支应落地 then");
        assert_eq!(w.body.view().globals()[21], 20, "false 分支应落地 else");
        assert_eq!(w.body.view().diag().task_faults, 0);
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
        assert_eq!(w.body.view().globals()[20], 5);
        assert_eq!(w.body.view().diag().task_faults, 0);
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
        assert_eq!(w.body.view().globals()[20], 18);
        assert_eq!(w.body.view().diag().task_faults, 0);
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
        assert_eq!(
            w.body.view().diag().task_faults,
            0,
            "左假应短路，右操作数不该求值"
        );
        assert_eq!(
            w.body.view().globals()[20],
            1,
            "短路后应正常继续执行到之后的语句"
        );
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
            w.body.view().diag().task_faults,
            1,
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
        assert_eq!(
            w.body.view().diag().task_faults,
            0,
            "左真应短路，右操作数不该求值"
        );
        assert_eq!(w.body.view().globals()[20], 1);
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
            w.body.view().diag().task_faults,
            1,
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
            [
                w.body.view().globals()[20],
                w.body.view().globals()[21],
                w.body.view().globals()[22]
            ],
            [11, 22, 33]
        );
        assert_eq!(w.body.view().diag().task_faults, 0);
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
        assert_eq!(w.body.view().globals()[20], 33);
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    /// `fire(...)` 的 `task` 参数（`CallArg::SubRef`）：与 `xf` 参数（`CallArg::XformRef`）
    /// 是独立的 codegen 分支——`xformdef_turn_changes_bullet_trajectory_survival` 只覆盖了
    /// `xf`，这里单独钉 `task`：挂在新弹上的 async sub 应经真实调度（次帧首跑）执行。
    #[test]
    fn fire_task_script_attaches_and_runs_async_sub_on_new_bullet() {
        let src = "sub main() {\n\
                     _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, none, on_bullet);\n\
                     wait(1000);\n\
                   }\n\
                   async sub on_bullet() {\n\
                     set_global(20, 1);\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（fire 挂任务，born_frame=1）；2=挂载任务首跑。
        let w = run(src, 3);
        assert_eq!(w.body.view().globals()[20], 1);
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    // ── xformdef TURN：轨迹差分观测（模块文档）────────────────────────

    #[test]
    fn xformdef_turn_changes_bullet_trajectory_survival() {
        // 无转向：沿 +x 飞出 [-256,256] 边界（200 + 4*15 = 260 > 256）。
        let no_turn = "sub main() {\n\
                         _ = fire(0, 0, 200fx, 100fx, 4.0fx, 0deg, none, none);\n\
                         wait(1000);\n\
                       }";
        let w1 = run(no_turn, 20);
        assert_eq!(
            w1.body.view().bullets().iter_alive().count(),
            0,
            "未转向应沿 x 轴飞出边界被回收"
        );

        // TURN +90°（16384 BAM，同帧生效——`wait=0`）：转向沿 +y（下）飞，同一时窗内
        // y ∈ [-64,512] 远未触边，应仍存活。
        let with_turn = "xformdef RING { turn(90deg); }\n\
                          sub main() {\n\
                            _ = fire(0, 0, 200fx, 100fx, 4.0fx, 0deg, RING, none);\n\
                            wait(1000);\n\
                          }";
        let w2 = run(with_turn, 20);
        assert_eq!(
            w2.body.view().bullets().iter_alive().count(),
            1,
            "转向后应改沿 y 轴飞，同一时窗内不出界，应存活——\
             反证 xformdef staging → fire xf 解析 → OP_TURN 派发链路生效"
        );
        assert_eq!(w2.body.view().diag().task_faults, 0);
    }

    /// 颜色轴糖（xformdef 侧）：`set_sprite(shape, color)` 两参必须折叠进 **`args[0]`**
    /// ——核心 `OP_SET_SPRITE` 只读 `args[0]`（`world/transform.rs`），若按普通双参 op
    /// 让颜色落进 `args[1]`，颜色会被无声丢弃、弹变成该形的 0 号色。
    /// 判别力所在：断言 `sprite == 16 + 3 == 19`，若丢色则是 `16`，若丢形则是 `3`。
    #[test]
    fn set_sprite_folds_shape_and_color_into_args0() {
        let src = "xformdef RECOLOR { set_sprite(16, 3); }\n\
                   sub main() {\n\
                     _ = fire(0, 0, 0fx, 0fx, 0fx, 0deg, RECOLOR, none);\n\
                     wait(1000);\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（fire 建弹）；2=相位 4 跑完 wait=0 的槽。
        let w = run(src, 3);
        let view = w.body.view();
        let p = view.bullets();
        let i = p.iter_alive().next().expect("应有一颗弹");
        assert_eq!(
            p.sprite()[i],
            19,
            "set_sprite(16, 3) 必须折成单个 id 19（丢色会是 16，丢形会是 3）"
        );
        assert_eq!(view.diag().task_faults, 0);
    }

    /// 部分设（颜色轴刀 T7）端到端穿线判别（复审必修一修法）：`gen_xformdef_staging` 的
    /// `OpWithStride` 分支把**绑定表**的 `color_stride` 写进 `args[1]`（`codegen.rs`
    /// `args: [vals[0], i32::from(table.color_stride)]` 那一行）——这是整个方案的枢纽，
    /// 但此前只有引擎侧单测（手搓槽、stride 是测试自己写的字面量）和编译器侧"编译过/
    /// 报错含某字样"两头断开的测试，从没有一条测试真正跑真实 `.ecl` 源码、编译、跑真
    /// `World`、断言产出的槽让 sprite 落在正确值上。若 `:295` 那一行被改成
    /// `args: [vals[0], 0]`（stride 丢失）或写反两参，此前**全部**测试仍绿，而真实
    /// `set_color`/`set_shape` 会在运行期静默 no-op（stride<=0 分支）——正是 CLAUDE.md
    /// 点名"金向量守不住、只能靠单测"的行为回归。
    ///
    /// `fire(32, 5, …)` 折叠出的初始 sprite = 32+5 = 37（第 2 形第 5 色）；
    /// `set_color(9)` 只应改色 → 2×16+9 = 41（stride 丢失/为 0 会停在 37）。
    #[test]
    fn set_color_op_threads_bound_table_stride_into_args1_end_to_end() {
        let src = "xformdef X { set_color(9); }\n\
                   sub main() {\n\
                     _ = fire(32, 5, 0fx, 0fx, 0fx, 0deg, X, none);\n\
                     wait(1000);\n\
                   }";
        let w = run(src, 3);
        let view = w.body.view();
        let p = view.bullets();
        let i = p.iter_alive().next().expect("应有一颗弹");
        assert_eq!(
            p.sprite()[i],
            41,
            "set_color(9) 只应改色 → 2*16+9=41（stride 丢失/为 0 会停在 37，不会是 41）"
        );
        assert_eq!(view.diag().task_faults, 0);
    }

    /// 同上，`set_shape` 一侧（两个新 op 各补一条，别只补一边）。
    /// `set_shape(48)` 只应改形 → 48 + (37%16=5) = 53（stride 丢失/为 0 会停在 37）。
    #[test]
    fn set_shape_op_threads_bound_table_stride_into_args1_end_to_end() {
        let src = "xformdef X { set_shape(48); }\n\
                   sub main() {\n\
                     _ = fire(32, 5, 0fx, 0fx, 0fx, 0deg, X, none);\n\
                     wait(1000);\n\
                   }";
        let w = run(src, 3);
        let view = w.body.view();
        let p = view.bullets();
        let i = p.iter_alive().next().expect("应有一颗弹");
        assert_eq!(
            p.sprite()[i],
            53,
            "set_shape(48) 只应改形 → 48+(37%16)=53（stride 丢失/为 0 会停在 37，不会是 53）"
        );
        assert_eq!(view.diag().task_faults, 0);
    }

    /// STEP 族 scratch 自动补槽的行为学判别（T3 复审 Important 修法）：
    /// `step_speed` 物理双槽——编译器不补 scratch 时，紧随其后的 `set_life(1)` 会落在
    /// scratch 槽位、被引擎运行期覆写而**永不执行**（弹永生）；补了则 set_life 照常
    /// 生效（弹快速回收）。本测试在"补"分支断言弹死——把足枪钉死成可红命题。
    #[test]
    fn step_op_auto_scratch_keeps_following_slot_alive() {
        let src = "xformdef S { step_speed(2.0fx, 4); set_life(1); }\n\
                    sub main() {\n\
                      _ = fire(0, 0, 0fx, 100fx, 0.5fx, 0deg, S, none);\n\
                      wait(1000);\n\
                    }";
        let w = run(src, 20);
        assert_eq!(
            w.body.view().bullets().iter_alive().count(),
            0,
            "set_life(1) 必须在 STEP 的 scratch 槽之后照常执行——弹应已回收；\
             若此断言红 = scratch 未自动补，set_life 被引擎 scratch 覆写"
        );
        assert_eq!(w.body.view().diag().task_faults, 0);
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
            w.body.view().globals()[20],
            5,
            "int→fx→int 精确往返（整数无小数损失）"
        );
        assert_eq!(w.body.view().globals()[21], 1000, "int↔angle 位穿透往返");
        assert_eq!(w.body.view().diag().task_faults, 0);
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
        assert_eq!(w.body.view().globals()[FREE as usize], 1);
        assert_eq!(w.body.view().diag().task_faults, 0);
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
                      _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, RING, none);\n\
                      var i: int = 0;\n\
                      while i < 3 { i = i + 1; }\n\
                    }";
        let img1 = compile(src, "det.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        let img2 = compile(src, "det.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(img1, img2);
    }

    // ── `wait_spell()` 语法糖：判别测试"糖=纯展开"────────────────────────

    /// `wait_spell();` 与手写的等价 `while spell_timer() >= 0 { wait(1); }` 必须编译出
    /// **逐字节相同**的 `EclImage`——这是"纯前端展开、codegen 无感"的判别式证据（不是
    /// "行为大致相同"，是字节级相同：`EclImage` 的 `PartialEq` 覆盖全部段，两者的唯一
    /// 差异只应是源码文本本身，折叠进 AST 后不留痕迹）。
    #[test]
    fn wait_spell_sugar_compiles_to_identical_bytecode_as_hand_written_while() {
        let sugared = "sub main() {\n\
                         wait_spell();\n\
                       }";
        let hand_written = "sub main() {\n\
                              while spell_timer() >= 0 { wait(1); }\n\
                            }";
        let img_sugar = compile(sugared, "e2e.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        let img_hand = compile(hand_written, "e2e.ecl").unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            img_sugar, img_hand,
            "wait_spell() 应与手写等价 while 编译出逐字节相同的 EclImage"
        );
    }

    // ── 符卡表层端到端：spell_begin(pattern) + wait_spell() ──────────────

    /// 端到端全链路：`spell_begin` 起模式（`SubRef` 走 fire 同款 codegen 通道）→ 卡活期间
    /// 模式任务运行（`globals[20]` 逐帧递增可观测）→ 到时限（普通卡，`flags=0`）超时结算
    /// （`EVT_SPELL_FAILED`，reason=2；矩阵见 spec §4：普通卡超时恒 FAILED，不看资格）
    /// → 模式任务随卡死（`spell_bound` 调度门禁，结算后 `globals[20]` 不再增长——"模式随卡
    /// 生死"的行为学判别，`fire` 挂弹任务不继承这条规则的对照见 `stg-core::ecl::vm` 测试）
    /// → `wait_spell()` 糖循环正确退出，`main` 恢复执行后续语句（`globals[21]` 落地）。
    ///
    /// 需要 owner=ENEMY（`spell_begin`/`spell_timer` misuse 策略：非敌 owner 直接 Fault/押
    /// -1），故不用本文件的 `run()` 助手（恒 `start_main`/STAGE owner）——照 harness
    /// `build_rainbow_world` 的样板手搭一个 boss 敌 + `start_main_with_owner`。
    #[test]
    fn spell_begin_pattern_bound_lifecycle_and_wait_spell_e2e() {
        use stg_core::ecl::binding::EclOwner;
        use stg_core::enemy::EnemyInit;
        use stg_core::events::{EVT_SPELL_DECLARED, EVT_SPELL_FAILED};

        // 复审 Minor 强化（Task 3 复审修）：`p()` 每帧真开一发弹（`fire(...)`），不再只是
        // 计数器自增——生命周期断言不能只靠 globals 代理"模式在跑"，要能看见真实的弹幕
        // 输出。速度取 0.5fx、原点 (0,0)：60 帧时限内最远飞行 30 单位，远小于场界半宽
        // （见 `xformdef_turn_changes_bullet_trajectory_survival` 同款边界常量），保证
        // 观测窗口内弹不会因出界被回收，count 能稳定 > 0。
        let src = "async sub p() {\n\
                     loop {\n\
                       var c: int = global(20);\n\
                       set_global(20, c + 1);\n\
                       _ = fire(0, 0, 0fx, 0fx, 0.5fx, 0deg, none, none);\n\
                       wait(1);\n\
                     }\n\
                   }\n\
                   sub main() {\n\
                     spell_begin(0, 5, p, 60, 1000, 0, 0);\n\
                     wait_spell();\n\
                     set_global(21, 777);\n\
                   }";
        let image = compile(src, "spell_e2e.ecl").unwrap_or_else(|e| panic!("编译失败：{e:?}"));
        let mut w = World::new(1);
        let boss = w.body.create_enemy(EnemyInit {
            x: stg_core::math::Fx::ZERO,
            y: stg_core::math::Fx::ZERO,
            vx: stg_core::math::Fx::ZERO,
            vy: stg_core::math::Fx::ZERO,
            speed: stg_core::math::Fx::ZERO,
            angle: stg_core::math::Angle::ZERO,
            vel_from_0: 0,
            vel_from_1: 0,
            vel_to_0: 0,
            vel_to_1: 0,
            vel_t: 0,
            vel_dur: 0,
            vel_easing: 0,
            vel_active: 0,
            vel_space: 0,
            vel_touched: 0,
            mv_from_x: stg_core::math::Fx::ZERO,
            mv_from_y: stg_core::math::Fx::ZERO,
            mv_to_x: stg_core::math::Fx::ZERO,
            mv_to_y: stg_core::math::Fx::ZERO,
            mv_t: 0,
            mv_dur: 0,
            mv_easing: 0,
            mv_active: 0,
            hp: 1000,
            hp_max: 1000,
            radius: stg_core::math::Fx::from_int(12),
            hurtbox: stg_core::math::Fx::from_int(16),
            invuln: 0,
            hit_flash: 0,
            flags: 0,
            sprite: 0,
            anm_state: 0,
            main_task: 0,
            death_script: 0,
            drop_count: [0; stg_core::items::ITEM_TYPE_COUNT],
            score: 100,
        });
        w.start_main_with_owner(&image, EclOwner::Enemy(boss))
            .expect("main 应能以 enemy owner 派生（新镜像/新池，容量均未耗尽）");

        let mut declared = false;
        let mut end_frame: Option<u32> = None;
        let mut end_data: Option<[i32; 2]> = None;
        let mut history: Vec<i32> = Vec::with_capacity(100);
        let mut bullet_counts: Vec<usize> = Vec::with_capacity(100);
        for f in 0..100u32 {
            step(&mut w, &TABLES_V0, &image, &InputFrame::empty(f));
            for ev in w.body.frame_events() {
                if ev.kind == EVT_SPELL_DECLARED {
                    declared = true;
                    assert_eq!(ev.data, [5, 1000], "DECLARED 事件应带 [spell_id, bonus0]");
                }
                if ev.kind == EVT_SPELL_FAILED && end_frame.is_none() {
                    end_frame = Some(f);
                    end_data = Some(ev.data);
                }
            }
            history.push(w.body.view().globals()[20]);
            bullet_counts.push(w.body.view().bullets().iter_alive().count());
        }
        assert!(
            declared,
            "应在某帧观察到 EVT_SPELL_DECLARED（spell_begin 成功宣告）"
        );
        assert!(
            history.iter().any(|&c| c > 0),
            "模式任务 p 应已起跑（globals[20] 应递增过至少一次）"
        );
        let end_frame = end_frame
            .expect("60 帧时限内应观察到 EVT_SPELL_FAILED（普通卡超时恒 FAILED）")
            as usize;
        assert_eq!(
            end_data.unwrap(),
            [5, 2],
            "reason=2（超时）——普通卡（flags=0）超时恒 FAILED，不看资格"
        );
        // 复审 Minor 强化：模式任务真开了弹（不只是计数器动）——卡活跃期间（结算帧之前）
        // 应观察到存活弹数 > 0，直接证明 `windchime_pattern` 风格"宣言 + fire" 的行为，
        // 不是靠 globals 计数代理"起弹"这一层间接证据。
        assert!(
            bullet_counts[..end_frame].iter().any(|&c| c > 0),
            "符卡活跃期间应观察到真实存活弹数 > 0（模式 p 每帧 fire）：{bullet_counts:?}"
        );
        let counter_at_end = history[end_frame];
        assert!(
            history[(end_frame + 1)..]
                .iter()
                .all(|&c| c == counter_at_end),
            "模式任务应随卡死（spell_bound 调度门禁）：收卡结算后 globals[20] 不应再增长，\
             实际历史：{history:?}（结算帧={end_frame}）"
        );
        assert_eq!(
            w.body.view().globals()[21],
            777,
            "wait_spell() 糖应在卡结束后正确退出循环，main 恢复执行后续语句"
        );
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    // ── spawn_enemy task 参（A5 乙案；task-1-brief.md）─────────────────────

    /// `spawn_enemy(...)` 尾追的 `task` 参数（`CallArg::SubRef`）走 `fire`/`spell_begin`
    /// 同款 codegen 通道（sub 名标识符 / `none`，编译期解析，不参与求值型检查）——钉编译面：
    /// 标识符/none 两形态都应通过。语义端到端断言在 `stg-core::ecl::syscall` 测试；这里只钉
    /// "能编译通过"，同 `fire_task_script_attaches_and_runs_async_sub_on_new_bullet` 先例。
    #[test]
    fn spawn_enemy_task_param_lowers_like_fire() {
        let src = "async sub boss_main() {\n\
                     loop { wait(60); }\n\
                   }\n\
                   sub main() {\n\
                     _ = spawn_enemy(0.0fx, 96.0fx, 100, 1, 500, 3, boss_main);\n\
                     _ = spawn_enemy(1.0fx, 2.0fx, 10, 0, 0, 0, none);\n\
                   }";
        let _image = compile(src, "spawn_enemy_task.ecl")
            .unwrap_or_else(|e| panic!("7 参 spawn_enemy 应编译通过：{e:?}"));
    }

    /// `task` 位不收求值表达式——语法上必须是裸标识符 / `none`（同 `fire` 的 xf/task 位、
    /// `spell_begin` 的 `pattern` 位一脉；typeck 侧判据见 `lang::typeck::tests`）。
    #[test]
    fn spawn_enemy_task_param_rejects_value_expr() {
        let src = "sub main() { _ = spawn_enemy(0.0fx, 0.0fx, 1, 0, 0, 0, 1 + 2); }";
        assert!(compile(src, "bad.ecl").is_err());
    }

    /// A5 乙案 e2e（task-1 复审 Important-3 附加）：sprite/task 两位真实端到端连通——
    /// sprite 落新敌池（判别值 3，防 params 表 sprite/task 两位对调静默错位），task 挂的
    /// async sub 真实经调度执行（同 `fire_task_script_attaches_and_runs_async_sub_on_new_bullet`
    /// 先例）。本 crate P1 边界不摸 `TaskPool` 内部字段（见模块文档"观测口"），owner 三元组
    /// 绑定的核侧断言见 `stg-core::ecl::syscall::tests::spawn_enemy_with_task_binds_owner_and_main_task`。
    ///
    /// `on_enemy` 写完 global 后 `loop { wait(1); }`——D9 落地后主协程自然返回会自燃 owner
    /// （`ecl::vm::run_tasks` 的 `Exec::End` 分支），若这里让脚本直接 return，本测试第 3 帧
    /// 断言"敌还活着"时敌已被相位 9 回收，与本测试意图（sprite/task 接线）无关却会误红。
    #[test]
    fn spawn_enemy_sprite_and_task_wire_correctly_end_to_end() {
        let src = "sub main() {\n\
                     _ = spawn_enemy(0.0fx, 96.0fx, 100, 1, 500, 3, on_enemy);\n\
                     wait(1000);\n\
                   }\n\
                   async sub on_enemy() {\n\
                     set_global(20, 1);\n\
                     loop { wait(1); }\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（spawn_enemy 挂任务，born_frame=1）；
        // 2=挂载任务首跑。
        let w = run(src, 3);
        let enemies = w.body.view().enemies();
        let idx = enemies
            .iter_alive()
            .next()
            .expect("main 首跑后应已造出一只敌");
        assert_eq!(
            enemies.sprite()[idx],
            3,
            "sprite 位应落到新敌（防 params 表 sprite/task 两位对调错位）"
        );
        assert_eq!(
            w.body.view().globals()[20],
            1,
            "task 位挂的 async sub 应真实调度执行"
        );
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    // ── 错误路径：xformdef 参数非编译期常量 ─────────────────────────────

    #[test]
    fn xformdef_non_const_arg_is_a_compile_error() {
        let src = "xformdef BAD { turn($frame); }\n\
                    sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, BAD, none); }";
        let errors = match compile(src, "bad.ecl") {
            Err(e) => e,
            Ok(_) => panic!("期望编译失败（xformdef 参数非常量）"),
        };
        assert!(
            errors.iter().any(|e| e.msg.contains("编译期常量")),
            "{errors:?}"
        );
    }

    /// C14 收编附带能力：xformdef 槽参数改走 `const_eval::evaluate` 后，不再局限于
    /// 字面量/const 引用/一元负号（旧 `eval_const_arg` 的能力上限），支持完整的二元
    /// const 算术表达式。
    #[test]
    fn xformdef_slot_arg_accepts_binary_const_expr() {
        // 收编后 xformdef 槽参数走 const_eval，支持二元算术（原 eval_const_arg 只字面量/一元负）
        let src = "const A: int = 2;\n\
                   xformdef OK { turn(A + 1); }\n\
                   sub main() { _ = fire(0, 0, 0fx, 0fx, 1.0fx, 0deg, OK, none); loop { wait(1); } }";
        assert!(
            compile(src, "ok.ecl").is_ok(),
            "二元 const 表达式应被 xformdef 槽参数接受"
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
        let ti = crate::lang::typeck::check(&prog, &[], None).expect("判型失败");
        let mut sm = crate::lang::slots::allocate(&prog, &ti).expect("槽分配失败");
        *sm.subs
            .get_mut("main")
            .unwrap()
            .locals
            .get_mut("x")
            .unwrap() = 999;
        let _ = generate(&prog, &ti, &sm, 0, None);
    }

    // ── 通道 B `emit_req` 表层端到端 ─────────────────────────────────────

    /// 通道 B 表层端到端：字面量折叠（1.5fx→98304、90deg→16384）叠加 RawVal 三型 raw 直通、
    /// RawVal 位表达式求值（2+3→5），各位判别值防错位假绿。帧序：第 0 帧是 main 出生帧
    /// （跳过），emit 落在第 1 帧（main 首跑）内——begin 每帧清缓冲，故恰步 2 帧后读。
    #[test]
    fn emit_req_rawval_literal_folding_reaches_channel_b() {
        let src = "sub main() {\n\
                     emit_req(64, 1.5fx, -3, 90deg, 2 + 3, 0, 0);\n\
                     wait(10);\n\
                   }";
        let w = run(src, 2);
        let reqs = w.body.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!((reqs[0].id, reqs[0].seq), (64, 0));
        assert_eq!(reqs[0].args, [98304, -3, 16384, 5, 0, 0]);
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    // ── 敌人死亡效果：`die()` 的两指令降低（T3；人类裁定 D-4）────────────────

    /// D-4 判别腿：`die()` 立即终止调用它的任务——后续语句不执行。
    ///
    /// **必须走真编译产物**：`die()` 降低成 `SYS(SYS_DIE)` + `OP_KILL_SELF` 两条指令是
    /// **编译器**的事，手拼字节码时作者自己会记得发 `OP_KILL_SELF`，那就测不到"编译器
    /// 有没有发它"。故本测试住这里（有编译 + 跑真世界的现成脚手架），不住 `syscall.rs`。
    ///
    /// 两条断言缺一不可：
    /// - `globals[21] != 777` —— 后续语句没跑（若 codegen 漏发 `OP_KILL_SELF`，任务继续
    ///   执行到 `set_global`，这条立刻红）；
    /// - 通道 B 有 `REQ_ENEMY_DEATH` —— `die()` 本身真生效了，排除"根本没执行到 die"的
    ///   假绿。**不能用"敌没了"当判据**：`on_enemy` 是这只敌的 main_task，即便 `die()`
    ///   从未执行，主协程自然返回也会触发 D9 自燃把敌收走——但自燃是**静默退场**，
    ///   不掉道具不加分**不发这条请求**，故请求的有无恰好把两条路径分开。
    #[test]
    fn die_terminates_the_calling_task_immediately() {
        let src = "sub main() {\n\
                     _ = spawn_enemy(0.0fx, 96.0fx, 100, 1, 500, 3, on_enemy);\n\
                     loop { wait(1); }\n\
                   }\n\
                   async sub on_enemy() {\n\
                     die();\n\
                     set_global(21, 777);\n\
                   }";
        // 帧序：0=main 出生跳过；1=main 首跑（spawn_enemy 挂 on_enemy）；2=on_enemy 首跑。
        let w = run(src, 3);
        assert_ne!(
            w.body.view().globals()[21],
            777,
            "die() 之后的语句不得执行（codegen 漏发 OP_KILL_SELF 就会执行）"
        );
        assert_eq!(w.body.view().diag().task_faults, 0);
        let reqs = w.body.take_requests();
        assert!(
            reqs.iter()
                .any(|r| r.id == stg_core::consts::REQ_ENEMY_DEATH),
            "die() 本身必须生效（D9 自燃是静默退场、不发 REQ_ENEMY_DEATH）"
        );
    }

    /// 小清洗刀（110/140/141）：三条新内建从 `.ecl` 源码一路走到真 VM——本测试证的是**通电**
    /// （typeck 认这三个名字 + codegen 发得出 `OP_SYS 110/140/141` + 派发臂接得住），
    /// 数值判别力由 `syscall.rs` 侧那三条判别腿承担（参数序 / 真开根 / 真取最近）。
    ///
    /// `nearest_enemy` 这条尤其要走真链路：它的世界实现自 M0-13 起就有、也一直有测试绿着，
    /// **绿的一直是死代码**——只有"脚本能调到"才是本刀新增的东西。
    #[test]
    fn atan2_dist_and_nearest_enemy_are_reachable_from_ecl_source() {
        let src = "sub main() {\n\
                     var e: int = spawn_enemy(30.0fx, 40.0fx, 100, 0, 0, 3, none);\n\
                     set_global(20, atan2(1.0fx, 0.0fx) as int);\n\
                     set_global(21, dist(3.0fx, 4.0fx) as int);\n\
                     set_global(22, nearest_enemy(31.0fx, 41.0fx));\n\
                     set_global(23, e);\n\
                     loop { wait(1); }\n\
                   }";
        let w = run(src, 2);
        let g = w.body.view().globals();
        assert_eq!(g[20], 16384, "atan2(y=1, x=0) = +90° = BAM 16384");
        assert_eq!(
            g[21], 5,
            "dist(3,4) = 5.0fx，`as int` 截断后仍是 5（不是 25）"
        );
        assert!(g[23] >= 0, "敌应建成（否则下一条断言退化成 -1 == -1 假绿）");
        assert_eq!(g[22], g[23], "查询点就在那只敌旁边，返的就是它的池 index");
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    /// 敌坐标读口刀（syscall 101/102）**存在的全部理由**：把上一刀留下的断头路接上——
    /// 脚本拿得到敌号（`nearest_enemy`）却读不到它的坐标，于是"查最近的敌 → 朝它开火"
    /// 算不出角度。本条走**真编译 + 真 VM** 的 `.ecl` 源码，一路
    /// `nearest_enemy → 探活 → enemy_x/enemy_y → atan2 → fire`，落到弹池里那颗弹的角度上。
    ///
    /// 为什么必须是 e2e：`enemy_x` 的世界侧单测绿证明不了"脚本够得着"——`nearest_enemy`
    /// 的世界实现自 M0-13 就有测试一直绿着，绿的却是**没人能调的死代码**，上一刀才发现。
    ///
    /// 判别力：敌放在 **x ≠ y** 的 (60, −80)，故"两条读口写反"会让 `atan2` 落到
    /// 另一个象限——`swapped` 那条 `assert_ne!` 就是把这个变异钉死的靶子。
    #[test]
    fn nearest_enemy_to_enemy_pos_to_atan2_to_fire_is_wired_end_to_end() {
        use stg_core::math::{Fx, cordic};
        let src = "sub main() {\n\
                     var e: int = spawn_enemy(60.0fx, -80.0fx, 100, 0, 0, 3, none);\n\
                     set_global(20, e);\n\
                     var n: int = nearest_enemy(64.0fx, -84.0fx);\n\
                     set_global(21, n);\n\
                     if enemy_hp(n) != -1 {\n\
                       var ex: fx = enemy_x(n);\n\
                       var ey: fx = enemy_y(n);\n\
                       set_global(22, ex as int);\n\
                       set_global(23, ey as int);\n\
                       _ = fire(0, 0, 0.0fx, 0.0fx, 3.0fx, atan2(ey, ex), none, none);\n\
                     }\n\
                     loop { wait(1); }\n\
                   }";
        let w = run(src, 2);
        let view = w.body.view();
        let g = view.globals();
        assert!(g[20] >= 0, "敌应建成（否则后面几条断言全退化成假绿）");
        assert_eq!(g[21], g[20], "nearest_enemy 返的就是刚建的那只敌");
        assert_eq!(g[22], 60, "enemy_x 读的是 x=60（读成 y 会是 -80）");
        assert_eq!(g[23], -80, "enemy_y 读的是 y=-80（读成 x 会是 60）");

        let expect = cordic::atan2(Fx::from_int(-80), Fx::from_int(60));
        let swapped = cordic::atan2(Fx::from_int(60), Fx::from_int(-80));
        assert_ne!(expect, swapped, "x≠y 的位置才让'两条读口写反'可辨");
        let p = view.bullets();
        let i = p
            .iter_alive()
            .next()
            .expect("链路接通 ⇒ 必须真的开出一颗弹");
        assert_eq!(
            p.angle()[i],
            expect,
            "弹的角度 = atan2(敌 y, 敌 x)，即真的朝那只敌打"
        );
        assert_eq!(view.diag().task_faults, 0);
    }

    /// 探活读口刀（syscall 103）：同一条链路换成 **`enemy_alive(n) == 1`** 探活。
    ///
    /// **与上一条并存、不是替换**——上一条用旧探针 `enemy_hp(n) != -1` 走同一条链路，
    /// 两条同时绿正好证明新旧判据在**正常路径上等效**（新口只在"活敌血量恰为 −1"那一格
    /// 与旧探针分岔，那一格由 `syscall.rs` 侧的判别腿②押着）。
    ///
    /// 本条证的是**通电**：typeck 认这个名字、`enemy_alive` 返 `int` 能直接进 `if` 条件、
    /// codegen 发得出 `OP_SYS 103`、派发臂接得住，并且它对**真的活着的敌**返 1
    /// （返 0 的话整个 `if` 体不执行 ⇒ 那颗弹压根不存在，`expect` 就地红）。
    #[test]
    fn enemy_alive_probe_drives_the_same_snipe_chain_end_to_end() {
        use stg_core::math::{Fx, cordic};
        let src = "sub main() {\n\
                     var e: int = spawn_enemy(60.0fx, -80.0fx, 100, 0, 0, 3, none);\n\
                     set_global(20, e);\n\
                     var n: int = nearest_enemy(64.0fx, -84.0fx);\n\
                     set_global(21, n);\n\
                     set_global(22, enemy_alive(n));\n\
                     set_global(23, enemy_alive(9999));\n\
                     if enemy_alive(n) == 1 {\n\
                       _ = fire(0, 0, 0.0fx, 0.0fx, 3.0fx, atan2(enemy_y(n), enemy_x(n)), none, none);\n\
                     }\n\
                     loop { wait(1); }\n\
                   }";
        let w = run(src, 2);
        let view = w.body.view();
        let g = view.globals();
        assert!(g[20] >= 0, "敌应建成（否则后面几条断言全退化成假绿）");
        assert_eq!(g[21], g[20], "nearest_enemy 返的就是刚建的那只敌");
        assert_eq!(g[22], 1, "活敌 ⇒ 1");
        assert_eq!(g[23], 0, "越界句柄 ⇒ 0（恒返 1 的实现在这里红）");

        let expect = cordic::atan2(Fx::from_int(-80), Fx::from_int(60));
        let p = view.bullets();
        let i = p
            .iter_alive()
            .next()
            .expect("探活为真 ⇒ if 体执行 ⇒ 必须真的开出一颗弹");
        assert_eq!(p.angle()[i], expect, "弹真的朝那只敌打（链路整条接通）");
        assert_eq!(view.diag().task_faults, 0);
    }

    /// 敌人运动动词族刀（2026-07-31）的 e2e：复刻 spec §3.2 那个编排——杂兵**被拉到点位**
    /// 的同时把速度缓动到朝下，**到点后继续按那个速度飘走**。走真编译 + 真 VM，链路是
    /// `.ecl 源码 → move_to/move_vel → syscall 400/410 → 世界层写 API → 相位 5 分层仲裁`，
    /// 外加 `$self_vy`/`$self_y`（syscall 023/021 那批引擎变量）把结果读回脚本侧。
    ///
    /// **两个 dur 必须严格不等且速度那条先到期**（位置 `dur=4` / 速度 `dur=2`）：这正是
    /// 黏滞位 `vel_touched` 存在的全部理由。到点那帧 `vel_active` 早已归 0，若判据写成
    /// `vel_active != 0` 就会把刚缓好的速度误清 ⇒ 敌人到点即死站，本条的 `vy == 3.0`
    /// 与"又飘了两帧"两格同时红。dur 取相等会让这条 e2e 退化成假守卫（那时两个判据等价）。
    ///
    /// 帧序（`run` 的 `step` 序号）：
    /// 0 = main 出生跳过；1 = main 首跑（`spawn_enemy` 挂 `on_enemy`）；
    /// 2 = `on_enemy` 首跑，两条动词武装，同帧相位 5 走第 1 帧插值（`mv_t=1`/`vel_t=1`）；
    /// 3 = `vel_t=2` 到期（速度落 3.0 朝下、`vel_active=0`），`mv_t=2`；4 = `mv_t=3`；
    /// 5 = `mv_t=4` 到点（精确 (80,180)、`mv_active=0`、**不清速**）；
    /// 6/7 = 无位置插值 ⇒ `pos += vel`，y 各 +3 → 186；
    /// 8 = `on_enemy` 的 `wait(5)` 醒来（睡过 3~7 共 5 帧；相位 2 早于相位 5），读到的
    /// 正是 186 / 3.0，本帧相位 5 再走一步 → 189。
    #[test]
    fn e2e_enemy_lands_and_keeps_drifting() {
        use stg_core::math::Fx;
        let src = "sub main() {\n\
                     _ = spawn_enemy(0.0fx, 100.0fx, 100, 0, 0, 3, on_enemy);\n\
                     loop { wait(1); }\n\
                   }\n\
                   async sub on_enemy() {\n\
                     move_to(4, 80.0fx, 180.0fx, 0);\n\
                     move_vel(2, 90deg, 3.0fx, 0);\n\
                     wait(5);\n\
                     set_global(20, $self_vy as int);\n\
                     set_global(21, $self_y as int);\n\
                     set_global(22, $self_x as int);\n\
                     loop { wait(1); }\n\
                   }";
        let w = run(src, 9);
        let view = w.body.view();
        let e = view.enemies();
        let i = e.iter_alive().next().expect("杂兵应还活着（否则全是假绿）");

        // ── 位置插值：到点了，且落在精确终点的 x 上（x 方向速度为 0，到点后不再动）──
        assert_eq!(e.mv_active()[i], 0, "位置插值 dur=4 早已到点");
        assert_eq!(e.x()[i], Fx::from_int(80), "到点 x 精确 80，此后 vx=0 不漂");

        // ── 黏滞位那格：速度插值先到期，但到点**不得**清速 ──
        assert_eq!(e.vel_active()[i], 0, "速度插值 dur=2 更早到期（前提）");
        assert_eq!(
            e.vy()[i],
            Fx::from_int(3),
            "到点不清速——判据写成 vel_active 的话这里是 0"
        );
        assert_eq!(e.vx()[i], Fx::ZERO, "90deg 是正下方，x 分量恰为 0");

        // ── 落地继续飘：到点(y=180) 之后 step 6/7/8 各走 3.0 ──
        assert_eq!(
            e.y()[i],
            Fx::from_int(180 + 9),
            "到点后 3 帧 × 3.0/帧 = 189（死站的话恒 180）"
        );

        // ── 脚本侧读回（$self_* 引擎变量真读活值，不是快照/常量）──
        assert_eq!(w.body.view().globals()[20], 3, "$self_vy 读到 3.0fx");
        assert_eq!(
            w.body.view().globals()[21],
            186,
            "$self_y：wait(5) 醒来那帧（相位 2）位置是 180 + 2×3"
        );
        assert_eq!(w.body.view().globals()[22], 80, "$self_x 停在终点 x");
        assert_eq!(w.body.view().diag().task_faults, 0);
    }

    /// 敌句柄打包刀（2026-07-31）：`godot/ecl/demo/boss_windchime.ecl` 的**等 boss 死**
    /// 轮询（`while ... { if enemy_hp(boss) < 0 { ... } }`）在打包后仍正确 —— 而且
    /// **打包实际上修好了这里一个潜在 bug**：boss 死、槽被回收之后若有杂兵落进那个槽，
    /// 打包前 `enemy_hp(boss)` 会读到**杂兵的血**（比如 40），`< 0` 永远不成立，轮询
    /// 就卡住不退（demo 靠"boss 段后不再造敌"这条口头惯例绕开，没有任何东西押着它）。
    ///
    /// 本条走真编译 + 真 VM 复刻那个形状：boss 主任务跑完即自燃（D9）→ 相位 9 回收槽
    /// → 轮询退出 → 造一只杂兵**占进同一个槽** → 断言旧 boss 号仍返 `-1`（关键那格，
    /// 打包前是杂兵的 `40`），同时新杂兵号一切正常。
    #[test]
    fn boss_death_poll_survives_a_zako_taking_the_boss_slot() {
        // `boss_main` 只 wait 就返回 ⇒ D9「主协程返回即自燃」把 boss 标 dying，相位 9 回收。
        let src = "async sub boss_main() {\n\
                     wait(20);\n\
                   }\n\
                   sub main() {\n\
                     var boss: int = spawn_enemy(0.0fx, 96.0fx, 900, 0, 5000, 1, boss_main);\n\
                     set_global(20, boss);\n\
                     var waiting: int = 1;\n\
                     var guard: int = 0;\n\
                     while waiting == 1 {\n\
                       if enemy_hp(boss) < 0 { waiting = 0; }\n\
                       guard = guard + 1;\n\
                       if guard > 100 { waiting = 0; }\n\
                       wait(1);\n\
                     }\n\
                     set_global(21, guard);\n\
                     var zako: int = spawn_enemy(0.0fx, 0.0fx, 40, 0, 0, 0, none);\n\
                     set_global(22, zako);\n\
                     set_global(23, enemy_hp(boss));\n\
                     set_global(24, enemy_hp(zako));\n\
                     loop { wait(1); }\n\
                   }";
        let w = run(src, 60);
        let g = w.body.view().globals();

        assert!(g[20] >= 0, "boss 应建成（否则后面几条全退化成假绿）");
        assert!(
            g[21] < 100,
            "轮询应当**自己**退出，而不是撞上 guard 兜底（got {})",
            g[21]
        );
        // 前提：杂兵必须落进 boss 的旧槽，否则本测试什么也没证明。
        assert_eq!(
            g[22] & 0xFFFF,
            g[20] & 0xFFFF,
            "前提：杂兵应落进 boss 的旧槽（分配器取最低空位）"
        );
        assert_ne!(g[22], g[20], "同槽不同敌 ⇒ 两个敌号必须可辨");

        assert_eq!(
            g[23], -1,
            "**关键格**：旧 boss 号仍返 -1；打包前这里会读到杂兵的血 40，轮询卡住不退"
        );
        assert_eq!(g[24], 40, "杂兵的新号一切正常（防「全都读不到」的假绿）");
        assert_eq!(w.body.view().diag().task_faults, 0);
    }
}
