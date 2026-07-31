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
- **`wait(n)` 的 `n` 被静默截成低 16 位**：`wait(65536)` 等 **0** 帧、`wait(-1)` 等 **65535**
  帧、`wait(70000)` 变成 4464——没有任何诊断。上限 65535 帧（≈18 分钟），要等更久套循环。
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
- **敌的主任务（`spawn_enemy` 的 `task` 参）一返回，这只敌就退场**——不是"任务没了敌还在"。
  要它留着就别让主任务返回（末尾 `loop { wait(1); }`）。这条最容易踩，详见下方
  "敌主任务跑完 = 这只敌退场"。
- `add_lives`/`add_bombs`/`add_power` 是**增量**且**双边钳位**（钳位是正常语义，不报错）；
  `add_power` 的单位是厘火力（`100` = 显示 1.00，上限 `400`），别写成 `add_power(4)`。
  **没有 `set_*` 版本**，开局装备走菜单侧的 `Loadout`。
- `clear_bullets()` 清的每颗弹都会**原位转一颗星星**（30 分），且**不给无敌帧**——它不是
  bomb，别拿它保命。
- **`drop_items()` 吐完不清空计数**——`drop_items(); die();` 会把同一批道具掉**两份**。
  这是照 ZUN 的字面语义，不是 bug；只想掉一份就别在 `die()` 前调它。详见下方"敌人的三条
  死亡路径与掉落控制"。
- **`die()` 立即终止本任务**（降低成两条指令，第二条是终止），它后面的语句一句都不执行。
- **`atan2(y, x)` 的 `y` 在前**（同 libm），两参同为 `fx` ⇒ 写反了**不报错**，只会把角度
  沿 45° 对角线镜像；`dist(dx, dy)` 是**向量模**不是两点距离（两点距离自己减）。
- **`enemy_x`/`enemy_y` 的无效句柄返 `0`，那不是哨兵**——坐标没有哨兵位可用（任何 `fx`
  都可能是真坐标），所以"敌恰在原点"与"号无效"读起来一样。读坐标前先
  `enemy_hp(e) != -1` 探活；**别用 `enemy_hp(e) >= 0` 探**（被打穿的敌血量是真实负数，
  那样会把还在场上的敌误判成无效）。详见下方"数学与查询"。
- 发射器（`sh_*` 族）两条最容易搞混的：**fan 以基准方向为中心对称展开**（改颗数不用重算
  `angle0`），而 **ring 下 `angle_step` 转义成逐层偏移**、不再是逐弹增量；以及
  **`sh_task` 是每颗弹派一个任务**——`sh_count(0, 28, 1)` + `sh_task` 一句话吃 28 个任务槽
  （池共 256），池满时**弹保留、任务丢**，静默无提示。详见下方"发射器"节。

## 一分钟样例

```ecl
const SPELL_WINDCHIME: int = 1;
const RICE: int = 64; // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）

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
            _ = batch(RICE, i % BULLET_COLOR_STRIDE,
                      $self_x, $self_y, ways, base, 0deg, 1,
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
`for i in a..b {}`（半开区间，`i` 为 `int`）· `break`/`continue` ·
`wait(n);`（n: int 帧，**低 16 位截断**，见下）·
`spawn f(args);` · `return;` · `wait_spell();`（符卡等待语法糖，纯前端展开为
`while spell_timer() >= 0 { wait(1); }`，见下"符卡"节）· 表达式语句（**值必须消费**——
有返回的内建不接收就 `_ = fire(...);` 显式丢弃，不丢弃 = 编译错误；这是"忘 POP 远处爆栈"
足枪的语言层灭除）。

> ⚠️ **`wait(n)` 的 `n` 被截成 `u16`（取低 16 位），静默、无诊断**：任务的等待计数器是
> `u16`，`OP_WAIT` 的实现就是 `task.wait = frames as u16`。所以
> **`wait(65536)` = 等 0 帧**（不是等 65536 帧）、**`wait(-1)` = 等 **65535** 帧**
> （不是"立即继续"），`wait(70000)` 悄悄变成 4464。语义有测试钉死
> （`ecl/vm.rs` 的 `wait_truncates_to_low_16_bits`），**不是 bug、不会改**。
> 一帧 1/60 秒，65535 帧 ≈ 18 分钟——正常关卡碰不到上限；要等更久就套循环
> （`for i in 0..10 { wait(30000); }`），别写一个大数上去。

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
- `fire(RICE, COLOR_RED, $self_x, $self_y, 0fx, 0deg, WIND_CHIME, trail_task)`
  同理——`trail_task` 作为
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
- **`const` 跨文件可见**（同一张扁平符号表的直接推论）：一个文件里 `const RICE: int = 64;`，
  别的文件直接写 `RICE` 就能用，**不需要重复声明、也不受文件先后序影响**（合并后单管线
  编译，不是逐文件独立编译再链接）。所以整局脚本的**共享词表**——弹型/色号、`globals`
  自由段槽号、符卡 id——就该单独摊一个 `00_defs.ecl` 放 `const`，别在每个文件里各抄一份
  （抄岔了是静默的：数值不同但都能编过）。`godot/ecl/demo/bullets.ecl` 就是这个用法。
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
| `GVAR_RANK` | `0` | `globals` 系统段内 RANK（难度）槽号，见上节 |
| `GLOBALS_SYS_SEGMENT` | `16` | `globals` 系统段/自由段分界槽号，见上节 |
| `REQ_*` | 见 `consts.rs` | 通道 B 引擎保留请求 id（`REQ_STAGE_CLEAR`/`REQ_BGM`/…） |
| `ITEM_POWER` / `ITEM_POINT` / `ITEM_LIFE_PIECE` / `ITEM_BOMB_PIECE` / `ITEM_STAR` | `0`/`1`/`2`/`3`/`4` | 道具类型号（编号**冻结**，非表驱动）。`drop_add(type, n)` 的第一参，见"敌人的三条死亡路径与掉落控制" |
| `BULLET_COLOR_STRIDE` | 内建 `16` | **表派生**：当前绑定表的每种弹型色数，见下 |

**弹型名与颜色名不是引擎常量**（旧的 `APPEARANCE_*` 已随颜色轴刀退场）——它们归**内容包**，
由你自己的 `.ecl` 用 `const` 声明（示例见 `godot/ecl/demo/bullets.ecl`）。同一编译单元
（= 同一目录）内 `const` 跨文件可见，所以整局脚本只需要在一个文件里声明一次。这样 mod
作者与内建内容地位对等。写"轮转全部颜色"用 `BULLET_COLOR_STRIDE`，别硬编码 16。
⚠️ 稀疏弹型（只做了部分色）的其余列是图集空格，盲目轮转全色会被编译期/运行期拒收。

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
- `fire(shape: int, color: int, x: fx, y: fx, speed: fx, angle: angle, xf: xform|none, task: sub|none) -> int` — 发一颗弹;shape/color 查外观表(越界/空格 编译期或 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1
- `batch(shape: int, color: int, x: fx, y: fx, n_angle: int, angle0: angle, angle_step: angle, n_speed: int, speed0: fx, speed_step: fx) -> int` — N-way 批量发环;shape/color 同 fire;返实际创建数
- `spawn_enemy(x: fx, y: fx, hp: int, drop_table: int, score: int, sprite: int, task: sub|none) -> int` — 造敌;判定 12/16 默认;task 为敌主任务 async sub 名或 none(owner=新敌;敌死任务亡,任务跑完敌也亡——静默退场,不掉道具不发死亡事件);返敌句柄,失败 -1
- `enemy_hp(handle: int) -> int` — 查敌当前 hp;死/悬垂/越界句柄返 -1(P4-b;句柄是池 index,槽复用不可辨)——stage 编排等 boss 死用
- `drop_item(x: fx, y: fx, item_type: int) -> int` — 掉一颗道具(带随机喷发速度,消耗模拟 RNG);返句柄,失败 -1
- `move_to(dur: int, x: fx, y: fx, easing: int)` — 敌自身(self owner 非 ENEMY → Fault)按 easing 缓动、dur 帧内平移到 (x,y);四参数皆真实压栈(不同于下方弹 setter 族的占位 handle 首参)
- `boss_set(slot: int, hp_ratio: fx, spell_id: int, timer_frames: int, phase_left: int, active: int)` — 整槽写 boss_ui 公告板(脚本写/UI 读);enemy 字段取自 self owner(非 ENEMY → NULL,不 Fault);符卡 active 期 enemy/spell_id/timer_frames/hp_ratio 由引擎逐帧自动覆写,phase_left 不受影响仍归脚本
- `pulse_signal(channel: int)` — 脉冲一条信号通道(边沿语义,仅当帧有效);放行处于弹变换 WAIT_SIGNAL 停驻态的弹(非 ECL 任务)
- `emit_req(id: int, a0: int|fx|angle, a1: int|fx|angle, a2: int|fx|angle, a3: int|fx|angle, a4: int|fx|angle, a5: int|fx|angle)` — 通道 B 渲染请求;void 只能裸语句;args 裸载荷(fx 过 raw/angle 过 BAM/int 原样)
- `rand(n: int) -> int` — 模拟 RNG 均匀 [0,n);确定性,随快照回卷
- `global(slot: int) -> int` — 读 globals 槽(GVAR_RANK=0 为难度)
- `set_global(slot: int, value: int)` — 写 globals 槽;系统段(slot<16)脚本写为 no-op+计数,不 Fault(GVAR_RANK=0 建议脚本只读)
- `aim_player() -> angle` — 自身(敌/弹属主)指向自机的 BAM 角
- `atan2(y: fx, x: fx) -> angle` — 任意向量的方向角(整数 CORDIC,16 轮);参数序 (y, x) 同 libm;(0,0) 返 0 不报错;比 aim_player 通用——能瞄任意点
- `dist(dx: fx, dy: fx) -> fx` — 向量 (dx,dy) 的模长(开根,不是平方);**不是两点距离**——两点距离自己减: dist(bx-ax, by-ay)
- `nearest_enemy(x: fx, y: fx) -> int` — 离 (x,y) 最近的活敌(非 dying;并列取低索引);无敌返 -1;返的是池 index,可直接喂 enemy_hp/enemy_x/enemy_y(悬垂/复用不可辨,同 enemy_hp)
- `enemy_x(handle: int) -> fx` — 按敌号读 x;死/悬垂/越界句柄返 0(**不是哨兵**——0 是合法坐标,先用 enemy_hp(e) != -1 探活再读)
- `enemy_y(handle: int) -> fx` — 按敌号读 y;死/悬垂/越界句柄返 0(同 enemy_x,先探活再读);配 enemy_x + atan2 即可朝任意敌开火
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
- `clear_bullets()` — 全场清弹:铺一个覆盖全场、存活 1 帧的消弹区(复用 FieldPool),每颗被消的弹原位转一颗星星(M0-15);不给护盾帧
- `add_lives(delta: int)` — 增减残机:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_
- `add_bombs(delta: int)` — 增减 bomb 数:delta 允许负,双边钳 [0,255] 不回绕;开局初值走 Loadout,故只有 add_ 没有 set_
- `add_power(delta: int)` — 增减火力:delta 允许负,双边钳 [0,POWER_MAX=400](即显示 4.00,不是 u16::MAX);开局初值走 Loadout
- `drop_clear()` — 清空自身待掉落计数;self 必须是敌
- `drop_add(type: int, n: int)` — 自身待掉落计数增量加 n 颗 type(只增不减,要清空用 drop_clear);计数上限 255 饱和
- `drop_items()` — 立刻撒出自身待掉落计数;**吐完不清空**(故 drop_items();die(); 掉双份);不加分不发死亡事件
- `die()` — 就地阵亡:掉落+加分+死亡事件+死亡特效,并**立即终止本任务**(后续语句不执行)
- `sh_reset(id: int)` — 重置发射器槽 id 为默认(1×1 单发、无 xform/挂弹任务/请求)
- `sh_sprite(id: int, shape: int, color: int)` — 设发射器的弹型与颜色;查外观表(越界/空格 编译期或 Fault)
- `sh_offset(id: int, x: fx, y: fx)` — 设出弹点**相对 owner** 的偏移;与 sh_offset_abs 写同一对字段,后写的赢(本条清绝对位标志)
- `sh_offset_abs(id: int, x: fx, y: fx)` — 设出弹点的**绝对**坐标(不跟随 owner);与 sh_offset 写同一对字段,后写的赢
- `sh_offset_rad(id: int, angle: angle, r: fx)` — 设出弹点的极坐标偏移;与 sh_offset/sh_offset_abs **永远叠加**,不是覆盖
- `sh_dist(id: int, d: fx)` — 出生后沿**各自角度**把弹推出去的距离(逐颗方向不同,不是整体平移)
- `sh_angle(id: int, angle0: angle, step: angle)` — 设基准角与逐弹角增量;开了 sh_aim 时 angle0 是相对自机方向的偏移,开了 sh_ring 时 step 转义成逐层偏移
- `sh_speed(id: int, speed0: fx, step: fx)` — 设基准速度与逐层速度增量(层数 = sh_count 的 n_speed)
- `sh_count(id: int, n_angle: int, n_speed: int)` — 设发弹阵列规模:角度向 n_angle 颗 × 速度向 n_speed 层;双边钳 [0,255] 不回绕
- `sh_aim(id: int, on: int)` — 开/关自机狙(on!=0 为开):开则 sh_angle 的 angle0 是相对自机方向的偏移,而非绝对方向
- `sh_ring(id: int, on: int)` — 开/关整周环(on!=0 为开):开则 n_angle 颗自动均分整周;关则是以基准方向为中心对称展开的 fan
- `sh_xform(id: int, xf: xform|none)` — 给发射器挂 xformdef(名或 none);开火时每颗弹都带上
- `sh_task(id: int, sub: sub|none)` — 给发射器挂弹任务 async sub(名或 none);开火时每颗弹都派一个,owner=该弹
- `sh_req(id: int, req_id: int)` — 设开火时顺带发的通道 B 请求 id(音效等);0 = 不发
- `sh_fire(id: int)` — 用发射器槽 id 的参数开火;无返回值;池满走 P4-a 计数
<!-- gen:builtins:end -->

> **弹 setter 族的 handle 参数是陷阱位**（`set_speed`/`set_angle`/`turn`/`set_vel`/
> `set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player` 九个）：首参
> `handle:int` **求值后即丢弃**，setter 恒作用于**当前任务的 owner 弹**（`self` 语义）——
> 不能借句柄定向操纵别的弹；owner 不是弹的任务调它 → 任务 Fault。想操纵 `fire(...)`
> 出来的那颗弹，用 xformdef 或 `fire` 的 `task` 参数挂子任务。

## 数学与查询（`atan2` / `dist` / `nearest_enemy` / `enemy_x` / `enemy_y`）

前三个是引擎里早就有、脚本此前够不着的东西（小清洗刀 2026-07-31 通电）；后两个是敌坐标
读口（敌坐标读口刀 2026-07-31），数据本就在敌池里躺着。五个都零新机制。

- **`atan2(y, x) -> angle`**——任意向量的方向角。**参数序是 `(y, x)`**（`y` 在前，同 libm
  惯例），两位都是 `fx`。这一位最容易写反：两参同型，写成 `atan2(dx, dy)` **不会有任何
  编译错误**，只会让角度沿 45° 对角线镜像。`(0, 0)` 返 `0deg`，不报错。
  与 `aim_player()` 的分工：`aim_player()` 只能瞄自机（0 参、基点是自己），`atan2` 能瞄
  任意点——瞄某只敌就是 `atan2(ey - $self_y, ex - $self_x)`。
- **`dist(dx, dy) -> fx`**——**向量的模长，不是两点距离**。要两点距离自己减：
  `dist(bx - ax, by - ay)`。开的是真根（`dist(3.0fx, 4.0fx)` 正好 `5.0fx`），不是平方距离。
  屏幕尺度上不会溢出（满屏对角线离上限还有两个数量级）；真喂进天文数字的 `fx` 时结果
  **饱和在最大可表示距离**，不会回绕成负数。
- **`nearest_enemy(x, y) -> int`**——离 `(x, y)` 最近的活敌，返**池 index**；场上无敌返 **-1**。
  候选是"存活且未在死亡态"的敌，并列时取低索引，无距离上限。
  返回值可以直接喂 `enemy_hp(h)` / `enemy_x(h)` / `enemy_y(h)`——四者是配对的
  （拿号 → 轮询血量 / 读坐标）。
  ⚠️ **返的是池 index，不带 generation**：那只敌死了、槽被新敌复用之后，你手上这个号会
  静默指向**新的那只**（和 `enemy_hp` 同一个已知口子）。别把它当长期句柄存着，每次要用
  就现查一次。
- **`enemy_x(handle) -> fx` / `enemy_y(handle) -> fx`**——按敌号读它的坐标。有了这两条，
  "查最近的敌 → 朝它开火"才接得通（下面的例子就是那条链路）。句柄口径同 `enemy_hp`：
  池 index、不比对 generation。

  ⚠️ **降级值是 `0`，不是哨兵**。`enemy_hp` 能用 `-1` 表示"这个号没用"，是因为血量天然
  非负；坐标没有这个便利——`-1` 是个完全合法的 `fx`，任何取值都可能是真坐标，**没有哨兵
  位可用**。所以死槽 / 越界 / 负句柄一律返 `0`，代价是**「那只敌恰好停在原点」与「这个号
  无效」读起来一模一样**。

  ⇒ **探活惯例：先 `enemy_hp(e) != -1` 探一下，再读坐标。**
  ⚠️ 别写成 `enemy_hp(e) >= 0`——那是错的。被打穿（overkill）的敌血量是**真实负值**，
  引擎只把它压到 `min(0)`、不抹平，所以"刚被打穿、槽还在场上"的敌 `enemy_hp` 返的是个
  负数；用 `>= 0` 探活会把它误判成无效句柄，而它其实还在屏幕上、还该被瞄。
  `!= -1` 是与引擎降级值直接对应的那个判据。（残余的一格缝：某只活敌血量**恰好**是 −1 时
  探活会误判——`-1` 同时是"无效"的返回值。真要一格不漏，配合 `nearest_enemy` 当帧现查的
  号用，那个号本来就是活敌。）

```ecl
// 关卡编排：等某个区域附近最后一只敌死掉再往下走
sub wait_area_cleared() {
    loop {
        var e: int = nearest_enemy(0.0fx, -96.0fx);
        if e < 0 || enemy_hp(e) <= 0 { return; }
        wait(4);
    }
}

// 查最近的敌 → 探活 → 读它的坐标 → 算方向 → 朝它开火（本节的招牌用法）
sub snipe_nearest() {
    loop {
        var e: int = nearest_enemy($self_x, $self_y);
        if e >= 0 && enemy_hp(e) != -1 {
            var dx: fx = enemy_x(e) - $self_x;
            var dy: fx = enemy_y(e) - $self_y;
            if dist(dx, dy) < 240.0fx {
                _ = fire(64, 2, $self_x, $self_y, 3.0fx, atan2(dy, dx), none, none);
            }
        }
        wait(20);
    }
}

sub main() {
    snipe_nearest();
}
```

## `spawn_enemy` 的 `task` 参与 `enemy_hp`（敌生成与轮询，A5 乙案）

`spawn_enemy` 第 7 参 `task` 与 `fire` 第 7 参**同构**：编译期解析的 async 无参 sub 名，或
字面量 `none`；非 `none` 时新敌的 owner 三元组落 `(ENEMY, 新敌 index, generation)`，随之
解锁 owner 门禁——`spell_begin`/`move_to`/`self_*` 系列只在这颗新敌自己的任务里才能过闸
（旧态下这些调用永远 Fault，纯 `.ecl` 摆不出 boss 正是这条门禁挡的）。`none` 时敌照常建成、
不派任务，`main_task` 保持 0。

**敌死任务亡**：owner-liveness gate（相位 2）在敌死后的下一相位清杀整棵 task 树。
**反过来也成立**——主任务跑完这只敌就退场，见下一小节；`stage` 侧仍应轮询 `enemy_hp` 而不是
去猜某个 sub 有没有退出。`enemy_hp(handle) -> int` 是 STAGE 层等 boss/敌死的标准写法：
死亡/悬垂/越界句柄统一返 `-1`（P4-b，槽复用后句柄不可辨，不区分"真死"与"槽已挪作他用"）：

```ecl
async sub boss_main() {
    wait(60);
}

sub boss_battle() {
    var boss: int = spawn_enemy(0.0fx, 96.0fx, 900, 1, 5000, 1, boss_main);
    var waiting: int = 1;
    while waiting == 1 {
        if enemy_hp(boss) < 0 { waiting = 0; }
        wait(10);
    }
}

sub main() {
    boss_battle();
}
```

（`boss_battle` 一段按 `godot/ecl/demo/boss_windchime.ecl` 原文精简改写——真实版本的
`boss_main` 跑非符+符卡两阶段、`boss_battle` 的等待循环带 75 秒挂死兜底，此处只留轮询
`enemy_hp` 这条主干；真实关卡编排务必带超时兜底，见该文件注释。上面这个精简版的
`boss_main` 只 `wait(60)` 就返回了，按下一小节的规则**这只 boss 会在 60 帧后自己退场**，
等待循环随之结束——它演示的是"怎么轮询"，不是"boss 该怎么写"。）

### ⚠️ 敌主任务跑完 = 这只敌退场（D9，写敌任务前先读这条）

`spawn_enemy` 的 `task` 参挂上去的那个 sub 是这只敌的**主任务**。**它一 `return`（或自然
跑到末尾），引擎立刻把这只敌标 `ENEMY_DYING`，相位 9 回收**——ZUN ECL 的"主协程返回即
自燃"语义（实现在 `ecl::vm::run_tasks` 的 `Exec::End` 分支）。所以：

- **要敌留在场上** → 主任务不能返回，末尾拿 `loop { wait(1); }` 挂住（或本来就是 `loop{}`
  编排）。**写完一段编排就 `return` = 这只敌当场消失**，这是最容易踩的一脚。
- **要敌退场** → 让主任务自然结束就行，不用把它移到越界线外骗回收。

退场是**静默的**：不掉道具、不加分、不发 `EVT_ENEMY_DIED`、不发死亡特效请求，**连 hp 都不
动**（满血退场就是满血）——脚本跑完是"退场"不是"被击破"。掉落与记分只属于**被击破**的那两
条路径：被自机打死，或脚本显式 `die()`（见下方"三条死亡路径对照"）。

**只有主任务有这个效果**。`spawn` 出来的伴生任务、`fire(..., task)` 挂在弹上的任务、
`spell_begin` 的 `pattern` 任务，结束了都只是它自己没了，跟敌的存亡无关（反过来敌一死，
owner 门禁会把整棵 task 树清杀）。主任务因 Fault 死也**不**自燃——那是报错路径，
已有 `EVT_TASK_FAULT`。

```ecl
const BALL: int = 48;
const COLOR_CYAN: int = 7;

// 飞进来 → 打一轮 → 飞出去 → sub 结束 = 这只敌自动退场（不需要飞到界外）
async sub zako_dive() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(BALL, COLOR_CYAN, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 500.0fx, 1);
    wait(150);            // 等 move_to 走完；不等的话敌会在半路上就消失
}

sub main() {
    _ = spawn_enemy(0.0fx, 2.0fx, 40, 1, 300, 0, zako_dive);
    wait(400);
}
```

（`wait(150)` 那条注释是重点：`move_to` 是引擎侧插值器，发起后立即返回；主任务不 `wait`
够帧数就走到末尾的话，敌会在缓动跑完之前退场。`godot/ecl/demo/stage1.ecl` 的杂兵是这段的
真实版本。）

**boss 血条手喂顺带一句**（`boss_set` 常见惯用法，C13②）：敌自身任务里手算
`boss_set(0, $self_hp as fx / $self_hp_max as fx, ...)`。`int as fx` 是 `×65536`
（见上"类型"节），走 `wrapping_mul`——**`$self_hp` 超过 32767 时这个乘法在 i32 里回绕**
（不 panic，静默产出一个无意义的 `fx`），血条手喂公式因此隐含"敌 hp ≤32767"这条前提；
真出现更高血量的敌，`hp_ratio` 换个不直接 cast 整段 hp 的算法（比如先各自钳到安全范围
再除）。

## 敌人的三条死亡路径与掉落控制（`drop_clear` / `drop_add` / `drop_items` / `die`）

四个内建都是 **`self` 作用**（与弹 setter 族同构）：它们操作的永远是**当前任务的 owner 敌**，
没有句柄参数，也就够不着别的敌。owner 不是敌的任务（关卡根脚本、`fire` 挂在弹上的任务）
调它们一律 **Fault(0)**、任务当场被杀——和 `move_to`/`spell_begin` 同一条门禁。

**`spell_begin` 的模式任务是敌 owner**（owner 就是宣言那只 boss，与 boss 主任务同一只敌），
所以这四个在模式任务里全都合法：`$self_x`/`$self_y`/`move_to` 能用，`die()` 也能用——
"符卡最后一发打完让 boss 就地阵亡"直接在模式任务末尾写 `die();` 即可。

### 待掉落计数是敌身上的可变状态

每只敌带一份**逐类型的待掉落计数**（五个类型各一个字节）。`spawn_enemy` 的 `drop_table`
参数只是**生成时的初值**：建敌那一刻把表展开进这份计数，此后**再没人读过表号**——同一张表
生出来的两只敌，可以被脚本各自改成完全不同的掉落。

- `drop_add(type, n)` —— **只增不减**。`type` 直接写引擎常量（`ITEM_POWER` / `ITEM_POINT` /
  `ITEM_LIFE_PIECE` / `ITEM_BOMB_PIECE`，编译器预置注入，不用自己 `const`；`ITEM_STAR`
  也是合法类型号，但星星按设计只由消弹转化产生，别拿它当掉落）。`n` 先钳进 `[0, 255]`
  （**负数视同 0**，这个内建不做"减掉落"）再累加，计数**封顶 255 饱和**、不回绕。
  坏 `type`（负数或 ≥ 类型数）走 P4-b 降级：整条调用 no-op + 违约计数，**不 Fault**，
  脚本继续往下跑——所以写错了不会响亮地炸，只会静默地不掉东西。
- `drop_clear()` —— 五个计数一次清零。要"这只敌什么都不掉"就在它死前调一次。
- 撒出去的**顺序恒按类型编号升序**，与 `drop_add` 的调用顺序、掉落表的书写顺序都无关。

### ⚠️ `drop_items()` 吐完**不清空**——`drop_items(); die();` 掉两份

**这是本节最容易踩的一脚。** `drop_items()` 把当前计数照单撒一遍（带随机喷发速度，
**消耗模拟 RNG**，同 `drop_item`），但撒完**不把计数清零**。于是：

- `drop_items()` 之后这只敌再死一次（被打死或 `die()`），同一批道具**再撒一遍**。
- 想只掉一份 → **别在 `die()` 之前调 `drop_items()`**；真要"先撒再死"，中间补一句
  `drop_clear()`。

保留这个字面语义是照 ZUN 的裁定，不是待修的 bug（引擎侧有测试钉死它）。另外
`drop_items()` **只撒道具**——不加分、不发死亡事件、不发死亡特效请求、不标死亡。

### 三条死亡路径对照

| 路径 | 掉落 | 加分（敌的 `score`） | 死亡事件 / 死亡特效请求 | 死后的 `$self_hp` | 触发方式 |
|---|---|---|---|---|---|
| 被自机打死 | ✔ | ✔ | ✔ | ≤ 0（打穿多少是多少，overkill 留负值） | hp ≤ 0（自机弹或消弹区伤害） |
| `die()` | ✔ | ✔ | ✔ | 强制 `min(0)`——满血 boss 也当场归 0 | 脚本显式调用 |
| 主任务跑完（D9） | ✘ | ✘ | ✘ | **完全不动**（满血就还是满血） | 主任务自然 `return` / 跑到末尾 |

前两行是**同一份引擎实现**，四件事一起发生，且**幂等**：已经在死的敌再被 `die()` 一次是
no-op，不会掉双份（`drop_items()` 那条坑不受此保护——它走的是另一条口子）。第三行是
"退场"不是"被击破"：静默消失，什么都不给（详见上一节 D9）。

**hp 那一列不是学究**：三条路径里只有 D9 让敌带着一身血退场，所以"轮询 `enemy_hp` 等 boss
死"这种编排**不能**靠 hp 判死（D9 退场时 hp 一点没动，可能还是满的；`enemy_hp` 只在槽被
**回收之后**才返 -1，那才是判据）。绑卡 boss 尤其要留意：`die()` 把 hp 压到 0（压过血线
下钳，见下方"符卡计器"节），D9 自燃则完全不碰 hp——两条都照样收卡结算，因为触发的是破卡
三路 OR 里的 `ENEMY_DYING` 那一路，不是 hp 那一路。

所以：

- 想让**自然退场也掉落** → 主任务 `return` 之前自己调一次 `drop_items()`（只补掉落；
  仍然不加分、不发死亡事件——要那些就得用 `die()`）。
- 想让敌**就地阵亡** → `die()`。跑完整死亡效果，然后**立即终止本任务**：`die()`
  后面的语句一句都不执行。

```ecl
const BALL: int = 48;
const COLOR_CYAN: int = 7;

// ① 自然退场也想掉落：return 之前自己撒一次
async sub zako_leaves_gift() {
    for i in 0..3 {
        _ = fire(BALL, COLOR_CYAN, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    drop_items();                    // 不撒的话这只敌白死（D9 退场是静默的）
}

// ② 定时自爆：改掉掉落清单，然后就地阵亡
async sub bomber() {
    wait(180);
    drop_clear();                    // 丢掉 spawn_enemy 那份初值
    drop_add(ITEM_POWER, 4);
    drop_add(ITEM_LIFE_PIECE, 1);
    die();                           // 掉落 + 加分 + 死亡事件/特效，本任务到此为止
    drop_items();                    // 永远不执行（die() 已终止本任务）
}

sub main() {
    _ = spawn_enemy(-60.0fx, 2.0fx, 40, 1, 300, 0, zako_leaves_gift);
    _ = spawn_enemy(60.0fx, 2.0fx, 900, 1, 5000, 1, bomber);
    wait(400);
}
```

### ⚠️ 死了的敌**当帧仍参与碰撞**，仍能撞死自机

三条路径都**只标记不回收**——槽要活到相位 8 供表现层读，相位 9 的 cleanup 才收尸。
而体碰检测**不查死亡标记**：这只敌在被回收之前照常算数，自机撞上去照样中弹。

被打死的敌也是这样（死在相位 7，相位 6 已经碰过了），只是 `die()` 的窗口更长：ECL 任务跑在
**相位 2**，远早于碰撞（相位 6），于是"一只已经宣告死亡的敌在同一帧里撞死了自机"这种场面
在 `die()` 路径上明显得多。别把 `die()` 当"立刻从场上消失"使——它是"阵亡"，不是"消失"。

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

- **⚠️ `die()` 压过血线下钳**——`hp_threshold` 的下钳只管**伤害**这条路径（自机弹/消弹区
  打不穿血线）；脚本显式 `die()` 走的是另一条路，它把 hp 直接 `min(0)`、**不看血线**，
  绑卡 boss 当场阵亡并收卡结算。这是有意的（显式动作压过为伤害设的保护），不是漏判。
  所以"非最终卡的 boss 中途自爆"是写得出来的：在模式任务里调 `die()` 即可——但也意味着
  一句手滑的 `die()` 会把整套多卡序直接掐断，不会被血线兜住。
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
- **符卡练习（select 语义）不需要任何新引擎面**：练习是"只打一张，打完即散"而非"跳过前面
  接着打"，靠 `mark` 垫片在 boss 主控 sub 开头写选卡分派即可——`mark(N) { set_global(槽,
  值); }` 落进卡序中间，boss 主控读该 globals 槽非零就走单卡分支（`spell_begin`+
  `wait_spell`+`return`），恒 0 走整战；选卡号随 `start`（`new_game_at` 的中段启动参数）
  携带，正常整局流程被跳过零污染。槽号须落**自由段（≥16）**，系统段脚本写保护会挡。

字节码层完整规则（syscall 号、越界/坏参数处置、事件/请求 id）见
[`ecl-ops.md`](ecl-ops.md)"符卡计器"节；世界侧机构设计（结算矩阵、伤害下钳、逐卡血条公式）
见 `docs/superpowers/specs/2026-07-24-spell-meter-design.md`。

## 场面与账面写操作（`clear_bullets` / `add_lives` / `add_bombs` / `add_power`）

签名见上方生成段；这里是签名说不出来的部分。

### `clear_bullets()` —— 全场清弹

清的是**敌弹**（自机弹、道具、敌人都不受影响）。实现上不是"遍历弹池挨个删"，而是铺一个
覆盖全场、`life=1` 的消弹作用区（复用 `FieldPool`，同符卡结算清弹走的是同一个构造口），
由此带来三条要记住的性质：

- **当帧生效**。`life=1` 的作用区恰好活一帧且当帧参与碰撞（相位 5 减到 0、相位 6 照常判定、
  相位 9 才回收），所以调用它的那一帧弹就被消掉了，不用等下一帧。
- **每颗被消的弹在原位转一颗星星**（M0-15），不是白清——星星 30 分一颗、会被磁吸，是一次
  经济回流。清一屏弹会一次性掉出一屏星星，别在"想安静地把场面抹掉"的地方随手调它。
  星星池满时逐颗降级计数，不 Fault。
- **不给无敌帧**。它只消弹，不碰自机状态——刚清完弹下一帧照样能被新弹打死。想要"清弹 +
  保命"的是 bomb（消弹区 + 伤害位 + 自机无敌），那是另一刀的事，别拿 `clear_bullets()` 当
  bomb 用。

典型用法是关底转场：`clear_bullets();` 把残留弹幕抹掉，再 `add_score(bonus);` 记完账，
最后 `emit_req(REQ_STAGE_CLEAR, …);` 挂牌（见下方"关卡结算转场协议"）。

作用区池（cap 16）满时走 `create_field` 自身的降级——这一帧的清弹**静默失效**（计
`diag.pool_full[POOL_FIELD]`），不 Fault、不报错。正常脚本碰不到，写"每帧清弹"这种就会。

### `add_lives(d)` / `add_bombs(d)` / `add_power(d)` —— 账面增减

三个都是**增量**，`d` 允许负值（`add_lives(-1)` 就是扣一条命），结果**双边钳位**：

| builtin | 钳到 |
|---|---|
| `add_lives` | `[0, 255]` |
| `add_bombs` | `[0, 255]` |
| `add_power` | `[0, 400]` |

**钳位是正常语义，不是错误**——不计 `contract_viol`、不 Fault、不回绕（同 `add_score` 的
饱和口径）。`add_lives(999)` 就是"加到上限为止"，0 命时再 `add_lives(-1)` 停在 0 而不会绕成
255。极端 `delta`（`i32::MIN`/`i32::MAX`）也安全。

`power` 的单位是**厘火力**：`100` = 显示的 `1.00`，上限 `400` = `4.00`。想"加满火力"写
`add_power(400)`，不是 `add_power(4)`。上限是 400 而不是 `u16` 上界，因为再往上
`power_tier` 档位索引就越界了。

**没有 `set_lives`/`set_bombs`/`set_power`，这是刻意的，不是漏了。** 绝对赋值只有开局装备
这一个确定场景，而它已经被菜单侧的 `Loadout`（`new_game_at` 的装备参）收编了；运行期脚本
要的都是"奖命 +1 / 中弹 −1"这类记账。多一条绝对写路径就多一处跟 `Loadout` 抢开局初值的
歧义，所以别去"补全"成四件套。

三个都只作用于自机 0（当前单人），owner 类别无限制——`stage` 任务、敌任务、弹任务都能调。

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

const BALL: int = 48; // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const COLOR_CYAN: int = 7;

sub main() {
    _ = fire(BALL, COLOR_CYAN, 0fx, 0fx, 1.0fx, 0deg, ARC_SHOT, none);
}
```

- op 名 = [`xform-ops.md`](xform-ops.md) 小写助记（`turn`/`set_speed`/`set_ang_vel`/
  `step_speed`…）；`@N` 前缀 = 该槽 wait N 帧；参数必须**编译期常量**（字面量/const/一元负号）。
- **STEP 族（`step_speed`/`step_angle`）物理占 2 槽**——scratch 由编译器自动补，作者按 1 条写；
  物理槽总数 ≤16。`loop`/`end` 不开放（复杂控制流写任务弹；尾部零填充天然 END）。
- 被 `fire(..., NAME, ...)` 引用才占 locals 空间（3 字/物理槽，算进引用它的 sub 的容量账）。

### 部分设三兄弟：`set_sprite` / `set_shape` / `set_color`

外观值 = `形 × color_stride + 色`（identity：表索引 ≡ 图集格号 ≡ 池 `sprite` 值，见
[`render-contract.md`](render-contract.md) §3）。三个 xform op 都改弹当前的外观值，区别
在改哪一维：

- `set_sprite(形, 色)`——**全设**，两维一起换（`fire`/`batch` 内部折叠出的 op 就是它）。
- `set_shape(形)`——**只换形状**，保住当前色位不动。
- `set_color(色)`——**只换颜色**，保住当前形位不动。

```ecl
xformdef SWAP_LOOK {
    set_color(COLOR_BLUE);        // 保形：不管当前是什么形，只把颜色换成蓝
    @10 set_shape(BALL); // 保色：不管当前是什么色，只把形状换成大玉
}

const OUTLINE: int = 32; // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const BALL: int = 48;
const COLOR_BLUE: int = 8;

sub main() {
    _ = fire(OUTLINE, COLOR_BLUE, 0fx, 0fx, 1.0fx, 0deg, SWAP_LOOK, none);
    wait(60);
}
```

⚠️ **部分设不查空格——这是设计允许的行为，不是漏洞，复审别把它当 bug 修回去**：
`set_shape`/`set_color` 编译期只查"值本身合不合法"（色号落在 `[0, BULLET_COLOR_STRIDE)`、
形状基址是 stride 的整倍数且落在表范围内），**不查"这个形+色组合在图集里是不是空格"**。
原因是部分设只改一维，落点还取决于弹**当时的另一维**——这是运行期状态（可能来自 `fire`
给的初始外观，也可能来自之前执行过的另一次部分设），编译期看不到那个值，做不了跨维校验。
曾提议一条"跨形状安全"判据（`set_color(c)` 要求 `c` 在图集里所有弹型上都有图）被**人类
裁定否决**：图集里只要存在一两个稀疏弹型（某行缺几个色），这条判据就会把那几号色在
**所有**弹型上一起禁掉，代价远大于收益。（当前内建图集 12 行全满 16 色、没有空格，
但这条裁定是针对机制的，不随某一版美术变化。）**结论**：落到空格 = 该弹变透明，这是设计允许的降级路径，
由作者自己负责别把部分设用在会撞空的组合上；运行期也**不**替你兜底——部分设的两个解释臂
只护 stride 合法性（防除零/溢出），不查 `valid`，撞空格既不报错也不 Fault，弹会悄悄变
透明地继续飞。想要"越界就出错"的效果，只有 `fire`/`batch`/`set_sprite` 的两参全设才有
这道闸（`set_sprite` 编译期走同一份 `check_shape_color`，含 `valid` 检查——见
`set_sprite_blank_atlas_cell_is_compile_error` 单测）。

⚠️ **稀疏弹型不能盲目轮转全色**：只做了部分色的弹型（如上面的心弹/蝶弹），
`for i in 0..BULLET_COLOR_STRIDE { ... }` 这类轮转写法在色号跑到空格区间时，`fire`/
`batch` 会在编译期/运行期被拒收（两参全设查 `valid`）；换成部分设则不会报错，只会让弹
在那几帧变透明。两种后果都不是作者通常想要的——轮转全色的写法只对满色弹型安全，稀疏
弹型要么显式列出可用色，要么整体避开轮转写法。

## 发射器（`sh_*` 族）——配一遍，开多次火

`fire`/`batch` 是"一句话说完全部参数"，参数一多就写成一行几十个逗号；`sh_*` 族是另一条路：
**先把一组发射参数存进槽里，再按需要反复开火**（参照 ZUN ECL 的 `et*` 族）。改一个字段
再开一次火，就是下一波。

**槽是每任务私有的四个，编号 `0..=3`**（写 `4` 或负数 = 整条调用 no-op + 违约计数，不
Fault，也就是**静默不生效**）。四个槽互不干扰，够一只 boss 同时挂"主环 / 点射 / 收尾"再
留一格。任务槽被复用时四个 shooter 一律抹回默认，**不会继承上一个任务的残留**；但同一个
任务里跨帧是留着的——这正是"配一遍、开多次火"能成立的原因。子任务不继承父任务的 shooter，
各自从默认开始。

默认值是 **1 角 × 1 层的单发**（不是"什么都不发"），其余字段全零 / 无 xform / 无挂弹任务 /
不发请求。`sh_reset(id)` 把槽抹回这个默认——**换一段弹幕前先 reset**，否则会继承上一段
设过的 `sh_aim`/`sh_ring`/`sh_dist` 这些位，表现为"莫名其妙多了个偏移"。

```ecl
const RICE: int = 64;      // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const COLOR_RED: int = 0;

xformdef SLOW_DOWN { @30 set_speed(0.6fx); }

async sub windchime() {
    var base: angle = 0deg;
    sh_reset(0);                     // 先把 0 号槽抹回默认，别继承上一段的残留
    sh_sprite(0, RICE, COLOR_RED);
    sh_ring(0, 1);                   // 整周环
    sh_count(0, 28, 3);              // 28 颗 × 3 层
    sh_speed(0, 1.2fx, 0.35fx);      // 层速 1.20 / 1.55 / 1.90
    sh_xform(0, SLOW_DOWN);
    sh_req(0, REQ_SCRIPT_BASE);      // 开火时顺带发一条通道 B 请求（音效）
    loop {
        sh_angle(0, base, 3deg);     // 只改这一句，就是下一波
        sh_fire(0);
        base = base + 7deg;
        wait(50);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 900, 1, 5000, 1, windchime);
    wait(600);
}
```

发出来的是一张 **`n_angle` × `n_speed` 的网格**（`sh_count` 的两个数）：角度方向 `n_angle`
颗、速度方向 `n_speed` 层，逐层速度 `speed0 + j × speed_step`（`sh_speed`）。发弹顺序是
**角度外层、速度内层**，与 `batch` 同序。

### fan 与 ring 是两种排布，`angle_step` 的含义跟着变（最容易搞混的一处）

`sh_ring(id, 0)`（默认）是 **fan**：`n_angle` 颗按 `angle_step` 逐弹排开，**以基准方向为
中心对称展开**。第 i 颗的角度是 `base + i×step − (n−1)×step/2`。

```
        n=5, step=8deg              n=4, step=8deg
             ↑ base                      ↑ base            ← base 落在中间两颗之间
      ＼  ＼  |  ／  ／            ＼  ＼ | ／  ／
       ＼  ＼ | ／  ／              ＼  ＼|／  ／
        -16 -8 0 +8 +16              -12 -4  +4 +12
      （奇数路：正中一颗正对 base）  （偶数路：base 在正中的缝里）
```

**推论：改颗数不用重算 `angle0`。** `angle0` 恒是"扇形的中轴"，`sh_count` 从 3 路改到 7 路，
扇形只是变宽，中轴不动——所以下面这种写法是对的，不需要每次自己算 `−(n−1)·step/2`：

```ecl
const NEEDLE: int = 16;   // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const COLOR_WHITE: int = 4;

async sub aimed_fan() {
    sh_reset(1);
    sh_sprite(1, NEEDLE, COLOR_WHITE);
    sh_aim(1, 1);                  // 开自机狙：angle0 从此是"相对自机方向的偏移"
    sh_angle(1, 0deg, 8deg);       // 0deg = 正打；8deg = 相邻两路的夹角
    sh_speed(1, 2.0fx, 0fx);
    loop {
        for ways in 3..8 {
            sh_count(1, ways, 1);  // 3→7 路轮着来，angle0 一次都不用重算
            sh_fire(1);
            wait(20);
        }
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 400, 1, 2000, 1, aimed_fan);
    wait(600);
}
```

`sh_ring(id, 1)` 是 **ring**：`n_angle` 颗**自动均分整周**，`angle_step` **不再是逐弹增量**，
转义成**逐层**偏移（第 j 层整层多转 `j × angle_step`）。第 i 颗第 j 层的角度是
`base + (i × 65536)/n + j × step`。

```
      n=6 的 ring（step 与颗间距无关）        两层、step = 半个间隔
            ·                                    ·  ∘  ·  ∘  ·
        ·       ·                              ∘             ∘
            ✳            间距恒 = 360°/n          ✳              · = 第 0 层
        ·       ·                              ∘             ∘   ∘ = 第 1 层
            ·                                    ·  ∘  ·  ∘  ·
```

均分是**逐颗算 `(i × 65536)/n`**（不是"预乘一个整数步长"），余数被均摊掉，所以环**精确
闭合**——最后一颗与第一颗的间隔和别处一样（差 ≤1 BAM 单位）。`n` 不整除 65536 时也不会
攒出一条肉眼可见的缝。

**惯用法：两层错开半个间隔**——ring 下 `angle_step` 就是干这个的，写 `32768 / n`
（半个间隔 = 半个 `65536/n`）：

```ecl
const RICE: int = 64;     // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const COLOR_BLUE: int = 8;

async sub two_layer_ring() {
    var n: int = 24;
    sh_reset(2);
    sh_sprite(2, RICE, COLOR_BLUE);
    sh_ring(2, 1);
    sh_count(2, n, 2);                       // n 颗均分整周 × 2 层
    sh_speed(2, 1.0fx, 0.6fx);
    sh_angle(2, 0deg, (32768 / n) as angle); // 第 2 层错开半个间隔
    loop {
        sh_fire(2);
        wait(40);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 400, 1, 2000, 1, two_layer_ring);
    wait(600);
}
```

（`(32768 / n) as angle` 里的 `as angle` 是**位穿透** cast，不是"转成度"——见上文"类型"节。
这里要的正是位穿透：`32768/n` 算出来的就是 BAM 原值。）

### `sh_aim` 下 `angle0` 是**偏移**而不是方向

`sh_aim(id, 1)` 之后，`sh_angle` 的 `angle0` 不再是绝对方向，而是**叠在"正对自机"那个方向
上的偏移**：

- `sh_angle(id, 0deg, ...)` = **正打**（打在自机身上）。
- `sh_angle(id, 15deg, ...)` = 从正对自机的方向再拧 15°（BAM 增大的一侧 = 屏幕上顺时针，
  见 [`render-contract.md`](render-contract.md) §2 的朝向约定）。

自机方向是**开火那一刻**才解析的（不是设 `sh_angle` 那一刻），所以"配一遍、循环里反复
`sh_fire`"每一发都跟着自机走，不会锁死在配置时的角度上。基点是**出弹点**（含各种偏移之后
的那个点），不是 owner 的位置——`sh_offset_abs` 把出弹点挪到别处时，瞄的是从**那个点**看
自机的方向。

### 四条坑

**① 直角偏移与极坐标偏移是相加，不是覆盖；但 `sh_offset` 会清掉 `sh_offset_abs` 的位。**
出弹点 = `基点 + (off_x, off_y) + 极坐标偏移`。`sh_offset` / `sh_offset_abs` 写的是**同一对**
`off_x/off_y`（后写的赢），区别只在基点：前者相对 owner（并**清掉**绝对位），后者绝对
（基点固定为世界原点）。`sh_offset_rad` 写的是**另一对**字段、**永远叠加**上去，且**不碰**
那个绝对位。所以 `sh_offset_abs(0, 300fx, 0fx); sh_offset(0, 10fx, 0fx);` 的净效果是
"相对 owner 偏 10"——绝对模式被第二句关掉了，这是有意设计（两条互为反向），不是 bug。

**② `dist` 是逐颗沿各自角度推，不是整环平移。** `sh_dist(id, d)` 让每颗弹出生时沿**它自己
那颗的角度**推 `d`——一个 ring 配上 `dist` 是"半径 d 的圆环出生"，不是"整个环朝某个方向
挪了 d"。想要后者请用 `sh_offset`。

**③ 挂弹任务很吃任务槽——这是 shooter 新引入的压力面。** `sh_task(id, sub)` 是"**每颗**弹
派一个任务"，所以 `sh_count(0, 28, 1)` + `sh_task` = **一句 `sh_fire` 吃掉 28 个任务槽**
（池共 256 个）。池满走 P4-a 降级：**弹保留、任务丢**，不报错、不 Fault，表现为"一环里有
几颗静默地没有该有的行为"——很难 debug，因为画面上弹都在。

> 今天的 `batch` 没有 `task` 参数，想给一环弹逐颗挂任务只能写 `for` 循环逐颗 `fire`，写的
> 时候自然会掂量颗数；`sh_task` 让它变成一句话。**给多颗弹挂任务前先算一下 `n_angle ×
> n_speed × 同时在场的波数` 会不会顶到 256。**弹的自主行为能用 `sh_xform`（xformdef，
> 零任务槽）表达的就别用 `sh_task`。

**④ `sh_xform` 同样吃池——吃的是 xform 段池，账和 `batch` 一模一样。** 上一条说"能用
`sh_xform` 表达的就别用 `sh_task`"，但 `sh_xform` 不是免费的：配了它以后**每颗弹都要一份
自己的段拷贝**，于是 `sh_xform` + `sh_count(0, 28, 5)` = **一句 `sh_fire` 吃 140 个段**
（段池共 **2048**）。这与 [`xform-ops.md`](xform-ops.md) 给 `batch` 的段消耗警告是逐字
同一件事——只不过 `batch` 是"一句话传全部参数"，颗数就写在眼前那一行；shooter 把
`sh_count` 和 `sh_fire` 拆到了两处，循环里那句 `sh_fire(0)` 看上去人畜无害。

> 段满的表现和任务满**不一样**：任务满是"弹在、行为没了"，段满是**这颗弹压根没建出来**
> ——从满的那一颗起本次开火的剩余部分整个短路（同弹池满），计在 `pool_full[XFORM]`。
> 多波同时在场时按 `n_angle × n_speed × 同时在场的波数` 估段，和估任务槽是同一笔账，
> 只是分母换成 2048。

### 什么会 Fault、什么只是静默降级

写 `.ecl` 时值得记住的分界（完整口径见 [`ecl-ops.md`](ecl-ops.md) 62-76 号表）：

- **静默降级（no-op + 违约计数，任务继续跑）**：槽号 `id` 越界；`n_angle` 或 `n_speed` 为
  0，或两者之积超过弹池容量（整条 `sh_fire` 一颗不发）。**这几种最难查**——脚本照跑、
  画面上什么都没有。顺带一提 `sh_count` 的两个数各自**先钳进 `[0,255]`**（不回绕），所以
  写 `sh_count(0, 300, 1)` 得到的是 255 路而不是报错。
- **Fault（任务当场被杀，发 `EVT_TASK_FAULT`）**：`sh_fire` 时发现 appearance 越界或落在
  图集空格；`sh_xform` 的区间越界；`sh_task` 的 sub 号不在册 / 不是零参 `async sub`。
  这些都在**开火那一刻**才查——setter 只写字段、不校验，所以错误的行列会指到 `sh_fire`
  那一行，不是设错的那一行。
- **弹池满**：从满的那一颗起**停止本次开火的剩余部分**（同 `batch` 的短路），已发的留着。
- **xform 段池满**（只在配了 `sh_xform` 时可能）：**和弹池满同样短路**，已发的留着，不
  Fault。这是**另一个池、另一个计数器**（`pool_full[XFORM]`），别把它和弹池满混作一谈——
  段池只有 **2048** 个，而配了 `sh_xform` 的一句 `sh_fire` 一次就吃掉 `n_angle × n_speed`
  个（账见上面坑④）。

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
  时间标签 `+N:` 未进 v1（显式 `wait`）。
  > 曾列在这里的"**跨 `.ecl` 文件无共享的自定义常量机制**"**已作废**（2026-07-31 实测
  > 否定）：多文件是"合并后单管线编译"，`const` 天然跨文件可见，见上文"多文件"节。
