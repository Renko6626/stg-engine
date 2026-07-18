# M1.9 ECL 表层语言 + 编译器 —— 设计 spec

> 状态：已过 grill 拍板（2026-07-18，七项）。定位：**把 M1 的字节码 VM 变成正常编程语言**
> ——"接近写一个正常的编程语言的手感，不要满地 jmp 和 push"（用户原话，总纲）。
> 需求底稿：follow-ups C13 全部七条（T4 转写摩擦四条 + ZUN 对照三条）。builder DSL 自此
> 降级为 codegen 后端（用户拍板"临时凑数"兑现）。

## 拍板纪要（grill，2026-07-18）

1. **C 系花括号语法**——thecl/truth 社区语料直译红利（转写真符卡零形状转换）；parser 好写、
   错误恢复容易。
2. **轻量静态三型 `int / fx / angle`**——编译器按类型选指令（`fx*fx`→MULF、`int*int`→MUL、
   `angle±angle/int`→回绕加减、`angle*` 一律编译错误）；跨型显式 cast（`as int`/`as fx`）；
   单位字面量：`90deg`（BAM 换算）、`1.5fx`（Q16.16）、裸整数 = int；`px` 同 `fx` 别名。
   定点头号翻车型（MUL/MULF 选错、角度当整数乘）从此编译期拦截。
3. **函数 = 静态槽分配 + 禁递归**——每任务入口做调用图着色：同一调用路径上的 sub 分互不
   相交的 locals 槽段；参数传递 = caller 写 callee 槽再 CALL；**递归编译期拒绝**（报错给出
   环路径）；求值栈深静态计算，>32 编译错误。**VM 小扩展：`SPAWN` 带参**——从父任务栈拷
   argc 个值进子任务 `locals[0..argc]`（EclImage 未冻结，扩展免费；spawn 语义仍次帧首跑）。
4. **启动时源码编译**——`.ecl` 文本（harness `include_str!` 或 `--script` 路径），
   stg-ecl-compiler(lib) 现场编译；**编译器必须确定性**（同源码→逐字节同镜像，入测试）；
   二进制格式/content_hash/加载校验整包归 C11 刀。
5. **狗粮验收**——彩虹风铃卡重写为 `.ecl`，金向量二号重生成（流变化、确定性照旧三平台）；
   C13 摩擦逐条还账实证（rank 真表达式进密度、hp_ratio = `$self_hp * / $self_hp_max` 真算、
   值消费检查灭足枪）；builder API 文档标注"编译器后端，不推荐直接使用"。
6. **v1 手感特性**：**`$` 前缀引擎状态变量**（`$frame/$player_x/$player_y/$self_x/$self_y/
   $self_hp/$self_hp_max/$self_age` 读取编译成对应 SYS 读；`$` 标明"引擎状态非你的变量"；
   globals 走 `global(n)`/内建函数不走 `$`——它们可写）；**值消费静态检查**（表达式语句值
   未消费 = 编译错误，`_ = expr;` 显式丢弃）。**时间标签糖不进 v1**（留 C13 备胎不删档）。
7. **命名：就叫 ECL，扩展名 `.ecl`**。

## 语言草图 v1

```ecl
// 彩虹风铃（示意）
const RANK_SLOT: int = 0;          // 编译期常量

sub patrol() {                      // 普通 sub：同任务内 CALL
    loop {
        move_to(90, -100fx, 100fx, EASE_QUAD_OUT);
        wait(90);
        move_to(90, 100fx, 100fx, EASE_QUAD_OUT);
        wait(90);
    }
}

async sub timer_ui(spell: int) {    // async sub：SPAWN 出新协程，参数经 SPAWN 带参落子任务 locals
    var t: int = 600;
    while t > 0 {
        boss_set(0, $self_hp fdiv $self_hp_max, spell, t, 1, 1);
        wait(60);
        t = t - 60;
    }
}

sub main() {
    spawn patrol();
    spawn timer_ui(1);
    var base: angle = 0deg;
    loop {
        var ways: int = 28 + global(RANK_SLOT) * 2;
        for i in 0..5 {             // 计数 for（编译成 locals 计数器 + 比较跳转）
            _ = ring(0, $self_x, $self_y, ways, base, 1.0fx + i as fx * 0.25fx, i);
        }
        base = base + 7deg;
        wait(50);
    }
}
```

- **语句**：`var name: type = expr;`（局部，槽分配器落位）· 赋值 · `if/else` · `while` ·
  `loop {}`（无限）· `for i in a..b {}` · `wait(n);`（内建）· `spawn f(args);` ·
  `return;` · 表达式语句（值必须消费）· `break/continue`（v1 收——循环没 break 不算正常语言）。
- **表达式**：算术（按型选指令）· 比较/逻辑（短路 `&&`/`||` 编译成跳转）· 括号 · 调用 ·
  `$` 引擎变量 · `global(n)` / `set_global(n, v)`（系统段写保护是运行时语义，编译器不管）·
  cast `expr as type`。`fdiv` = 定点除（`/` 在 fx 型自动选 DIVF——`fdiv` 关键字不需要，
  实施时按型自动选；示意里保留可读性）。
- **内建函数表** = syscall 号表 v1.1 全员类型化签名（`ring`/`fire` 等名字在实施计划里与
  `create_bullets_batch`/`create_bullet` 对齐定稿；参数类型错 = 编译错误）。
- **注释** `//` `/* */`；**错误报告**：文件:行:列 + 源行摘录 + 修法提示（正常语言的底线）。

## 成熟项目参考（2026-07-18 调研拍板，实施计划照抄惯用法）

- **Crafting Interpreters/clox**（craftinginterpreters.com，全文免费）——codegen 蓝本：
  Pratt 表达式解析、`emitJump/patchJump` 跳转回填、循环栈簿记（break/continue fixup 列表）；
- **Pawn**（compuphase）——"C 系 + 无类型 32 位 cell + 确定性抽象机"同位体先例，其 **tag
  系统**（编译期标签盖 cell）= 我们三型设计的 20 年工业验证；
- **Monkey（Writing a Compiler in Go）**——AST 多趟 + 显式回填的骨架参照（类型趟插中间）;
- AngelScript（静态类型栈 VM 商业出货）与 Wren（栈 VM + fibers）为存在性证明。
- **控制流降低模板**（已验证与弹栈式 JZ 兼容，短路逻辑不需要 JNZ——C13⑦ 降级为纯密度优化）：
  `if/else` = JZ+JMP 双回填；`while` = 顶测 JZ + 回跳；`for a..b` = 计数器 + LT/JZ，
  continue 指向自增段；`&&`/`||` = JZ 短路 + 常量臂。codegen 扩展 T3 builder 既有回填器，
  非从零。

## 编译管线

`lexer → parser（AST，错误恢复到语句边界）→ 类型检查（三型 + 单位字面量折叠 + 值消费）→
槽分配（任务入口调用图着色 + 递归环检测 + 求值栈深静态验证 ≤32 + locals 总量 ≤64 检查）→
codegen（现有 builder 后端，jnz 等缺失指令用现有 op 合成——**不动 VM op 表**，SPAWN 带参
是唯一 VM 触碰）`。全程无 IO 无时钟无 HashMap（BTreeMap/Vec——**编译器也要确定性**）。

## 测试与验收

- 判别单测：lexer/parser 逐构造（含错误路径行列号断言）· 类型检查矩阵（合法/非法组合表）·
  槽分配（两 sub 同路径不相交/姊妹 sub 可重叠/递归环报错/栈深超限报错/locals 超限报错）·
  值消费检查 · SPAWN 带参 VM 扩展（子任务 locals 收到实参，次帧语义不变）· 编译器确定性
  （同源双编译逐字节同镜像）· 端到端（`.ecl` 小脚本过 VM 行为断言）。
- 风铃卡 `.ecl` 重写：行为断言全保（稳态弹数/boss_ui/零 Fault）+ hp_ratio 真值化 +
  rank 表达式化；金向量二号重生成，双跑 + 三平台。
- 变异候选 ≥3：类型检查关掉 fx/int 混算（矩阵测试红）· 槽着色改全零起点（不相交测试红）·
  SPAWN 带参拷贝删（传参测试红）。

## 收尾义务

`docs/ecl-lang.md` 新建（语言参考：语法/类型/内建表/$变量表/错误码——作者第一入口）；
ecl-ops.md 标注"字节码层参考，作者请看 ecl-lang.md"；design_doc §4.5 落地括注；
C13 逐条销账（⑤时间标签留档改"未进 v1"）；PROGRESS 史行 + 现在段。

## 验收

判别单测全绿过变异；编译器确定性实证；金向量二号（.ecl 版风铃卡）双跑 + 三平台全等、
一号不动；clippy/fmt/防火墙零告警；VM 唯一变更 = SPAWN 带参（其余零触碰）。
