# ECL 常量注入 + const 求值收编（C14 / Spec 1）设计

> 状态：设计已拍板（2026-07-21 brainstorm）。这是"写 .ecl 让游戏跑"完整闭环的第一刀，
> 纯编译器级、零运行时、零确定性面。C11 表文件加载（`WorldTables` owned 化 + content_hash）
> 是**独立后续 Spec 2**，不在本刀范围。

## 目标与非目标

**目标（本刀交付）**：
1. `.ecl` 源码能引用引擎侧命名常量（`APPEARANCE_STAR`、`GVAR_RANK` 等），不再靠魔数复写。
2. 引擎常量的**值定义 + Rust 常量 + 脚本注入列表**收敛到**单一 `.rs` 注册表**，一处定义、两头自动生成，永不漂移。
3. const 求值逻辑从**三份**（`typeck/consts.rs::fold_const`、`codegen.rs::eval_const_arg`、`typeck/matrix.rs`）收编成**一份**共享纯模块（`lang::const_eval` + `lang::type_rules`），比动手前更少。

**非目标（划归 Spec 2 / 后续）**：
- 表文件加载、`WorldTables` 从 `&'static` 改 owned、`&WorldTables` 全链穿参。
- `EclImage.content_hash` / `WorldTables.content_hash` 真哈希（双身份链）。
- 引擎常量从"数据文件驱动"自动生成（C11 落地后，owned 表带名字的行喂 `engine_consts`，届时再接）。

## 确定性说明

本刀全部改动在**断层线以上**（编译器 `stg-ecl-compiler` + `stg-core` 的**静态常量定义**，无运行时逻辑）。
注入的常量在 codegen 阶段折叠为字面量 `push_i`（现状 `TypedExprKind::ConstRef(v)` 已如此），
运行时字节码不含"名字"、只含值。金向量校验和不受影响（值不变时逐位一致）。唯一的语义前提由
[coherence 不变量](#coherence-不变量c11-预留) 声明。

---

## Part A · const 求值三合一收编

### 现状（三份，且矩阵藏在 typeck 内）

| 位置 | 职责 | 可见性问题 |
|---|---|---|
| `typeck/matrix.rs::binary_result` | 类型矩阵（判型 + 常量折叠共用） | `pub(super)`，codegen 看不见 |
| `typeck/intents.rs::BinIntent` | 后端降低意图枚举 | typeck 内 |
| `typeck/consts.rs::fold_const` | const 求值器 #1（判型趟） | 吃 `Checker` 内部状态 + `self.err` |
| `codegen.rs::eval_const_arg` | const 求值器 #2（xformdef 槽参数） | 只支持字面量/const 引用/一元负 |

`matrix.rs` 模块文档自称"类型矩阵唯一权威——杜绝两处写两套规则、迟早分叉"，但 codegen 因为看不见它，
另起了 `eval_const_arg`——已经是"两套求值器"的既成事实。

### 目标（一份，提到 `lang::` 级）

已存在两个未跟踪 prep 文件，本刀正式收编：

- **`lang/type_rules.rs`**（已写）：矩阵唯一权威。导出 `binary_result`、`cast_intent`、`op_symbol`、
  `BinIntent`、`UnIntent`、`CastIntent`。**不持符号表、不构造诊断、不依赖阶段状态**——纯规则表。
- **`lang/const_eval.rs`**（已写）：const 求值器唯一实现。
  `evaluate(e: &Expr, consts: &BTreeMap<String,(Ty,i32)>) -> Result<(Ty,i32), ConstEvalError>`。
  `ConstEvalError { span, msg, needs_cast_hint }` 由调用方转成各自诊断（typeck→`self.err`，codegen→`CompileError`）。

**收编动作**：
1. 在 `lang/mod.rs` 注册 `mod type_rules;` `mod const_eval;`（当前是死文件，无 `mod` 引用）。
2. **删** `typeck/matrix.rs` + `typeck/intents.rs`。`typeck/consts.rs` 的 `fold_const` /
   `fold_binary_const` 改为薄封装：调 `const_eval::evaluate`，把 `ConstEvalError` 转 `self.err`
   （保留既有错误顺序与文案契约——`const_eval.rs` 的文案已与现状对齐）。
3. `codegen.rs::eval_const_arg` 删除，xformdef 槽参数求值改调 `const_eval::evaluate`。
4. typeck 其余对 `matrix::binary_result` 的引用（`exprs::type_expr` 等）改指 `type_rules::binary_result`。

**附带效果（接受，非坑）**：xformdef 槽参数从"只字面量/const/一元负"升级为**支持完整 const 表达式**
（二元算术等）——`const_eval::evaluate` 本就支持、有自测守。能力严格超集，无回归风险。

**验收**：`rg 'mod matrix|mod intents' crates/stg-ecl-compiler/src` 无结果；矩阵与求值器全仓各恰一份。

---

## Part B · 单一注册表：`stg_core::consts` + `engine_consts!` 宏

### 新 leaf 模块

新建 `crates/stg-core/src/consts.rs`，挂在 **crate 根**（不在 `ecl::` 下——否则 `tables.rs`/`world.rs`
依赖它会与"ecl 依赖 tables/world"成环）。它是叶子：`tables`、`world`、`ecl` 均可依赖，无环。

### `EngineConst` 类型（stg-core）

```rust
// consts.rs
pub struct EngineConst {
    pub name: &'static str,
    pub ty: crate::ecl::image::EclValueType,   // Int / Fx / Angle
    pub value: i32,                            // 脚本侧统一 i32（Fx=raw，Angle=raw as i32）
}

impl EngineConst {
    pub const fn int(name: &'static str, value: i32) -> Self {
        Self { name, ty: EclValueType::Int, value }
    }
    // 需要时补 fx()/angle()（本刀 v0 全是 int id/slot，暂只用 int）
}
```

### `engine_consts!` 宏：一行两出

```rust
engine_consts! {
    //  名字                  Rust 类型  脚本类型  值
    APPEARANCE_SMALL:         u16  as int = 0;
    APPEARANCE_MEDIUM:        u16  as int = 1;
    APPEARANCE_LARGE:         u16  as int = 2;
    APPEARANCE_STAR:          u16  as int = 3;
    GVAR_RANK:                u16  as int = 0;
    GLOBALS_SYS_SEGMENT:      u16  as int = 16;
    // 加新配置 = 加一行
}
```

宏对每行**同时展开**：
- `pub const APPEARANCE_STAR: u16 = 3;`（Rust 侧类型保真，现有 `as usize` 索引照用）
- 汇入 `pub const ENGINE_CONSTS: &[EngineConst] = &[ EngineConst::int("APPEARANCE_STAR", 3), … ];`
  （脚本注入，`u16` 值经 `as i32` 归一）

`as int` / `as fx` / `as angle` 决定 `EngineConst.ty` 与脚本类型；Rust 类型段（`u16`/`usize`/`i32`）
决定 `pub const` 的类型。宏内 `value as i32` 做脚本侧归一。

### 迁移：`pub use` 再导出压低 churn

现有 `APPEARANCE_*`（`tables.rs`）、`GVAR_RANK`/`GLOBALS_SYS_SEGMENT`（`world.rs`）的**定义**搬进宏块。
为不动全仓引用、不丢 `world.rs` 段纪律文档：

```rust
// world.rs：保留段纪律 doc + 再导出，旧引用继续可用
/// globals 段纪律（甲案，M1.5）…（原 doc 保留）
pub use crate::consts::{GVAR_RANK, GLOBALS_SYS_SEGMENT};
// tables.rs 同理 pub use APPEARANCE_*;
```

`crate::tables::APPEARANCE_STAR` / `crate::world::GVAR_RANK` 全部照旧编译，定义权威归一到 `consts.rs`。
`ENGINE_CONSTS` 里的值经再导出仍是同一常量——单一权威不破。

**为什么不用外部数据文件 + build.rs**：① stg-core 构建期敏感，多一个 build.rs + 解析是新失败面；
② 这些是整数 id，宏产物与 build.rs 等价、宏零成本；③ 真正的"数据文件驱动常量"是 C11 owned 表的
自然形态，届时再接顺理成章。

---

## Part C · 注入接线

### 编译入口签名变更

`engine_consts` 作**独立参**穿过管线（不塞进 `CompileOptions`，保持它 `Copy`、无生命周期；
C11 时换传 owned 表的常量切片即可）：

```rust
pub fn compile_with_options(
    src: &str,
    file: &str,
    options: CompileOptions,
    engine_consts: &[EngineConst],          // 新增
) -> Result<CompiledEcl, Vec<CompileError>>;

// 便利包装默认注入静态注册表，现有调用方自动获得引擎名：
pub fn compile(src: &str, file: &str) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(src, file, CompileOptions { debug_info: DebugInfo::None },
                         stg_core::consts::ENGINE_CONSTS).map(|ce| ce.image)
}
```

### typeck 预填

`typeck::check` 增参，在处理脚本 `const` **之前**把引擎常量当"第 1 行前声明的 const"预填进 `c.consts`：

```rust
pub fn check(prog: &Program, engine_consts: &[EngineConst])
    -> Result<TypedInfo, Vec<CompileError>>
{
    let mut c = Checker { consts: BTreeMap::new(), /* … */ };
    for ec in engine_consts {
        c.consts.insert(ec.name.to_string(), (eclty_to_ty(ec.ty), ec.value));
    }
    for cdef in &prog.consts { c.check_const_def(cdef); }   // 脚本 const 在其后
    // …
}
```

`eclty_to_ty`：`EclValueType::{Int,Fx,Angle}` → `Ty::{Int,Fx,Angle}`（编译器侧小转换函数）。

### 命名碰撞：脚本不许影子引擎常量

引擎常量已在 `c.consts` 里，脚本写 `const APPEARANCE_STAR: int = 5;` → 撞 `check_const_def` 现有
"重复定义"分支。**保持拒绝**（脚本不得覆盖引擎常量）。错误文案改进：当被撞的名字属于 `engine_consts`
时报"'APPEARANCE_STAR' 与引擎常量重名，不能重新声明"，否则维持"常量 '…' 重复定义"。

### rainbow.ecl 去魔数

把"单指某个 appearance"的魔数换成注入名（如某处固定发星弹 → `APPEARANCE_STAR`）。
`var appearance = i % 4` 这类**循环遍历全表**的写法保留（它就是要轮转 0..4，非单指）。
不为影响 SubId/PC 而重排声明；金向量校验和若变，须先核对 ECL 事件/计数断言再接受。

---

## coherence 不变量（C11 预留）

注入的常量在字节码里落为**字面量值**（非名字）。故存在一条语义前提，本刀记档、Spec 2 用 content_hash 焊死：

> **同一份常量来源必须同时喂"编译期注入"与"运行期查表"。** 本刀 v0：`ENGINE_CONSTS` 的值
> 由 `consts.rs` 单一权威提供，且 `WorldTables`（v0 静态）与之同源（`APPEARANCE_STAR=3` 既是注入值、
> 也是 `TABLES_V0.appearances` 的索引），二者天然一致。C11 表文件加载后，若注入常量取自表 A、
> 却拿表 B 跑 → appearance id 可能错位且**三平台一致地错**（金向量闸门抓不到，见 CLAUDE.md 警告）。
> Spec 2 令 `EclImage` 记录其编译所依赖表的 `content_hash`，运行期比对拦截。

---

## 影响文件清单

**stg-core**：
- 新建 `src/consts.rs`（`EngineConst` + `engine_consts!` 宏 + `ENGINE_CONSTS`）
- `src/lib.rs`（`pub mod consts;`）
- `src/tables.rs` / `src/world.rs`（常量定义搬走 + `pub use` 再导出）

**stg-ecl-compiler**：
- `src/lang/mod.rs`（注册 `mod type_rules; mod const_eval;`；`compile_with_options` 增参；`compile` 默认注入）
- **删** `src/lang/typeck/matrix.rs` + `src/lang/typeck/intents.rs`
- `src/lang/typeck.rs`（`check` 增参 + 预填 engine consts）
- `src/lang/typeck/consts.rs`（`fold_const` 薄封装调 `const_eval`）
- `src/lang/typeck/exprs.rs` 等（`matrix::` 引用改 `type_rules::`）
- `src/lang/codegen.rs`（删 `eval_const_arg`，改调 `const_eval::evaluate`）
- （`const_eval.rs` / `type_rules.rs` 已存在，本刀正式纳入 `mod`）

**stg-harness**：
- `src/main.rs`（`compile` 调用点自动获得引擎名；rainbow 去魔数）
- `scenes/rainbow.ecl`（魔数换注入名）

**docs**：
- `docs/ecl-lang.md`（新增"引擎常量"一节：可引用哪些、不可影子）
- `docs/follow-ups.md`（C13/C14 销 C14 那半；coherence 不变量记档；C11 仍开放）

## 测试策略（TDD，回归只增不减）

- **收编等价**：收编前后 `cargo test -p stg-ecl-compiler` 全绿；const 折叠既有测试不改断言（纯搬家）。
- **注入正向**：`fire(APPEARANCE_STAR, …)` 编译通过，字节码含字面量 `3`（`ConstRef(3)`）。
- **注入负向**：脚本 `const APPEARANCE_STAR: int = 5;` → "与引擎常量重名"报错。
- **类型正确**：注入常量参与类型检查（`APPEARANCE_STAR`（int）用在需 fx 处 → 判型报错）。
- **xformdef 超集**：xformdef 槽参数用 `A + B`（二元 const 表达式）编译通过（收编附带能力）。
- **金向量**：`cargo run -p stg-harness -- golden` 两次 diff 空；去魔数后事件/计数断言先核对再接受。
- **firewall**：`cargo tree -p stg-core` 不新增外部依赖（宏纯静态）。
