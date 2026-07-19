//! ECL 表层语言词法器（手写、无第三方 crate；`Vec<char>` 游标 + 行列跟踪——`Peekable<Chars>`
//! 只给 1 字符前瞻，数字字面量的 `0..5` vs `1.5` 消歧需要 2 字符前瞻，`Vec<char>` 游标是同一
//! 手写理念下更省事的等价选择，见 T1 报告）。
//!
//! ## 设计取舍：批量产出 `Vec<Token>` + `Vec<CompileError>`，不做流式恢复
//!
//! 词法层没有语法层那种"跳到下一 `;`/`}`"结构化恢复——它自己就得保证**每条扫描分支都至少
//! 消费一个字符**（否则主循环会在同一个坏字符上死循环）。所以每个 `scan_*` 函数无论成功/
//! 失败都先吃掉触发它的字符，错误只是"记一笔，不产出 token，继续扫下一个"。整份源码扫描
//! 完毕后一次性交出 `(tokens, errors)`，语法层（`lang::parse`）在拿到的 token 流上单独做它
//! 自己的错误恢复。
//!
//! ## 单位字面量折叠规则（在这里完成，产物是确定整数——契约钉死值见本文件测试）
//!
//! - `Nfx`/`N.Nfx`/`Npx`/`N.Npx` → Q16.16 原始值：**精确十进制定点算术**（不过 f64），
//!   小数部分按 `round-half-to-even` 归一（`1.5fx→98304` 精确、`0.1fx→6554` 银行家舍入）。
//! - `Ndeg`/`N.Ndeg` → BAM 原始值：`deg × 65536 / 360` 四舍五入到偶（`round-half-to-even`），
//!   编译期 f64 折叠（spec 明文允许——编译器住断层线之上），`90deg → 16384`。
//! - `Nbam` → 原值直通（必须是不带小数点的整数，超出 `u16` 范围是词法错误）。
//! - 裸整数（无后缀、无小数点）→ `int`；有小数点却无后缀 → 词法错误（"缺单位后缀"）。

use crate::lang::ast::{CompileError, Span};

/// 词法单元种类。含全部关键字、运算符/标点、已折叠的字面量值、标识符与 `$` 引擎变量。
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── 字面量 / 名字 ──────────────────────────────────────────────────────
    IntLit(i32),
    FxLit(i32),
    AngleLit(u16),
    Ident(String),
    /// `$name`（`$` 之后的裸名字；是否是 v1 白名单内的合法引擎变量留给语法层判定——见
    /// `lang::parse::resolve_engine_var`，词法层只管切词）。
    EngineVar(String),

    // ── 关键字 ────────────────────────────────────────────────────────────
    Sub,
    Async,
    Var,
    If,
    Else,
    While,
    Loop,
    For,
    In,
    Wait,
    Spawn,
    Return,
    Break,
    Continue,
    Const,
    Xformdef,
    As,
    KwInt,
    KwFx,
    KwAngle,
    /// 单独的 `_`（丢弃前缀 `_ = expr;` 用；标识符里带下划线的其它写法仍是 `Ident`）。
    Underscore,

    // ── 运算符 / 标点 ─────────────────────────────────────────────────────
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Bang,
    Eq,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semi,
    Colon,
    Comma,
    /// `..`（计数 `for` 专用语法糖，见 `lang::parse` 对 `For` 语句的处理；不是通用范围表达式）。
    DotDot,
    /// `@`（`xformdef` 槽的 wait 前缀 `@8 turn(...)`，见计划 Global Constraints 选定语法）。
    At,

    Eof,
}

/// 一个词法单元：种类 + 起始位置（`Span`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// 手写词法器：`Vec<char>` 游标 + 行/列跟踪（列按字符数、非字节数）。
pub struct Lexer<'s> {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    lines: Vec<&'s str>,
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s str) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            lines: src.lines().collect(),
        }
    }

    #[inline]
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    #[inline]
    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// 消费当前字符并推进行列（`\n` 换行归位，其它字符列 +1）。
    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    #[inline]
    fn here(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    fn push_error(&self, errors: &mut Vec<CompileError>, span: Span, msg: impl Into<String>) {
        errors.push(CompileError::at(span, msg, &self.lines));
    }

    /// 跳过空白与两种注释（`//` 行注释、`/* */` 块注释，不支持嵌套）。未终止的块注释在
    /// EOF 处报错（位置=注释起点）并停止（后续没有更多输入可扫）。
    fn skip_ws_and_comments(&mut self, errors: &mut Vec<CompileError>) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.advance();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek2() == Some('*') => {
                    let start = self.here();
                    self.advance(); // '/'
                    self.advance(); // '*'
                    let mut closed = false;
                    while let Some(c) = self.peek() {
                        if c == '*' && self.peek2() == Some('/') {
                            self.advance();
                            self.advance();
                            closed = true;
                            break;
                        }
                        self.advance();
                    }
                    if !closed {
                        self.push_error(errors, start, "未终止的块注释（缺少匹配的 '*/'）");
                    }
                }
                _ => break,
            }
        }
    }

    /// 扫描一个数字字面量（含单位后缀折叠）。始终消费至少触发它的那个数字字符；错误路径
    /// 已在扫描过程中把相关字符吃掉，返回 `None`（不产出 token，调用方继续下一轮扫描）。
    fn scan_number(&mut self, span: Span, errors: &mut Vec<CompileError>) -> Option<TokenKind> {
        let mut int_digits = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                int_digits.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // 小数点仅在"点后紧跟数字"时才算作本字面量的一部分——`0..5` 的第一个 '.' 后面是
        // 另一个 '.'，不消费，把它留给主循环切成 `DotDot`。
        let mut frac_digits: Option<String> = None;
        if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            self.advance(); // '.'
            let mut f = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    f.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            frac_digits = Some(f);
        }

        let mut suffix = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                suffix.push(c);
                self.advance();
            } else {
                break;
            }
        }

        match suffix.as_str() {
            "" => {
                if frac_digits.is_some() {
                    self.push_error(errors, span, "浮点数字面量缺少单位后缀（fx 或 px）");
                    return None;
                }
                match int_digits.parse::<i64>() {
                    Ok(v) if v <= i32::MAX as i64 => Some(TokenKind::IntLit(v as i32)),
                    _ => {
                        self.push_error(errors, span, format!("整数字面量超出范围：{int_digits}"));
                        None
                    }
                }
            }
            "fx" | "px" => {
                let int_part: i64 = match int_digits.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        self.push_error(errors, span, format!("fx 字面量超出范围：{int_digits}"));
                        return None;
                    }
                };
                match fold_fx_decimal(int_part, frac_digits.as_deref()) {
                    Some(raw) => Some(TokenKind::FxLit(raw)),
                    None => {
                        self.push_error(errors, span, format!("fx 字面量超出范围：{int_digits}"));
                        None
                    }
                }
            }
            "deg" => {
                let text = format!("{int_digits}.{}", frac_digits.as_deref().unwrap_or("0"));
                let deg_val: Option<f64> = text.parse().ok();
                match deg_val.and_then(fold_deg_to_bam) {
                    Some(bam) => Some(TokenKind::AngleLit(bam)),
                    None => {
                        self.push_error(errors, span, format!("deg 字面量超出范围：{int_digits}"));
                        None
                    }
                }
            }
            "bam" => {
                if frac_digits.is_some() {
                    self.push_error(errors, span, "bam 字面量不接受小数点");
                    return None;
                }
                match int_digits.parse::<u32>() {
                    Ok(v) if v <= u16::MAX as u32 => Some(TokenKind::AngleLit(v as u16)),
                    _ => {
                        self.push_error(
                            errors,
                            span,
                            format!("bam 字面量超出 u16 范围：{int_digits}"),
                        );
                        None
                    }
                }
            }
            other => {
                self.push_error(
                    errors,
                    span,
                    format!("未知的数字单位后缀 '{other}'（支持 fx/px/deg/bam）"),
                );
                None
            }
        }
    }

    /// 标识符或关键字（`[A-Za-z_][A-Za-z0-9_]*`；单独的 `_` 是 `Underscore`）。
    fn scan_ident_or_keyword(&mut self) -> TokenKind {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        match s.as_str() {
            "sub" => TokenKind::Sub,
            "async" => TokenKind::Async,
            "var" => TokenKind::Var,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "loop" => TokenKind::Loop,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "wait" => TokenKind::Wait,
            "spawn" => TokenKind::Spawn,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "const" => TokenKind::Const,
            "xformdef" => TokenKind::Xformdef,
            "as" => TokenKind::As,
            "int" => TokenKind::KwInt,
            "fx" => TokenKind::KwFx,
            "angle" => TokenKind::KwAngle,
            "_" => TokenKind::Underscore,
            _ => TokenKind::Ident(s),
        }
    }

    /// `$name`：`$` 恒先消费（保证进度），之后若下一字符不是合法标识符起始字符则报错。
    fn scan_engine_var(&mut self, span: Span, errors: &mut Vec<CompileError>) -> Option<TokenKind> {
        self.advance(); // '$'
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        s.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
                Some(TokenKind::EngineVar(s))
            }
            _ => {
                self.push_error(errors, span, "'$' 后缺少引擎变量名");
                None
            }
        }
    }

    /// 运算符 / 标点（含未知字符兜底——恒消费触发字符，保证主循环前进）。
    fn scan_operator(
        &mut self,
        c: char,
        span: Span,
        errors: &mut Vec<CompileError>,
    ) -> Option<TokenKind> {
        match c {
            '+' => {
                self.advance();
                Some(TokenKind::Plus)
            }
            '-' => {
                self.advance();
                Some(TokenKind::Minus)
            }
            '*' => {
                self.advance();
                Some(TokenKind::Star)
            }
            '/' => {
                self.advance();
                Some(TokenKind::Slash)
            }
            '%' => {
                self.advance();
                Some(TokenKind::Percent)
            }
            '=' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Some(TokenKind::EqEq)
                } else {
                    Some(TokenKind::Eq)
                }
            }
            '!' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Some(TokenKind::NotEq)
                } else {
                    Some(TokenKind::Bang)
                }
            }
            '<' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Some(TokenKind::LtEq)
                } else {
                    Some(TokenKind::Lt)
                }
            }
            '>' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Some(TokenKind::GtEq)
                } else {
                    Some(TokenKind::Gt)
                }
            }
            '&' if self.peek2() == Some('&') => {
                self.advance();
                self.advance();
                Some(TokenKind::AndAnd)
            }
            '|' if self.peek2() == Some('|') => {
                self.advance();
                self.advance();
                Some(TokenKind::OrOr)
            }
            '(' => {
                self.advance();
                Some(TokenKind::LParen)
            }
            ')' => {
                self.advance();
                Some(TokenKind::RParen)
            }
            '{' => {
                self.advance();
                Some(TokenKind::LBrace)
            }
            '}' => {
                self.advance();
                Some(TokenKind::RBrace)
            }
            ';' => {
                self.advance();
                Some(TokenKind::Semi)
            }
            ':' => {
                self.advance();
                Some(TokenKind::Colon)
            }
            ',' => {
                self.advance();
                Some(TokenKind::Comma)
            }
            '.' if self.peek2() == Some('.') => {
                self.advance();
                self.advance();
                Some(TokenKind::DotDot)
            }
            '@' => {
                self.advance();
                Some(TokenKind::At)
            }
            other => {
                self.advance();
                self.push_error(errors, span, format!("意外的字符 '{other}'"));
                None
            }
        }
    }

    /// 扫描整份源码，交出词法单元流（恒以 `Eof` 收尾）+ 收集到的全部词法错误。
    pub fn lex(mut self) -> (Vec<Token>, Vec<CompileError>) {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();
        loop {
            self.skip_ws_and_comments(&mut errors);
            let span = self.here();
            let Some(c) = self.peek() else {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span,
                });
                break;
            };
            if c.is_ascii_digit() {
                if let Some(kind) = self.scan_number(span, &mut errors) {
                    tokens.push(Token { kind, span });
                }
                continue;
            }
            if c.is_ascii_alphabetic() || c == '_' {
                let kind = self.scan_ident_or_keyword();
                tokens.push(Token { kind, span });
                continue;
            }
            if c == '$' {
                if let Some(kind) = self.scan_engine_var(span, &mut errors) {
                    tokens.push(Token { kind, span });
                }
                continue;
            }
            if let Some(kind) = self.scan_operator(c, span, &mut errors) {
                tokens.push(Token { kind, span });
            }
        }
        (tokens, errors)
    }
}

/// `int_part.frac_digits` → Q16.16 原始值，精确十进制算术（round-half-to-even），不经过
/// 浮点。`None` = 结果超出 `i32` 范围。
fn fold_fx_decimal(int_part: i64, frac_digits: Option<&str>) -> Option<i32> {
    let mut value: i64 = int_part.checked_mul(65536)?;
    if let Some(f) = frac_digits
        && !f.is_empty()
    {
        let frac_num: i64 = f.parse().unwrap_or(0);
        let denom: i64 = 10i64.checked_pow(f.len() as u32)?;
        let numerator = frac_num.checked_mul(65536)?;
        let quotient = numerator / denom;
        let remainder = numerator % denom;
        let double_rem = remainder * 2;
        let rounded = match double_rem.cmp(&denom) {
            std::cmp::Ordering::Greater => quotient + 1,
            std::cmp::Ordering::Less => quotient,
            std::cmp::Ordering::Equal => {
                if quotient % 2 == 0 {
                    quotient
                } else {
                    quotient + 1
                }
            }
        };
        value = value.checked_add(rounded)?;
    }
    if (i32::MIN as i64..=i32::MAX as i64).contains(&value) {
        Some(value as i32)
    } else {
        None
    }
}

/// 十进制角度 → BAM u16（`deg × 65536 / 360`，round-half-to-even，天然回绕）。
/// `None` = 输入不是有限值（字面量位数过多导致 f64 解析饱和为 `inf`，`round_half_even`
/// 对 `inf - inf = NaN` 会算出溢出的 `i64` 加法——在此提前挡住，不留给它）。
fn fold_deg_to_bam(deg: f64) -> Option<u16> {
    if !deg.is_finite() {
        return None;
    }
    let raw = deg * 65536.0 / 360.0;
    if !raw.is_finite() {
        return None;
    }
    Some(round_half_even(raw).rem_euclid(65536) as u16)
}

/// f64 的 round-half-to-even（银行家舍入）到 `i64`。
fn round_half_even(x: f64) -> i64 {
    let floor = x.floor();
    let diff = x - floor;
    let floor_i = floor as i64;
    if diff < 0.5 {
        floor_i
    } else if diff > 0.5 {
        floor_i + 1
    } else if floor_i % 2 == 0 {
        floor_i
    } else {
        floor_i + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (tokens, errors) = Lexer::new(src).lex();
        assert!(errors.is_empty(), "意外词法错误：{errors:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    // ── 关键字 ────────────────────────────────────────────────────────────

    #[test]
    fn all_keywords_lex_to_dedicated_kinds() {
        let pairs: &[(&str, TokenKind)] = &[
            ("sub", TokenKind::Sub),
            ("async", TokenKind::Async),
            ("var", TokenKind::Var),
            ("if", TokenKind::If),
            ("else", TokenKind::Else),
            ("while", TokenKind::While),
            ("loop", TokenKind::Loop),
            ("for", TokenKind::For),
            ("in", TokenKind::In),
            ("wait", TokenKind::Wait),
            ("spawn", TokenKind::Spawn),
            ("return", TokenKind::Return),
            ("break", TokenKind::Break),
            ("continue", TokenKind::Continue),
            ("const", TokenKind::Const),
            ("xformdef", TokenKind::Xformdef),
            ("as", TokenKind::As),
            ("int", TokenKind::KwInt),
            ("fx", TokenKind::KwFx),
            ("angle", TokenKind::KwAngle),
            ("_", TokenKind::Underscore),
        ];
        for (src, expect) in pairs {
            let got = kinds(src);
            assert_eq!(got[0], *expect, "关键字 '{src}' 词法种类不对");
            assert_eq!(got[1], TokenKind::Eof);
        }
    }

    /// 负例的对照：与关键字形似但不同的普通标识符不应被误吞成关键字。
    #[test]
    fn near_keyword_identifiers_stay_identifiers() {
        assert_eq!(
            kinds("subroutine"),
            vec![TokenKind::Ident("subroutine".to_string()), TokenKind::Eof]
        );
        assert_eq!(
            kinds("_foo"),
            vec![TokenKind::Ident("_foo".to_string()), TokenKind::Eof]
        );
    }

    // ── 运算符 / 标点全集 ─────────────────────────────────────────────────

    #[test]
    fn all_operators_and_punctuation_lex_correctly() {
        let src = "+ - * / % == != < <= > >= && || ! = ( ) { } ; : , .. @";
        let expect = vec![
            TokenKind::Plus,
            TokenKind::Minus,
            TokenKind::Star,
            TokenKind::Slash,
            TokenKind::Percent,
            TokenKind::EqEq,
            TokenKind::NotEq,
            TokenKind::Lt,
            TokenKind::LtEq,
            TokenKind::Gt,
            TokenKind::GtEq,
            TokenKind::AndAnd,
            TokenKind::OrOr,
            TokenKind::Bang,
            TokenKind::Eq,
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::Semi,
            TokenKind::Colon,
            TokenKind::Comma,
            TokenKind::DotDot,
            TokenKind::At,
            TokenKind::Eof,
        ];
        assert_eq!(kinds(src), expect);
    }

    /// `0..5`：数字-点点-数字不应被误吞成一个带小数点的字面量（消歧的核心判别测试）。
    #[test]
    fn range_dots_are_not_swallowed_into_decimal_literal() {
        assert_eq!(
            kinds("0..5"),
            vec![
                TokenKind::IntLit(0),
                TokenKind::DotDot,
                TokenKind::IntLit(5),
                TokenKind::Eof,
            ]
        );
    }

    // ── 标识符 / `_` / `$` 引擎变量 ──────────────────────────────────────

    #[test]
    fn identifiers_and_engine_vars_lex_correctly() {
        assert_eq!(
            kinds("foo_bar123"),
            vec![TokenKind::Ident("foo_bar123".to_string()), TokenKind::Eof]
        );
        assert_eq!(
            kinds("$self_hp"),
            vec![TokenKind::EngineVar("self_hp".to_string()), TokenKind::Eof]
        );
    }

    /// 负例：孤立的 `$`（后面不是合法标识符起始字符）→ 报错且带准确位置，且不产出 token
    /// （错误后续扫描仍继续，验证于下一断言：`; ` 仍被正常切出）。
    #[test]
    fn lone_dollar_without_name_is_a_positioned_error() {
        let (tokens, errors) = Lexer::new("$ ;").lex();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[0].col, 1);
        assert!(
            errors[0].msg.contains('$'),
            "错误信息应点名 '$'：{}",
            errors[0].msg
        );
        assert_eq!(
            tokens,
            vec![
                Token {
                    kind: TokenKind::Semi,
                    span: Span { line: 1, col: 3 }
                },
                Token {
                    kind: TokenKind::Eof,
                    span: Span { line: 1, col: 4 }
                },
            ]
        );
    }

    // ── 单位字面量折叠——契约钉死值 ───────────────────────────────────────

    #[test]
    fn unit_literal_folding_pins_contract_values() {
        assert_eq!(
            kinds("90deg"),
            vec![TokenKind::AngleLit(16384), TokenKind::Eof]
        );
        assert_eq!(
            kinds("1.5fx"),
            vec![TokenKind::FxLit(98304), TokenKind::Eof]
        );
        assert_eq!(kinds("0.1fx"), vec![TokenKind::FxLit(6554), TokenKind::Eof]);
        assert_eq!(
            kinds("16384bam"),
            vec![TokenKind::AngleLit(16384), TokenKind::Eof]
        );
        assert_eq!(
            kinds("1.5px"),
            vec![TokenKind::FxLit(98304), TokenKind::Eof],
            "px 是 fx 别名"
        );
        assert_eq!(
            kinds("42"),
            vec![TokenKind::IntLit(42), TokenKind::Eof],
            "裸整数=int"
        );
    }

    /// 负例：小数点字面量缺单位后缀 → 词法错误（int 不允许小数）。
    #[test]
    fn float_literal_without_suffix_is_an_error() {
        let (tokens, errors) = Lexer::new("1.5;").lex();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].msg.contains("单位后缀"), "{}", errors[0].msg);
        // 出错的字面量本身不产出 token；后续 ';' 仍应正常切出。
        assert_eq!(
            tokens,
            vec![
                Token {
                    kind: TokenKind::Semi,
                    span: Span { line: 1, col: 4 }
                },
                Token {
                    kind: TokenKind::Eof,
                    span: Span { line: 1, col: 5 }
                },
            ]
        );
    }

    /// 负例：fx/px 整数部分溢出应报错，不能被 `unwrap_or(0)` 静默吞成 `FxLit(0)`
    /// （对照裸整数/bam 分支：同样的溢出场景两者都正确报错）。
    #[test]
    fn fx_literal_integer_part_overflow_is_an_error_not_silent_zero() {
        let src = "999999999999999999999999999999fx"; // 30 位 9，远超 i64/Q16.16 范围
        let (tokens, errors) = Lexer::new(src).lex();
        assert!(
            !errors.is_empty(),
            "整数部分溢出应报错，不应静默产出 token：{tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .all(|t| !matches!(t.kind, TokenKind::FxLit(_))),
            "溢出不应产出任何 FxLit token（哪怕是 0）：{tokens:?}"
        );
    }

    /// 负例：deg 字面量极端溢出（f64 解析饱和为 `inf`）不应让编译器 panic，必须走词法错误。
    #[test]
    fn deg_literal_extreme_overflow_is_an_error_not_a_panic() {
        let src = format!("{}deg", "9".repeat(320));
        let (tokens, errors) = Lexer::new(&src).lex();
        assert!(
            !errors.is_empty(),
            "极端 deg 字面量应报错而不是静默产出垃圾角度：{tokens:?}"
        );
    }

    /// 负例：未知的数字单位后缀 → 词法错误，报出坏后缀本身。
    #[test]
    fn unknown_numeric_suffix_is_an_error() {
        let (tokens, errors) = Lexer::new("5xf").lex();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].msg.contains("xf"), "{}", errors[0].msg);
        assert_eq!(
            tokens,
            vec![Token {
                kind: TokenKind::Eof,
                span: Span { line: 1, col: 4 }
            }]
        );
    }

    // ── 注释 ──────────────────────────────────────────────────────────────

    #[test]
    fn line_comment_is_skipped() {
        assert_eq!(
            kinds("// 这是一整行注释\n42"),
            vec![TokenKind::IntLit(42), TokenKind::Eof]
        );
    }

    #[test]
    fn block_comment_is_skipped_including_multiline() {
        assert_eq!(
            kinds("/* 块注释\n跨行 */ 42"),
            vec![TokenKind::IntLit(42), TokenKind::Eof]
        );
    }

    /// 负例：未终止的块注释 → 报错在注释起点，且不会死循环（测试本身能跑完即是证明）。
    #[test]
    fn unterminated_block_comment_is_an_error_at_comment_start() {
        let (tokens, errors) = Lexer::new("1; /* 没有收尾").lex();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[0].col, 4, "错误应指向注释起点 '/*' 的位置");
        assert!(errors[0].msg.contains("未终止"), "{}", errors[0].msg);
        // 注释之前的两个 token 仍正常切出。
        assert_eq!(tokens[0].kind, TokenKind::IntLit(1));
        assert_eq!(tokens[1].kind, TokenKind::Semi);
    }

    // ── 未知字符：位置断言 ───────────────────────────────────────────────

    /// 未知字符 `~` 报错且携带精确行列，扫描在错误后仍继续（前后 token 都正常切出）。
    #[test]
    fn unknown_char_reports_position_and_scanning_continues() {
        let (tokens, errors) = Lexer::new("1 ~ 2").lex();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[0].col, 3, "'~' 是源码里第 3 个字符");
        assert!(errors[0].msg.contains('~'), "{}", errors[0].msg);
        assert_eq!(
            tokens,
            vec![
                Token {
                    kind: TokenKind::IntLit(1),
                    span: Span { line: 1, col: 1 }
                },
                Token {
                    kind: TokenKind::IntLit(2),
                    span: Span { line: 1, col: 5 }
                },
                Token {
                    kind: TokenKind::Eof,
                    span: Span { line: 1, col: 6 }
                },
            ]
        );
    }

    /// 多行源码下未知字符的行号也要准确（不仅仅是列）。
    #[test]
    fn unknown_char_line_tracking_across_newlines() {
        let (_, errors) = Lexer::new("var x: int = 1;\n#\n").lex();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[0].col, 1);
    }
}
