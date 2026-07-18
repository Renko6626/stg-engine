//! ECL 表层语言语法器：递归下降语句 + Pratt 表达式（clox 同款优先级表）。
//!
//! ## 错误恢复策略（跳到下一 `;`/`}`，计划钉死的策略）
//!
//! 所有"期待 X"式失败（[`Parser::expect`]/[`Parser::expect_ident`]/`parse_primary` 的兜底
//! 分支……）**只记一条错误、绝不消费当前 token**——真正的"跳过若干 token 直到同步点"全部
//! 集中在 [`Parser::recover`] 一处。这样设计是为了让恢复精确落在**触发错误的语句自己的**
//! `;`/`}` 上，不多吞下一条语句（若改成"报错分支顺手吃掉坏 token"，遇到"缺失的表达式恰好
//! 紧邻分号"这种常见笔误时，`;` 会被那次"吃掉坏 token"提前吞掉，导致 `recover()` 从下一条
//! 语句内部才开始扫，殃及下一条语句——见 `parse::tests::error_recovery_reports_two_independent_errors_same_file`
//! 的构造用例，两个独立错误精确落在各自的语句边界上）。
//!
//! `recover()` 自身保证前进：非 `;`/`}`/EOF 就消费一个 token 再继续；已经站在 `}`/EOF 上则
//! 直接返回不消费——此时外层的语句循环条件本身也会随之退出，不会死循环。
//!
//! ## Pratt 优先级（低→高，clox 同款）：`||` < `&&` < 比较 < `+ -` < `* / %` < 一元 < cast 后缀

use crate::lang::ast::{
    BinOp, Block, CompileError, ConstDef, EngVar, Expr, Program, Span, Stmt, SubDef, Ty, UnOp,
    XfSlotLit, XformDef,
};
use crate::lang::lex::{Token, TokenKind};

const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_CMP: u8 = 3;
const PREC_TERM: u8 = 4;
const PREC_FACTOR: u8 = 5;

fn infix_binding(kind: &TokenKind) -> Option<(u8, BinOp)> {
    match kind {
        TokenKind::OrOr => Some((PREC_OR, BinOp::Or)),
        TokenKind::AndAnd => Some((PREC_AND, BinOp::And)),
        TokenKind::EqEq => Some((PREC_CMP, BinOp::Eq)),
        TokenKind::NotEq => Some((PREC_CMP, BinOp::Ne)),
        TokenKind::Lt => Some((PREC_CMP, BinOp::Lt)),
        TokenKind::LtEq => Some((PREC_CMP, BinOp::Le)),
        TokenKind::Gt => Some((PREC_CMP, BinOp::Gt)),
        TokenKind::GtEq => Some((PREC_CMP, BinOp::Ge)),
        TokenKind::Plus => Some((PREC_TERM, BinOp::Add)),
        TokenKind::Minus => Some((PREC_TERM, BinOp::Sub)),
        TokenKind::Star => Some((PREC_FACTOR, BinOp::Mul)),
        TokenKind::Slash => Some((PREC_FACTOR, BinOp::Div)),
        TokenKind::Percent => Some((PREC_FACTOR, BinOp::Mod)),
        _ => None,
    }
}

/// `$` 引擎变量白名单（拍板 6，v1 固定 8 个）——词法层只切词，这里判定是否合法。
fn resolve_engine_var(name: &str) -> Option<EngVar> {
    match name {
        "frame" => Some(EngVar::Frame),
        "player_x" => Some(EngVar::PlayerX),
        "player_y" => Some(EngVar::PlayerY),
        "self_x" => Some(EngVar::SelfX),
        "self_y" => Some(EngVar::SelfY),
        "self_hp" => Some(EngVar::SelfHp),
        "self_hp_max" => Some(EngVar::SelfHpMax),
        "self_age" => Some(EngVar::SelfAge),
        _ => None,
    }
}

/// 人类可读的 token 描述，供错误信息里的"实际是 ..."拼接。
fn token_desc(kind: &TokenKind) -> String {
    match kind {
        TokenKind::IntLit(v) => format!("整数字面量 {v}"),
        TokenKind::FxLit(v) => format!("fx 字面量 {v}"),
        TokenKind::AngleLit(v) => format!("angle 字面量 {v}"),
        TokenKind::Ident(s) => format!("标识符 '{s}'"),
        TokenKind::EngineVar(s) => format!("'${s}'"),
        TokenKind::Sub => "'sub'".into(),
        TokenKind::Async => "'async'".into(),
        TokenKind::Var => "'var'".into(),
        TokenKind::If => "'if'".into(),
        TokenKind::Else => "'else'".into(),
        TokenKind::While => "'while'".into(),
        TokenKind::Loop => "'loop'".into(),
        TokenKind::For => "'for'".into(),
        TokenKind::In => "'in'".into(),
        TokenKind::Wait => "'wait'".into(),
        TokenKind::Spawn => "'spawn'".into(),
        TokenKind::Return => "'return'".into(),
        TokenKind::Break => "'break'".into(),
        TokenKind::Continue => "'continue'".into(),
        TokenKind::Const => "'const'".into(),
        TokenKind::Xformdef => "'xformdef'".into(),
        TokenKind::As => "'as'".into(),
        TokenKind::KwInt => "'int'".into(),
        TokenKind::KwFx => "'fx'".into(),
        TokenKind::KwAngle => "'angle'".into(),
        TokenKind::Underscore => "'_'".into(),
        TokenKind::Plus => "'+'".into(),
        TokenKind::Minus => "'-'".into(),
        TokenKind::Star => "'*'".into(),
        TokenKind::Slash => "'/'".into(),
        TokenKind::Percent => "'%'".into(),
        TokenKind::EqEq => "'=='".into(),
        TokenKind::NotEq => "'!='".into(),
        TokenKind::Lt => "'<'".into(),
        TokenKind::LtEq => "'<='".into(),
        TokenKind::Gt => "'>'".into(),
        TokenKind::GtEq => "'>='".into(),
        TokenKind::AndAnd => "'&&'".into(),
        TokenKind::OrOr => "'||'".into(),
        TokenKind::Bang => "'!'".into(),
        TokenKind::Eq => "'='".into(),
        TokenKind::LParen => "'('".into(),
        TokenKind::RParen => "')'".into(),
        TokenKind::LBrace => "'{'".into(),
        TokenKind::RBrace => "'}'".into(),
        TokenKind::Semi => "';'".into(),
        TokenKind::Colon => "':'".into(),
        TokenKind::Comma => "','".into(),
        TokenKind::DotDot => "'..'".into(),
        TokenKind::At => "'@'".into(),
        TokenKind::Eof => "文件末尾".into(),
    }
}

/// 递归下降 + Pratt 语法器。持有完整 token 流（词法阶段已一次性产出，见 `lang::lex`）。
pub struct Parser<'s> {
    tokens: Vec<Token>,
    pos: usize,
    lines: Vec<&'s str>,
    errors: Vec<CompileError>,
}

impl<'s> Parser<'s> {
    pub fn new(tokens: Vec<Token>, src: &'s str) -> Self {
        Parser {
            tokens,
            pos: 0,
            lines: src.lines().collect(),
            errors: Vec::new(),
        }
    }

    // ── token 流游标 ─────────────────────────────────────────────────────

    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_kind_at(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| &t.kind)
    }

    fn current_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// 消费当前 token 并返回它（末尾 `Eof` 恒重复返回自身，不越界）。
    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn push_error(&mut self, span: Span, msg: String) {
        self.errors.push(CompileError::at(span, msg, &self.lines));
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        let span = self.current_span();
        self.push_error(span, msg.into());
    }

    /// 精确匹配则消费并返回；否则**不消费**，记一条"{msg}，实际是 {found}"错误。
    fn expect(&mut self, kind: TokenKind, msg: &str) -> Result<Token, ()> {
        if *self.peek_kind() == kind {
            Ok(self.advance())
        } else {
            let span = self.current_span();
            let found = token_desc(self.peek_kind());
            self.push_error(span, format!("{msg}，实际是 {found}"));
            Err(())
        }
    }

    fn expect_ident(&mut self, msg: &str) -> Result<String, ()> {
        match self.peek_kind().clone() {
            TokenKind::Ident(name) => {
                self.advance();
                Ok(name)
            }
            other => {
                let span = self.current_span();
                self.push_error(span, format!("{msg}，实际是 {}", token_desc(&other)));
                Err(())
            }
        }
    }

    /// 恢复：跳到下一个 `;`（连它一起消费）或 `}`/EOF（不消费，留给外层语句循环判断退出）。
    fn recover(&mut self) {
        loop {
            match self.peek_kind() {
                TokenKind::Semi => {
                    self.advance();
                    return;
                }
                TokenKind::RBrace | TokenKind::Eof => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ── 顶层 ─────────────────────────────────────────────────────────────

    /// 消费整份 token 流，产出尽力而为的 [`Program`]（结构不完整的部分对应的错误已收进
    /// 返回的 `Vec<CompileError>`）。
    pub fn parse_program(mut self) -> (Program, Vec<CompileError>) {
        let mut consts = Vec::new();
        let mut xformdefs = Vec::new();
        let mut subs = Vec::new();
        while *self.peek_kind() != TokenKind::Eof {
            let kind = self.peek_kind().clone();
            let ok = match kind {
                TokenKind::Const => self.parse_const_def().map(|c| consts.push(c)).is_ok(),
                TokenKind::Xformdef => self.parse_xformdef().map(|x| xformdefs.push(x)).is_ok(),
                TokenKind::Sub => self.parse_sub_def(false).map(|s| subs.push(s)).is_ok(),
                TokenKind::Async => self.parse_async_sub_def().map(|s| subs.push(s)).is_ok(),
                _ => {
                    self.error_here("期待顶层声明（const / xformdef / sub / async sub）");
                    false
                }
            };
            if !ok {
                self.recover();
            }
        }
        (
            Program {
                consts,
                xformdefs,
                subs,
            },
            self.errors,
        )
    }

    fn parse_const_def(&mut self) -> Result<ConstDef, ()> {
        let span = self.current_span();
        self.advance(); // 'const'
        let name = self.expect_ident("期待常量名")?;
        self.expect(TokenKind::Colon, "期待 ':'")?;
        let ty = self.parse_ty()?;
        self.expect(TokenKind::Eq, "期待 '='")?;
        let value = self.parse_top_expr()?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(ConstDef {
            name,
            ty,
            value,
            span,
        })
    }

    fn parse_async_sub_def(&mut self) -> Result<SubDef, ()> {
        let span = self.current_span();
        self.advance(); // 'async'
        self.expect(TokenKind::Sub, "期待 'sub'")?;
        self.parse_sub_def_body(true, span)
    }

    fn parse_sub_def(&mut self, is_async: bool) -> Result<SubDef, ()> {
        let span = self.current_span();
        self.advance(); // 'sub'
        self.parse_sub_def_body(is_async, span)
    }

    fn parse_sub_def_body(&mut self, is_async: bool, span: Span) -> Result<SubDef, ()> {
        let name = self.expect_ident("期待 sub 名")?;
        self.expect(TokenKind::LParen, "期待 '('")?;
        let mut params = Vec::new();
        if *self.peek_kind() != TokenKind::RParen {
            loop {
                let pname = self.expect_ident("期待参数名")?;
                self.expect(TokenKind::Colon, "期待 ':'")?;
                let pty = self.parse_ty()?;
                params.push((pname, pty));
                if *self.peek_kind() == TokenKind::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen, "期待 ')'")?;
        let body = self.parse_block();
        Ok(SubDef {
            name,
            is_async,
            params,
            body,
            span,
        })
    }

    fn parse_xformdef(&mut self) -> Result<XformDef, ()> {
        let span = self.current_span();
        self.advance(); // 'xformdef'
        let name = self.expect_ident("期待 xformdef 名")?;
        self.expect(TokenKind::LBrace, "期待 '{'")?;
        let mut slots = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            match self.parse_xf_slot() {
                Ok(s) => slots.push(s),
                Err(()) => self.recover(),
            }
        }
        self.expect(TokenKind::RBrace, "期待 '}'")?;
        Ok(XformDef { name, slots, span })
    }

    /// `[@wait] op_name(args);`（计划 Global Constraints 选定的 wait 前缀语法）。
    fn parse_xf_slot(&mut self) -> Result<XfSlotLit, ()> {
        let span = self.current_span();
        let wait: u16 = if *self.peek_kind() == TokenKind::At {
            self.advance();
            match self.peek_kind().clone() {
                TokenKind::IntLit(v) if (0..=u16::MAX as i32).contains(&v) => {
                    self.advance();
                    v as u16
                }
                _ => {
                    self.error_here("期待 0..=65535 的等待帧数");
                    return Err(());
                }
            }
        } else {
            0
        };
        let op_name = self.expect_ident("期待 xform 操作名")?;
        self.expect(TokenKind::LParen, "期待 '('")?;
        let args = self.parse_args()?;
        self.expect(TokenKind::RParen, "期待 ')'")?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(XfSlotLit {
            wait,
            op_name,
            args,
            span,
        })
    }

    fn parse_ty(&mut self) -> Result<Ty, ()> {
        match self.peek_kind() {
            TokenKind::KwInt => {
                self.advance();
                Ok(Ty::Int)
            }
            TokenKind::KwFx => {
                self.advance();
                Ok(Ty::Fx)
            }
            TokenKind::KwAngle => {
                self.advance();
                Ok(Ty::Angle)
            }
            _ => {
                self.error_here("期待类型（int/fx/angle）");
                Err(())
            }
        }
    }

    // ── 语句块 / 语句 ────────────────────────────────────────────────────

    /// 语句块（花括号内容）——**始终**返回一个 `Block`（尽力而为）：缺 `{` 时返回空块并留下
    /// 一条错误（当前 token 不消费，交给调用方自己的同步点处理）；块内每条语句独立恢复
    /// （一条语句出错不吞掉块内后续语句，见模块文档）；缺 `}`（EOF 先到）同样只记错误。
    fn parse_block(&mut self) -> Block {
        if self.expect(TokenKind::LBrace, "期待 '{'").is_err() {
            return Vec::new();
        }
        let mut stmts = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            match self.parse_stmt() {
                Ok(s) => stmts.push(s),
                Err(()) => self.recover(),
            }
        }
        let _ = self.expect(TokenKind::RBrace, "期待 '}'");
        stmts
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ()> {
        let kind = self.peek_kind().clone();
        match kind {
            TokenKind::Var => self.parse_var_stmt(),
            TokenKind::If => self.parse_if_stmt(),
            TokenKind::While => self.parse_while_stmt(),
            TokenKind::Loop => self.parse_loop_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::Wait => self.parse_wait_stmt(),
            TokenKind::Spawn => self.parse_spawn_stmt(),
            TokenKind::Return => {
                let span = self.current_span();
                self.advance();
                self.expect(TokenKind::Semi, "期待 ';'")?;
                Ok(Stmt::Return { span })
            }
            TokenKind::Break => {
                let span = self.current_span();
                self.advance();
                self.expect(TokenKind::Semi, "期待 ';'")?;
                Ok(Stmt::Break { span })
            }
            TokenKind::Continue => {
                let span = self.current_span();
                self.advance();
                self.expect(TokenKind::Semi, "期待 ';'")?;
                Ok(Stmt::Continue { span })
            }
            TokenKind::Underscore => self.parse_discard_stmt(),
            TokenKind::Ident(_) if self.peek_kind_at(1) == Some(&TokenKind::Eq) => {
                self.parse_assign_stmt()
            }
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_var_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'var'
        let name = self.expect_ident("期待变量名")?;
        self.expect(TokenKind::Colon, "期待 ':'")?;
        let ty = self.parse_ty()?;
        self.expect(TokenKind::Eq, "期待 '='")?;
        let init = self.parse_top_expr()?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::Var {
            name,
            ty,
            init,
            span,
        })
    }

    fn parse_assign_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        let name = self.expect_ident("期待标识符")?;
        self.expect(TokenKind::Eq, "期待 '='")?;
        let value = self.parse_top_expr()?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::Assign { name, value, span })
    }

    /// `_ = expr;`（值消费显式丢弃前缀——语义检查在 T2，T1 只记 `discarded=true`）。
    fn parse_discard_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // '_'
        self.expect(TokenKind::Eq, "期待 '='")?;
        let expr = self.parse_top_expr()?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::ExprStmt {
            expr,
            discarded: true,
            span,
        })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        let expr = self.parse_top_expr()?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::ExprStmt {
            expr,
            discarded: false,
            span,
        })
    }

    /// `if cond { } [else { } | else if ...]`——无强制括号（语言草图里 `while`/`for` 同款风格）。
    /// `else if` 链就地降低为"只含一条嵌套 `Stmt::If` 的单语句块"，AST 不设独立 `ElseIf`。
    fn parse_if_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'if'
        let cond = self.parse_top_expr()?;
        let then_b = self.parse_block();
        let else_b = if *self.peek_kind() == TokenKind::Else {
            self.advance();
            if *self.peek_kind() == TokenKind::If {
                let inner = self.parse_if_stmt()?;
                Some(vec![inner])
            } else {
                Some(self.parse_block())
            }
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_b,
            else_b,
            span,
        })
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'while'
        let cond = self.parse_top_expr()?;
        let body = self.parse_block();
        Ok(Stmt::While { cond, body, span })
    }

    fn parse_loop_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'loop'
        let body = self.parse_block();
        Ok(Stmt::Loop { body, span })
    }

    /// `for VAR in FROM..TO { }`：`..` 是本语句专用语法糖，不是通用范围表达式。
    fn parse_for_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'for'
        let var = self.expect_ident("期待循环变量名")?;
        self.expect(TokenKind::In, "期待 'in'")?;
        let from = self.parse_top_expr()?;
        self.expect(TokenKind::DotDot, "期待 '..'")?;
        let to = self.parse_top_expr()?;
        let body = self.parse_block();
        Ok(Stmt::For {
            var,
            from,
            to,
            body,
            span,
        })
    }

    fn parse_wait_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'wait'
        self.expect(TokenKind::LParen, "期待 '('")?;
        let frames = self.parse_top_expr()?;
        self.expect(TokenKind::RParen, "期待 ')'")?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::Wait { frames, span })
    }

    fn parse_spawn_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'spawn'
        let name = self.expect_ident("期待要 spawn 的 sub 名")?;
        self.expect(TokenKind::LParen, "期待 '('")?;
        let args = self.parse_args()?;
        self.expect(TokenKind::RParen, "期待 ')'")?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        Ok(Stmt::Spawn { name, args, span })
    }

    // ── 表达式：Pratt ────────────────────────────────────────────────────

    fn parse_top_expr(&mut self) -> Result<Expr, ()> {
        self.parse_expr(PREC_OR)
    }

    fn parse_expr(&mut self, min_prec: u8) -> Result<Expr, ()> {
        let mut lhs = self.parse_unary()?;
        loop {
            let Some((prec, op)) = infix_binding(self.peek_kind()) else {
                break;
            };
            if prec < min_prec {
                break;
            }
            let span = self.current_span();
            self.advance();
            // 左结合：右操作数至少要求 `prec+1`，同级运算符不会把自己吃进右子树。
            let rhs = self.parse_expr(prec + 1)?;
            lhs = Expr::Binary {
                op,
                l: Box::new(lhs),
                r: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ()> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let span = self.current_span();
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    e: Box::new(e),
                    span,
                })
            }
            TokenKind::Bang => {
                let span = self.current_span();
                self.advance();
                let e = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    e: Box::new(e),
                    span,
                })
            }
            _ => self.parse_cast(),
        }
    }

    /// cast 是紧贴在 primary 之后的后缀，绑定力比一元还强（`x as fx` 里 `x` 先取到，
    /// 再套一层 cast；链式 `x as fx as int` 左结合逐层包裹）。
    fn parse_cast(&mut self) -> Result<Expr, ()> {
        let mut e = self.parse_primary()?;
        while *self.peek_kind() == TokenKind::As {
            let span = self.current_span();
            self.advance();
            let ty = self.parse_ty()?;
            e = Expr::Cast {
                e: Box::new(e),
                to: ty,
                span,
            };
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, ()> {
        let span = self.current_span();
        let kind = self.peek_kind().clone();
        match kind {
            TokenKind::IntLit(v) => {
                self.advance();
                Ok(Expr::IntLit(v))
            }
            TokenKind::FxLit(v) => {
                self.advance();
                Ok(Expr::FxLit(v))
            }
            TokenKind::AngleLit(v) => {
                self.advance();
                Ok(Expr::AngleLit(v))
            }
            TokenKind::EngineVar(name) => {
                self.advance();
                match resolve_engine_var(&name) {
                    Some(ev) => Ok(Expr::EngineVar(ev, span)),
                    None => {
                        self.push_error(
                            span,
                            format!(
                                "未知的引擎变量 '${name}'（v1 支持：frame/player_x/player_y/\
                                 self_x/self_y/self_hp/self_hp_max/self_age）"
                            ),
                        );
                        Err(())
                    }
                }
            }
            TokenKind::LParen => {
                self.advance();
                let e = self.parse_top_expr()?;
                self.expect(TokenKind::RParen, "期待 ')'")?;
                Ok(e)
            }
            TokenKind::Ident(name) => {
                self.advance();
                if *self.peek_kind() == TokenKind::LParen {
                    self.advance(); // '('
                    if name == "global" {
                        // `global(n)`：契约块钉死的独立 AST 节点（区别于普通 `Call`），
                        // 恰好一个参数。
                        let slot = self.parse_top_expr()?;
                        self.expect(TokenKind::RParen, "期待 ')'")?;
                        Ok(Expr::GlobalRead {
                            slot: Box::new(slot),
                            span,
                        })
                    } else {
                        let args = self.parse_args()?;
                        self.expect(TokenKind::RParen, "期待 ')'")?;
                        Ok(Expr::Call { name, args, span })
                    }
                } else {
                    Ok(Expr::Var(name, span))
                }
            }
            other => {
                self.push_error(span, format!("期待表达式，遇到 {}", token_desc(&other)));
                Err(())
            }
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ()> {
        let mut args = Vec::new();
        if *self.peek_kind() != TokenKind::RParen {
            loop {
                args.push(self.parse_top_expr()?);
                if *self.peek_kind() == TokenKind::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::lex::Lexer;

    fn parse_src(src: &str) -> (Program, Vec<CompileError>) {
        let (tokens, mut errors) = Lexer::new(src).lex();
        let (program, parse_errors) = Parser::new(tokens, src).parse_program();
        errors.extend(parse_errors);
        (program, errors)
    }

    fn must_parse(src: &str) -> Program {
        let (program, errors) = parse_src(src);
        assert!(errors.is_empty(), "意外解析错误：{errors:?}\n源码：\n{src}");
        program
    }

    fn single_stmt(stmt_src: &str) -> Stmt {
        let src = format!("sub main() {{ {stmt_src} }}");
        let p = must_parse(&src);
        let mut body = p.subs.into_iter().next().expect("应有 main sub").body;
        assert_eq!(body.len(), 1, "期望恰一条语句，源码：{stmt_src}");
        body.remove(0)
    }

    /// 递归把 `Expr` 树里所有 `Span` 归零，供结构对比时忽略精确位置（位置正确性另有
    /// 专门的错误路径测试覆盖，这里只关心 AST 形状）。
    fn zero(e: &Expr) -> Expr {
        match e {
            Expr::IntLit(v) => Expr::IntLit(*v),
            Expr::FxLit(v) => Expr::FxLit(*v),
            Expr::AngleLit(v) => Expr::AngleLit(*v),
            Expr::Var(n, _) => Expr::Var(n.clone(), Span::default()),
            Expr::EngineVar(ev, _) => Expr::EngineVar(*ev, Span::default()),
            Expr::GlobalRead { slot, .. } => Expr::GlobalRead {
                slot: Box::new(zero(slot)),
                span: Span::default(),
            },
            Expr::Call { name, args, .. } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(zero).collect(),
                span: Span::default(),
            },
            Expr::Binary { op, l, r, .. } => Expr::Binary {
                op: *op,
                l: Box::new(zero(l)),
                r: Box::new(zero(r)),
                span: Span::default(),
            },
            Expr::Unary { op, e, .. } => Expr::Unary {
                op: *op,
                e: Box::new(zero(e)),
                span: Span::default(),
            },
            Expr::Cast { e, to, .. } => Expr::Cast {
                e: Box::new(zero(e)),
                to: *to,
                span: Span::default(),
            },
        }
    }

    /// 把 `_ = <expr>;` 包成一条语句解析出来，取其（span 归零后的）表达式树——专供表达式
    /// 形状类测试复用，避免每条测试都手写 `sub main(){}` 包装 + 目标语句解构。
    fn expr_of(expr_src: &str) -> Expr {
        let s = single_stmt(&format!("_ = {expr_src};"));
        match s {
            Stmt::ExprStmt {
                expr, discarded, ..
            } => {
                assert!(discarded);
                zero(&expr)
            }
            other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
        }
    }

    fn var(name: &str) -> Expr {
        Expr::Var(name.to_string(), Span::default())
    }

    // ── 顶层声明形状 ─────────────────────────────────────────────────────

    #[test]
    fn top_level_const_def_shape() {
        let p = must_parse("const RANK_SLOT: int = 0;");
        assert_eq!(p.consts.len(), 1);
        assert_eq!(p.consts[0].name, "RANK_SLOT");
        assert_eq!(p.consts[0].ty, Ty::Int);
        assert_eq!(zero(&p.consts[0].value), Expr::IntLit(0));
        assert!(p.subs.is_empty());
        assert!(p.xformdefs.is_empty());
    }

    #[test]
    fn top_level_sub_def_no_params_shape() {
        let p = must_parse("sub foo() { }");
        assert_eq!(p.subs.len(), 1);
        let s = &p.subs[0];
        assert_eq!(s.name, "foo");
        assert!(!s.is_async);
        assert!(s.params.is_empty());
        assert!(s.body.is_empty());
    }

    #[test]
    fn top_level_async_sub_with_param_shape() {
        let p = must_parse("async sub timer_ui(spell: int) { }");
        assert_eq!(p.subs.len(), 1);
        let s = &p.subs[0];
        assert_eq!(s.name, "timer_ui");
        assert!(s.is_async);
        assert_eq!(s.params, vec![("spell".to_string(), Ty::Int)]);
    }

    #[test]
    fn top_level_xformdef_with_wait_prefix_syntax_shape() {
        // 选定语法：`@N` 前缀 = 该条 op 之前的等待帧数（Global Constraints 推荐写法）。
        let p = must_parse("xformdef RING { turn(90deg); @8 set_ang_vel(128); }");
        assert_eq!(p.xformdefs.len(), 1);
        let xf = &p.xformdefs[0];
        assert_eq!(xf.name, "RING");
        assert_eq!(xf.slots.len(), 2);
        assert_eq!(xf.slots[0].wait, 0);
        assert_eq!(xf.slots[0].op_name, "turn");
        assert_eq!(
            xf.slots[0].args.iter().map(zero).collect::<Vec<_>>(),
            vec![Expr::AngleLit(16384)]
        );
        assert_eq!(xf.slots[1].wait, 8);
        assert_eq!(xf.slots[1].op_name, "set_ang_vel");
        assert_eq!(
            xf.slots[1].args.iter().map(zero).collect::<Vec<_>>(),
            vec![Expr::IntLit(128)]
        );
    }

    /// 负例：顶层出现语句层面的 token（既非 const/xformdef/sub/async）→ 报错，且恢复后
    /// 仍能继续解析后面合法的顶层声明。
    #[test]
    fn top_level_unknown_token_is_an_error_and_recovers() {
        let (p, errors) = parse_src("42;\nsub main() { }");
        assert!(!errors.is_empty());
        assert_eq!(p.subs.len(), 1);
        assert_eq!(p.subs[0].name, "main");
    }

    // ── 语句形状 ─────────────────────────────────────────────────────────

    #[test]
    fn var_stmt_shape() {
        let s = single_stmt("var x: fx = 1.5fx;");
        match s {
            Stmt::Var { name, ty, init, .. } => {
                assert_eq!(name, "x");
                assert_eq!(ty, Ty::Fx);
                assert_eq!(zero(&init), Expr::FxLit(98304));
            }
            other => panic!("期望 Stmt::Var，得到 {other:?}"),
        }
    }

    #[test]
    fn assign_stmt_shape() {
        let s = single_stmt("x = x + 1;");
        match s {
            Stmt::Assign { name, value, .. } => {
                assert_eq!(name, "x");
                assert_eq!(
                    zero(&value),
                    Expr::Binary {
                        op: BinOp::Add,
                        l: Box::new(var("x")),
                        r: Box::new(Expr::IntLit(1)),
                        span: Span::default(),
                    }
                );
            }
            other => panic!("期望 Stmt::Assign，得到 {other:?}"),
        }
    }

    #[test]
    fn discard_prefix_sets_discarded_flag() {
        let s = single_stmt("_ = fire(1, 2, 3);");
        match s {
            Stmt::ExprStmt {
                discarded, expr, ..
            } => {
                assert!(discarded);
                assert_eq!(
                    zero(&expr),
                    Expr::Call {
                        name: "fire".to_string(),
                        args: vec![Expr::IntLit(1), Expr::IntLit(2), Expr::IntLit(3)],
                        span: Span::default(),
                    }
                );
            }
            other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
        }
    }

    #[test]
    fn expr_stmt_without_discard_prefix_shape() {
        let s = single_stmt("move_to(90, -100fx, 100fx, 1);");
        match s {
            Stmt::ExprStmt {
                discarded, expr, ..
            } => {
                assert!(!discarded);
                let expected = Expr::Call {
                    name: "move_to".to_string(),
                    args: vec![
                        Expr::IntLit(90),
                        Expr::Unary {
                            op: UnOp::Neg,
                            e: Box::new(Expr::FxLit(6_553_600)),
                            span: Span::default(),
                        },
                        Expr::FxLit(6_553_600),
                        Expr::IntLit(1),
                    ],
                    span: Span::default(),
                };
                assert_eq!(zero(&expr), expected);
            }
            other => panic!("期望 Stmt::ExprStmt，得到 {other:?}"),
        }
    }

    #[test]
    fn if_else_shape() {
        let s = single_stmt("if a > 0 { b = 1; } else { b = 2; }");
        match s {
            Stmt::If {
                cond,
                then_b,
                else_b,
                ..
            } => {
                assert_eq!(
                    zero(&cond),
                    Expr::Binary {
                        op: BinOp::Gt,
                        l: Box::new(var("a")),
                        r: Box::new(Expr::IntLit(0)),
                        span: Span::default(),
                    }
                );
                assert_eq!(then_b.len(), 1);
                assert!(matches!(then_b[0], Stmt::Assign { .. }));
                let else_b = else_b.expect("应有 else 分支");
                assert_eq!(else_b.len(), 1);
                assert!(matches!(else_b[0], Stmt::Assign { .. }));
            }
            other => panic!("期望 Stmt::If，得到 {other:?}"),
        }
    }

    #[test]
    fn if_without_else_has_none_else_branch() {
        let s = single_stmt("if a { b = 1; }");
        match s {
            Stmt::If { else_b, .. } => assert!(else_b.is_none()),
            other => panic!("期望 Stmt::If，得到 {other:?}"),
        }
    }

    /// `else if` 链降低为"单语句块套一条嵌套 `Stmt::If`"，不是独立 AST 变体。
    #[test]
    fn else_if_chain_desugars_to_nested_if() {
        let s = single_stmt("if a { } else if b { } else { }");
        match s {
            Stmt::If { else_b, .. } => {
                let outer_else = else_b.expect("应有 else 分支");
                assert_eq!(outer_else.len(), 1);
                match &outer_else[0] {
                    Stmt::If {
                        else_b: inner_else, ..
                    } => assert!(inner_else.is_some()),
                    other => panic!("期望嵌套 Stmt::If，得到 {other:?}"),
                }
            }
            other => panic!("期望 Stmt::If，得到 {other:?}"),
        }
    }

    #[test]
    fn while_stmt_shape() {
        let s = single_stmt("while t > 0 { t = t - 1; }");
        match s {
            Stmt::While { cond, body, .. } => {
                assert_eq!(
                    zero(&cond),
                    Expr::Binary {
                        op: BinOp::Gt,
                        l: Box::new(var("t")),
                        r: Box::new(Expr::IntLit(0)),
                        span: Span::default(),
                    }
                );
                assert_eq!(body.len(), 1);
            }
            other => panic!("期望 Stmt::While，得到 {other:?}"),
        }
    }

    #[test]
    fn loop_stmt_shape() {
        let s = single_stmt("loop { wait(1); }");
        match s {
            Stmt::Loop { body, .. } => {
                assert_eq!(body.len(), 1);
                assert!(matches!(body[0], Stmt::Wait { .. }));
            }
            other => panic!("期望 Stmt::Loop，得到 {other:?}"),
        }
    }

    #[test]
    fn for_stmt_shape() {
        let s = single_stmt("for i in 0..5 { }");
        match s {
            Stmt::For {
                var: name,
                from,
                to,
                body,
                ..
            } => {
                assert_eq!(name, "i");
                assert_eq!(zero(&from), Expr::IntLit(0));
                assert_eq!(zero(&to), Expr::IntLit(5));
                assert!(body.is_empty());
            }
            other => panic!("期望 Stmt::For，得到 {other:?}"),
        }
    }

    #[test]
    fn wait_stmt_shape() {
        let s = single_stmt("wait(90);");
        match s {
            Stmt::Wait { frames, .. } => assert_eq!(zero(&frames), Expr::IntLit(90)),
            other => panic!("期望 Stmt::Wait，得到 {other:?}"),
        }
    }

    #[test]
    fn spawn_stmt_shape_no_args() {
        let s = single_stmt("spawn patrol();");
        match s {
            Stmt::Spawn { name, args, .. } => {
                assert_eq!(name, "patrol");
                assert!(args.is_empty());
            }
            other => panic!("期望 Stmt::Spawn，得到 {other:?}"),
        }
    }

    #[test]
    fn spawn_stmt_shape_with_args() {
        let s = single_stmt("spawn timer_ui(1);");
        match s {
            Stmt::Spawn { name, args, .. } => {
                assert_eq!(name, "timer_ui");
                assert_eq!(
                    args.iter().map(zero).collect::<Vec<_>>(),
                    vec![Expr::IntLit(1)]
                );
            }
            other => panic!("期望 Stmt::Spawn，得到 {other:?}"),
        }
    }

    #[test]
    fn return_break_continue_shapes() {
        assert!(matches!(single_stmt("return;"), Stmt::Return { .. }));
        assert!(matches!(single_stmt("break;"), Stmt::Break { .. }));
        assert!(matches!(single_stmt("continue;"), Stmt::Continue { .. }));
    }

    // ── 优先级 / 括号 / cast / $ 变量 / global ──────────────────────────

    #[test]
    fn precedence_mul_binds_tighter_than_add() {
        let e = expr_of("1+2*3");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinOp::Add,
                l: Box::new(Expr::IntLit(1)),
                r: Box::new(Expr::Binary {
                    op: BinOp::Mul,
                    l: Box::new(Expr::IntLit(2)),
                    r: Box::new(Expr::IntLit(3)),
                    span: Span::default(),
                }),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn precedence_arithmetic_binds_tighter_than_comparison() {
        let e = expr_of("1 + 2 > 3");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinOp::Gt,
                l: Box::new(Expr::Binary {
                    op: BinOp::Add,
                    l: Box::new(Expr::IntLit(1)),
                    r: Box::new(Expr::IntLit(2)),
                    span: Span::default(),
                }),
                r: Box::new(Expr::IntLit(3)),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn precedence_and_binds_looser_than_comparison() {
        // a > 0 && b < 1  =>  (a>0) && (b<1)
        let e = expr_of("a > 0 && b < 1");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinOp::And,
                l: Box::new(Expr::Binary {
                    op: BinOp::Gt,
                    l: Box::new(var("a")),
                    r: Box::new(Expr::IntLit(0)),
                    span: Span::default(),
                }),
                r: Box::new(Expr::Binary {
                    op: BinOp::Lt,
                    l: Box::new(var("b")),
                    r: Box::new(Expr::IntLit(1)),
                    span: Span::default(),
                }),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn precedence_or_is_lowest() {
        // a && b || c  =>  (a&&b) || c
        let e = expr_of("a && b || c");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinOp::Or,
                l: Box::new(Expr::Binary {
                    op: BinOp::And,
                    l: Box::new(var("a")),
                    r: Box::new(var("b")),
                    span: Span::default(),
                }),
                r: Box::new(var("c")),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn parens_override_precedence() {
        let e = expr_of("(1+2)*3");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinOp::Mul,
                l: Box::new(Expr::Binary {
                    op: BinOp::Add,
                    l: Box::new(Expr::IntLit(1)),
                    r: Box::new(Expr::IntLit(2)),
                    span: Span::default(),
                }),
                r: Box::new(Expr::IntLit(3)),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn cast_postfix_shape() {
        let e = expr_of("x as fx");
        assert_eq!(
            e,
            Expr::Cast {
                e: Box::new(var("x")),
                to: Ty::Fx,
                span: Span::default(),
            }
        );
    }

    #[test]
    fn cast_postfix_chains_left_to_right() {
        let e = expr_of("x as fx as int");
        assert_eq!(
            e,
            Expr::Cast {
                e: Box::new(Expr::Cast {
                    e: Box::new(var("x")),
                    to: Ty::Fx,
                    span: Span::default(),
                }),
                to: Ty::Int,
                span: Span::default(),
            }
        );
    }

    #[test]
    fn engine_var_shape() {
        assert_eq!(
            expr_of("$self_hp"),
            Expr::EngineVar(EngVar::SelfHp, Span::default())
        );
    }

    #[test]
    fn unknown_engine_var_is_an_error() {
        let (_, errors) = parse_src("sub main() { _ = $bogus; }");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].msg.contains("bogus"), "{}", errors[0].msg);
    }

    #[test]
    fn global_read_is_a_dedicated_ast_node() {
        let e = expr_of("global(RANK_SLOT)");
        assert_eq!(
            e,
            Expr::GlobalRead {
                slot: Box::new(var("RANK_SLOT")),
                span: Span::default(),
            }
        );
    }

    #[test]
    fn set_global_is_a_generic_call_not_global_read() {
        let e = expr_of("set_global(0, 1)");
        assert_eq!(
            e,
            Expr::Call {
                name: "set_global".to_string(),
                args: vec![Expr::IntLit(0), Expr::IntLit(1)],
                span: Span::default(),
            }
        );
    }

    // ── 错误路径：行列 + 源行摘录 + 恢复 ────────────────────────────────

    #[test]
    fn missing_semicolon_error_message_is_pinned() {
        let (_, errors) = parse_src("sub main() { var x: int = 5 }");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[0].msg, "期待 ';'，实际是 '}'");
    }

    #[test]
    fn missing_closing_brace_error_message_is_pinned() {
        let (_, errors) = parse_src("sub main() { if a { ");
        assert!(!errors.is_empty());
        assert!(
            errors.iter().any(|e| e.msg == "期待 '}'，实际是 文件末尾"),
            "{errors:?}"
        );
    }

    #[test]
    fn bad_token_at_expression_start_is_an_error() {
        let (_, errors) = parse_src("sub main() { var x: int = @; }");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].msg, "期待表达式，遇到 '@'");
    }

    /// 恢复策略核心判别：两处**独立**语句各自的错误都要报出来，不能第一个错误的恢复
    /// 把第二个错误所在的整条语句都吞掉。
    #[test]
    fn error_recovery_reports_two_independent_errors_same_file() {
        let src = "sub main() {\n    var x: int = ;\n    var y: int = 1 + ;\n}";
        let (_, errors) = parse_src(src);
        assert_eq!(errors.len(), 2, "两处独立错误都应报出：{errors:?}");
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[1].line, 3);
    }

    /// 端到端：真实解析产出的错误经 `render()` 精确匹配契约格式（行/列/源行/caret 对齐）。
    #[test]
    fn end_to_end_error_render_matches_pinned_format() {
        let src = "sub main() {\n  wait(1;\n}\n";
        let (_, errors) = parse_src(src);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[0].col, 9);
        assert_eq!(errors[0].msg, "期待 ')'，实际是 ';'");
        assert_eq!(errors[0].src_line, "  wait(1;");

        let rendered = errors[0].render("boss.ecl");
        let expected_line3 = format!("  {}^", " ".repeat(8));
        let expected = format!(
            "boss.ecl:2:9: 期待 ')'，实际是 ';'\n  {}\n{}",
            "  wait(1;", expected_line3
        );
        assert_eq!(rendered, expected);
    }
}
