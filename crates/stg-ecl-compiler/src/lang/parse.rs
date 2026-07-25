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
                // `recover()` 的契约是"扫到 `}`/Eof 不消费，留给外层花括号循环判断退出"——
                // 顶层没有外层花括号循环接住它，孤立/泄漏的 `}` 必须在这里自己吃掉，否则
                // `while` 条件不变、`recover()` 原地返回，死循环。Eof 不消费也没关系，
                // `while` 条件本身会终止循环。
                if *self.peek_kind() == TokenKind::RBrace {
                    self.advance();
                } else {
                    self.recover();
                }
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
            // `wait_spell()`——符卡机构 spec 2026-07-24 §5 语法糖：不是关键字（词法层没有
            // 为它开专属 token，见 `lex::scan_ident_or_keyword`），只在"名字恰为
            // 'wait_spell' 且紧跟 '('"时才在这里截胡展开；guard 落空（比如误当变量名用
            // 在别处）照常落回下面通用的赋值/表达式语句路径，不占关键字位。
            TokenKind::Ident(ref name)
                if name == "wait_spell" && self.peek_kind_at(1) == Some(&TokenKind::LParen) =>
            {
                self.parse_wait_spell_stmt()
            }
            // `mark(...)`——同 `wait_spell` 的语句位前瞻截胡（整局流程刀 spec §2，Task 4）：
            // 不是关键字，只在"名字恰为 'mark' 且紧跟 '('"时才在这里截胡；guard 落空
            // （比如作为变量名用在别处，词法层未开专属 token）照常落回下面通用的赋值/
            // 表达式语句路径，不占关键字位——同 `wait_spell` 一致的"糖不抢词法位"纪律。
            TokenKind::Ident(ref name)
                if name == "mark" && self.peek_kind_at(1) == Some(&TokenKind::LParen) =>
            {
                self.parse_mark_stmt()
            }
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

    /// `wait_spell();`——纯前端语法糖（符卡机构 spec 2026-07-24 §5）：等价于
    /// `while spell_timer() >= 0 { wait(1); }`。**直接产出 `Stmt::While` 子树**，不新增
    /// 任何 AST 变体、不碰 codegen/typeck——两者对这条语句和手写的等价 `while` 一视同仁，
    /// 天然保证"糖=纯展开"（判别测试见 `codegen::tests`：两份源码编译出逐字节相同的
    /// `EclImage`）。`spell_timer`/`wait` 都是已注册的内建/语句，展开点必然可见。
    fn parse_wait_spell_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'wait_spell'（普通 Ident，非关键字）
        self.expect(TokenKind::LParen, "期待 '('")?;
        self.expect(TokenKind::RParen, "期待 ')'")?;
        self.expect(TokenKind::Semi, "期待 ';'")?;
        let cond = Expr::Binary {
            op: BinOp::Ge,
            l: Box::new(Expr::Call {
                name: "spell_timer".to_string(),
                args: Vec::new(),
                span,
            }),
            r: Box::new(Expr::IntLit(0)),
            span,
        };
        let body = vec![Stmt::Wait {
            frames: Expr::IntLit(1),
            span,
        }];
        Ok(Stmt::While { cond, body, span })
    }

    /// `mark(<id>);` 或 `mark(<id>) { <补偿块> }`（整局流程刀 spec §2，Task 4）：吃
    /// `mark ( <expr> )`，后随 `{` 则 [`Self::parse_block`] 收补偿块（此时无分号——同
    /// `if`/`while`/`for` 块收尾惯例），否则期待 `;`（纯落点，无补偿块）。位置/id 合法性
    /// （仅 main 顶层、编译期常量、正整数、不重复）不在这里查——那是 `lang::typeck`
    /// 的 `validate_marks` 的职责，语法层只管形状。
    fn parse_mark_stmt(&mut self) -> Result<Stmt, ()> {
        let span = self.current_span();
        self.advance(); // 'mark'（普通 Ident，非关键字）
        self.expect(TokenKind::LParen, "期待 '('")?;
        let id = self.parse_top_expr()?;
        self.expect(TokenKind::RParen, "期待 ')'")?;
        let block = if *self.peek_kind() == TokenKind::LBrace {
            Some(self.parse_block())
        } else {
            self.expect(TokenKind::Semi, "期待 ';'")?;
            None
        };
        Ok(Stmt::Mark { id, block, span })
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
                    // `global(n)` 不再特判——它是 `lang::builtins` 表里的普通一员（C16
                    // 复审修复：特判会让它永远绕过 `check_call` 的"先查 subs、再查
                    // builtins"顺序，同名 sub 静默调不到，见 `lang::builtins` 模块文档）。
                    let args = self.parse_args()?;
                    self.expect(TokenKind::RParen, "期待 ')'")?;
                    Ok(Expr::Call { name, args, span })
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
mod tests;
