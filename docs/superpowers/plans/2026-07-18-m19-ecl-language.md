# M1.9 ECL 表层语言 + 编译器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `.ecl` 文本 → 正常编程语言手感（三型/具名函数/if-while-for/`$` 引擎变量/值消费检查）→ 现有栈机字节码；风铃卡重写吃狗粮，金向量二号重生成。

**Architecture:** 全部住 `stg-ecl-compiler`（断层线之上）新模块 `src/lang/`：Monkey 式多趟（lex → parse(AST) → 类型趟 → 槽分配趟 → codegen），codegen 惯用法照 clox（emit/patch 回填 + 循环栈），后端复用 T3 builder 的回填器。**VM 唯一触碰 = `OP_SPAWN` 带参扩展**（stg-core 小刀，隔离在 T3 首段）。上游 spec：`docs/superpowers/specs/2026-07-18-m19-ecl-language.md`（**先整读**——拍板七项、语言草图、降低模板都在）。

**Tech Stack:** Rust 2024，零第三方依赖（手写 lexer/递归下降 + Pratt——clox 同款；编译器内部禁 HashMap 参与任何影响输出序的路径，用 BTreeMap/Vec 保**编译器确定性**）。

## Global Constraints

- **VM 除 SPAWN 带参外零改动**；op/syscall 号表不增不改（短路逻辑用弹栈 JZ 模板，见 spec 降低模板节——照抄）。
- **编译器确定性**：同源码两次编译逐字节同镜像（测试钉）；错误信息格式 `文件:行:列: 错误说明` + 源行摘录 + `^` 定位（测试钉格式）。
- **计划级决策（spec 补遗）——xform 序列的表层形态**：顶层 `xformdef NAME { op(args); ... }` 声明（op 名 = xform-ops.md 表的小写助记，如 `turn(90deg)`/`set_ang_vel(128)`/`wait_slots(8)` 前缀 wait 用 `@8 turn(...)` 形式——实施者定一种语法并测试钉死）；编译器：序列折叠为 3 字/槽常量 → 槽分配器在**使用它的任务**的 locals 里划连续区 → sub 入口 staging（PUSHI+POPL 序列）→ `fire(..., NAME, ...)` 处自动填 `(off, cnt)`。未被任何 fire 引用的 xformdef 不占槽。
- git → mingw64 真身；NO stash；cargo 卡死近零 CPU 杀进程重试；测试前台。commit 中文 conventional + 尾签 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 名字适配纪律照旧；每任务报告写 `.superpowers/sdd/task-N-report.md`。

## 核心接口（跨任务契约；T1 产出、T2/T3 消费）

```rust
// stg-ecl-compiler/src/lang/ast.rs
pub enum Ty { Int, Fx, Angle }                       // 三型（拍板 2）
pub struct Span { pub line: u32, pub col: u32 }
pub struct Program { pub consts: Vec<ConstDef>, pub xformdefs: Vec<XformDef>, pub subs: Vec<SubDef> }
pub struct SubDef { pub name: String, pub is_async: bool, pub params: Vec<(String, Ty)>,
                    pub body: Block, pub span: Span }
pub struct XformDef { pub name: String, pub slots: Vec<XfSlotLit>, pub span: Span } // 编译期全常量
pub enum Stmt { Var { name: String, ty: Ty, init: Expr, span: Span },
                Assign { name: String, value: Expr, span: Span },
                If { cond: Expr, then_b: Block, else_b: Option<Block>, span: Span },
                While { cond: Expr, body: Block, span: Span },
                Loop { body: Block, span: Span },
                For { var: String, from: Expr, to: Expr, body: Block, span: Span },
                Wait { frames: Expr, span: Span },
                Spawn { name: String, args: Vec<Expr>, span: Span },
                ExprStmt { expr: Expr, discarded: bool, span: Span },   // discarded = `_ =` 前缀
                Return { span: Span }, Break { span: Span }, Continue { span: Span } }
pub enum Expr { IntLit(i32), FxLit(i32 /*raw*/), AngleLit(u16 /*BAM*/),
                Var(String, Span), EngineVar(EngVar, Span),             // $frame/$player_x/...
                GlobalRead { slot: Box<Expr>, span: Span },              // global(n)
                Call { name: String, args: Vec<Expr>, span: Span },      // sub 或内建
                Binary { op: BinOp, l: Box<Expr>, r: Box<Expr>, span: Span },
                Unary { op: UnOp, e: Box<Expr>, span: Span },
                Cast { e: Box<Expr>, to: Ty, span: Span } }
pub struct CompileError { pub line: u32, pub col: u32, pub msg: String, pub src_line: String }
// Display: "{file}:{line}:{col}: {msg}\n  {src_line}\n  {^ 定位}"
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>>  // lang/mod.rs 总入口
```

- **内建函数表**（`lang/builtins.rs`，name → syscall 号 + 参数型 + 返回型；号表 = ecl-ops.md v1.1）：
  `fire(appearance:int, x:fx, y:fx, speed:fx, angle:angle, xf:XFORMDEF名或none, task:SUB名或none) -> int`
  （丙方案 8 参的表层化——xf/task 是**标识符参数**非表达式，编译期解析）；
  `batch(appearance:int, x:fx, y:fx, n_angle:int, angle0:angle, angle_step:angle, n_speed:int, speed0:fx, speed_step:fx) -> int`；
  `spawn_enemy(x:fx,y:fx,hp:int,drop_table:int,score:int) -> int`；`drop_item(x:fx,y:fx,ty:int) -> int`；
  `move_to(dur:int,x:fx,y:fx,easing:int)`；`boss_set(slot:int,ratio:fx,spell:int,timer:int,phase:int,active:int)`；
  `pulse_signal(ch:int)`；`rand(n:int) -> int`；`global(n:int) -> int`；`set_global(n:int,v:int)`；
  `aim_player() -> angle`；`sin(a:angle) -> fx`；`cos(a:angle) -> fx`；弹 setter 族九连（handle:int 首参）。
- **`$` 引擎变量表**：`$frame:int $player_x:fx $player_y:fx $self_x:fx $self_y:fx $self_hp:int
  $self_hp_max:int $self_age:int`（→ SYS 0-5/9/10）。
- **类型规则表**（T2 实现、测试矩阵钉）：`int⊕int→int(MUL/DIV/…)`；`fx±fx→fx(ADD/SUB)`；
  `fx*int→fx(MUL)`；`fx/int→fx(DIV)`；`fx*fx→fx(MULF)`；`fx/fx→fx(DIVF)`；`angle±angle→angle`；
  `angle*/ 任何 → 编译错误`；同型比较 → int(0/1)；异型运算无隐式转换 → 编译错误 + 提示 cast；
  cast：`int as fx` = ×65536（PUSHI 65536; MUL）、`fx as int` = ÷65536（DIV，**向零截断**，文档注明与
  算术右移的负数差异）、`int as angle`/`angle as int` = 位穿透（BAM 回绕天然）；字面量：裸整数=int、
  `1.5fx`/`1.5px`=fx raw、`90deg`=BAM（`deg×65536/360` 就近取整，编译期 f64 折叠——**编译器在
  断层线之上，字面量折叠可用浮点**，产物是确定整数）、`16384bam`=angle 原值。

---

### Task 1: 前端——lexer + AST + parser（含错误报告）

**Files:** Create `stg-ecl-compiler/src/lang/mod.rs`（`compile` 入口暂 stub 到 parse）、`lang/lex.rs`、`lang/ast.rs`、`lang/parse.rs`；Modify `stg-ecl-compiler/src/lib.rs`（`pub mod lang;`）

- [ ] Step 0: `git checkout -b m1-9-ecl-lang`
- [ ] 失败测试先行（每构造一正一误）：lexer（全 token 种类/单位字面量 `1.5fx` `90deg` `16384bam`/注释两种/未知字符报错行列）；parser（每语句构造 AST 形状断言 · 优先级表（`1+2*3`、比较低于算术、`&&` 低于比较、`||` 最低）· 括号 · cast 后缀 · `$`变量 · `_ =` 丢弃前缀 · xformdef/const/sub/async sub 顶层 · **错误路径**：缺分号/缺花括号/坏 token 的行列号与源行摘录断言（错误信息格式在此钉死）· 错误恢复：一个错不吞后续语句（同文件报出第二个独立错误））。
- [ ] 实现：手写 lexer（Peekable<Chars> + 行列跟踪）；递归下降语句 + **Pratt 表达式**（clox 优先级表同款）；恢复策略 = 跳到下一 `;`/`}`。
- [ ] 全套门 + Commit `feat(lang): ECL 表层语言前端——lexer/AST/Pratt parser + 行列错误报告与恢复`

### Task 2: 语义趟——类型检查 + 槽分配

**Files:** Create `lang/typeck.rs`、`lang/slots.rs`；Modify `lang/mod.rs`（管线接入）

**Interfaces (Produces):** `typeck::check(&Program) -> Result<TypedInfo, Vec<CompileError>>`（每 Expr 判型 + 每 Binary 选好指令意图 + 值消费检查）；`slots::allocate(&Program, &TypedInfo) -> Result<SlotMap, Vec<CompileError>>`（`SlotMap: sub 名 → { param/var 名 → 槽号, xformdef 引用 → (off, cnt), 求值栈深上界 }`）。

- [ ] 失败测试先行：**类型矩阵**（上表逐格：合法格断言判型、非法格断言报错文案含"提示 cast"）· 值消费（`fire(...);` 未丢弃 → 错；`_ = fire(...);` 过；无返回内建语句 过）· `$` 变量判型 · 字面量折叠（`90deg`→16384、`1.5fx`→98304、`-90deg` 回绕）· **槽分配**：同调用路径两 sub 槽不相交（判别：A 调 B，断言 B 的槽起点 ≥ A 的槽用量）· 姊妹 sub 槽可重叠 · **递归环报错含路径文案**（直接递归 + A→B→A 间接）· spawn 目标视为新任务根（槽从 0 重新着色，**与 caller 无约束**）· locals 总量 >64 报错 · 求值栈深 >32 报错（构造 33 深表达式）· xformdef 区分配（被引用才占槽、3×slots 对齐、off+cnt 上界校验）。
- [ ] 实现：类型趟自底向上；槽分配 = 调用图 DFS 着色（`caller_end` 为基址给 callee），spawn 边切断着色域；栈深 = 表达式树后序模拟。
- [ ] 全套门 + Commit `feat(lang): 类型趟 + 槽分配趟——三型矩阵/值消费/调用图着色/递归环与容量静态检查`

### Task 3: SPAWN 带参（stg-core 小刀）+ codegen + 编译器确定性

**Files:** Modify `stg-core/src/ecl/{ops,vm}.rs`（SPAWN 扩展）+ 相应测试；Create `lang/codegen.rs`；Modify `lang/mod.rs`（`compile` 全管线成型）

**Commit A（stg-core 隔离刀）**：`OP_SPAWN` 元数 1→**2**（script id + argc）；语义：弹 argc 个值（逆序压栈约定同 syscall）拷进子任务 `locals[0..argc]`，argc > 64 或超父栈 → Fault(2)；既有 SPAWN 调用点（builder/测试/风铃卡 builder 版）全部补 argc=0——**金向量一号逐位不变、二号逐位不变**（argc=0 时行为等价，worktree 实证）。测试：带参 spawn 子任务 locals 收到实参 + 次帧语义不变 + argc 边界。
Commit：`feat(ecl): OP_SPAWN 带参——argc 操作数 + 父栈拷子 locals（表层 async 传参地基，金向量两段不变实证）`

**Commit B（codegen）**：
- 降低模板照 spec（if/while/for/`&&`/`||`）——**复用 builder 回填器**（T3 既有），循环栈簿记 break/continue；
- 语句/表达式全量发射：类型趟选好的指令意图直译；`$`变量 → SYS 读；内建 → 参数正序压栈 + SYS；有返回内建在 `discarded`/未消费位置自动补 POP；`wait(e)` → 表达式 + OP_WAIT；`spawn f(args)` → args 压栈 + SPAWN(script, argc)；sub 参数传递 = caller 对 callee 槽 POPL 序列 + CALL；
- xformdef staging：sub 入口对每个被引用 xformdef 发 PUSHI+POPL×(3×cnt)（仅一次，入口直排——LOOP 重入不重复：staging 在函数体第一条语句之前，`loop` 回跳点在 staging 之后，测试钉）；
- `compile()` 全线贯通 + **确定性测试**（同源双编译逐字节 assert_eq）+ 端到端小脚本群（`.ecl` 源码字符串 → compile → World 跑 N 帧行为断言：if 分支/while 计数/for 累加/短路副作用序/spawn 带参/xformdef 弹真转向）。
Commit：`feat(lang): codegen 全量——降低模板/xformdef staging/内建直译 + 编译器确定性实证（端到端过 VM）`

### Task 4: 风铃卡 `.ecl` 重写 + 金向量二号重生成

**Files:** Create `crates/stg-harness/scenes/rainbow.ecl`（`include_str!` 进 harness）；Modify `stg-harness/src/main.rs`（场景二改走 `lang::compile`；builder 版卡函数删除）

- [ ] `.ecl` 版风铃卡：结构照 spec 语言草图（main 五环 loop + patrol async + timer_ui async 带参）——**C13 摩擦还账清单逐条兑现**：环密度 = `28 + global(RANK) * 2` 真表达式；`boss_set` 的 ratio = `$self_hp` 与 `$self_hp_max` 真算（占位符退役）；TURN 环走 `xformdef`；全部返回值显式 `_ =` 或消费。
- [ ] 行为断言测试保全（稳态弹数>阈值/boss_ui active/任务数/`task_faults==0`）；金向量：场景一逐位不变（worktree 对比）、场景二**重生成**（流变化预期——报告记录新旧首分歧帧 = 场景二第 1 行）+ 双跑一致；builder API 文档注（lib.rs 顶部："codegen 后端——脚本作者请写 .ecl，见 docs/ecl-lang.md"）。
- [ ] Commit `feat(harness): 风铃卡重写为 .ecl 表层语言——金向量二号重生成（C13 摩擦逐条还账，狗粮验收）`

### Task 5: 变异 + 收尾 + 终审 + 收枝

- [ ] 变异三杀（反向 Edit 还原）：①类型矩阵关 fx/int 混算拦截（typeck 对应臂放行）→ 矩阵测试红；②槽着色 callee 基址改恒 0 → 不相交测试红；③SPAWN 带参拷贝删 → 传参测试红。
- [ ] 收尾：`docs/ecl-lang.md` 新建（作者第一入口：语法/类型规则表/内建表/$变量表/xformdef/错误格式/已知限制：禁递归+槽静态分配+值消费）；ecl-ops.md 顶部加"字节码层参考，作者看 ecl-lang.md"；design_doc §4.5 落地括注；follow-ups C13 销账（⑤时间标签改"未进 v1 备胎"）；PROGRESS 史行 + 现在段（下一步候选：bomb · homing · M2 前收口 · M3 回滚 · C11 资产管线）。
- [ ] 全绿门（含金向量双跑）→ 终审（最强模型全分支：类型矩阵完备性/槽分配对抗审/编译器确定性/错误信息质量抽查/降低模板正确性）→ 修复波 → finishing（问收枝）。

---

## Self-Review 记录

- **Spec 覆盖**：拍板 1→T1；2→T2 类型趟 + T3 指令选择；3→T2 槽分配 + T3 Commit A；4→T3 `compile()` 入口（启动时编译由 T4 harness 消费实证）；5→T4；6（$变量/值消费）→T1 语法 + T2 检查 + T3 发射；7 命名贯穿。降低模板/参考惯用法→T3。xformdef 为计划级补遗（Global Constraints 注明）。
- **占位说明**：xformdef 槽内 wait 前缀语法（`@8 turn(...)` vs 参数式）留实施者定一种并测试钉死；内建名最终表（fire/batch/…）T3 builtins.rs 为准据——均为受控自由度。
- **类型一致**：`Ty/Span/Program/SubDef/Stmt/Expr/CompileError/SlotMap` T1 定义 T2/T3 消费；SPAWN 元数 2 在 T3-A/T3-B/T4 一致；错误格式字符串 T1 钉、T2 复用。
- **依赖方向**：全部新增住 stg-ecl-compiler（断层线上，字面量折叠可用浮点、产物确定）；stg-core 唯一触碰 = T3-A 且带独立金向量门。
