# ECL VM 技术路线调研结论（2026-07-18 记录）

> 状态：**调研结论落档**（非新设计——是对 design_doc §4.2/4.3 v0.3 栈机草案的复核确认）。
> M1 开工前的 grill 议程附文末。

## 结论：维持栈机（stack-based VM）

ZUN 实况（调研证实）：ECL V1（红魔乡~文花帖）无栈；**V2（风神录/TH10 起沿用至今）是栈机**
——每任务自有求值栈、sub 入口 `stackAlloc` 开局部变量（栈偏移寻址，负偏移=入参）、`call`
同栈压帧、**`callAsync` 开新任务 = ZUN 的 coroutine**、`wait` 写计时器让出。
"变量/循环/条件/函数/async spawn"的作者手感清单逐项即 V2 能力集。

**三条独立论证收敛于栈机**（非因循 ZUN）：

1. **I5 之下栈机/寄存器机打平**（两者执行状态都可扁平定长），胜负手在**编译器复杂度**：
   栈机码生成 = AST 后序遍历直接吐指令、零寄存器分配——M1 Rust DSL 手拼字节码与将来的
   表层语言编译器都受益；
2. **寄存器机唯一优势（dispatch 少 ~30-50%）在我们负载下无意义**：每帧 ECL 指令量级
   10²~10³（任务大多在 wait），对比碰撞相位不是热点；
3. **fuzz 面最小**（§9 随机字节码只许确定性报错）：栈机越权面 = sp/csp 边界 + 跳转目标
   两处；寄存器机逐指令 2-3 个操作数都要验址。

另有语料红利：thtk/thecl/Priw8/LiveECL 的 V2 反编译生态可近乎直译成我们的 DSL（金向量
复刻真符卡的转写效率）。

## 与 ZUN 的三处有意分歧（我们更严）

- **定长栈**：64 字求值栈 + 8 层调用栈 + 64 字 locals ≈ 600B/任务、512 任务 ≈ 300KB
  （rollback 快照预算钉死）；溢出 = 确定性杀任务 + 记事件，绝不 UB。
- **纯 i32/Q16.16 栈**（`addi/addf` 区分语义）；ZUN V2 栈上有 f32，I1 禁。
- **运行时不带 rank/time 字段**：纯 `wait` 驱动，难度处理留编译期——**待 grill**
  （ZUN per-instruction rank mask 让一份脚本四难度复用，编译期展开则 EclImage 变四份）。

## M1 开工前 grill 议程

指令编码（定长 vs 变长）· 栈深 64/调用深 8 校准（benchmark 后定）· CallFrame 内容 ·
每帧指令预算（防死循环的确定性上限，benchmark 后定）· rank/难度处理层 · Rust DSL 形态
（builder vs 宏）。

**前置**：M0-18 benchmark 基线（step 均时曲线/快照/校验和成本/内存占用）——指令预算与
栈容量的数字要有实测支撑。

来源：Touhou Wiki ECL 规格（Mddass，V2 自 MoF 起栈机）· pytouhou ECL（V1）·
Priw8/ECLjs · LiveECL。
