//! Xform-local scope, phase checks, and lowering staged expressions to core IR.
use super::checker::Checker;
use super::scope::LocalScope;
use super::typed_ast::{CallArg, TypedExpr, TypedExprKind};
use crate::lang::ast::{self, Expr, Span, Ty};
use crate::lang::type_rules::{BinIntent, CastIntent, UnIntent};
use crate::lang::xform_map::{self, XformOp};
use std::collections::{BTreeMap, BTreeSet};
use stg_core::xform::expr::*;
use stg_core::xform::program::*;
use stg_core::xform::{self, XformSlot};

impl Checker<'_, '_> {
    pub(super) fn check_xform_ref(
        &mut self,
        expr: &Expr,
        span: Span,
        locals: &LocalScope,
    ) -> Option<CallArg> {
        let (name, args) = match expr {
            Expr::Var(name, _) if name == "none" => return Some(CallArg::XformRef(None)),
            Expr::Var(name, _) => (name, &[][..]),
            Expr::Call { name, args, .. } => (name, args.as_slice()),
            _ => {
                self.err(
                    span,
                    "xform 引用期待标识符（模板名、NAME(args) 或 none）".into(),
                );
                return None;
            }
        };
        let Some(def) = self.xformdefs.get(name).copied() else {
            self.err(span, format!("未知的 xformdef 名 '{name}'"));
            return None;
        };
        if args.len() != def.params.len() {
            self.err(
                span,
                format!(
                    "xform '{name}' 参数个数不匹配：期待 {}，实际 {}",
                    def.params.len(),
                    args.len()
                ),
            );
            return None;
        }
        let mut typed = Vec::new();
        for (expr, (_, ty)) in args.iter().zip(&def.params) {
            let t = self.type_expr(expr, locals)?;
            if t.ty != *ty {
                self.push_type_mismatch(
                    span,
                    format!(
                        "类型不匹配：xform '{name}' 参数期待 {ty:?}，实际 {:?}",
                        t.ty
                    ),
                );
                return None;
            }
            typed.push(t);
        }
        self.push_xform_ref(name.clone());
        let legacy = def.params.is_empty()
            && def
                .slots
                .iter()
                .flat_map(|s| &s.args)
                .all(|a| crate::lang::const_eval::evaluate(a, &self.consts).is_ok());
        if legacy {
            Some(CallArg::XformRef(Some(name.clone())))
        } else {
            Some(CallArg::XformRefArgs(name.clone(), typed))
        }
    }

    pub(super) fn check_xforms(
        &mut self,
        prog: &ast::Program,
    ) -> BTreeMap<String, XformProgramInit> {
        let mut result = BTreeMap::new();
        for def in &prog.xformdefs {
            // Existing zero-argument constant definitions retain their exact raw ABI.
            if def.params.is_empty()
                && def
                    .slots
                    .iter()
                    .flat_map(|s| &s.args)
                    .all(|a| crate::lang::const_eval::evaluate(a, &self.consts).is_ok())
            {
                continue;
            }
            let before = self.errors.len();
            if let Some(init) = self.check_xform(def)
                && self.errors.len() == before
            {
                result.insert(def.name.clone(), init);
            }
        }
        result
    }

    fn check_xform(&mut self, def: &ast::XformDef) -> Option<XformProgramInit> {
        let mut scope = LocalScope::default();
        let mut names = BTreeMap::new();
        for (i, (name, ty)) in def.params.iter().enumerate() {
            if self.check_not_reserved_sugar_name(def.span, name) {
                return None;
            }
            if scope.is_declared(name) || self.consts.contains_key(name) {
                self.err(def.span, format!("xform 形参 '{name}' 重复或与常量重名"));
                return None;
            }
            scope.declare(name, *ty);
            names.insert(name.clone(), i as u8);
        }
        let mut init = XformProgramInit {
            slots: vec![],
            argc: def.params.len().min(255) as u8,
            bind: vec![],
            spawn: vec![],
            params: vec![],
            persistent: vec![],
        };
        let mut next_cache = def.params.len();
        for source in &def.slots {
            let (op, arity, physical) = match xform_map::lookup(&source.op_name) {
                Some(XformOp::Op(op, n, p)) => (op, n, p),
                Some(XformOp::OpFold2(op, p)) => (op, 2, p),
                Some(XformOp::OpWithStride(op, p)) => (op, 1, p),
                _ => {
                    self.err(
                        source.span,
                        format!("未知或预留的 xform 操作 '{}'", source.op_name),
                    );
                    return None;
                }
            };
            if source.args.len() != arity {
                self.err(
                    source.span,
                    format!(
                        "xform 操作 '{}' 期待 {arity} 个参数，实际 {}",
                        source.op_name,
                        source.args.len()
                    ),
                );
                return None;
            }
            let slot = init.slots.len();
            let mut params = Vec::new();
            for (k, raw) in source.args.iter().enumerate() {
                let (phase, expr) = match raw {
                    Expr::Call { name, args, .. } if name == "at_spawn" || name == "at_effect" => {
                        if args.len() != 1 {
                            self.err(source.span, "求值标记期待一个完整表达式".into());
                            return None;
                        }
                        (if name == "at_spawn" { 1 } else { 2 }, &args[0])
                    }
                    _ => (0, raw),
                };
                if !allowed_in_xform(expr) {
                    self.err(ast::expr_span(raw).unwrap_or(source.span), "xform 体仅允许 global/sin/cos/atan2/dist/aim_player/rand；at_spawn/at_effect 只能位于完整参数外层，不能嵌套".into());
                    return None;
                }
                let typed = self.type_expr(expr, &scope)?;
                let expected = arg_type(op, k);
                if typed.ty != expected {
                    self.push_type_mismatch(
                        source.span,
                        format!(
                            "类型不匹配：xform '{}' 第 {} 个参数期待 {expected:?}，实际 {:?}",
                            source.op_name,
                            k + 1,
                            typed.ty
                        ),
                    );
                    return None;
                }
                let mut code = Vec::new();
                lower(&typed, &names, &mut code);
                let expression = match make_expr(code) {
                    Ok(e) => e,
                    Err(_) => {
                        self.err(
                            source.span,
                            "xform 表达式超过 128 条指令、32 字栈或控制流无效".into(),
                        );
                        return None;
                    }
                };
                params.push(match expression {
                    XformExpr::Constant(v) => XformParam::Constant(v),
                    expr if phase == 2 => XformParam::Effect(expr),
                    XformExpr::Read(XformRead::Capture(k)) => XformParam::Cached(k),
                    expr => {
                        if next_cache >= 8 {
                            self.err(
                                source.span,
                                "xform 形参、绑定和出生采样合计超过 8 个逻辑缓存值".into(),
                            );
                            return None;
                        }
                        let index = next_cache as u8;
                        next_cache += 1;
                        let sample = XformSample { index, expr };
                        if phase == 1 {
                            init.spawn.push(sample);
                        } else {
                            init.bind.push(sample);
                        }
                        XformParam::Cached(index)
                    }
                });
            }
            let stride = self.table.map(|t| t.color_stride as i32);
            if matches!(op, xform::OP_SET_SHAPE | xform::OP_SET_COLOR) && stride.is_none() {
                self.err(source.span, "xform 部分外观设置需要绑定的外观表".into());
                return None;
            }
            // Constants are checked here; dynamic axes are checked as soon as known at runtime.
            if let Some(table) = self.table {
                let constant = |k: usize| match params.get(k) {
                    Some(XformParam::Constant(v)) => Some(*v),
                    _ => None,
                };
                let check = match op {
                    xform::OP_SET_SPRITE => match (constant(0), constant(1)) {
                        (Some(s), Some(c)) => {
                            crate::lang::atlas::check_shape_color(table, "set_sprite", s, c)
                        }
                        (Some(s), None) => crate::lang::atlas::check_shape_only(table, s),
                        (None, Some(c)) => crate::lang::atlas::check_color_only(table, c),
                        (None, None) => Ok(()),
                    },
                    xform::OP_SET_SHAPE => constant(0)
                        .map_or(Ok(()), |v| crate::lang::atlas::check_shape_only(table, v)),
                    xform::OP_SET_COLOR => constant(0)
                        .map_or(Ok(()), |v| crate::lang::atlas::check_color_only(table, v)),
                    _ => Ok(()),
                };
                if let Err(e) = check {
                    self.err(source.span, e.msg);
                    return None;
                }
            }
            init.slots.push(XformSlot {
                wait: source.wait,
                op,
                _pad: 0,
                args: [
                    0,
                    if matches!(op, xform::OP_SET_SHAPE | xform::OP_SET_COLOR) {
                        stride.unwrap_or(0)
                    } else {
                        0
                    },
                ],
            });
            init.slots
                .extend((1..physical).map(|_| XformSlot::default()));
            init.params.push(XformOpParams {
                slot: slot as u8,
                args: params,
            });
        }
        if next_cache > 8 {
            self.err(
                def.span,
                "xform 形参、绑定和出生采样合计超过 8 个逻辑缓存值".into(),
            );
            return None;
        }
        let mut persistent = BTreeSet::new();
        for params in &init.params {
            for p in &params.args {
                if let XformParam::Effect(e) = p {
                    persistent.extend(e.captures());
                }
            }
            if init.slots[params.slot as usize].op == xform::OP_SET_SPRITE
                && params
                    .args
                    .iter()
                    .any(|p| matches!(p, XformParam::Effect(_)))
            {
                for p in &params.args {
                    if let XformParam::Cached(k) = p {
                        persistent.insert(*k);
                    }
                }
            }
        }
        init.persistent = persistent.into_iter().collect();
        for params in &mut init.params {
            for p in &mut params.args {
                if let XformParam::Effect(e) = p {
                    e.remap_captures(|k| {
                        init.persistent.iter().position(|v| *v == k).unwrap() as u8
                    });
                }
            }
        }
        let total = init.slots.len() + init.persistent.len().div_ceil(2);
        if total > 16 {
            self.err(def.span, format!("xform 操作含 scratch {} 槽，持久捕获 {} 字占 {} 数据槽，共 {total} 槽，超过 16", init.slots.len(), init.persistent.len(), init.persistent.len().div_ceil(2)));
            return None;
        }
        if XformPrograms::try_new(vec![init.clone()]).is_err() {
            self.err(def.span, "xform 程序阶段布局无效".into());
            return None;
        }
        Some(init)
    }
}

fn allowed_in_xform(e: &Expr) -> bool {
    match e {
        Expr::Call { name, args, .. } => {
            matches!(
                name.as_str(),
                "global" | "sin" | "cos" | "atan2" | "dist" | "aim_player" | "rand"
            ) && args.iter().all(allowed_in_xform)
        }
        Expr::Binary { l, r, .. } => allowed_in_xform(l) && allowed_in_xform(r),
        Expr::Unary { e, .. } | Expr::Cast { e, .. } => allowed_in_xform(e),
        _ => true,
    }
}
fn arg_type(op: u8, index: usize) -> Ty {
    match op {
        xform::OP_SET_SPEED | xform::OP_ADD_SPEED | xform::OP_SET_ACCEL | xform::OP_SET_GRAVITY => {
            Ty::Fx
        }
        xform::OP_STEP_SPEED
        | xform::OP_STEP_ACCEL
        | xform::OP_STEP_VX
        | xform::OP_STEP_VY
        | xform::OP_STEP_AX
        | xform::OP_STEP_AY
            if index == 0 =>
        {
            Ty::Fx
        }
        xform::OP_STEP_ANGLE | xform::OP_STEP_ANG_VEL if index == 0 => Ty::Angle,
        xform::OP_SET_ANGLE | xform::OP_TURN | xform::OP_AIM_PLAYER | xform::OP_SET_ANG_VEL => {
            Ty::Angle
        }
        _ => Ty::Int,
    }
}
fn make_expr(code: Vec<XformInstr>) -> Result<XformExpr, XformEvalError> {
    if code.len() == 1 {
        match code[0] {
            XformInstr::Constant(v) => return Ok(XformExpr::Constant(v)),
            XformInstr::Read(r) => return Ok(XformExpr::Read(r)),
            _ => {}
        }
    }
    let e = XformExpr::try_code(code)?;
    // Constant folding never observes world state or consumes random draws.
    if let Some(v) = constant_eval(&e) {
        Ok(XformExpr::Constant(v))
    } else {
        Ok(e)
    }
}
fn constant_eval(e: &XformExpr) -> Option<i32> {
    // Reuse the existing constant evaluator by lowering only read-free arithmetic below.
    let XformExpr::Code(code) = e else {
        return None;
    };
    if code.iter().any(|i| {
        matches!(
            i,
            XformInstr::Read(_) | XformInstr::Unary(XformUnary::Global | XformUnary::Rand)
        )
    }) {
        return None;
    }
    stg_core::xform::expr::evaluate_constant(e)
}
fn lower(e: &TypedExpr, names: &BTreeMap<String, u8>, out: &mut Vec<XformInstr>) {
    use XformInstr::*;
    let start = out.len();
    match &e.kind {
        TypedExprKind::IntLit(v) | TypedExprKind::FxLit(v) | TypedExprKind::ConstRef(v) => {
            out.push(Constant(*v))
        }
        TypedExprKind::AngleLit(v) => out.push(Constant(*v as i32)),
        TypedExprKind::LocalRef(n) => out.push(Read(XformRead::Capture(names[n]))),
        TypedExprKind::EngineVar(v) => out.push(Read(match v {
            ast::EngVar::Frame => XformRead::Frame,
            ast::EngVar::PlayerX => XformRead::PlayerX,
            ast::EngVar::PlayerY => XformRead::PlayerY,
            ast::EngVar::SelfX => XformRead::SelfX,
            ast::EngVar::SelfY => XformRead::SelfY,
            ast::EngVar::SelfVx => XformRead::SelfVx,
            ast::EngVar::SelfVy => XformRead::SelfVy,
            ast::EngVar::SelfSpeed => XformRead::SelfSpeed,
            ast::EngVar::SelfAngle => XformRead::SelfAngle,
            ast::EngVar::SelfHp => XformRead::SelfHp,
            ast::EngVar::SelfHpMax => XformRead::SelfHpMax,
            ast::EngVar::SelfAge => XformRead::SelfAge,
            ast::EngVar::SelfEnemy => XformRead::SelfEnemy,
        })),
        TypedExprKind::Unary { e, intent, .. } => {
            lower(e, names, out);
            out.push(Unary(match intent {
                UnIntent::Neg => XformUnary::Neg,
                UnIntent::Not => XformUnary::Not,
            }));
        }
        TypedExprKind::Cast { e, intent } => {
            lower(e, names, out);
            match intent {
                CastIntent::IntToFx => out.push(Unary(XformUnary::IntToFx)),
                CastIntent::FxToInt => out.push(Unary(XformUnary::FxToInt)),
                CastIntent::Bitcast => {}
            }
        }
        TypedExprKind::Binary { op, l, r, intent } => {
            lower(l, names, out);
            if matches!(intent, BinIntent::LogicAnd | BinIntent::LogicOr) {
                if *intent == BinIntent::LogicOr {
                    out.push(Unary(XformUnary::Not));
                }
                let branch = out.len();
                out.push(JumpIfZero(0));
                lower(r, names, out);
                out.push(Unary(XformUnary::Bool));
                let jump = out.len();
                out.push(Jump(0));
                out[branch] = JumpIfZero(out.len() as u8);
                out.push(Constant(if *intent == BinIntent::LogicAnd { 0 } else { 1 }));
                out[jump] = Jump(out.len() as u8);
            } else {
                lower(r, names, out);
                out.push(Binary(match intent {
                    BinIntent::AddI => XformBinary::Add,
                    BinIntent::SubI => XformBinary::Sub,
                    BinIntent::MulI => XformBinary::Mul,
                    BinIntent::DivI => XformBinary::Div,
                    BinIntent::ModI => XformBinary::Rem,
                    BinIntent::MulF => XformBinary::MulFx,
                    BinIntent::DivF => XformBinary::DivFx,
                    BinIntent::Cmp => match op {
                        ast::BinOp::Eq => XformBinary::Eq,
                        ast::BinOp::Ne => XformBinary::Ne,
                        ast::BinOp::Lt => XformBinary::Lt,
                        ast::BinOp::Le => XformBinary::Le,
                        ast::BinOp::Gt => XformBinary::Gt,
                        ast::BinOp::Ge => XformBinary::Ge,
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }));
            }
        }
        TypedExprKind::Call(c) => {
            for arg in &c.args {
                if let CallArg::Val(e) = arg {
                    lower(e, names, out);
                }
            }
            out.push(match c.name.as_str() {
                "global" => Unary(XformUnary::Global),
                "rand" => Unary(XformUnary::Rand),
                "sin" => Unary(XformUnary::Sin),
                "cos" => Unary(XformUnary::Cos),
                "atan2" => Binary(XformBinary::Atan2),
                "dist" => Binary(XformBinary::Dist),
                "aim_player" => Read(XformRead::AimPlayer),
                _ => unreachable!(),
            });
        }
    }
    if out.len() > start + 1
        && !out[start..].iter().any(|i| {
            matches!(
                i,
                Read(_) | Jump(_) | JumpIfZero(_) | Unary(XformUnary::Global | XformUnary::Rand)
            )
        })
        && let Some(v) =
            stg_core::xform::expr::evaluate_constant(&XformExpr::Code(out[start..].into()))
    {
        out.truncate(start);
        out.push(Constant(v));
    }
}
