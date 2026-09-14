//! `typeck` 子模块：表达式判型（自底向上，值恒 `Some`——调用到 sub / `ret=None` 内建一律
//! 拒绝，见 `check_call` 的 `nested=true` 分支）+ 调用解析（sub / 内建共用一套名字解析；
//! `nested` 区分"表达式内部"（拒绝 `ret=None`）与"独立语句"（`ret=None` 合法）两种语境）。

use super::checker::Checker;
use super::scope::LocalScope;
use super::typed_ast::{CallArg, CallTarget, TypedCall, TypedExpr, TypedExprKind, const_val};
use crate::lang::ast::{Expr, Span, Ty, UnOp, expr_span};
use crate::lang::atlas;
use crate::lang::builtins::{self, Builtin, ParamKind};
use crate::lang::type_rules::{CastIntent, UnIntent, binary_result, op_symbol};

enum RefKind {
    Xform,
    Sub,
}

impl<'p, 't> Checker<'p, 't> {
    pub(super) fn type_expr(&mut self, e: &Expr, locals: &LocalScope) -> Option<TypedExpr> {
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
                if let Some(ty) = locals.type_of(name) {
                    if locals.is_visible(name) {
                        Some(TypedExpr {
                            ty,
                            kind: TypedExprKind::LocalRef(name.clone()),
                        })
                    } else {
                        self.err(
                            *span,
                            format!(
                                "变量 '{name}' 在此处不可见（声明在可能未执行的分支或\
                                 循环体内，此处不保证已初始化）"
                            ),
                        );
                        None
                    }
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

    pub(super) fn check_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        locals: &LocalScope,
        nested: bool,
    ) -> Option<(TypedCall, Option<Ty>)> {
        if name == "main" {
            self.err(
                span,
                "main 只能作为关卡根入口启动，不能同步调用".to_string(),
            );
            return None;
        }
        if let Some(sub) = self.subs.get(name).copied() {
            // async/同步途径强制分离（T2 复审 Critical 修复）：async sub 的参数槽恒基址 0
            // （SPAWN 把实参拷进子任务 locals[0..argc)），被同步 CALL 会让实参与参数槽错位
            // ——语言层禁止，编译期打回。
            if sub.is_async {
                self.err(
                    span,
                    format!("'{name}' 是 async sub，只能被 spawn——需要同步执行请改为普通 sub"),
                );
                return None;
            }
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

    pub(super) fn check_sub_call_args(
        &mut self,
        params: &[(String, Ty)],
        args: &[Expr],
        span: Span,
        locals: &LocalScope,
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
        locals: &LocalScope,
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
                ParamKind::RawVal => match self.type_expr(a, locals) {
                    // 三型任意，良型即过；raw 直通（无转换、无收窄）
                    Some(t) => out.push(CallArg::Val(t)),
                    None => ok = false,
                },
                ParamKind::XformRef => {
                    match self.resolve_ident_ref(a, span, RefKind::Xform, b.name) {
                        Some(r) => {
                            if let Some(n) = &r {
                                self.push_xform_ref(n.clone());
                            }
                            out.push(CallArg::XformRef(r));
                        }
                        None => ok = false,
                    }
                }
                ParamKind::SubRef => match a {
                    Expr::Call {
                        name,
                        args: sub_args,
                        span: cspan,
                    } => match self
                        .resolve_sub_ref_with_args(b.name, name, sub_args, *cspan, locals)
                    {
                        Some(arg) => out.push(arg),
                        None => ok = false,
                    },
                    _ => match self.resolve_ident_ref(a, span, RefKind::Sub, b.name) {
                        Some(r) => out.push(CallArg::SubRef(r)),
                        None => ok = false,
                    },
                },
            }
        }
        if ok && let Some(f) = builtins::fold_start(b.name) {
            self.check_shape_color(b.name, &out, args, span, f);
        }
        if ok { Some(out) } else { None }
    }

    /// 形/色两参判据的 typeck 侧接线——判据本体住 `lang::atlas`（xformdef 的
    /// `set_sprite` 走同一份，见该模块文档；顺序"色号 → 弹型 → 空格"是契约）。
    ///
    /// 本层只负责两件事：① **只在两参都是编译期常量、且绑定了表时**才问判据（变量色
    /// 跳过，由 syscall 的 `valid` 判据在运行期兜底——`sys_create_bullet(s_batch)` 先验
    /// 后建）；② 把否决按 [`atlas::Blame`] 挂到出错那一位的 span 上。
    /// `f` = 两参糖的起始下标（`builtins::fold_start`）——`fire`/`batch` 是 0，
    /// `sh_sprite(id, shape, color)` 是 1。**不能硬编码 0**，否则会拿 `id` 当弹型去查表。
    fn check_shape_color(
        &mut self,
        name: &str,
        out: &[CallArg],
        args: &[Expr],
        span: Span,
        f: usize,
    ) {
        let Some(table) = self.table else { return };
        let (Some(shape), Some(color)) = (const_val(&out[f]), const_val(&out[f + 1])) else {
            return; // 变量参：运行期由 syscall 的 valid 判据兜底
        };
        if let Err(e) = atlas::check_shape_color(table, name, shape, color) {
            let blamed = match e.blame {
                atlas::Blame::Shape => &args[f],
                atlas::Blame::Color => &args[f + 1],
            };
            self.err(expr_span(blamed).unwrap_or(span), e.msg);
        }
    }

    /// task 位的 `name(实参…)` 写法（boss 换段刀 spec §4.1）：只 `spawn_enemy` 接受；
    /// 目标须为已声明 async sub，实参按其签名判型（同 `spawn f(args);` 的规则，复用
    /// `check_sub_call_args`——实参里嵌套的同步调用照常记调用图边）。
    fn resolve_sub_ref_with_args(
        &mut self,
        builtin_name: &str,
        name: &str,
        args: &[Expr],
        span: Span,
        locals: &LocalScope,
    ) -> Option<CallArg> {
        if builtin_name != "spawn_enemy" {
            self.err(
                span,
                format!(
                    "'{builtin_name}' 的 task 引用不能带实参（目前只有 spawn_enemy 的 task 位支持带参）"
                ),
            );
            return None;
        }
        if name == "none" {
            self.err(span, "'none' 不能带实参".into());
            return None;
        }
        let Some(sub) = self.subs.get(name).copied() else {
            self.err(span, format!("未知的 sub 名 '{name}'"));
            return None;
        };
        if !sub.is_async {
            self.err(
                span,
                format!("'{name}' 用作 spawn_enemy 的 task 引用必须声明为 async sub"),
            );
            return None;
        }
        let typed = self.check_sub_call_args(&sub.params, args, span, locals)?;
        let exprs = typed
            .into_iter()
            .map(|a| match a {
                CallArg::Val(t) => t,
                _ => unreachable!("check_sub_call_args 只产 Val"),
            })
            .collect();
        Some(CallArg::SubRefArgs(name.to_string(), exprs))
    }

    fn resolve_ident_ref(
        &mut self,
        a: &Expr,
        call_span: Span,
        kind: RefKind,
        builtin_name: &str,
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
                    // fire 的 task 引用与 spawn 同途（新任务根 + 实参基址 0），
                    // 同样只许 async sub（分离规则第三腿）——`spawn_enemy`/`spell_begin`
                    // 的 task/pattern 位走同一通道，报错文案按实际调用的 builtin 名报，
                    // 不硬写 "fire"（task-1 复审 Minor-3）。
                    if let (RefKind::Sub, Some(sub)) = (kind, self.subs.get(name).copied()) {
                        if !sub.is_async {
                            self.err(
                                *vspan,
                                format!(
                                    "'{name}' 用作 {builtin_name} 的 task 引用必须声明为 async sub"
                                ),
                            );
                            return None;
                        }
                        // 分离规则第四腿（M1.9 终审 Critical）：fire 的派生走 syscall 内部
                        // spawn，**不带实参**——带参 async sub 在此通道参数恒读零（T2 那颗
                        // 实参错位 Critical 的孪生路径）。语言层拒绝：task 引用必须无参
                        // （同一约束适用于 fire/spawn_enemy/spell_begin 等全部 SubRef 位）。
                        if !sub.params.is_empty() {
                            self.err(
                                *vspan,
                                format!(
                                    "'{name}' 用作 {builtin_name} 的 task 引用必须是无参 async sub\
                                     （派生不带实参——需要传参请用 spawn；spawn_enemy 的 task 位可写 name(实参…)）"
                                ),
                            );
                            return None;
                        }
                    }
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
}
