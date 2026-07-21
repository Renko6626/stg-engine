# ECL 表层语言参考（作者第一入口）

> **这是什么**：`.ecl` 脚本作者手册——语法、类型、内建函数、`$` 引擎变量、xformdef、错误格式。
> **权威来源**（冲突时以它们为准）：`crates/stg-ecl-compiler/src/lang/`（`builtins.rs` 内建表 /
> `xform_map.rs` xform 操作表）· spec `docs/superpowers/specs/2026-07-18-m19-ecl-language.md`。
> 字节码层参考（op/syscall/fault 码）见 [`ecl-ops.md`](ecl-ops.md)——作者通常不需要看它。
> 编译时机：启动时从源码文本编译（`lang::compile`），编译器确定性有测试押运。

## 一分钟样例

```ecl
const RANK_SLOT: int = 0;

xformdef WIND_CHIME { set_speed(2.0fx); @30 turn(90deg); }

async sub patrol() {
    loop {
        move_to(180, -120fx, 100fx, 2); wait(180);
        move_to(180,  120fx, 100fx, 2); wait(180);
    }
}

async sub timer_ui(spell: int) {
    var t: int = 600;
    while t > 0 {
        boss_set(0, $self_hp as fx / $self_hp_max as fx, spell, t, 1, 1);
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
        for i in 0..5 {
            _ = batch(i % 4, $self_x, $self_y, ways, base, 0deg, 1,
                      1.0fx + i as fx * 0.25fx, 0fx);
        }
        base = base + 7deg;
        wait(50);
    }
}
```

## 类型：`int / fx / angle`（三型，无隐式转换）

- 字面量：裸整数 = `int`；`1.5fx`/`1.5px` = `fx`（Q16.16，精确十进制折叠、round-half-even）；
  `90deg` = `angle`（BAM）；`16384bam` = `angle` 原值。
- 运算矩阵（非法组合 = 编译错误并提示 cast）：
  `int⊕int→int` · `fx±fx→fx` · `fx*int→fx`（普通乘）· `fx*fx→fx`（定点乘）· `fx/int→fx` ·
  `fx/fx→fx`（定点除）· **`int/fx` 非法**（方向性陷阱：整除会错位 65536 倍）·
  `angle±angle→angle`（回绕）· **`angle` 乘除一律非法** · 同型比较→`int`(0/1) ·
  `&&`/`||`/`!` 仅 `int`（短路求值）。
- cast 白名单：`int as fx`（×65536）· `fx as int`（÷65536，**向零截断**——与引擎内算术右移
  对负数不同，`-1.5fx as int == -1`）· `int as angle`/`angle as int`（位穿透）。其余组合拒绝。

## 语句

`var name: type = expr;` · 赋值 · `if c {} else {}` · `while c {}` · `loop {}` ·
`for i in a..b {}`（半开区间，`i` 为 `int`）· `break`/`continue` · `wait(n);`（n: int 帧）·
`spawn f(args);` · `return;` · 表达式语句（**值必须消费**——有返回的内建不接收就
`_ = fire(...);` 显式丢弃，不丢弃 = 编译错误；这是"忘 POP 远处爆栈"足枪的语言层灭除）。

## sub 与 async sub（调用途径强制分离）

### 根入口 `sub main()`

每一份 `.ecl` 文件**必须且仅有一个**零参数 `sub main()`（`async` 不可修饰 main）。
它是关卡的根入口脚本，只能通过引擎的 `start_main` / `start_main_with_owner` API 启动。
生命周期是**singleton**：每个 `World` 实例最多成功启动一次——即使 main 任务自然结束
或 fault，再次调用 `start_main` 也会返回 `MainAlreadyStarted`（同时触发 `contract_viol`
计数）。这一保护确保确定性回放中 main 不会重复派发。

### 普通 sub vs async sub

- **`sub`**：只能被**同步调用**（`f(args);` 语句或通过 `CALL` op 从其他 sub 调用）。
  参数/局部变量由编译器静态分配 locals 槽（调用图着色）；**禁递归**（直接/间接均编译错误，
  报错含环路径）；sub 无返回值。普通 sub 的形式名称（如 `helper`）只存在于调试符号侧载
  （`DebugInfo::Full`），运行时 `EclImage` 不为其保留 named entry——它们是 `CallOnly` sub，
  只能被其他 sub 通过 `call` 指令调用，不能 `spawn`、不能从引擎层按名解析。
- **`async sub`**：只能被 **`spawn`**（或 `fire` 的 task 引用）——开新协程（次帧首跑），
  实参拷进新任务；**不能被同步调用**（编译错误）。每个 `async sub` 都是一个**公共 named
  entry**，其名称注册在 `EclImage` 的 entry 表中。引擎层（C/Rust 宿主）可通过
  `image.resolve_entry("patrol")` 按名解析，然后通过 `world.spawn_entry()` 或
  `world.spawn_entry_named()` 启动——这是跨语言/跨脚本引用的确定性基础。
- 同一 sub 想两用？拆成两个——这是实参槽位健全性的硬约束，编译器不放行。
- 容量红线（编译期检查）：单任务 locals 总量 ≤64 字、求值栈深 ≤32、调用深 ≤8。

### 持久引用使用名称

脚本之间的持久引用（`spawn` 目标、`fire` 的 task 参数）一律使用 **sub 名称字符串**：
编译器在编译期解析名称并编码为 `canonical SubId`（运行时 `EclImage` 无字符串表，
只有 `(SubId, code_entry)` 的扁平元数据）。这意味着：
- `spawn patrol()` 在编译期解析 `patrol` 到其 `SubId`，存入 `SPAWN` 指令的操作数。
- `fire(1, $self_x, $self_y, 0fx, 0deg, WIND_CHIME, timer_ui)` 同理——`timer_ui` 作为
  `async sub` 的名称在编译期被解析并编码。
- **不存在的 sub 名称在编译期即报错**，不存在运行期"名字未找到"的分支。

## `$` 引擎变量（只读；读取即 syscall）

| 变量 | 类型 | 含义 |
|---|---|---|
| `$frame` | `int` | 当前世界帧号 |
| `$player_x` / `$player_y` | `fx` | 玩家 0 的位置 |
| `$self_x` / `$self_y` | `fx` | 任务 owner 的位置——敌→敌池坐标，弹→弹池坐标，关卡(STAGE)→`(0,0)` |
| `$self_hp` | `int` | owner 当前血量——仅敌（ENEMY）有意义，其余 owner 种类恒 0 |
| `$self_hp_max` | `int` | owner 上限血量——仅敌（ENEMY）有意义，其余 owner 种类恒 0 |
| `$self_age` | `int` | **任务**（不是 owner 实体）出生以来的帧数，对全部 owner 种类（含关卡）均有意义 |

## 引擎提供的全局状态：`globals` / `boss_ui` / `signals`

三套独立的"状态通道"，语义不同、别混用：

| 通道 | 范围 | 脚本读 | 脚本写 | 内容 / 语义 |
|---|---|:---:|:---:|---|
| `globals` 系统段 | `[0, 16)` | ✓ | ✗（no-op + `contract_viol` 计数，不 Fault） | **目前仅槽 0 有意义**：`GVAR_RANK`（难度值，game 层建场代码经世界 API 写入，脚本只读后自决）；槽 1-15 保留未用 |
| `globals` 自由段 | `[16, 1024)` | ✓ | ✓ | 脚本自定义草稿区，语义靠作者自己约定；`n` 是任意运行期表达式（不限编译期常量，可以是循环变量） |
| `boss_ui[]` | 每 boss 一份 | ✗（无读 syscall） | ✓（`boss_set`） | 血条/spell/计时状态，写给表现层 UI 消费，脚本读不回自己刚写的值 |
| `signals[8]` | 8 通道 | — | — | 不是存值用的：`pulse_signal(ch)` 发边沿脉冲，`wait_signal` xform op 在变换序列里等；只唤醒当帧已在等待的弹，不锁存 |

`globals`/`set_global` 读写走 `global(n)`/`set_global(n,v)`。同一 `.ecl` 文件内建议配 `const`
给自由段槽号起名（如样例里的 `const RANK_SLOT: int = 0;`）避免魔数；**跨 `.ecl` 文件没有
共享机制**（见"已知限制"）——多个脚本文件各自手选槽号，选中同一个存不同东西不会有任何
编译或运行期报错，纯靠作者自律对齐。

字节码层完整规则（segment 边界钉法、校验和归属等）见 [`ecl-ops.md`](ecl-ops.md)——本节只讲
脚本作者需要知道的部分。

## 引擎常量（编译器预置注入，C14）

编译器在处理脚本自己的 `const`/`xformdef`/`sub` **之前**，先把一批 Rust 侧命名常量当作
"第 1 行前已声明的 `const`" 预填进类型检查的常量表——脚本表达式、`const` 初始值、
xformdef 槽参数（编译期常量位置）处都能直接引用，不用再手写字面量镜像：

| 名字 | 值 | 含义 |
|---|---:|---|
| `APPEARANCE_SMALL` | `0` | 弹外观表——小 |
| `APPEARANCE_MEDIUM` | `1` | 弹外观表——中 |
| `APPEARANCE_LARGE` | `2` | 弹外观表——大 |
| `APPEARANCE_STAR` | `3` | 弹外观表——星 |
| `GVAR_RANK` | `0` | `globals` 系统段内 RANK（难度）槽号，见上节 |
| `GLOBALS_SYS_SEGMENT` | `16` | `globals` 系统段/自由段分界槽号，见上节 |

值即引擎侧同名 Rust 常量（脚本侧统一按 `int` 携带：`Fx`/`Angle` 类型的常量会带原始
raw 值，不是十进制含义值——目前表里的名字都恰好是 `int` 类型，无此坑；新增 `fx`/`angle`
类型的引擎常量时留意）。脚本**不得**重新声明同名 `const`——会在类型检查阶段报错
`'NAME' 与引擎常量重名，不能重新声明`（`typeck/consts.rs`），无论脚本里写的值是否一致。

权威定义是 `stg-core` 的 `crates/stg-core/src/consts.rs`，`engine_consts!` 宏对每行同时
生成 Rust 侧 `pub const`（供世界层/harness 代码用，类型保真）与注入表
`ENGINE_CONSTS: &[EngineConst]`（脚本侧统一 `i32`，供编译器 `lang::compile` 默认注入）——
两头共享同一处字面量，杜绝手写复写漂移。**加一个新的引擎常量 = 在该宏调用里加一行**；
C11（`WorldTables` 文件加载）落地后，appearance/道具等表驱动的常量可能改由数据文件
（连同其 `content_hash`）生成而非手写宏调用，命名注入的使用方式不受影响。

## 内建函数（签名以 `builtins.rs` 为准）

`fire(appearance:int, x:fx, y:fx, speed:fx, angle:angle, xf:XFORMDEF名|none, task:ASYNC_SUB名|none) -> int` ·
`batch(appearance, x, y, n_angle:int, angle0:angle, angle_step:angle, n_speed:int, speed0:fx, speed_step:fx) -> int` ·
`spawn_enemy(x,y,hp,drop_table,score) -> int` · `drop_item(x,y,ty) -> int` ·
`move_to(dur:int,x:fx,y:fx,easing:int)` · `boss_set(slot,ratio:fx,spell,timer,phase,active)` ·
`pulse_signal(ch)` · `rand(n:int) -> int` · `global(n) -> int` · `set_global(n,v)`
（槽 0-15 系统段脚本只读）· `aim_player() -> angle` · `sin/cos(a:angle) -> fx` · 弹 setter 族。

## xformdef（弹变换序列声明）

```ecl
xformdef NAME { op(args); @wait op(args); ... }
```

- op 名 = [`xform-ops.md`](xform-ops.md) 小写助记（`turn`/`set_speed`/`set_ang_vel`/
  `step_speed`…）；`@N` 前缀 = 该槽 wait N 帧；参数必须**编译期常量**（字面量/const/一元负号）。
- **STEP 族（`step_speed`/`step_angle`）物理占 2 槽**——scratch 由编译器自动补，作者按 1 条写；
  物理槽总数 ≤16。`loop`/`end` 不开放（复杂控制流写任务弹；尾部零填充天然 END）。
- 被 `fire(..., NAME, ...)` 引用才占 locals 空间（3 字/物理槽，算进引用它的 sub 的容量账）。

## 错误格式与已知限制

- 错误：`文件:行:列: 说明` + 源行摘录 + `^` 定位；一个错误不吞后续（恢复到语句边界）。
- 已知限制（v1）：禁递归 · locals 静态分配（同 sub 内变量名不可重名）· sub 无返回值 ·
  **无跨语言常量引用**（appearance id/globals 槽号需手写数字镜像，见 follow-ups）·
  时间标签 `+N:` 未进 v1（显式 `wait`）。
