# 6 · 符卡与整局编排

> 这一篇把单只敌扩成一场戏：符卡机构、`mark` 中段启动、多文件整局布局、引擎全局状态
> （难度/boss_ui/信号）、账面增减、以及推给表现层的渲染请求。读之前先读
> [3 · 敌人](3-enemy.md) 和 [4 · 弹](4-bullets.md)——符卡就是"一只 boss + 一段弹幕 +
> 引擎替你记账"。

## 一整张符卡长什么样

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
    sh_reset(0);
    sh_ring(0, 1);                   // 整周环：路数交给引擎均分，不手算步长
    var base: angle = 0deg;
    loop {
        var ways: int = 28 + global(GVAR_RANK) * 2;   // 路数随难度走
        sh_count(0, ways, 1);
        sh_angle(0, base, 0deg);
        for i in 0..5 {              // 五环逐环换色换速（颜色是发射器级的，塌不进 n_speed）
            sh_sprite(0, RICE, i % BULLET_COLOR_STRIDE);
            sh_speed(0, 1.0fx + i as fx * 0.25fx, 0fx);
            sh_fire(0);
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

符卡的记账——计时、衰减、超时、破卡、UI 喂送——全归引擎机构，脚本只管宣言 + 弹幕行为 +
收尾等待。细节见下一节。

## 符卡（`spell_begin` / `spell_end` / `spell_timer` / `wait_spell`）

一张符卡的记账全归引擎机构（`SpellState`，settle 相位符卡趟）：计时递减、bonus 衰减、资格
判定、破卡自动检测、超时判定、`boss_ui[]` 自动喂送。脚本只管**宣言 + 弹幕行为 + 收尾等待**，
两行范式：

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

参数表（声明序）：

| 位 | 类型 | 含义 |
|---|---|---|
| `slot` | `int` | 符卡槽号（`0..MAX_BOSSES`，v1 每 boss 一份） |
| `id` | `int` | 卡 id——纯脚本词汇，引擎不登记，见下 |
| `pattern` | sub 名 \| `none` | 模式 sub 引用（同 `fire` 的 `task` 参同款 `SubRef`）：必须是**无参** `async sub`；`none` 不 spawn（自定义卡留口，配合 `spell_end()` 逃生舱口自行判定结束） |
| `time_limit` | `int` | 时限，单位帧，必须 `>0` |
| `bonus0` | `int` | 起始 bonus（分），必须 `≥0`；衰减地板 = `bonus0/10`，衰减速率 = `(bonus0-地板)/time_limit`，均在 `spell_begin` 当帧一次算定 |
| `flags` | `int` | 位标志：bit0 `SPELL_SURVIVAL`（耐久卡——活到超时即收卡点，而非失败）；bit1 `SPELL_NO_CLEAR`（结束时不自动铺全屏消弹 field） |
| `hp_threshold` | `int` | 破卡血线，必须 `≥0` 且 `≤` 当前 owner 血量；绑定敌 hp 触底/低于此值时自动收卡结算，且伤害结算对绑定敌**下钳**在此值（防打穿到非最终卡血线以下）——多卡序用"一池总血 + 逐卡递降血线"表达，最终卡 `hp_threshold=0` |

三条原语加一条糖：

- `spell_timer() -> int` 读 owner 当前绑定槽的剩余帧数 `frames_left`。owner 没有绑定槽
  （还没 `spell_begin`，或卡已经结束）恒返回 `-1`。
- `wait_spell()` 是语法糖，编译期展开为 `while spell_timer() >= 0 { wait(1); }`。纯前端展开，
  不新增字节码语义，和你手写这行 `while` 编译出逐字节相同的 `EclImage`。需要自定义等待逻辑时
  直接手写 `spell_timer()` 循环即可。
- `spell_end()`（无参、无返回值）是逃生舱口，给非 HP/超时的自定义结束条件用，比如剧情触发
  提前收卡。owner 有绑定槽时走 HP 路径结算：资格在 → CAPTURED 付 `bonus_now`，资格失 →
  FAILED。无绑定槽调用是 no-op，重复调用安全，不算违约。

⚠️ **`wait_spell` 在语句位置（`wait_spell(`）总是被语法糖截胡**，即使你恰好声明了同名 sub 也
调不到它。它不是真正的词法关键字（词法层没有为它开专属 token），但对作者而言效果等同保留字。

⚠️ **`die()` 压过血线下钳。** `hp_threshold` 的下钳只管伤害这条路径，自机弹和消弹区打不穿
血线；脚本显式 `die()` 走的是另一条路，把 hp 直接 `min(0)`、不看血线，绑卡 boss 当场阵亡并
收卡结算。这是有意的——显式动作压过为伤害设的保护，不是漏判。所以"非最终卡的 boss 中途自爆"
是写得出来的，在模式任务里调 `die()` 即可；但也意味着一句手滑的 `die()` 会把整套多卡序直接
掐断，不会被血线兜住。

**模式随卡生死**是 `pattern` 参数的核心承诺。`spell_begin` spawn 出的模式任务绑定本卡槽，它
自己 `spawn` 出的整棵子任务树继承同一绑定；收卡结算的瞬间，这整棵树下一帧起自动终止（相位 2
调度门禁杀，脚本不必写 `kill_children`）。反例：`fire(...)` 挂在弹上的任务不继承这条规则，
弹命归弹，残留弹演完自己剧本，是 ZUN 语义。唯一后果：模式 sub 可以放心写 `loop { ... }` 死
循环，不用自己判断"卡是不是已经结束"，引擎替你收尾。

**多卡序**就是主控 sub 里顺序执行下一组 `spell_begin`/`wait_spell`，两行一卡，前一张
`wait_spell()` 返回后紧跟下一张 `spell_begin`。不需要任何引擎侧"下一张卡"排程。

**卡 id** 纯粹是脚本/关卡资产，引擎不做任何登记，不校验唯一、不映射名字或立绘。每份 `.ecl`
建议用 `const` 命名，如 `const SPELL_WINDCHIME: int = 1;`，同文件内每张卡起一个数字即可。
`const` 跨文件可见，但引擎侧对 id 零协调，撞号不会有任何报错，纯靠作者自律对齐。

<details><summary>为什么不做自动切卡，以及符卡练习（select 语义）怎么用 mark 实现</summary>

引擎不提供、也不打算提供自动切卡机制（`spell_bound` 只管单卡模式树的生死，不管卡与卡之间
怎么编排，见世界侧机构 spec 的"不做什么"节）。

**符卡练习（select 语义）不需要任何新引擎面**：练习是"只打一张，打完即散"而非"跳过前面
接着打"，靠 `mark` 垫片在 boss 主控 sub 开头写选卡分派即可——`mark(N) { set_global(槽,
值); }` 落进卡序中间，boss 主控读该 globals 槽非零就走单卡分支（`spell_begin`+
`wait_spell`+`return`），恒 0 走整战；选卡号随 `start`（`new_game_at` 的中段启动参数）
携带，正常整局流程被跳过零污染。槽号须落**自由段（≥16）**，系统段脚本写保护会挡。

</details>

字节码层完整规则（syscall 号、越界/坏参数处置、事件/请求 id）见
[`ecl-ops.md`](../ecl-ops.md)"符卡计器"节；世界侧机构设计（结算矩阵、伤害下钳、逐卡血条公式）
见 `docs/superpowers/specs/2026-07-24-spell-meter-design.md`。

## mark（中段启动标记）

`mark` 给整局脚本开"练习 / 中段启动"的落点。语义是**规范态开局**（符卡练习那味儿），不是
"仿佛打过来的状态"：跳进来的世界不重放被跳过的帧，靠脚本自己声明"这里该长成什么样"。

两种写法：`mark(<id>);` 是纯落点；`mark(<id>) { … }` 带一段"跳入时才执行"的补偿块。

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

写 `mark` 的四条硬约束：

- 只能在 `sub main` 的**顶层语句位**——不进 `if`/`while`/`for`/`loop` 块，也不能出现在别的
  sub 里。
- 编号是 **int 型编译期常量**，必须是正整数，且全镜像唯一（跨文件合并后仍在同一份名字空间
  里查重）。
- main 顶层的 `var` 不得先于任何 `mark`。任务帧局部是零初始化的，跳入 mark 落点时局部区
  还没被跑到。
- 跳进来的世界里，`mark` 之前正常流程本该写过的全局变量和局部变量**全是初始零值**，需要的
  值在补偿块里手写补。

引擎会替你补的只有背景锚点，而且**只认顶层线性位的常量参声明**：

⚠️ `if` / 难度分支里、`spawn` / `fire` 挂出的异步任务里写的 `bgm`/`bg`/`bg_phase` 一律扫不到
（扫描不下潜控制流、不跟异步边）。中段启动时只会拿到"扫描能看见的那条"最近声明；这类分支或
异步声明得靠 `mark` 块里手写覆盖。

<details><summary>landing pad 的降低形式、自动补偿的完整扫描规则、上例的两条路径逐句对照</summary>

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

</details>

<details><summary>宿主侧怎么调：new_game_at 的 start 与 loadout 参数</summary>

**`new_game_at(seed, rank, start, loadout)`**：`start=0` 等价于从头开局
（`new_game(seed, rank, image)` 就是 `new_game_at(seed, rank, 0, Loadout::default(), image)`
的委托）；`start=<mark id>` 直接把根任务 `pc` 定到该 `mark` 的落点（`EclImage::resolve_mark`
查表）。`start` 若未在镜像标记表命中（含负值——标记表 id 恒正，天然不命中）是**宿主期
响亮错**（`TaskStartError::UnknownMark`），发生在 `World` 被分配之前，不会返回一个半初始化
的世界。`loadout`（`character`/`power`/`lives`/`bombs` 四个标量）与 `start` 相互独立、
同一次调用一起给：`power`/`lives`/`bombs` 越界直接钳位（P4-b），`character` 越
`WorldTables::characters` 表界是另一条宿主期响亮错（`TaskStartError::InvalidCharacter`）——
装备是"玩家在菜单调好的数据"，从建世界的门直接进，不走脚本。

</details>

## 多文件（整局脚本布局）

一局完整的游戏可能是好几十个符卡/关卡拼起来的，巨型单文件 `.ecl` 不好维护。表层语言把
"一局一镜像"这个约束（一局 = 一个 `EclImage` = 一个 `content_hash`，回放/握手身份的一部分）
和"源码摊几个文件"这件事解耦：

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

- **目录 = 编译单元集**。`stg-harness check <目录>` 和引擎宿主的加载入口都一样：收到目录路径
  就收集该目录下全部 `*.ecl`、按文件名字节序排序后逐个读入，作为一批编译单元合并编译。单文件
  路径照旧当成单元素单元集，行为与只有单文件时完全一致。零配置格式，没有 manifest，文件名
  怎么排全靠文件名本身的字节序。
- **扁平命名空间。** 所有文件共享同一个全局符号表，`sub`/`const`/`xformdef` 的名字不分文件。
  跨文件重名在编译期报错，错误信息带两处位置（本次撞上的文件:行 + 另一处定义所在的文件:行）。
  多文件不是模块系统，不能靠文件名做命名空间隔离——两个文件各写一个同名 `sub helper()` 就是
  重复定义。
- **`const` 跨文件可见。** 一个文件里 `const RICE: int = 64;`，别的文件直接写 `RICE` 就能用，
  不需要重复声明，也不受文件先后序影响。所以整局脚本的共享词表（弹型/色号、`globals` 自由段
  槽号、符卡 id）就该单独摊一个 `00_defs.ecl` 放 `const`，别在每个文件里各抄一份——抄岔了是
  静默的，数值不同但都能编过。`godot/ecl/demo/bullets.ecl` 就是这个用法。
- **收集顺序不影响产物字节。** 不管传入的文件先后序是 A→B 还是 B→A，合并后的 `EclImage`
  逐位相同。

<details><summary>编译管线怎么合并多文件，以及明确不做 include / 模块系统的理由</summary>

编译期先各自独立 `lex`/`parse`：每个文件的语法错误各自带该文件的文件名 + 局部行号，不是
拼接后的全局行号。预检阶段同时收集全部顶层名字（`sub`/`const`/`xformdef`）做跨文件撞名
检查；预检通过后合并 AST 走已有 `typeck`/`slots`/`codegen` 单管线（这几个阶段本就不知道
"文件"这个概念，产物类型检查/字节码生成侧零改动）。`const` 跨文件可见正是"合并后单管线
编译，不是逐文件独立编译再链接"的直接推论。

产物字节与收集顺序无关，也是因为 codegen 本就按 sub 名排序出 canonical id，与源码收集
顺序无关；目录收集仍然固定按文件名字节序，只是"结果不随之改变"，不是"顺序随意写"。

明确不做的两件事：`include` 语法（会逼编译器做路径解析，破坏"编译器是纯函数、文件收集归
调用方"这条断层线纪律）、模块系统/命名空间（扁平全局名字空间 + 撞名报错已经够用）。

</details>

## 引擎提供的全局状态：`globals` / `boss_ui` / `signals`

三套独立的"状态通道"，语义不同，别混用：

| 通道 | 范围 | 脚本读 | 脚本写 | 内容 / 语义 |
|---|---|:---:|:---:|---|
| `globals` 系统段 | `[0, 16)` | ✓ | ✗（no-op + `contract_viol` 计数，不 Fault） | **目前仅槽 0 有意义**：`GVAR_RANK`（难度档，**值域 `0..=4`**，game 层开机时经世界 API 写入，脚本只读后自决）；槽 1-15 保留未用 |
| `globals` 自由段 | `[16, 1024)` | ✓ | ✓ | 脚本自定义草稿区，语义靠作者自己约定；`n` 是任意运行期表达式（不限编译期常量，可以是循环变量） |
| `boss_ui[]` | 每 boss 一份 | ✗（无读 syscall） | ✓（`boss_set`） | 血条/spell/计时状态，写给表现层 UI 消费，脚本读不回自己刚写的值；**符卡 active 期间** `enemy`/`spell_id`/`timer_frames`/`active`/`hp_ratio` 由符卡机构逐帧自动覆写（见「符卡」节），脚本的 `boss_set` 此时只对 `phase_left`（阶段号）全权——非符卡段（卡与卡之间）`boss_set` 照旧全权写全部字段 |
| `signals[8]` | 8 通道 | — | — | 不是存值用的：`pulse_signal(ch)` 发边沿脉冲，`wait_signal` xform op 在变换序列里等；只唤醒当帧已在等待的弹，不锁存 |

`globals` 读写走 `global(n)` / `set_global(n, v)`。难度就用 `global(GVAR_RANK)` 读，**别自己
另起变量镜像它**，也别自己给系统段槽号起名——引擎已经注入了具名常量，见 [7 · 速查](7-reference.md)「引擎常量」节。

**难度档的值域是 `0..=4`**，五个档位各有注入的具名常量，数值顺序即难度序，所以 `>=` 比较是
正规写法：

```ecl
sub main() {
    // RANK_EASY(0) / RANK_NORMAL(1) / RANK_HARD(2) / RANK_LUNATIC(3) / RANK_EXTRA(4)
    var ways: int = 8;
    if global(GVAR_RANK) >= RANK_HARD { ways = 16; }
    set_global(20, ways);
    loop { wait(1); }
}
```

档位是**四档**（Easy/Normal/Hard/Lunatic）——本引擎不做 ZUN 那套"连续 rank + 档位并存"的
双轨，`GVAR_RANK` 里就只会是这几个整数。`RANK_EXTRA`(4) 是**预留位、不是第五档难度**：
Extra 关在现代作品里是独立关卡走自己的脚本，通常不靠 rank 分支；留这个号是以防将来有共享
sub 需要判它。开机时 `rank` 越 `0..=4` 一律被拒（宿主 `new_game_at` 返 `RankOutOfRange`，
不钳位不开局），所以脚本可以放心假定读到的值落在域内。

自由段（`[16, 1024)`，即 `[GLOBALS_SYS_SEGMENT, GLOBALS_CAP)`）没有引擎预置名字，配 `const`
给自己用到的槽号起名避免魔数（`const MY_SLOT: int = 16;`）。整局的槽号词表摊在一个文件里，
`const` 跨文件可见（见「多文件」节）；但**引擎侧对槽号零协调**——两处脚本选中同一个槽存不同
东西不会有任何编译或运行期报错，纯靠作者自律对齐。

字节码层完整规则（segment 边界钉法、校验和归属等）见 [`ecl-ops.md`](../ecl-ops.md)。

<details><summary>boss 血条手喂的隐含前提：敌 hp ≤ 32767</summary>

`boss_set` 常见惯用法（C13②）是在敌自身任务里手算
`boss_set(0, $self_hp as fx / $self_hp_max as fx, ...)`。`int as fx` 是 `×65536`
（见 [5 · 三型、字面量与语句](5-types.md)），走 `wrapping_mul`——**`$self_hp` 超过 32767 时这个乘法在 i32 里回绕**
（不 panic，静默产出一个无意义的 `fx`），血条手喂公式因此隐含"敌 hp ≤32767"这条前提；
真出现更高血量的敌，`hp_ratio` 换个不直接 cast 整段 hp 的算法（比如先各自钳到安全范围
再除）。

</details>

## 场面与账面写操作

签名见 [7 · 速查](7-reference.md) 的生成段；这里是签名说不出来的部分。

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
最后 `emit_req(REQ_STAGE_CLEAR, …);` 挂牌（见「关卡结算转场协议」）。

作用区池（cap 16）满时走 `create_field` 自身的降级——这一帧的清弹**静默失效**（计
`diag.pool_full[POOL_FIELD]`），不 Fault、不报错。正常脚本碰不到，写"每帧清弹"这种就会。

### `add_lives` / `add_bombs` / `add_power` / `add_score` —— 账面增减

四个都是**增量**，`delta` 允许负值（`add_lives(-1)` 就是扣一条命），结果**双边钳位**或饱和：

| builtin | 钳到 |
|---|---|
| `add_lives` | `[0, 255]` |
| `add_bombs` | `[0, 255]` |
| `add_power` | `[0, 400]` |
| `add_score` | `[0, u64::MAX]` |

**钳位是正常语义，不是错误**——不计 `contract_viol`、不 Fault、不回绕。`add_lives(999)` 就是
"加到上限为止"，0 命时再 `add_lives(-1)` 停在 0 而不会绕成 255；`add_score` 扣穿只会停在 0，
不会像有符号回绕那样绕成一个巨大正数。极端 `delta`（`i32::MIN`/`i32::MAX`）也安全。

`power` 的单位是**厘火力**：`100` = 显示的 `1.00`，上限 `400` = `4.00`。想"加满火力"写
`add_power(400)`，不是 `add_power(4)`。上限是 400 而不是 `u16` 上界，因为再往上
`power_tier` 档位索引就越界了。

`add_lives`/`add_bombs`/`add_power` 三个都只作用于自机 0（当前单人），owner 类别无限制——
`stage` 任务、敌任务、弹任务都能调。**没有 `set_lives`/`set_bombs`/`set_power`**，开局装备
走菜单侧的 `Loadout`。

<details><summary>为什么没有 set_lives / set_bombs / set_power</summary>

**这是刻意的，不是漏了。** 绝对赋值只有开局装备这一个确定场景，而它已经被菜单侧的
`Loadout`（`new_game_at` 的装备参）收编了；运行期脚本要的都是"奖命 +1 / 中弹 −1"这类记账。
多一条绝对写路径就多一处跟 `Loadout` 抢开局初值的歧义，所以别去"补全"成四件套。

</details>

## 渲染请求（通道 B）

`emit_req(id:int, a0,a1,a2,a3,a4,a5: raw)` —— 向表现层推送一次性演出请求（爆炸/音效/宣言/
震屏）。固定 7 参，不足位补 `0`；六个载荷位是 **raw 参数**：接受 `int`/`fx`/`angle` 任意型
表达式**按位原样**传出（`1.5fx` → Q16.16 raw = 98304、`90deg` → BAM raw = 16384、int 原样），
表现层按 id 约定解码。无返回值（只能做语句）；缓冲满确定性丢弃（不 Fault）；无 owner 类别
限制——关卡任务也能发。

id 命名空间：`0` 保留无效 · `1..=63` 引擎保留（如 `REQ_ENEMY_DEATH`）· `64+` 脚本自由——
建议 `const MY_REQ: int = REQ_SCRIPT_BASE + n;` 起名。引擎 id `1..=3` 的逐位 args 约定表见
`stg-core/src/reqs.rs` 模块文档（编码律：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw）；
`4..=7`（同一编码律）见下表：

| id | args[0] | args[1..] |
|---|---|---|
| `REQ_STAGE_CLEAR`（4） | 脚本自定（挂牌用；见下方转场协议） | 0 |
| `REQ_BGM`（5） | `id`（int，同写入的 `bgm_id`） | 0 |
| `REQ_BG`（6） | `id`（int，同写入的 `bg_id`） | 0 |
| `REQ_BG_PHASE`（7） | `phase`（int，同写入的 `bg_phase`） | 0 |

`REQ_BGM`/`REQ_BG`/`REQ_BG_PHASE` 由 `bgm`/`bg`/`bg_phase` 三个 builtin 内部经对应的 5xx
syscall 自动发出——脚本不需要、也不应该自己再手写一次 `emit_req` 发这三个 id。
`REQ_STAGE_CLEAR` 没有专属 syscall/builtin，是纯粹的挂牌协议常量：脚本用通用的
`emit_req(REQ_STAGE_CLEAR, ...)` 自己发。

<details><summary>关卡结算转场协议（宿主怎么接这块挂牌）</summary>

spec §6 摘编；纯宿主约定，引擎侧零改动——`World` 是纯被动状态机，宿主不调 `step` 就是完美
冻结。

1. 脚本关底先把账在世界内记完（`add_score(bonus);`），再 `emit_req(REQ_STAGE_CLEAR, …);`
   挂牌，随后**直接续行**（比如接着调用 `stage2();`）——世界对"暂停"这件事零感知。
2. 宿主每帧 `step` 后经 `take_requests()` 看见 `REQ_STAGE_CLEAR` 挂牌，就此**停手不再
   `step`**，用读口数据画结算/菜单页（原生 UI，`World` 冻结不动，不进这条时间线）。
3. 玩家确认后宿主恢复 `step`——世界里 `stage2` 的第一帧才真正发生。回放文件里没有"结算页"
   这个概念（暂停期贡献零帧），重播时直接穿过，行为与真实机台一致。

</details>

---

**下一篇** → [7 · 速查](7-reference.md)：内建函数签名、`$` 引擎变量、引擎常量。
