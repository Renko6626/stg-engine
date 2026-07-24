# ECL 表层语言参考（作者第一入口）

> **这是什么**：`.ecl` 脚本作者手册——语法、类型、内建函数、`$` 引擎变量、xformdef、错误格式。
> **权威来源**（冲突时以它们为准）：`crates/stg-ecl-compiler/src/lang/`（`builtins.rs` 内建表 /
> `xform_map.rs` xform 操作表）· spec `docs/superpowers/specs/2026-07-18-m19-ecl-language.md`。
> 字节码层参考（op/syscall/fault 码）见 [`ecl-ops.md`](ecl-ops.md)——作者通常不需要看它。
> 编译时机：启动时从源码文本编译（`lang::compile`），编译器确定性有测试押运。

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
- `fire(1, $self_x, $self_y, 0fx, 0deg, WIND_CHIME, trail_task)` 同理——`trail_task` 作为
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

## 内建函数（签名以 `builtins.rs` 为准）

`fire(appearance:int, x:fx, y:fx, speed:fx, angle:angle, xf:XFORMDEF名|none, task:ASYNC_SUB名|none) -> int` ·
`batch(appearance, x, y, n_angle:int, angle0:angle, angle_step:angle, n_speed:int, speed0:fx, speed_step:fx) -> int` ·
`spawn_enemy(x,y,hp,drop_table,score) -> int` · `drop_item(x,y,ty) -> int` ·
`move_to(dur:int,x:fx,y:fx,easing:int)` · `boss_set(slot,ratio:fx,spell,timer,phase,active)` ·
`pulse_signal(ch)` · `emit_req(id:int, a0..a5:raw)`（通道 B 渲染请求，见下节）·
`rand(n:int) -> int` · `global(n) -> int` · `set_global(n,v)`
（槽 0-15 系统段脚本只读）· `aim_player() -> angle` · `sin/cos(a:angle) -> fx` · 弹 setter 族 ·
`spell_begin(slot,id,pattern:SUB名|none,time_limit,bonus0,flags,hp_threshold)` ·
`spell_end()` · `spell_timer() -> int`（符卡计器三连，详见下节"符卡"）。

> **弹 setter 的 handle 参数是陷阱位**:首参 `handle:int` **求值后即丢弃**,setter 恒作用于
> **当前任务的 owner 弹**(`self` 语义)——不能借句柄定向操纵别的弹;owner 不是弹的任务调它
> → 任务 Fault。想操纵 `fire(...)` 出来的那颗弹,用 xformdef 或 `fire` 的 `task` 参数挂子任务。

## 符卡（`spell_begin` / `spell_end` / `spell_timer` / `wait_spell`）

一张符卡的记账（计时递减、bonus 衰减、资格判定、破卡自动检测、超时判定、`boss_ui[]` 自动
喂送）全归引擎机构（`SpellState`，settle 相位符卡趟）；脚本只管**宣言 + 弹幕行为 + 收尾
等待**——两行范式：

```ecl
spell_begin(slot, id, pattern, time_limit, bonus0, flags, hp_threshold);
wait_spell();
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
  写 `wait_spell();` 只是省一行样板，语义上和手写等价 `while` 完全没有区别。
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
建议 `const MY_REQ: int = REQ_SCRIPT_BASE + n;` 起名。引擎 id 的逐位 args 约定表见
`stg-core/src/reqs.rs` 模块文档（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）。

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
  **跨 `.ecl` 文件无共享的自定义常量机制**（脚本各自的 `const`/自由段槽号约定不互通，纯靠
  作者自律对齐；引擎侧命名常量——appearance id/`GVAR_RANK` 等——已由预置注入解决，见上文
  "引擎常量"节与 `follow-ups.md` C14）· 时间标签 `+N:` 未进 v1（显式 `wait`）。
