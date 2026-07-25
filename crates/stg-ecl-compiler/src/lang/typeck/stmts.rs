//! `typeck` 子模块：语句判型——表达式语句的值消费检查、控制流分支的 definite-assignment
//! 快照/还原（`super::scope::LocalScope`）、sub 体总入口。

use super::checker::Checker;
use super::scope::LocalScope;
use super::typed_ast::{CallTarget, TypedCall, TypedExpr, TypedExprKind, TypedStmt, TypedSub};
use crate::lang::ast::{Block, Expr, Span, Stmt, SubDef, Ty, expr_span};

impl<'p> Checker<'p> {
    fn check_expr_stmt(
        &mut self,
        expr: &Expr,
        discarded: bool,
        span: Span,
        locals: &LocalScope,
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
        locals: &mut LocalScope,
        in_loop: bool,
    ) -> Option<TypedStmt> {
        match stmt {
            Stmt::Var {
                name,
                ty,
                init,
                span,
            } => {
                if self.check_not_reserved_sugar_name(*span, name) {
                    return None;
                }
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
                if locals.is_declared(name) {
                    self.err(
                        *span,
                        format!("变量名 '{name}' 重复声明（同一 sub 内需唯一，暂不支持遮蔽）"),
                    );
                    return None;
                }
                locals.declare(name, *ty);
                Some(TypedStmt::Var {
                    name: name.clone(),
                    ty: *ty,
                    init: t,
                })
            }
            Stmt::Assign { name, value, span } => {
                let t = self.type_expr(value, locals)?;
                match locals.type_of(name) {
                    Some(lt) => {
                        if !locals.is_visible(name) {
                            self.err(
                                *span,
                                format!(
                                    "变量 '{name}' 在此处不可见（赋值目标声明在可能未执行的\
                                     分支或循环体内，此处不保证已初始化）"
                                ),
                            );
                            return None;
                        }
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
                // then/else 各自是独立分支——都从同一份"进入前可见集"出发，谁声明的变量
                // 都不该让另一支看见，合流之后也一律视为"两条路径都不保证"（哪怕两支都有
                // 声明：同名重复声明本就被拒绝，intersection 语义永远退化为空集，见
                // `typeck` 模块文档"块作用域"节）。
                let visible_before = locals.snapshot();
                let then_typed = self.check_block(then_b, locals, in_loop);
                locals.restore(visible_before.clone());
                let else_typed = else_b
                    .as_ref()
                    .map(|b| self.check_block(b, locals, in_loop));
                locals.restore(visible_before);
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
                // while 循环体可能一次都不执行——体内声明的变量出循环后一律不可见。
                let visible_before = locals.snapshot();
                let body_typed = self.check_block(body, locals, true);
                locals.restore(visible_before);
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
                // `loop {}` 保证至少进入一次，但 `break` 可能发生在声明之前——保守起见同
                // while/for 一样出循环后不可见（不做"body 必然完整跑完一轮"这类流敏感证明）。
                let visible_before = locals.snapshot();
                let body_typed = self.check_block(body, locals, true);
                locals.restore(visible_before);
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
                // for 循环（含归纳变量本身）可能一次都不执行——归纳变量与体内声明的变量
                // 出循环后都不可见。快照必须在声明归纳变量*之前*拍，否则归纳变量会被错误
                // 地当成"循环外也可见"。
                let visible_before = locals.snapshot();
                let dup = locals.is_declared(var);
                if dup {
                    self.err(
                        *span,
                        format!("变量名 '{var}' 重复声明（同一 sub 内需唯一，暂不支持遮蔽）"),
                    );
                } else {
                    locals.declare(var, Ty::Int);
                }
                let body_typed = self.check_block(body, locals, true);
                locals.restore(visible_before);
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
                    // async/同步途径强制分离（对偶腿）：spawn 的实参落子任务 locals[0..argc)，
                    // 目标必须是 async sub（参数槽保证在基址 0）。
                    if !target.is_async {
                        self.err(
                            *span,
                            format!(
                                "spawn 目标 '{name}' 必须声明为 async sub——同步调用请用普通调用语句"
                            ),
                        );
                        return None;
                    }
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
            Stmt::Mark { id, block, .. } => {
                // 位置/id 合法性（仅 main 顶层、编译期常量、正整数、不重复）由独立趟
                // `validate_marks` 负责（见 `typeck` 模块入口 `check`）——错误已经/将会
                // 并入 `c.errors`，本臂不重复诊断，只管求值 + typecheck 补偿块，保证
                // 无论位置是否合法都能产出一份 `TypedStmt::Mark`（合法性判定失败时整体
                // `check()` 终归返回 `Err`，这里的产出不会被下游消费）。
                let id_val = match crate::lang::const_eval::evaluate(id, &self.consts) {
                    Ok((_ty, v)) => v,
                    Err(_) => 0,
                };
                // 补偿块是否执行取决于运行期入口（正常流跳过 / 中段跳入执行），同
                // if/while 的"可能不执行"分支一样，块内声明的局部出块后不可见。
                let visible_before = locals.snapshot();
                let body = match block {
                    Some(b) => self.check_block(b, locals, in_loop),
                    None => Vec::new(),
                };
                locals.restore(visible_before);
                Some(TypedStmt::Mark { id: id_val, body })
            }
        }
    }

    fn check_block(
        &mut self,
        block: &Block,
        locals: &mut LocalScope,
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

    pub(super) fn check_sub(&mut self, sub: &'p SubDef) -> TypedSub {
        self.cur_sync_calls = Vec::new();
        self.cur_xform_refs = Vec::new();
        let mut locals = LocalScope::default();
        for (pname, pty) in &sub.params {
            if locals.is_declared(pname) {
                self.err(sub.span, format!("参数名 '{pname}' 重复"));
            } else {
                locals.declare(pname, *pty);
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
