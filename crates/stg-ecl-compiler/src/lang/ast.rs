//! ECL 表层语言 AST —— **跨任务契约**（本模块 T1 产出，T2 类型趟/槽分配趟、T3 codegen 消费）。
//!
//! 类型形状照抄 `docs/superpowers/plans/2026-07-18-m19-ecl-language.md` 的「核心接口」代码块
//! 逐字段对齐；本文件是该契约在 T1 的**唯一权威实现**（T2/T3 直接 `use` 这里的类型，不得
//! 另起炉灶重复定义）。字面量折叠（单位后缀 → 原始数值）在词法阶段完成（见 `lang::lex`），
//! 故这里的字面量变体已经是编译期常量原值：`FxLit(raw)` 是 Q16.16 原始 i32、`AngleLit(raw)`
//! 是 BAM 原始 u16——**不是**十进制文本。
//!
//! ## 关于 `CompileError` 与契约块的一处必要出入（已在 T1 报告里点名，非疏漏）
//!
//! 契约块给出的 `Display` 格式是 `"{file}:{line}:{col}: {msg}\n  {src_line}\n  {^ 定位}"`，
//! 但契约块给出的结构体字段只有 `line/col/msg/src_line`——**没有 `file` 字段**。若照单全收
//! 会让 `impl std::fmt::Display for CompileError` 拿不到 `file`（trait 方法签名只有 `&self`）。
//! 权衡：**不给结构体加 `file` 字段**（同一次 `parse`/`compile` 调用产出的所有错误共享同一个
//! `file`，逐错误重复存一份是浪费，且会让契约块的字段列表失真），改用**固有方法**
//! `CompileError::render(&self, file: &str) -> String` 实现该格式（调用方在真正要打印时传入
//! `file`）。字段形状与契约块逐一对齐，只是格式化方式从 trait 换成了固有方法——效果一致。
use std::fmt::Write as _;

/// 三型系统（拍板 2）：`int`（32 位有符号整数）/`fx`（Q16.16 定点）/`angle`（BAM u16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Int,
    Fx,
    Angle,
}

/// 源码位置：1-based 行/列（列按字符计数，非字节——注释里的中文字符按 1 算，见 `lang::lex`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
}

/// 顶层编译单元：一份 `.ecl` 源文件解析后的全部声明（顺序保留，供 T2/T3 按源码序处理）。
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub consts: Vec<ConstDef>,
    pub xformdefs: Vec<XformDef>,
    pub subs: Vec<SubDef>,
}

/// `const NAME: ty = expr;` —— 编译期常量声明（值折叠在 T2，T1 只记录未求值的 `Expr`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDef {
    pub name: String,
    pub ty: Ty,
    pub value: Expr,
    pub span: Span,
}

/// 一个语句块 = 顺序语句列表（花括号内容，见 `parse::Parser::parse_block`）。
pub type Block = Vec<Stmt>;

/// `[async] sub NAME(params) { body }`。
#[derive(Debug, Clone, PartialEq)]
pub struct SubDef {
    pub name: String,
    pub is_async: bool,
    pub params: Vec<(String, Ty)>,
    pub body: Block,
    pub span: Span,
}

/// `xformdef NAME { [@wait] op(args); ... }`（计划级补遗选定语法，见 `lang::parse` 文档）—
/// 编译期全常量（`slots` 里的 `Expr` 必须是常量表达式，T2 折叠校验；T1 只负责语法形状）。
#[derive(Debug, Clone, PartialEq)]
pub struct XformDef {
    pub name: String,
    pub slots: Vec<XfSlotLit>,
    pub span: Span,
}

/// `xformdef` 内一条 op 记录：`[@wait] op_name(args);`。`op_name` 到 xform-ops.md 号表的
/// 助记符解析是 T3 的事——T1 只存字符串，不做任何合法性校验。
#[derive(Debug, Clone, PartialEq)]
pub struct XfSlotLit {
    pub wait: u16,
    pub op_name: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

/// 语句。
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Var {
        name: String,
        ty: Ty,
        init: Expr,
        span: Span,
    },
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_b: Block,
        /// `else if` 链在解析期就地降低为「只含一条 `Stmt::If` 的单语句块」，AST 层没有
        /// 独立的 `ElseIf` 变体（契约块也没有，保持逐字段对齐）。
        else_b: Option<Block>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    /// `loop { }`：无条件、无限（`break` 是唯一出口，T1 不检查是否真的可达）。
    Loop {
        body: Block,
        span: Span,
    },
    /// `for VAR in FROM..TO { }`：计数 for，`..` 是纯语法糖（不是通用范围表达式，不出现在
    /// `Expr` 里——见 `lang::parse` 对 `DotDot` 的处理）。
    For {
        var: String,
        from: Expr,
        to: Expr,
        body: Block,
        span: Span,
    },
    Wait {
        frames: Expr,
        span: Span,
    },
    Spawn {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// 表达式语句；`discarded=true` ⇔ 源码写了 `_ = expr;` 前缀（值消费检查在 T2，T1 只记录
    /// 语法层面是否显式丢弃）。
    ExprStmt {
        expr: Expr,
        discarded: bool,
        span: Span,
    },
    Return {
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
}

/// 二元运算符（`Binary` 携带的 `op`；具体选哪条 VM 指令是 T2 按类型趟判型后的事，T1 只记
/// 语法层面的运算符种类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// 一元运算符：`-`（取负）/`!`（逻辑非）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// `$` 引擎状态变量（拍板 6，v1 白名单固定 8 个；只读——写走 `set_global`/setter 族 syscall，
/// 不经这条路）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngVar {
    Frame,
    PlayerX,
    PlayerY,
    SelfX,
    SelfY,
    SelfHp,
    SelfHpMax,
    SelfAge,
}

/// 表达式。
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// 裸整数字面量（`int` 型）。
    IntLit(i32),
    /// `N.Nfx`/`N.Npx`/`Nfx`/`Npx`：已折叠为 Q16.16 原始值（词法阶段完成，见 `lang::lex`）。
    FxLit(i32),
    /// `Ndeg`/`Nbam`：已折叠为 BAM 原始值。
    AngleLit(u16),
    Var(String, Span),
    /// `$xxx`：见 [`EngVar`]。
    EngineVar(EngVar, Span),
    /// 具名调用：目标是某个 `sub`，或内建函数表（`lang::builtins`，T2/T3 落地）里的一员——
    /// `global(n)` 读全局槽也是这里的普通一员。T1 不区分，统一存字符串名字，解析在后续趟。
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        l: Box<Expr>,
        r: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        e: Box<Expr>,
        span: Span,
    },
    /// `expr as ty`：跨型显式转换（语义在 T2 落地；T1 只记录语法形状）。
    Cast {
        e: Box<Expr>,
        to: Ty,
        span: Span,
    },
}

/// 编译错误：一条诊断 + 定位 + 源行摘录（不含 `file`——见本模块顶部文档的契约出入说明）。
#[derive(Debug, Clone, PartialEq)]
pub struct CompileError {
    pub line: u32,
    pub col: u32,
    pub msg: String,
    pub src_line: String,
}

impl CompileError {
    /// 从 [`Span`] + 消息 + 全源码行表构造一条错误（`src_lines` 由调用方一次性
    /// `src.lines().collect()`，词法/语法两侧共用同一份，避免重复扫描）。行号越界（理论上
    /// 不该发生，防御性兜底）时 `src_line` 退化为空串，不 panic（P4-a 精神：诊断路径自身
    /// 也不能崩）。
    pub fn at(span: Span, msg: impl Into<String>, src_lines: &[&str]) -> Self {
        let src_line = src_lines
            .get(span.line.saturating_sub(1) as usize)
            .copied()
            .unwrap_or("")
            .to_string();
        CompileError {
            line: span.line,
            col: span.col,
            msg: msg.into(),
            src_line,
        }
    }

    /// 渲染为契约钉死格式：
    /// ```text
    /// {file}:{line}:{col}: {msg}
    ///   {src_line}
    ///   {col-1 个字符的 caret 前缀}^
    /// ```
    /// （前两行前缀两格缩进；caret 行同样两格缩进后再补 `col-1` 个字符，使 `^` 精确落在
    /// `src_line` 里第 `col` 个字符正下方——`col` 是 1-based。）
    ///
    /// **C18 复审修复：caret 前缀逐字符对齐 `src_line`，tab 保留为 tab**——`col` 计数对 tab
    /// 和普通字符一视同仁（各计 1 列，见 `lang::lex::Lexer::advance`），但若源码用 tab
    /// 缩进，终端/编辑器会把 `src_line` 里的 tab 渲染成多列宽；caret 行若无差别地补等宽
    /// 空格，`^` 会指偏。这里把 tab 前缀里的 tab 原样保留、其余字符替换成空格——两行的
    /// tab 被同一套渲染规则展开成同样宽度，天然对齐。`src_line` 比 `col-1` 短（越界防御，
    /// 理论上不该发生）时补空格，不 panic，同 `at()` 的越界退化精神。
    pub fn render(&self, file: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{file}:{}:{}: {}", self.line, self.col, self.msg);
        let _ = writeln!(out, "  {}", self.src_line);
        let col_idx = self.col.saturating_sub(1) as usize;
        let mut caret_pad: String = self
            .src_line
            .chars()
            .take(col_idx)
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        let short_by = col_idx.saturating_sub(caret_pad.chars().count());
        if short_by > 0 {
            caret_pad.push_str(&" ".repeat(short_by));
        }
        let _ = write!(out, "  {caret_pad}^");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约格式钉死：手工构造一条错误，`render()` 逐字节匹配预期布局（行1诊断行，行2源行
    /// 两格缩进，行3 caret 精确对齐第 `col` 列）。
    #[test]
    fn render_pins_exact_contract_format() {
        let err = CompileError {
            line: 3,
            col: 9,
            msg: "期待 ';'，实际是 标识符 'var'".to_string(),
            src_line: "    var x: int = 5".to_string(),
        };
        let rendered = err.render("boss.ecl");
        let expected = concat!(
            "boss.ecl:3:9: 期待 ';'，实际是 标识符 'var'\n",
            "      var x: int = 5\n",
            "          ^" // 2 缩进 + 8 空格（col=9 → col-1=8）+ '^'
        );
        assert_eq!(rendered, expected);
    }

    /// C18 复审修复：源行含 tab 缩进时，caret 行必须保留原 tab 字符（不是替换成等宽空格）——
    /// 终端/编辑器把 src_line 和 caret 行里的 tab 展开成同样宽度，`^` 才能精确落在目标字符
    /// 正下方。`col` 计数对 tab 和普通字符一视同仁（各计 1 列，见 `lang::lex::advance`），
    /// 但如果 caret 行无差别地补等宽空格，tab 在渲染时展开的实际宽度会让 `^` 指偏。
    #[test]
    fn render_caret_preserves_tabs_in_src_line_prefix() {
        let err = CompileError {
            line: 1,
            col: 2, // 'x' 在 tab 之后，lex 记的是第 2 列（tab 本身计 1 列）
            msg: "占位".to_string(),
            src_line: "\tx = 1".to_string(),
        };
        let rendered = err.render("t.ecl");
        let caret_line = rendered.lines().nth(2).unwrap();
        assert_eq!(
            caret_line, "  \t^",
            "caret 前缀应保留原 tab 字符，而不是替换成等宽空格"
        );
    }

    /// `CompileError::at`：行号在 `src_lines` 范围内 → 摘录对应源行；越界 → 空串兜底不 panic。
    #[test]
    fn at_extracts_src_line_and_degrades_on_out_of_range() {
        let src_lines = ["line one", "line two", "line three"];
        let span = Span { line: 2, col: 5 };
        let e = CompileError::at(span, "测试消息", &src_lines);
        assert_eq!(e.line, 2);
        assert_eq!(e.col, 5);
        assert_eq!(e.msg, "测试消息");
        assert_eq!(e.src_line, "line two");

        let oob = CompileError::at(Span { line: 99, col: 1 }, "越界", &src_lines);
        assert_eq!(oob.src_line, "");
    }
}
