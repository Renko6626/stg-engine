# ECL 表层语言参考（作者第一入口）

> **给 coding agent 的三行须知**
> 1. **本文档是写/改 `.ecl` 的唯一权威**——不要凭对 ZUN ECL 或其它弹幕 DSL 的记忆脑补语法。
> 2. **改完必跑** `cargo run -p stg-harness -- check <file.ecl|目录>`——看行列错误（目录 =
>    多文件整局，见下"多文件"节），debug 环见下"debug 循环"节。
> 3. **builtin 签名以下方"内建函数"生成段为准**（`gen-ecl-meta` 单一真相源，改 `builtins.rs`
>    才是正确改法，不要手改生成段——手改会被 `cargo test` 的防漂移断言打回）。

> **这是什么**：`.ecl` 脚本作者手册——语法、类型、内建函数、`$` 引擎变量、xformdef、错误格式。
> **权威来源**（冲突时以它们为准）：`crates/stg-ecl-compiler/src/lang/`（`builtins.rs` 内建表 /
> `xform_map.rs` xform 操作表）· spec `docs/superpowers/specs/2026-07-18-m19-ecl-language.md`。
> 字节码层参考（op/syscall/fault 码）见 [`ecl-ops.md`](ecl-ops.md)——作者通常不需要看它。
> 编译时机：启动时从源码文本编译（`lang::compile`），编译器确定性有测试押运。

## 坑清单（全部实证，逐条对照代码核实过）

- 小数字面量必须带 `fx`/`px` 后缀——裸 `1.5` 无后缀是**词法错误**（"浮点数字面量缺少单位
  后缀"），不会被当成 int 静默截断。
- 角度字面量必须带 `deg`/`bam` 后缀——裸 `90` 是 `int`，用在角度位**不会隐式转换**
  （报"期待 Angle，实际 Int"）；`int as angle` cast 也救不了，它是**位穿透**不是"转成度"
  （`90 as angle` ≠ 90°）。
- 有返回值的 builtin **不作表达式用时必须 `_ = ` 显式弃值**（如 `_ = fire(...);`）——不丢弃
  是编译错误（"返回值未消费"）。
- **void builtin 只能裸语句**——它前面加 `_ = ` 反而是编译错误（"无返回值，无值可丢弃"）；
  `_ =` 只对**有**返回值的调用有意义。
- `fire`/`spell_begin` 的 xf/task/pattern 位是**标识符或字面量 `none`**（xformdef 名 / sub
  名），不是求值表达式——写 `fire(..., xf_or_none_expr(), ...)` 这种"算出来的引用"通不过。
- **新建任务出生当帧不跑**：`spawn`/`fire` 的 `task` 参/`spell_begin` 的 `pattern` 新起的
  协程，创建那一帧（`born_frame`）不执行任何一条指令，**下一帧才首次跑**——从"存在"的
  角度说是 spawn 后第 2 个 step 才真正活起来，别指望它当帧就能观察到效果。
- `drop_item` 消耗模拟 RNG（带随机喷发速度）——想要逐帧确定性回放对拍时，留意它和其它
  `rand(n)` 调用共享同一颗 PRNG 流，调用顺序会影响后续随机数。
- 难度用 `global(GVAR_RANK)` 读，不要自己另起变量镜像它。
- 符卡全套 = `spell_begin(slot, id, pattern, time_limit, bonus0, flags, hp_threshold);` +
  `wait_spell();` 两行；`wait_spell` 在语句位置（`wait_spell(`）**总是**被语法糖截胡，
  即使你恰好声明了同名 sub 也调不到它——按保留字对待。
- `mark` 只能在 `sub main` 顶层语句位（不进 `if`/`while`/`for`/`loop` 块，也不能出现在
  别的 sub 里），编号是 **int 型编译期常量**、必须是正整数、且全镜像唯一（跨文件合并后
  仍在同一份名字空间里查重）；main 顶层的 `var` 不得先于任何 `mark`（任务帧局部零初始化，
  跳入 mark 落点时局部区还没被跑到）。跳进来的世界里，`mark` 之前正常流程本该写过的全局
  变量/局部变量全是初始零值，需要的值在 `mark` 的补偿块里手写补。
- 锚点自动补偿只认**顶层线性位**的常量参声明——`if`/难度分支里、`spawn`/`fire` 挂出的
  异步任务里写的 `bgm`/`bg`/`bg_phase` 一律扫不到（扫描不下潜控制流、不跟异步边），中段
  启动时只会拿到"扫描能看见的那条"最近声明；这类分支/异步声明得靠 `mark` 块里手写覆盖。
- `add_score` 的 `delta` 允许负值（扣分），结果**饱和钳**在 `[0, u64::MAX]`——扣穿只会
  停在 0，不会像有符号回绕那样绕成一个巨大正数。

## 一分钟样例

```ecl
const SPELL_WINDCHIME: int = 1;

xformdef WIND_CHIME { set_speed(2.0fx); @30 turn(90deg); }

async sub patrol() {
    loop {
        move_to(180, -120fx, 100fx, 2); wait(180);
        move_to(180,  120fx, 100fx, 2); wait(180);
    }
}

async sub windchime_pattern() {
    var base: angle = 0deg;
    loop {
        var ways: int = 28 + global(GVAR_RANK) * 2;
        for i in 0..5 {
            _ = batch(i % 4, $self_x, $self_y, ways, base, 0deg, 1,
                      1.0fx + i as fx * 0.25fx, 0fx);
        }
        base = base + 7deg;
        wait(50);
    }
}

sub main() {
    spawn patrol();
    spell_begin(0, SPELL_WINDCHIME, windchime_pattern, 3600, 100000, 0, 0);
    wait_spell();
}
```

（记账——计时/衰减/超时/破卡/UI 喂送——全归引擎机构，脚本只管宣言 + 弹幕行为 + 收尾等待；
细节见下方"符卡"节。）

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
`spawn f(args);` · `return;` · `wait_spell();`（符卡等待语法糖，纯前端展开为
`while spell_timer() >= 0 { wait(1); }`，见下"符卡"节）· 表达式语句（**值必须消费**——
有返回的内建不接收就 `_ = fire(...);` 显式丢弃，不丢弃 = 编译错误；这是"忘 POP 远处爆栈"
足枪的语言层灭除）。

## mark（中段启动标记）

`mark` 给整局脚本开"练习/中段启动"的落点（整局流程刀 spec §2）——语义是**规范态开局**
（符卡练习那味儿），不是"仿佛打过来的状态"：跳进来的世界不重放被跳过的帧，靠脚本自己
声明"这里该长成什么样"。

两种写法：

```ecl
const MARK_S2: int = 2;
const GVAR_ROUTE: int = 16;

sub stage1() { bgm(1); wait(60); }
sub stage2() { bgm(2); wait(60); }

sub main() {
    stage1();
    mark(MARK_S2) {
        set_global(GVAR_ROUTE, 1); // 被跳过流程写的全局变量,在此手写补
    }
    stage2();
    loop { wait(60); }
}
```

- `mark(<id>);`——纯落点，无补偿块。
- `mark(<id>) { <补偿块> }`——落点带一段"跳入时才执行"的补偿块。

**landing pad 语义**：降低为固定序列 `JMP after; landing: <编译期自动补偿><作者手写补偿块>;
after:`。正常流程（从头开局，或本次中段启动的落点不是这个 `mark`）执行到这条 `JMP` 时
一步**跨过**整段垫片（自动补偿 + 作者块都不执行），落进 `after` 接着往下——垫片不会被
正常流程重复执行第二遍。中段启动命中这个 `mark` 时，根任务的 `pc` 直接摆在 `landing`，
从垫片首指令开始顺序往下跑（补偿代码 → 作者块 → 自然接上 `after`），不会绕回来第二次。

**自动补偿规则**：编译器把 `main` 的同步调用链按**顶层线性位**展开（`main` 自己 + 递归进
被同步调用的 sub；不下潜 `if`/`while`/`for`/`loop`，不跟 `spawn`/`fire` 挂出的异步任务；
访问过的 sub 不重复展开，防环），沿途记录 `bgm`/`bg`/`bg_phase` 三类声明"当前最新的常量
实参值"（只认字面量/`const`，变量参视为"没声明过"）。每个 `mark` 处按这一刻的"最新值"
快照，为三类里作者没有手写覆盖的类别各注入一条 `push_i(值); sys(...)`（顺序固定
`bgm → bg → bg_phase`，严格排在作者手写块之前）；`bg_phase` 只有在其记录位置**严格晚于**
最近一次 `bg` 声明时才注入，否则视为"旧背景的段号"弃置不用。作者若在补偿块**顶层**手写了
同名的锚点调用（如上例没写 `bgm`/`bg`），对应那一类就不再自动注入——三类各自独立判断。

以上例为例：正常流程（`start=0`）执行到 `stage1()`（`bgm=1`）后一步跳过整段
`mark(MARK_S2)` 垫片（`GVAR_ROUTE` 不会被写），直接进 `stage2()`（`bgm=2`）。中段启动
`start=MARK_S2` 时 `stage1()` 整个不执行，垫片自动补 `bgm=1`（`stage1` 里的声明）后走
作者块把 `GVAR_ROUTE` 设成 1，再自然接上 `stage2()` 把 `bgm` 覆写成 2——两条路径最终看到
的 `bgm_id` 相同，但 `GVAR_ROUTE` 只有中段启动这条路径会被设置。

**`new_game_at(seed, rank, start, loadout)` 宿主侧对应关系**：`start=0` 等价于从头开局
（`new_game(seed, rank, image)` 就是 `new_game_at(seed, rank, 0, Loadout::default(), image)`
的委托）；`start=<mark id>` 直接把根任务 `pc` 定到该 `mark` 的落点（`EclImage::resolve_mark`
查表）。`start` 若未在镜像标记表命中（含负值——标记表 id 恒正，天然不命中）是**宿主期
响亮错**（`TaskStartError::UnknownMark`），发生在 `World` 被分配之前，不会返回一个半初始化
的世界。`loadout`（`character`/`power`/`lives`/`bombs` 四个标量）与 `start` 相互独立、
同一次调用一起给：`power`/`lives`/`bombs` 越界直接钳位（P4-b），`character` 越
`WorldTables::characters` 表界是另一条宿主期响亮错（`TaskStartError::InvalidCharacter`）——
装备是"玩家在菜单调好的数据"，从建世界的门直接进，不走脚本。

## sub 与 async sub（调用途径强制分离）

### 根入口 `sub main()`

整份编译产物（单文件，或"多文件"节说的多文件合并成的一整个 `EclImage`）**必须且仅有一个**
零参数 `sub main()`（`async` 不可修饰 main）——单文件时它就在那一个文件里；多文件时它只
出现在其中一个文件里，其余文件不需要、也不能再声明一个。它是关卡的根入口脚本，只能通过
引擎的 `start_main` / `start_main_with_owner` API 启动。
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
- `fire(1, $self_x, $self_y, 0fx, 0deg, WIND_CHIME, trail_task)` 同理——`trail_task` 作为
  `async sub` 的名称在编译期被解析并编码。
- **不存在的 sub 名称在编译期即报错**，不存在运行期"名字未找到"的分支。

## 多文件（整局脚本布局）

一局完整的游戏可能是好几十个符卡/关卡拼起来的——巨型单文件 `.ecl` 不好维护。表层语言把
"一局一镜像"这个约束（一局 = 一个 `EclImage` = 一个 `content_hash`，回放/握手身份的一部分，
整局流程刀 spec §1/§0）和"源码摊几个文件"这件事解耦：

```
stage/
  01_intro.ecl    // sub main() 在这里
  02_stage1.ecl   // sub stage1() 等
  03_stage2.ecl
  99_boss.ecl
```

```
cargo run -p stg-harness -- check stage/
```

- **目录 = 编译单元集**：无论是 `stg-harness check <目录>` 还是引擎宿主的加载入口，收到
  一个目录路径时都会收集该目录下全部 `*.ecl` 文件、**按文件名字节序排序**后逐个读入，
  作为一批编译单元合并编译；单文件路径照旧当成单元素单元集处理，行为与只有单文件时完全
  一致。这是零配置格式——没有 manifest/清单文件，文件名怎么排全靠文件名本身的字节序。
- **编译期先各自独立 `lex`/`parse`**——每个文件的语法错误各自带**该文件的文件名 + 局部
  行号**（不是拼接后的全局行号），预检阶段同时收集全部顶层名字（`sub`/`const`/`xformdef`）
  做跨文件撞名检查；预检通过后合并 AST 走已有 `typeck`/`slots`/`codegen` 单管线（这几个
  阶段本就不知道"文件"这个概念，产物类型检查/字节码生成侧零改动）。
- **扁平命名空间**：所有文件共享同一个全局符号表——`sub`/`const`/`xformdef` 的名字不分
  文件，跨文件重名在编译期报错，错误信息带**两处位置**（本次撞上的文件:行 + 另一处定义
  所在的文件:行）。这意味着多文件不是模块系统——不能靠文件名做命名空间隔离，两个文件各写
  一个同名 `sub helper()` 就是重复定义，不会因为在不同文件里而相安无事。
- **收集顺序不影响产物字节**：不管传入的文件先后序是 A→B 还是 B→A，合并后的 `EclImage`
  逐位相同——codegen 本就按 sub 名排序出 canonical id，与源码收集顺序无关；目录收集仍然
  固定按文件名字节序，只是"结果不随之改变"，不是"顺序随意写"。
- **明确不做**：`include` 语法（会逼编译器做路径解析，破坏"编译器是纯函数、文件收集归
  调用方"这条断层线纪律）、模块系统/命名空间（扁平全局名字空间 + 撞名报错已经够用）。

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
| `boss_ui[]` | 每 boss 一份 | ✗（无读 syscall） | ✓（`boss_set`） | 血条/spell/计时状态，写给表现层 UI 消费，脚本读不回自己刚写的值；**符卡 active 期间** `enemy`/`spell_id`/`timer_frames`/`active`/`hp_ratio` 由符卡机构逐帧自动覆写（见下"符卡"节），脚本的 `boss_set` 此时只对 `phase_left`（阶段号）全权——非符卡段（卡与卡之间）`boss_set` 照旧全权写全部字段 |
| `signals[8]` | 8 通道 | — | — | 不是存值用的：`pulse_signal(ch)` 发边沿脉冲，`wait_signal` xform op 在变换序列里等；只唤醒当帧已在等待的弹，不锁存 |

`globals`/`set_global` 读写走 `global(n)`/`set_global(n,v)`。**系统段**槽位已有引擎注入的具名
常量可直接用——如 RANK 槽写 `global(GVAR_RANK)`，见下节"引擎常量"，不需要也不应该自己再起
名字镜像槽号。**自由段**（`[16, 1024)`，即 `[GLOBALS_SYS_SEGMENT, GLOBALS_CAP)`）没有引擎预置
名字，同一 `.ecl` 文件内建议配 `const` 给自己用到的自由段槽号起名（如 `const MY_SLOT: int = 16;`）
避免魔数；**跨 `.ecl` 文件没有共享机制**（见"已知限制"）——多个脚本文件各自手选槽号，选中
同一个存不同东西不会有任何编译或运行期报错，纯靠作者自律对齐。

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

`.ecl` 编译现绑定一张表（`compile`/`compile_for_table`）：编译产物 `EclImage` 记录该表的
`content_hash`，运行时若加载的表与之不符，`start_main` 拒绝启动（`TableImageMismatch`）——
这是 C14 记档的"注入常量与运行期表必须同源"这条 coherence 不变量的机制化，见
`docs/follow-ups.md` C11/C14。

## 内建函数（生成段，签名以此为准；`cargo run -p stg-harness -- gen-ecl-meta` 从
`builtins.rs` 的 `Builtin.doc`/`param_names` 渲染，勿手改下面 `<!-- gen -->` 之间的内容——
改动 builtin 元数据请去改 `crates/stg-ecl-compiler/src/lang/builtins.rs` 再重跑生成器,
否则会被 `committed_doc_segment_matches_generated` 防漂移测试打回)

<!-- gen:builtins:begin -->
- `fire(appearance: int, x: fx, y: fx, speed: fx, angle: angle, xf: xform|none, task: sub|none) -> int` — 发一颗弹;appearance 查外观表(越界 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1
- `batch(appearance: int, x: fx, y: fx, n_angle: int, angle0: angle, angle_step: angle, n_speed: int, speed0: fx, speed_step: fx) -> int` — N-way 批量发环;返实际创建数
- `spawn_enemy(x: fx, y: fx, hp: int, drop_table: int, score: int) -> int` — 造敌;sprite 固定 0、判定 12/16 默认;返敌句柄,失败 -1
- `drop_item(x: fx, y: fx, item_type: int) -> int` — 掉一颗道具(带随机喷发速度,消耗模拟 RNG);返句柄,失败 -1
- `move_to(dur: int, x: fx, y: fx, easing: int)` — 敌自身(self owner 非 ENEMY → Fault)按 easing 缓动、dur 帧内平移到 (x,y);四参数皆真实压栈(不同于下方弹 setter 族的占位 handle 首参)
- `boss_set(slot: int, hp_ratio: fx, spell_id: int, timer_frames: int, phase_left: int, active: int)` — 整槽写 boss_ui 公告板(脚本写/UI 读);enemy 字段取自 self owner(非 ENEMY → NULL,不 Fault);符卡 active 期 enemy/spell_id/timer_frames/hp_ratio 由引擎逐帧自动覆写,phase_left 不受影响仍归脚本
- `pulse_signal(channel: int)` — 脉冲一条信号通道(边沿语义,仅当帧有效);放行处于弹变换 WAIT_SIGNAL 停驻态的弹(非 ECL 任务)
- `emit_req(id: int, a0: int|fx|angle, a1: int|fx|angle, a2: int|fx|angle, a3: int|fx|angle, a4: int|fx|angle, a5: int|fx|angle)` — 通道 B 渲染请求;void 只能裸语句;args 裸载荷(fx 过 raw/angle 过 BAM/int 原样)
- `rand(n: int) -> int` — 模拟 RNG 均匀 [0,n);确定性,随快照回卷
- `global(slot: int) -> int` — 读 globals 槽(GVAR_RANK=0 为难度)
- `set_global(slot: int, value: int)` — 写 globals 槽;系统段(slot<16)脚本写为 no-op+计数,不 Fault(GVAR_RANK=0 建议脚本只读)
- `aim_player() -> angle` — 自身(敌/弹属主)指向自机的 BAM 角
- `sin(angle: angle) -> fx` — 查表三角,返 fx(VM op 直发,非 syscall)
- `cos(angle: angle) -> fx` — 查表三角,返 fx(VM op 直发,非 syscall)
- `set_speed(handle: int, speed: fx)` — 弹 setter:改速率;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_angle(handle: int, angle: angle)` — 弹 setter:改方向;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `turn(handle: int, delta: angle)` — 弹 setter:转向增量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_vel(handle: int, vx: fx, vy: fx)` — 弹 setter:直设速度向量;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `set_ang_vel(handle: int, w: int)` — 弹 setter:角速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 angle+=w)
- `set_accel(handle: int, a: fx)` — 弹 setter:切向加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(POLAR_FX:每帧 speed+=a)
- `set_gravity(handle: int, gx: fx, gy: fx)` — 弹 setter:直角加速度;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(CART_FX:每帧 v+=(gx,gy);与 POLAR_FX 互斥)
- `stop_fx(handle: int)` — 弹 setter:停连续效果;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定(清 POLAR_FX/CART_FX 连续效果)
- `aim_at_player(handle: int, offset: angle)` — 弹 setter:指向自机方向再加 offset 偏移角;作用于自身(self owner 非 BULLET → Fault);首参 handle 为占位求值后丢弃,不参与判定
- `spell_begin(slot: int, spell_id: int, pattern: sub|none, time_limit: int, bonus0: int, flags: int, hp_threshold: int)` — 开卡:绑 boss/血线/计时/计分,spawn pattern 为卡绑定模式任务(随卡生死)
- `spell_end()` — 手动收卡(取卡按血线自动判,通常不需要)
- `spell_timer() -> int` — 当前卡剩余帧数
- `add_score(delta: int)` — 给自机记分:delta 允许负(扣分),饱和钳 [0,u64::MAX] 不回绕;关底 bonus/结算记账用
- `bgm(id: int)` — 声明当前 BGM:写世界锚点字段 bgm_id 并发 REQ_BGM;mark 跳入自动补偿最近声明(常量参)
- `bg(id: int)` — 声明当前背景:写锚点 bg_id 并发 REQ_BG;换背景隐含新的 phase 纪元(补偿细则见 ecl-lang)
- `bg_phase(phase: int)` — 声明背景演出段号:写 bg_phase 并自动盖 bg_phase_frame=当前帧,发 REQ_BG_PHASE;表现层按段内局部时间 seek
<!-- gen:builtins:end -->

> **弹 setter 族的 handle 参数是陷阱位**（`set_speed`/`set_angle`/`turn`/`set_vel`/
> `set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player` 九个）：首参
> `handle:int` **求值后即丢弃**，setter 恒作用于**当前任务的 owner 弹**（`self` 语义）——
> 不能借句柄定向操纵别的弹；owner 不是弹的任务调它 → 任务 Fault。想操纵 `fire(...)`
> 出来的那颗弹，用 xformdef 或 `fire` 的 `task` 参数挂子任务。

## 符卡（`spell_begin` / `spell_end` / `spell_timer` / `wait_spell`）

一张符卡的记账（计时递减、bonus 衰减、资格判定、破卡自动检测、超时判定、`boss_ui[]` 自动
喂送）全归引擎机构（`SpellState`，settle 相位符卡趟）；脚本只管**宣言 + 弹幕行为 + 收尾
等待**——两行范式：

```ecl
const SPELL_DEMO: int = 1;

async sub demo_pattern() {
    loop { wait(60); }
}

sub main() {
    spell_begin(0, SPELL_DEMO, demo_pattern, 3600, 100000, 0, 0);
    wait_spell();
}
```

（多张卡 = boss 主控 sub 里顺序执行多组这两行，前一张 `wait_spell()` 返回后紧跟下一张
`spell_begin`——见下"多卡序"。）

参数表（声明序）：

| 位 | 类型 | 含义 |
|---|---|---|
| `slot` | `int` | 符卡槽号（`0..MAX_BOSSES`，v1 每 boss 一份） |
| `id` | `int` | 卡 id——纯脚本词汇，引擎不登记，见下"卡 id 约定" |
| `pattern` | sub 名 \| `none` | 模式 sub 引用（同 `fire` 的 `task` 参同款 `SubRef`）：必须是**无参** `async sub`；`none` 不 spawn（自定义卡留口，配合 `spell_end()` 逃生舱口自行判定结束） |
| `time_limit` | `int` | 时限，单位帧，必须 `>0` |
| `bonus0` | `int` | 起始 bonus（分），必须 `≥0`；衰减地板 = `bonus0/10`，衰减速率 = `(bonus0-地板)/time_limit`，均在 `spell_begin` 当帧一次算定 |
| `flags` | `int` | 位标志：bit0 `SPELL_SURVIVAL`（耐久卡——活到超时即收卡点，而非失败）；bit1 `SPELL_NO_CLEAR`（结束时不自动铺全屏消弹 field） |
| `hp_threshold` | `int` | 破卡血线，必须 `≥0` 且 `≤` 当前 owner 血量；绑定敌 hp 触底/低于此值时自动收卡结算，且伤害结算对绑定敌**下钳**在此值（防打穿到非最终卡血线以下）——多卡序用"一池总血 + 逐卡递降血线"表达，最终卡 `hp_threshold=0` |

- **`spell_end()`**（无参、无返回值）——逃生舱口：给非 HP/超时的自定义结束条件用（比如
  剧情触发提前收卡）。owner 有绑定槽时走 HP 路径结算（资格在→CAPTURED 付 `bonus_now`，
  资格失→FAILED）；无绑定槽调用是 no-op（重复调用安全，不算违约）。
- **`spell_timer() -> int`**——读 owner 当前绑定槽的剩余帧数 `frames_left`；owner
  **没有**绑定槽（还没 `spell_begin`，或卡已经结束）恒返回 `-1`——这正是 `wait_spell()`
  糖的判据，也是原语本身，需要自定义等待逻辑时可以直接手写。
- **`wait_spell()` 语法糖**——编译期展开为 `while spell_timer() >= 0 { wait(1); }`：
  纯前端展开，不新增字节码语义,和你手写这行 `while` 编译出**逐字节相同**的 `EclImage`。
  写 `wait_spell();` 只是省一行样板，语义上和手写等价 `while` 完全没有区别。它只在
  **语句位置**（`wait_spell(`）拦截展开——不是真正的词法关键字（词法层没有为它开专属
  token），但对脚本作者而言效果等同保留字：这个名字用作调用永远被截胡，不会退回成
  同名 sub 调用。
- **模式随卡生死**（`pattern` 参数的核心承诺）：`spell_begin` spawn 出的模式任务绑定
  本卡槽；它自己 `spawn` 出的**整棵子任务树**继承同一绑定——收卡结算的瞬间，这整棵树
  下一帧起自动终止（相位 2 调度门禁杀，脚本不必写 `kill_children`）。反例：`fire(...)`
  挂在弹上的任务**不继承**这条规则（弹命归弹，残留弹演完自己剧本，ZUN 语义）。唯一后果：
  模式 sub 可以放心写 `loop { ... }` 死循环——不用自己判断"卡是不是已经结束"，引擎替你
  收尾。
- **卡 id 约定**：`id` 纯粹是脚本/关卡资产，引擎不做任何登记（不校验唯一、不映射名字或
  立绘）——每份 `.ecl` 建议用 `const` 命名，如 `const SPELL_WINDCHIME: int = 1;`，同文件
  内每张卡起一个数字即可；跨 `.ecl` 文件没有共享机制（同"引擎常量"节自由段槽号的纪律，
  纯靠作者自律对齐）。
- **多卡序**：卡切换就是主控 sub 里顺序执行下一组 `spell_begin`/`wait_spell`——两行一卡，
  不需要任何引擎侧"下一张卡"排程；引擎不提供、也不打算提供自动切卡机制（`spell_bound`
  只管单卡模式树的生死，不管卡与卡之间怎么编排，见世界侧机构 spec 的"不做什么"节）。

字节码层完整规则（syscall 号、越界/坏参数处置、事件/请求 id）见
[`ecl-ops.md`](ecl-ops.md)"符卡计器"节；世界侧机构设计（结算矩阵、伤害下钳、逐卡血条公式）
见 `docs/superpowers/specs/2026-07-24-spell-meter-design.md`。

## 渲染请求（通道 B）

`emit_req(id:int, a0,a1,a2,a3,a4,a5: raw)` —— 向表现层推送一次性演出请求（爆炸/音效/宣言/
震屏）。固定 7 参，不足位补 `0`；六个载荷位是 **raw 参数**：接受 `int`/`fx`/`angle` 任意型
表达式**按位原样**传出（`1.5fx` → Q16.16 raw = 98304、`90deg` → BAM raw = 16384、int 原样），
表现层按 id 约定解码。无返回值（只能做语句）；缓冲满确定性丢弃（不 Fault）；无 owner 类别
限制——关卡任务也能发。

id 命名空间：`0` 保留无效 · `1..=63` 引擎保留（如 `REQ_ENEMY_DEATH`）· `64+` 脚本自由——
建议 `const MY_REQ: int = REQ_SCRIPT_BASE + n;` 起名。引擎 id `1..=3` 的逐位 args 约定表见
`stg-core/src/reqs.rs` 模块文档（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）；
`4..=7`（整局流程刀新增，同一编码律）见下表：

| id | args[0] | args[1..] |
|---|---|---|
| `REQ_STAGE_CLEAR`（4） | 脚本自定（挂牌用；见下方转场协议） | 0 |
| `REQ_BGM`（5） | `id`（int，同写入的 `bgm_id`） | 0 |
| `REQ_BG`（6） | `id`（int，同写入的 `bg_id`） | 0 |
| `REQ_BG_PHASE`（7） | `phase`（int，同写入的 `bg_phase`） | 0 |

`REQ_BGM`/`REQ_BG`/`REQ_BG_PHASE` 由 `bgm`/`bg`/`bg_phase` 三个 builtin 内部经对应的 5x
syscall 自动发出——脚本不需要、也不应该自己再手写一次 `emit_req` 发这三个 id。
`REQ_STAGE_CLEAR` 没有专属 syscall/builtin，是纯粹的挂牌协议常量：脚本用通用的
`emit_req(REQ_STAGE_CLEAR, ...)` 自己发。

**关卡结算转场协议**（spec §6 摘编；纯宿主约定，引擎侧零改动——`World` 是纯被动状态机，
宿主不调 `step` 就是完美冻结）：

1. 脚本关底先把账在世界内记完（`add_score(bonus);`），再 `emit_req(REQ_STAGE_CLEAR, …);`
   挂牌，随后**直接续行**（比如接着调用 `stage2();`）——世界对"暂停"这件事零感知。
2. 宿主每帧 `step` 后经 `take_requests()` 看见 `REQ_STAGE_CLEAR` 挂牌，就此**停手不再
   `step`**，用读口数据画结算/菜单页（原生 UI，`World` 冻结不动，不进这条时间线）。
3. 玩家确认后宿主恢复 `step`——世界里 `stage2` 的第一帧才真正发生。回放文件里没有"结算页"
   这个概念（暂停期贡献零帧），重播时直接穿过，行为与真实机台一致。

## xformdef（弹变换序列声明）

```ecl
xformdef ARC_SHOT {
    set_speed(1.5fx);
    @20 turn(45deg);
    @20 turn(-45deg);
    set_life(180);
}

sub main() {
    _ = fire(APPEARANCE_SMALL, 0fx, 0fx, 1.0fx, 0deg, ARC_SHOT, none);
}
```

- op 名 = [`xform-ops.md`](xform-ops.md) 小写助记（`turn`/`set_speed`/`set_ang_vel`/
  `step_speed`…）；`@N` 前缀 = 该槽 wait N 帧；参数必须**编译期常量**（字面量/const/一元负号）。
- **STEP 族（`step_speed`/`step_angle`）物理占 2 槽**——scratch 由编译器自动补，作者按 1 条写；
  物理槽总数 ≤16。`loop`/`end` 不开放（复杂控制流写任务弹；尾部零填充天然 END）。
- 被 `fire(..., NAME, ...)` 引用才占 locals 空间（3 字/物理槽，算进引用它的 sub 的容量账）。

## debug 循环（改代码 → check → 再改）

写/改 `.ecl` 脚本的最短反馈环，人 / coding agent / CI 共用：

1. 改 `.ecl` 源码；
2. `cargo run -p stg-harness -- check <file.ecl>`——**通过** = 打印 `OK`、退出码 0；
   **不通过** = 逐条 `文件:行:列: 说明` + 源行摘录 + `^` 定位打到 stderr、退出码 1
   （文件不存在或不带参数是第三路，退出码 2）；回第 1 步照错误位置改，不通过就不必往下走。
3. `check` 只证明"编译通过"，**不证明"跑起来对"**——真要把脚本接进金向量/查看器/游戏前，
   先跑一遍 `cargo test --workspace` 保证没有引入既有回归（含编译器自身的确定性测试），
   再上场跑 `golden`/`serve`。

## 错误格式与已知限制

- 错误：`文件:行:列: 说明` + 源行摘录 + `^` 定位；一个错误不吞后续（恢复到语句边界）。
- 已知限制（v1）：禁递归 · locals 静态分配（同 sub 内变量名不可重名）· sub 无返回值 ·
  **跨 `.ecl` 文件无共享的自定义常量机制**（脚本各自的 `const`/自由段槽号约定不互通，纯靠
  作者自律对齐；引擎侧命名常量——appearance id/`GVAR_RANK` 等——已由预置注入解决，见上文
  "引擎常量"节与 `follow-ups.md` C14）· 时间标签 `+N:` 未进 v1（显式 `wait`）。
