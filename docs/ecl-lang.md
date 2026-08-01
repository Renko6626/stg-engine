# ECL 表层语言手册（索引）

`.ecl` 是本引擎的弹幕脚本语言。**这套文档是写 `.ecl` 的唯一权威**——语法、类型、内建函数、
`$` 引擎变量、xformdef、错误格式都在里面。别凭对 ZUN ECL 或别的弹幕 DSL 的记忆脑补语法。

正文拆在 [`docs/ecl-lang/`](ecl-lang/) 下，**按教学顺序编号**：从没写过一行 `.ecl` 的话，
从第 1 篇顺着读；查东西直接跳第 7 篇。

| 篇 | 讲什么 |
|---|---|
| [1 · 从零到一个弹幕](ecl-lang/1-hello-danmaku.md) | 一步一步长出一段能玩的弹幕：敌 → 动 → 一颗弹 → 一圈 → 关卡。**新手从这里开始** |
| [2 · 任务与时间](ecl-lang/2-tasks.md) | `sub` vs `async sub`、`spawn`、`wait(n)` 的准确周期、主任务返回 = 敌退场 |
| [3 · 敌人](ecl-lang/3-enemy.md) | 生成与轮询、五条运动动词、三条死亡路径与掉落、敌号与读口 |
| [4 · 弹](ecl-lang/4-bullets.md) | `fire` / `batch`、弹 setter 族、`xformdef` 变换序列、发射器 `sh_*` 族 |
| [5 · 三型、字面量与语句](ecl-lang/5-types.md) | `int`/`fx`/`angle` 三型、后缀、运算矩阵、cast 白名单、语句表 |
| [6 · 符卡与整局编排](ecl-lang/6-spell-and-stage.md) | 符卡机构、`mark` 中段启动、多文件、全局状态、账面、渲染请求 |
| [7 · 速查](ecl-lang/7-reference.md) | 内建函数**生成段** + `$` 引擎变量 + 引擎常量。查签名来这里 |
| [8 · 报错、静默降级与已知限制](ecl-lang/8-errors.md) | 编译错误格式、Fault vs 静默降级、debug 循环、v1 不支持什么 |

改完跑一句 `cargo run -p stg-harness -- check <file.ecl|目录>`（行列报错；目录 = 多文件整局，
见第 6 篇「多文件」节）。第 7 篇的「内建函数」是从 `builtins.rs` 生成的段，**签名以它为准**；
要改签名去改 `builtins.rs` 再跑 `gen-ecl-meta`，手改生成段会被 `cargo test` 的防漂移断言打回。
这套文档里所有 ` ```ecl ` 围栏都会被 `cargo test -p stg-harness` 真编译一遍，例子腐烂即红。

## 五条最容易踩的坑

其余的坑就近写在各自章节里。这五条的共同点是**不报错**，撞上了自己想不明白。

1. **`wait(n)` 的 `n` 被静默截成低 16 位。** `wait(-1)` 等 65535 帧，`wait(65536)` 变成
   `wait(0)`——而 `wait(0)` 是真 no-op，搁在 `loop` 里就是死循环。见
   [2 · 任务与时间](ecl-lang/2-tasks.md)。
2. **敌的主任务一 `return`，这只敌就退场。** 不是"任务没了敌还在"。要它留着就
   `loop { wait(1); }` 挂住。见 [2 · 任务与时间](ecl-lang/2-tasks.md)。
3. **`drop_items()` 吐完不清空计数**，`drop_items(); die();` 掉两份道具。见
   [3 · 敌人](ecl-lang/3-enemy.md)。
4. **`atan2(y, x)` 的 `y` 在前**（同 libm）。两参同为 `fx`，写反不报错，只把角度沿 45°
   对角线镜像。见 [3 · 敌人](ecl-lang/3-enemy.md)。
5. **新建任务出生当帧不跑。** `spawn`/`fire` 的 `task` 参、`spell_begin` 的 `pattern` 起的
   协程，创建那一帧一条指令都不执行，下一帧才首跑。别指望当帧观察到效果。见
   [2 · 任务与时间](ecl-lang/2-tasks.md)。

<details><summary>权威来源、编译时机、字节码层去哪看</summary>

- 实现：`crates/stg-ecl-compiler/src/lang/`（`builtins.rs` 内建表 / `xform_map.rs` xform 操作表）。
- spec：`docs/superpowers/specs/2026-07-18-m19-ecl-language.md`。与本文冲突时以代码和 spec 为准。
- 字节码层参考（op / syscall / fault 码）见 [`ecl-ops.md`](ecl-ops.md)——作者通常不需要看它。
- 编译时机：启动时从源码文本编译（`lang::compile`），编译器的确定性有测试押运。

</details>
