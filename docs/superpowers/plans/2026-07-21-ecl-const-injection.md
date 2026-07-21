# ECL 常量注入 + const 求值收编（C14）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `.ecl` 源码能引用引擎侧命名常量（`APPEARANCE_STAR` 等）并声明自身 const，同时把三份 const 求值逻辑收编成一份共享纯模块。

**Architecture:** stg-core 用单一 `.rs` 注册表（`engine_consts!` 宏）从一处定义同时生成 Rust 常量与脚本注入列表 `ENGINE_CONSTS`；编译器把 `lang::type_rules`（矩阵/意图）与 `lang::const_eval`（求值器）提到 `lang::` 级供 typeck 与 codegen 共享，并在 typeck 前把引擎常量当"第 1 行前声明的 const"预填。全部改动在断层线以上，零运行时、零确定性面。

**Tech Stack:** Rust 2024，`stg-core`、`stg-ecl-compiler`、`stg-harness`，手写 ECL 前端 + 栈机，`macro_rules!`，Cargo test/Clippy，确定性金向量。

## Global Constraints

- 全部改动在断层线以上：stg-core 侧只新增**静态常量定义**（无运行时逻辑、无 float、无外部依赖、无 build.rs）；编译器侧纯前端改动。
- 依赖防火墙不破：`cargo tree -p stg-core` 不新增任何外部依赖。
- 矩阵与 const 求值器全仓各恰**一份**：收编后 `typeck/matrix.rs` 与 `typeck/intents.rs` 不复存在。
- 引擎常量值单一权威：`engine_consts!` 宏是"值 + Rust 常量 + 脚本注入名"的唯一定义site；旧模块用 `pub use` 再导出，旧引用路径保持可用。
- 脚本**不得**影子引擎常量（重名 → 编译期拒绝）。
- 注入的常量在 codegen 落为字面量值（`ConstRef(v) → push_i`），运行时字节码不含名字；金向量校验和值不变时逐位一致。
- 保留用户对 `README.md` 的未暂存改动；不要 stage 它。
- 收编是纯搬家：既有 const 折叠/判型测试**断言不改**，靠"编译通过 + 全测试绿"证明等价。

---

### Task 1: 收编 const 求值——`lang::type_rules` + `lang::const_eval` 归一

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/const_eval.rs`（删 `GlobalRead` 臂——当前 AST 无此变体）
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（注册 `mod type_rules;` `mod const_eval;`）
- Modify: `crates/stg-ecl-compiler/src/lang/ast.rs`（迁入 `expr_span`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/consts.rs`（`fold_const`/`fold_binary_const` 改薄封装调 `const_eval::evaluate`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`（import 改指 `type_rules` + `ast::expr_span`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/stmts.rs`（`expr_span` import 改指 `ast`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/typed_ast.rs`（intent 类型改指 `type_rules`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck.rs`（re-export 改指 `type_rules`）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（删 `eval_const_arg`，xformdef 槽参数改调 `const_eval::evaluate`；intent import 就近核对）
- Delete: `crates/stg-ecl-compiler/src/lang/typeck/matrix.rs`
- Delete: `crates/stg-ecl-compiler/src/lang/typeck/intents.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/typeck.rs`（移除 `mod matrix;` `mod intents;`）
- Test: `crates/stg-ecl-compiler/src/lang/codegen.rs` 内联测试

**Interfaces:**
- Consumes: 现有 `Expr`/`Ty`/`BinOp`/`UnOp`（`lang::ast`），未跟踪的 `lang/type_rules.rs`（导出 `binary_result`、`cast_intent`、`op_symbol`、`BinIntent`、`UnIntent`、`CastIntent`）与 `lang/const_eval.rs`（导出 `pub(super) fn evaluate(&Expr, &BTreeMap<String,(Ty,i32)>) -> Result<(Ty,i32), ConstEvalError>` + `pub(super) struct ConstEvalError { span, msg, needs_cast_hint }`）。
- Produces: `crate::lang::type_rules::{BinIntent, UnIntent, CastIntent, binary_result, cast_intent, op_symbol}`（矩阵/意图唯一权威）；`crate::lang::const_eval::evaluate`（求值器唯一实现）；`crate::lang::ast::expr_span`。

- [ ] **Step 1: 先跑全绿基线（收编前）**

Run:
```bash
cargo test -p stg-ecl-compiler
```
Expected: 全绿。记下测试总数（收编后须一致或仅 +1 新测试）。

- [ ] **Step 2: 修 `const_eval.rs` 的 stale `GlobalRead` 臂**

`crates/stg-ecl-compiler/src/lang/const_eval.rs` 里删掉这一行（当前 `Expr` 无 `GlobalRead` 变体，`global()` 由下面的 `Expr::Call` 臂覆盖）：

```rust
        Expr::GlobalRead { span, .. } => Err(not_constant(*span, "`global()`（非编译期可求值）")),
```

删后 9 个 `Expr` 变体（IntLit/FxLit/AngleLit/Var/EngineVar/Call/Binary/Unary/Cast）由现存臂穷尽覆盖（`Unary` 被 `Neg`/`Not` 两臂覆盖，`UnOp` 恰两变体）。

- [ ] **Step 3: 把 `expr_span` 迁入 `ast.rs`**

在 `crates/stg-ecl-compiler/src/lang/ast.rs` 末尾（`Span` 定义之后同文件）加：

```rust
/// 表达式的诊断锚点 span（字面量无 span → `None`，调用方 `.unwrap_or(fallback)`）。
/// M0-9 前住 typeck::matrix，C14 收编时迁来 AST 层（它是 AST 遍历助手，非类型规则）。
pub fn expr_span(e: &Expr) -> Option<Span> {
    match e {
        Expr::IntLit(_) | Expr::FxLit(_) | Expr::AngleLit(_) => None,
        Expr::Var(_, s)
        | Expr::EngineVar(_, s)
        | Expr::Call { span: s, .. }
        | Expr::Binary { span: s, .. }
        | Expr::Unary { span: s, .. }
        | Expr::Cast { span: s, .. } => Some(*s),
    }
}
```

- [ ] **Step 4: 在 `lang/mod.rs` 注册两个纯模块**

`crates/stg-ecl-compiler/src/lang/mod.rs` 顶部模块声明区加（与既有 `mod` 并列）：

```rust
mod const_eval;
mod type_rules;
```

- [ ] **Step 5: 改 `typeck.rs` 的模块声明与 re-export**

`crates/stg-ecl-compiler/src/lang/typeck.rs`：删 `mod matrix;` 与 `mod intents;`；把
```rust
pub use intents::{BinIntent, CastIntent, UnIntent};
```
改为
```rust
pub use crate::lang::type_rules::{BinIntent, CastIntent, UnIntent};
```

- [ ] **Step 6: 改各 import 站点指向新家**

`typeck/exprs.rs` 顶部两行：
```rust
use super::intents::{CastIntent, UnIntent};
use super::matrix::{binary_result, expr_span, op_symbol};
```
改为：
```rust
use crate::lang::ast::expr_span;
use crate::lang::type_rules::{CastIntent, UnIntent, binary_result, op_symbol};
```

`typeck/stmts.rs`：`use super::matrix::expr_span;` → `use crate::lang::ast::expr_span;`

`typeck/typed_ast.rs`：`use super::intents::{BinIntent, CastIntent, UnIntent};` → `use crate::lang::type_rules::{BinIntent, CastIntent, UnIntent};`

`codegen.rs`：其 `BinIntent`/`CastIntent` 来自 `typed_ast` 的 re-export（`typed_ast` 已改指 `type_rules`），一般无需改；若 `cargo build` 报未解析，就把 codegen 里相应 import 直接改成 `use crate::lang::type_rules::{BinIntent, CastIntent};`。

- [ ] **Step 7: `typeck/consts.rs` 改薄封装调 `const_eval`**

把 `crates/stg-ecl-compiler/src/lang/typeck/consts.rs` 里 `fold_const` + `fold_binary_const` 两个方法整体替换为一个调 `const_eval::evaluate` 的薄封装（`check_const_def` 保持不变）：

```rust
use super::checker::Checker;
use crate::lang::ast::ConstDef;
use crate::lang::const_eval::{self, ConstEvalError};

impl<'p> Checker<'p> {
    /// 常量折叠：委托 `lang::const_eval` 唯一求值器，把其错误转本趟诊断（保留既有文案）。
    pub(super) fn fold_const(&mut self, e: &crate::lang::ast::Expr) -> Option<(crate::lang::ast::Ty, i32)> {
        match const_eval::evaluate(e, &self.consts) {
            Ok(pair) => Some(pair),
            Err(ConstEvalError { span, msg, needs_cast_hint }) => {
                if needs_cast_hint {
                    self.push_type_mismatch(span, msg);
                } else {
                    self.err(span, msg);
                }
                None
            }
        }
    }
}
```

> `push_type_mismatch` 是 `Checker` 既有方法（原 `fold_binary_const` 类型不匹配走它）；`needs_cast_hint` 由 `const_eval` 在类型不匹配时置位，对齐原行为。若 `check_const_def` 或其它 typeck 代码引用了已删的 `super::matrix::*`/`fold_binary_const`，一并清理。

- [ ] **Step 8: `codegen.rs` 删 `eval_const_arg`，改调 `const_eval::evaluate`**

`codegen.rs` 里 xformdef 槽参数求值处（原 `eval_const_arg(a, &self.consts)`）改为调 `const_eval::evaluate`。codegen 的 `self.consts` 是 `BTreeMap<String, i32>`（只值），而 `evaluate` 需 `BTreeMap<String,(Ty,i32)>`——就近构造带类型的临时表，或把 codegen 的 consts 表升为带 `Ty`（从 `ti.consts` 本就有 `Ty`，见 `generate` 里 `ti.consts.iter().map(|(n,_,v)|...)` 丢了 `Ty`，改为保留）。推荐后者：

在 `generate` 里把
```rust
let consts: BTreeMap<String, i32> = ti.consts.iter().map(|(n, _, v)| (n.clone(), *v)).collect();
```
改为
```rust
let consts: BTreeMap<String, (Ty, i32)> = ti.consts.iter().map(|(n, t, v)| (n.clone(), (*t, *v))).collect();
```
并把 xformdef 槽参数求值改为：
```rust
match const_eval::evaluate(a, &self.consts) {
    Ok((_ty, v)) => { /* 原成功分支：用 v */ }
    Err(e) => { /* 转 CompileError：msg 含"编译期常量"契合既有测试 */ }
}
```
错误转换须让 `msg` 仍含子串 `编译期常量`（`xformdef_non_const_arg_is_a_compile_error` 断言它）。`const_eval` 对 `$`/调用的报错文案已含"非编译期可求值"，对 xformdef 语境包一层 `format!("xformdef 槽参数必须是编译期常量：{}", e.msg)` 即可。顶部加 `use crate::lang::const_eval;`。

- [ ] **Step 9: 删两个旧文件并编译**

```bash
git rm crates/stg-ecl-compiler/src/lang/typeck/matrix.rs crates/stg-ecl-compiler/src/lang/typeck/intents.rs
cargo build -p stg-ecl-compiler
```
Expected: 编译通过。若报未解析符号，按报错把残余 `super::matrix::`/`super::intents::` 引用改指 `type_rules`/`ast`。

- [ ] **Step 10: 加一条 xformdef 二元 const 表达式测试（收编附带能力）**

`codegen.rs` 测试模块加（xformdef 槽参数现支持完整 const 表达式）：

```rust
#[test]
fn xformdef_slot_arg_accepts_binary_const_expr() {
    // 收编后 xformdef 槽参数走 const_eval，支持二元算术（原 eval_const_arg 只字面量/一元负）
    let src = "const A: int = 2;\n\
               xformdef OK { turn(A + 1); }\n\
               sub main() { _ = fire(0, 0fx, 0fx, 1.0fx, 0deg, OK, none); loop { wait(1); } }";
    assert!(compile(src, "ok.ecl").is_ok(), "二元 const 表达式应被 xformdef 槽参数接受");
}
```

> `turn` 的槽参数元数/类型按现有 xformdef 语法核对（照 `xformdef_non_const_arg_is_a_compile_error` 的 `turn($frame)` 同形，只把 `$frame` 换成 `A + 1`）。若 `turn` 参数是 angle 型，改用 `A + 1` 前给它配一个 angle const 或按实际签名调整字面量——以本文件既有 xformdef 测试的合法源码为准。

- [ ] **Step 11: 全测试 + clippy，提交**

Run:
```bash
cargo test -p stg-ecl-compiler
cargo clippy -p stg-ecl-compiler --all-targets -- -D warnings
```
Expected: 全绿（数量 = Step 1 基线 + 1 新测试）；clippy 无警告。用 `rg 'mod matrix|mod intents|super::matrix|super::intents' crates/stg-ecl-compiler/src` 确认无残留。

```bash
git add crates/stg-ecl-compiler/src/lang
git commit -m "refactor(lang): unify const evaluation into lang::const_eval + type_rules

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `engine_consts!` 单一注册表（stg-core）

**Files:**
- Create: `crates/stg-core/src/consts.rs`
- Modify: `crates/stg-core/src/lib.rs`（`pub mod consts;`）
- Modify: `crates/stg-core/src/tables.rs`（删 `APPEARANCE_*` 定义，改 `pub use crate::consts::...`）
- Modify: `crates/stg-core/src/world.rs`（删 `GVAR_RANK`/`GLOBALS_SYS_SEGMENT` 定义，改 `pub use`，保留段纪律 doc）
- Test: `crates/stg-core/src/consts.rs` 内联测试

**Interfaces:**
- Consumes: `crate::ecl::image::EclValueType`（`Int`/`Fx`/`Angle`，已 `derive(PartialEq, Eq)`）。
- Produces: `crate::consts::{EngineConst, ENGINE_CONSTS}`；`pub const APPEARANCE_SMALL/MEDIUM/LARGE/STAR: u16`、`GVAR_RANK: u16`、`GLOBALS_SYS_SEGMENT: u16`（经 `pub use` 从 `tables`/`world` 仍可达）。

- [ ] **Step 1: 写失败测试（注册表内容 + 类型）**

新建 `crates/stg-core/src/consts.rs`，先只放测试（实现下一步补）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::EclValueType;

    #[test]
    fn engine_consts_registry_exposes_named_ids() {
        // Rust 侧常量维持 u16 原型
        assert_eq!(APPEARANCE_STAR, 3u16);
        assert_eq!(GVAR_RANK, 0u16);
        assert_eq!(GLOBALS_SYS_SEGMENT, 16u16);
        // 注入列表把它们作为 i32/Int 携带
        let star = ENGINE_CONSTS.iter().find(|c| c.name == "APPEARANCE_STAR").unwrap();
        assert_eq!(star.ty, EclValueType::Int);
        assert_eq!(star.value, 3);
        // 名字唯一
        let n = ENGINE_CONSTS.len();
        let mut names: Vec<&str> = ENGINE_CONSTS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "引擎常量名必须唯一");
    }
}
```

- [ ] **Step 2: 跑测试验证红**

Run:
```bash
cargo test -p stg-core consts::tests::engine_consts_registry_exposes_named_ids -- --exact
```
Expected: 编译失败（`EngineConst`/`ENGINE_CONSTS`/常量未定义）。

- [ ] **Step 3: 实现 `EngineConst` + `engine_consts!` 宏 + 注册表**

在 `crates/stg-core/src/consts.rs` 测试模块**之上**加：

```rust
//! 脚本可见引擎常量的单一注册表（C14）。
//!
//! `engine_consts!` 宏对每行同时生成 ① `pub const NAME: <rust_ty>`（Rust 侧照常用，类型保真）
//! ② 汇入 `ENGINE_CONSTS: &[EngineConst]`（编译器注入进 `.ecl` 命名空间，脚本侧 i32）。
//! 值只写一次、两头自动出，杜绝漂移。加新配置 = 加一行。叶子模块（crate 根），无依赖环。

use crate::ecl::image::EclValueType;

/// 一条注入给 `.ecl` 编译器的命名常量（脚本侧统一 i32：Fx=raw、Angle=raw as i32）。
pub struct EngineConst {
    pub name: &'static str,
    pub ty: EclValueType,
    pub value: i32,
}

impl EngineConst {
    pub const fn new(name: &'static str, ty: EclValueType, value: i32) -> Self {
        Self { name, ty, value }
    }
}

macro_rules! engine_consts {
    ( $( $name:ident : $rust_ty:ty as $script:ident = $val:expr ; )* ) => {
        $( pub const $name: $rust_ty = $val; )*
        /// 全部脚本可见引擎常量（编译器注入用）。定义见本文件 `engine_consts!` 块。
        pub const ENGINE_CONSTS: &[EngineConst] = &[
            $( EngineConst::new(stringify!($name), engine_consts!(@ty $script), $val as i32), )*
        ];
    };
    (@ty int)   => { EclValueType::Int };
    (@ty fx)    => { EclValueType::Fx };
    (@ty angle) => { EclValueType::Angle };
}

engine_consts! {
    //  名字                Rust 类型  脚本类型  值
    APPEARANCE_SMALL:       u16 as int = 0;
    APPEARANCE_MEDIUM:      u16 as int = 1;
    APPEARANCE_LARGE:       u16 as int = 2;
    APPEARANCE_STAR:        u16 as int = 3;
    GVAR_RANK:              u16 as int = 0;
    GLOBALS_SYS_SEGMENT:    u16 as int = 16;
}
```

- [ ] **Step 4: 注册模块并跑测试验证绿**

`crates/stg-core/src/lib.rs` 加 `pub mod consts;`（与其它 `pub mod` 并列，放在 `ecl` 之后即可——它只依赖 `ecl::image`）。

Run:
```bash
cargo test -p stg-core consts::tests::engine_consts_registry_exposes_named_ids -- --exact
```
Expected: PASS。

- [ ] **Step 5: 迁移旧常量定义 + `pub use` 再导出**

`crates/stg-core/src/tables.rs`：删掉 `APPEARANCE_SMALL/MEDIUM/LARGE/STAR` 四个 `pub const` 定义，替换为一行：
```rust
pub use crate::consts::{APPEARANCE_LARGE, APPEARANCE_MEDIUM, APPEARANCE_SMALL, APPEARANCE_STAR};
```

`crates/stg-core/src/world.rs`：删掉 `GVAR_RANK`、`GLOBALS_SYS_SEGMENT` 两个 `pub const` 定义（**保留它们上方的段纪律 doc 注释**，挂到 `pub use` 行上）：
```rust
/// globals 段纪律（甲案，M1.5）…（原 doc 保留，说明系统段/自由段/RANK 槽语义）
pub use crate::consts::{GLOBALS_SYS_SEGMENT, GVAR_RANK};
```
`GLOBALS_CAP`（world.rs）**不迁**（脚本不引用，YAGNI），留原地。

- [ ] **Step 6: 全 crate 测试 + clippy + 防火墙，提交**

Run:
```bash
cargo test -p stg-core
cargo clippy -p stg-core --all-targets -- -D warnings
cargo tree -p stg-core | wc -l   # 记数，Task 结束再比，确认无新依赖
```
Expected: 全绿（旧引用 `crate::tables::APPEARANCE_STAR` / `crate::world::GVAR_RANK` 经再导出仍编译）；clippy 净；依赖树未变。

```bash
git add crates/stg-core/src/consts.rs crates/stg-core/src/lib.rs crates/stg-core/src/tables.rs crates/stg-core/src/world.rs
git commit -m "feat(core): single engine-const registry via engine_consts! macro

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: 把引擎常量注入编译管线

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/typeck.rs`（`check` 增参 + 预填 + `eclty_to_ty`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/checker.rs`（`Checker` 加 `engine_const_names` 字段）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/consts.rs`（`check_const_def` 区分引擎重名文案）
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（`compile_with_options` 增参；`compile` 默认注入；调用点更新）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/tests.rs`（`check` 调用点补 `&[]` + 注入正/负向测试）

**Interfaces:**
- Consumes: `stg_core::consts::{EngineConst, ENGINE_CONSTS}`（Task 2）；`stg_core::ecl::image::EclValueType`。
- Produces: `typeck::check(prog: &Program, engine_consts: &[EngineConst]) -> Result<TypedInfo, Vec<CompileError>>`；`compile_with_options(src, file, options, engine_consts: &[EngineConst])`；`compile(src, file)`（签名不变，内部默认注入 `ENGINE_CONSTS`）。

- [ ] **Step 1: 写失败测试（注入正向 + 影子负向）**

`crates/stg-ecl-compiler/src/lang/typeck/tests.rs` 顶部 import 处补：
```rust
use stg_core::consts::EngineConst;
use stg_core::ecl::image::EclValueType;
```
测试模块加：
```rust
fn check_with(src: &str, engine: &[EngineConst]) -> Result<TypedInfo, Vec<CompileError>> {
    typeck::check(&prog(src), engine)
}

#[test]
fn engine_consts_are_injected_as_predeclared_constants() {
    let ti = check_with(
        "sub main() { var a: int = FOO; loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 7)],
    )
    .expect("注入的 FOO 应可用");
    assert!(
        ti.consts.iter().any(|(n, t, v)| n == "FOO" && *t == Ty::Int && *v == 7),
        "注入常量应出现在折叠后的 const 表：{:?}", ti.consts
    );
}

#[test]
fn script_cannot_shadow_engine_const() {
    let errs = check_with(
        "const FOO: int = 5; sub main() { loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 1)],
    )
    .expect_err("脚本重声明引擎常量应失败");
    assert!(errs.iter().any(|e| e.msg.contains("与引擎常量重名")), "{errs:?}");
}

#[test]
fn engine_const_type_participates_in_checking() {
    // FOO 是 int，用在需 fx 的位置应判型失败（证明注入常量带类型进了判型）
    let errs = check_with(
        "sub main() { var a: fx = FOO; loop { wait(1); } }",
        &[EngineConst::new("FOO", EclValueType::Int, 7)],
    )
    .expect_err("int 注入常量赋给 fx 应失败");
    assert!(!errs.is_empty(), "应有判型错误");
}
```

- [ ] **Step 2: 跑测试验证红**

Run:
```bash
cargo test -p stg-ecl-compiler lang::typeck::tests::engine_consts_are_injected_as_predeclared_constants -- --exact
```
Expected: 编译失败（`typeck::check` 只收一个参 / `check_with` 里 `check(&prog, engine)` 签名不符）。

- [ ] **Step 3: `Checker` 加 `engine_const_names` 字段**

`crates/stg-ecl-compiler/src/lang/typeck/checker.rs` 的 `Checker<'p>` 结构体加字段：
```rust
    pub(super) engine_const_names: std::collections::BTreeSet<String>,
```

- [ ] **Step 4: `check` 增参 + 预填 + `eclty_to_ty`**

`crates/stg-ecl-compiler/src/lang/typeck.rs`：改 `check` 签名并在处理脚本 const **之前**预填引擎常量。顶部加 `use stg_core::consts::EngineConst; use stg_core::ecl::image::EclValueType;`。

```rust
pub fn check(prog: &Program, engine_consts: &[EngineConst]) -> Result<TypedInfo, Vec<CompileError>> {
    let mut c = Checker {
        subs: BTreeMap::new(),
        xformdefs: BTreeSet::new(),
        consts: BTreeMap::new(),
        engine_const_names: BTreeSet::new(),
        errors: Vec::new(),
        cur_sync_calls: Vec::new(),
        cur_xform_refs: Vec::new(),
    };

    // 引擎常量当"第 1 行前声明的 const"预填——脚本 const 在其后处理，重名即撞。
    for ec in engine_consts {
        c.consts.insert(ec.name.to_string(), (eclty_to_ty(ec.ty), ec.value));
        c.engine_const_names.insert(ec.name.to_string());
    }

    for xf in &prog.xformdefs {
        c.xformdefs.insert(xf.name.clone());
    }
    // …（sub 收集、const 处理、判型：其余 check 主体不变）…
}

fn eclty_to_ty(t: EclValueType) -> Ty {
    match t {
        EclValueType::Int => Ty::Int,
        EclValueType::Fx => Ty::Fx,
        EclValueType::Angle => Ty::Angle,
    }
}
```

> 其余 `check` 主体（sub 收集、`for cdef in &prog.consts { c.check_const_def(cdef); }`、判型循环、`TypedInfo` 组装）保持不变。末尾 `let consts = c.consts.iter().map(...)` 会把引擎常量一并收进 `TypedInfo.consts`——这是有意的：让 codegen 的 xformdef 槽参数也能引用引擎常量。

- [ ] **Step 5: `check_const_def` 区分引擎重名文案**

`crates/stg-ecl-compiler/src/lang/typeck/consts.rs` 的 `check_const_def` 开头的重复检查改为：
```rust
    pub(super) fn check_const_def(&mut self, cdef: &ConstDef) {
        if self.engine_const_names.contains(&cdef.name) {
            self.err(cdef.span, format!("'{}' 与引擎常量重名，不能重新声明", cdef.name));
            return;
        }
        if self.consts.contains_key(&cdef.name) {
            self.err(cdef.span, format!("常量 '{}' 重复定义", cdef.name));
            return;
        }
        // …（其余不变：fold_const + 类型核对 + insert）…
    }
```

- [ ] **Step 6: `compile_with_options` 增参 + `compile` 默认注入 + 更新调用点**

`crates/stg-ecl-compiler/src/lang/mod.rs`：

`compile_with_options` 加第四参并透传：
```rust
pub fn compile_with_options(
    src: &str,
    file: &str,
    options: CompileOptions,
    engine_consts: &[stg_core::consts::EngineConst],
) -> Result<CompiledEcl, Vec<CompileError>> {
    let program = parse(src, file)?;
    if let Err(mut errors) = entryck::check(&program) { attach_src_lines(&mut errors, src); return Err(errors); }
    let typed = match typeck::check(&program, engine_consts) {   // ← 透传
        Ok(t) => t,
        Err(mut errors) => { attach_src_lines(&mut errors, src); return Err(errors); }
    };
    // …（slots/codegen/debug 不变）…
}
```

`compile` 默认注入静态注册表：
```rust
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(src, file, CompileOptions { debug_info: DebugInfo::None },
                         stg_core::consts::ENGINE_CONSTS).map(|ce| ce.image)
}
```

更新 `mod.rs` 里两处调试测试对 `compile_with_options` 的调用（`debug_mode_does_not_change_runtime_image`、`full_debug_symbols_keep_call_only_and_parameter_names`）：末尾补第四参 `&[]`（它们测调试侧载，与注入无关，用空注入隔离）：
```rust
        let none = compile_with_options(src, "stage.ecl",
            CompileOptions { debug_info: DebugInfo::None }, &[]).unwrap();
        let full = compile_with_options(src, "stage.ecl",
            CompileOptions { debug_info: DebugInfo::Full }, &[]).unwrap();
```
（第二个测试同理补 `&[]`。）

- [ ] **Step 7: 更新 `typeck/tests.rs` 既有 `check` 调用点**

`tests.rs` 里 `ok`/`err` 两个助手（约 12、16 行）内的 `check(&prog(src))` 改为 `check(&prog(src), &[])`：
```rust
fn ok(src: &str) -> TypedInfo {
    check(&prog(src), &[]).unwrap_or_else(|e| panic!("判型失败：{e:?}\n源码：\n{src}"))
}
fn err(src: &str) -> Vec<CompileError> {
    check(&prog(src), &[]).expect_err(&format!("期望判型失败，源码：\n{src}"))
}
```

- [ ] **Step 8: 跑测试验证绿 + clippy**

Run:
```bash
cargo test -p stg-ecl-compiler
cargo clippy -p stg-ecl-compiler --all-targets -- -D warnings
```
Expected: 全绿（含三条新注入测试）；clippy 净。

- [ ] **Step 9: 提交**

```bash
git add crates/stg-ecl-compiler/src/lang
git commit -m "feat(lang): inject engine constants into typeck namespace

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: 集成——rainbow 去魔数、文档、金向量对拍

**Files:**
- Modify: `crates/stg-harness/scenes/rainbow.ecl`（单指某 appearance 的魔数换注入名）
- Modify: `crates/stg-harness/src/main.rs`（如有对 appearance/gvar 的魔数注释，同步；harness 经 `compile` 自动获得注入）
- Modify: `docs/ecl-lang.md`（新增"引擎常量"一节）
- Modify: `docs/follow-ups.md`（销 C14 那半 + coherence 不变量记档；C11 仍开放）
- Test: `crates/stg-harness/src/main.rs` 内联测试 + 金向量

**Interfaces:**
- Consumes: Task 1-3 全部产物（`compile` 默认注入引擎常量）。
- Produces: 作者面语义文档、去魔数金向量场景、迁移状态记录。

- [ ] **Step 1: rainbow.ecl 去魔数**

`crates/stg-harness/scenes/rainbow.ecl`：把**单指某一个 appearance** 的魔数换成注入名（例如某处固定发星弹的 `fire(3, ...)` → `fire(APPEARANCE_STAR, ...)`）。**保留** `var appearance = i % 4` 这类**遍历全表**的写法（它是有意轮转 0..4，非单指某个）。不为影响 SubId/PC 而重排声明。

> 先 `rg 'appearance|APPEARANCE|\bfire\(|batch\(' crates/stg-harness/scenes/rainbow.ecl` 找出所有出现点，逐个判断"单指 vs 轮转"，只换单指的。

- [ ] **Step 2: 编译校验 + 金向量两次对拍**

Run:
```bash
cargo test -p stg-harness
tmp_a=$(mktemp); tmp_b=$(mktemp)
cargo run -p stg-harness -- golden --out "$tmp_a"
cargo run -p stg-harness -- golden --out "$tmp_b"
diff -u "$tmp_a" "$tmp_b" && echo "GOLDEN DETERMINISTIC"
```
Expected: harness 测试绿；两次 golden `diff` 空。**若校验和相对上游有变**：去魔数只改源码书写不改值（`APPEARANCE_STAR` 折叠回同一字面量 3），校验和**不应**变——若变了，说明换错了地方（把轮转当单指），回退重判。

- [ ] **Step 3: `docs/ecl-lang.md` 新增"引擎常量"一节**

加一节，写清：
- 可引用哪些引擎常量（`APPEARANCE_SMALL/MEDIUM/LARGE/STAR`、`GVAR_RANK`、`GLOBALS_SYS_SEGMENT`），值即引擎侧同名常量；
- 它们是"第 1 行前声明的 const"，脚本表达式/`const`/xformdef 槽参数处均可引用；
- 脚本**不得**重声明同名 const（报"与引擎常量重名"）；
- 权威定义在 `stg-core` 的 `engine_consts!` 注册表；加新常量 = 注册表加一行（C11 表文件加载后可改由数据文件驱动）。

- [ ] **Step 4: `docs/follow-ups.md` 更新**

- C14 条目：标注常量注入 + const 求值收编**已完成**（引擎常量引用落地、三份求值器收成一份、`type_rules`/`const_eval` 提到 `lang::` 级）；
- 记档 **coherence 不变量**（照 spec §coherence）：注入常量落为字面量值，同一份常量来源须同时喂编译期注入与运行期查表；C11 用 `content_hash` 焊死；
- 明确 **C11 仍开放**：`WorldTables` owned 化、表文件加载、`EclImage.content_hash`/`WorldTables.content_hash` 真哈希划归 Spec 2。

- [ ] **Step 5: 全工作区质量闸门**

Run:
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out /tmp/stg-c14-checksums.txt
git diff --check
cargo tree -p stg-core | wc -l   # 与 Task 2 记数比对：不变
```
Expected: 全工作区测试绿；clippy 零警告；golden 成功；无空白错误；stg-core 依赖树未变。

- [ ] **Step 6: 审计 + 提交**

Run:
```bash
rg -n 'super::matrix|super::intents|eval_const_arg|mod matrix|mod intents' crates/stg-ecl-compiler/src
```
Expected: 无结果（旧求值器/矩阵路径全清）。

```bash
git add crates/stg-harness/scenes/rainbow.ecl crates/stg-harness/src/main.rs \
  docs/ecl-lang.md docs/follow-ups.md
git commit -m "docs(ecl): finalize const injection integration; de-magic rainbow

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 记录

- **Spec 覆盖**：Part A（收编）→ Task 1；Part B（注册表宏）→ Task 2；Part C（注入接线 + 命名碰撞）→ Task 3；rainbow 去魔数 + 文档 + coherence 记档 → Task 4。全覆盖。
- **排序修正**：原 spec 把"删 matrix/intents"与"重写 consts 求值器"分列，但 `consts.rs` 收编前仍引用 matrix 的 `mulf_const/divf_const`，故合并为原子 Task 1（否则中间态编译不过）。
- **stale 引用**：`const_eval.rs` 的 `Expr::GlobalRead` 臂在当前 AST 无对应变体（C16 已降级 `global()` 为普通 Call），Task 1 Step 2 显式删除。
- **类型一致**：`type_rules::{BinIntent,UnIntent,CastIntent}` 与被删的 `intents::` 同名同变体同 derive，drop-in 兼容；`EngineConst.ty: EclValueType` 经 `eclty_to_ty` 转 `Ty` 注入。
- **依赖方向**：`consts.rs` 挂 crate 根叶子（依赖 `ecl::image`），`tables`/`world` 经 `pub use` 依赖它，无环。
