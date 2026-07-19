//! `typeck` 子模块：表达式判型（自底向上，值恒 `Some`——调用到 sub / `ret=None` 内建一律
//! 拒绝，见 `check_call` 的 `nested=true` 分支）+ 调用解析（sub / 内建共用一套名字解析；
//! `nested` 区分"表达式内部"（拒绝 `ret=None`）与"独立语句"（`ret=None` 合法）两种语境）。

use super::checker::Checker;
use super::intents::{CastIntent, UnIntent};
use super::matrix::{binary_result, expr_span, op_symbol};
use super::scope::LocalScope;
use super::typed_ast::{CallArg, CallTarget, TypedCall, TypedExpr, TypedExprKind};
use crate::lang::ast::{Expr, Span, Ty, UnOp};
use crate::lang::builtins::{self, Builtin, ParamKind};

enum RefKind {
    Xform,
    Sub,
}

impl<'p> Checker<'p> {
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

    pub(super) fn check_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
        locals: &LocalScope,
        nested: bool,
    ) -> Option<(TypedCall, Option<Ty>)> {
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
                    // fire 的 task 引用与 spawn 同途（新任务根 + 实参基址 0），
                    // 同样只许 async sub（分离规则第三腿）。
                    if let (RefKind::Sub, Some(sub)) = (kind, self.subs.get(name).copied()) {
                        if !sub.is_async {
                            self.err(
                                *vspan,
                                format!("'{name}' 用作 fire 的 task 引用必须声明为 async sub"),
                            );
                            return None;
                        }
                        // 分离规则第四腿（M1.9 终审 Critical）：fire 的派生走 syscall 内部
                        // spawn，**不带实参**——带参 async sub 在此通道参数恒读零（T2 那颗
                        // 实参错位 Critical 的孪生路径）。语言层拒绝：fire task 引用必须无参。
                        if !sub.params.is_empty() {
                            self.err(
                                *vspan,
                                format!(
                                    "'{name}' 用作 fire 的 task 引用必须是无参 async sub\
                                     （fire 派生不带实参——需要传参请用 spawn）"
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
