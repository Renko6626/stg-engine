# 4 · 弹

> 这一篇讲弹怎么出来、出来之后怎么自己变化：三条发弹口（发射器 / `fire` / `batch`）的取舍、
> 弹自己的 setter 族、`xformdef` 变换序列。读之前先读
> [3 · 敌人](3-enemy.md)——发弹的通常是敌，出弹点和瞄准都建立在敌的位置上。

## 先决定用哪条路

引擎有三条发弹口。**默认选发射器**（`sh_*` 族）——第 1 篇发那个环用的就是它：

| 场合 | 用什么 | 为什么 |
|---|---|---|
| 反复开火、同一套参数（boss 非符连发 30 次） | **发射器** | 配一遍，此后每波一句 `sh_fire` |
| 整周环（任何路数） | **发射器**（`sh_ring`） | 引擎替你均分，不掉余数（见下一小节） |
| 真·一次性单发一颗 | `fire` | 发射器在这里是净亏：几句配置换一句调用 |
| 一次性的角度 × 速度网格 | `batch` 更短 | **但角度算术归你** |

诚实的账：拿发射器重写过整个 demo 局，**代码反而变长了**（+11 / +4 非注释行）。配一个发射器
要五六句，这本钱得靠"配一遍、开很多次"赚回来——非符段一次配置开火 30 次赚了，风铃卡每环都要
改弹型和速度没赚到，杂兵的 1×1 单发是净亏。所以正确的说法不是"`batch` 不好"，是：
**默认发射器；`fire` 留给一次性单发；`batch` 能用，但角度你自己负责。**

### `batch` 赢的那个场合：一次性爆散

路数写死、只开一次火——这时候 `batch` 一句就是一张网格，别的路都比它长：

```ecl
const RICE: int = 64;      // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const COLOR_RED: int = 0;

// 杂兵冲进来，撂一发爆散，走人
async sub zako_burst() {
    move_to(60, $self_x, 160.0fx, 2);
    wait(60);
    _ = batch(RICE, COLOR_RED, $self_x, $self_y,
              16, 0deg, 4096 as angle,     // 16 路整周：65536 / 16 = 4096
              3, 1.2fx, 0.4fx);            // 3 层速度：1.20 / 1.60 / 2.00
    wait(30);
    move_to(90, $self_x, 500.0fx, 1);
    wait(90);
}

sub main() {
    _ = spawn_enemy(0.0fx, 40.0fx, 60, 1, 300, 0, zako_burst);
    wait(400);
}
```

这里 `batch` 是对的，而且是明显对的：

- **路数是编译期常量** ⇒ `65536 / 16 = 4096` 手算一次就定死。下一节那笔余数账在这里不成立，
  不是因为躲过了，是因为**你不会再去改它**——"改了颗数忘记重算步长"这个风险源根本不存在。
- **只开一次火** ⇒ 发射器那四五句配置一次都摊不掉，配置句数比开火句数还多，纯亏。
- 一句话给出 16 × 3 的网格：`fire` 要写两层循环，发射器要写一段配置，`batch` 什么都不用写。

金向量场景 `crates/stg-harness/scenes/rainbow.ecl` 用的也是 `batch`——它在真实内容里活得
好好的，**看到 `batch` 不等于看到 bug**。（那一句的路数跟着 rank 走，所以它也带着下一节那个
缺口；不过 `rainbow.ecl` 是**诊断**场景，活儿是把形轴/色轴/速度轴和 xformdef 引用一次压满好做
逐帧对拍，环合不拢不影响这份工作。）

### 手算整周环会掉余数——一个真发生过的例子

`godot/ecl/game/boss_windchime.ecl` 的风铃卡原本手算 `astep = 65536 / ways`，而路数随难度走：
`ways = 28 + global(GVAR_RANK) * 2`。整数除法一取整，`ways` 颗弹就铺不满一整圈：

| 难度 | `ways` | `65536 / ways` | `ways × step` | 缺口 |
|---|---:|---:|---:|---:|
| Easy | 28 | 2340 | 65520 | **16 BAM** |
| Normal | 30 | 2184 | 65520 | **16 BAM** |
| Hard | 32 | 2048 | 65536 | 0（恰好整除）|
| Lunatic | 34 | 1927 | 65518 | **18 BAM** |

**四档里三档的环合不拢**，只有 Hard 躲过。缺口 16~18 BAM ≈ 0.1°，肉眼几乎看不出来，于是这个
bug 进了已发布内容，一直活到有人拿发射器重写它才被发现。`sh_ring` 不是这么算的：它逐颗算
`(i × 65536)/n`、把余数均摊掉，首尾精确闭合，相邻两颗的间隔极差 ≤1 BAM。

一句话：**手写 `batch` 发环，就是在手算一个会掉余数的步长。**

## `fire` 与 `batch`——两条直发口

签名见 [7 · 速查](7-reference.md)。两条的分工：

- `fire(shape, color, x, y, speed, angle, xf, task)` 发一颗，返回弹句柄，失败返 `-1`。
  `xf` 位收 `xformdef` 名或 `none`，`task` 位收 `async sub` 名或 `none`（两个位都是编译期
  解析的标识符，不是求值表达式，见 [2 · 任务与时间](2-tasks.md)）。
- `batch(shape, color, x, y, n_angle, angle0, angle_step, n_speed, speed0, speed_step)`
  一句话发一张 `n_angle × n_speed` 的网格，返回实际创建数。发弹顺序是角度外层、速度内层。

`batch` 没有 `task` 位：要给一环弹逐颗挂任务只能自己写 `for` 循环逐颗 `fire`，或者用下面的
发射器族（`sh_task`）。给 `batch` 挂变换用发射器的 `sh_xform`，或者逐颗 `fire` 时传 `xf`
——`batch` 的段消耗账见 [`xform-ops.md`](../xform-ops.md)。

⚠️ **`batch` 的 `angle_step` 是逐弹增量，不会替你均分整周。** 第 i 颗的角度是
`angle0 + i × angle_step`，写 `0deg` 就是 n 颗叠在同一个方向上。要整周均分自己算
`65536 / n`（BAM 一圈 65536）再 `as angle` 位穿透过去，同 `crates/stg-harness/scenes/rainbow.ecl`
的写法——**掉不掉余数你自己盯**（上一节那张表）。会自动均分的是发射器的 `sh_ring`
（见下方「发射器」节），两者别记混。

```ecl
const RICE: int = 64;      // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const COLOR_RED: int = 0;

async sub two_ways() {
    loop {
        // 一颗：朝自机。一次性单发，用 fire 最短
        _ = fire(RICE, COLOR_RED, $self_x, $self_y, 2.0fx, aim_player(), none, none);
        wait(30);
        // 一圈：12 颗均分整周 × 2 层速度。均分要自己算步长（见上面那条警告）
        var step: int = 65536 / 12;
        _ = batch(RICE, COLOR_RED, $self_x, $self_y,
                  12, 0deg, step as angle, 2, 1.5fx, 0.5fx);
        wait(60);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 400, 1, 2000, 1, two_ways);
    wait(600);
}
```

（顺手验一下上一节那笔账：12 并不整除 65536，`step = 5461`，`12 × 5461 = 65532`，这个"12 颗
均分整周"其实差着 4 BAM。真要严丝合缝的整周，走 `sh_ring`。）

## 发射器（`sh_*` 族）——配一遍，开多次火

`fire`/`batch` 是"一句话说完全部参数"，参数一多就写成一行几十个逗号。`sh_*` 族是另一条路：
先把一组发射参数存进槽里，再按需要反复开火（参照 ZUN ECL 的 `et*` 族）。改一个字段再开一次
火，就是下一波。

槽是每任务私有的四个，编号 `0 ..= SHOOTERS_PER_TASK - 1`（`SHOOTERS_PER_TASK` 是引擎注入的
常量，见 [7 · 速查](7-reference.md)「引擎常量」节，现值 `4`——别再手写 `0..=3`）。写
`SHOOTERS_PER_TASK` 或负数 = 整条调用 no-op + 违约计数，不 Fault，
也就是静默不生效。四个槽互不干扰，够一只 boss 同时挂"主环 / 点射 / 收尾"再留一格。任务槽被
复用时四个 shooter 一律抹回默认，不会继承上一个任务的残留；但同一个任务里跨帧是留着的，这
正是"配一遍、开多次火"能成立的原因。子任务也不继承父任务的 shooter，各自从默认开始。

默认值是 1 角 × 1 层的单发，不是"什么都不发"；其余字段全零、无 xform、无挂弹任务、不发请求。
`sh_reset(id)` 把槽抹回这个默认。**换一段弹幕前先 reset**，否则会继承上一段设过的
`sh_aim`/`sh_ring`/`sh_dist` 这些位，表现为"莫名其妙多了个偏移"。

```ecl
const RICE: int = 64;      // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const COLOR_RED: int = 0;

xformdef SLOW_DOWN { @30 set_speed(0.6fx); }   // 变换序列，见下面「xformdef」节

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

七句配置全在 `loop` **外面**，循环体里只剩"改一句 + 开火"——这才是发射器赚钱的地方。那个
28 路的环用 `batch` 写还得自己算 `65536 / 28`，掉 16 BAM（上面那张表的 Easy 那一行）。

发出来的是一张 `n_angle` × `n_speed` 的网格（`sh_count` 的两个数）：角度方向 `n_angle` 颗、
速度方向 `n_speed` 层，逐层速度 `speed0 + j × speed_step`（`sh_speed`）。发弹顺序是角度外层、
速度内层，与 `batch` 同序。

⚠️ **颜色是发射器级的，不逐层。** `sh_sprite` 一次设整个槽的弹型与色号，`n_speed` 那一维只
分速度、不分色。所以"五环逐环换色"塌不成一次 `sh_count(0, n, 5)` + `sh_speed`，只能保留外层
循环、每轮改 `sh_sprite` 再 `sh_fire`（`boss_windchime.ecl` 里就是这么写的，注释也钉在那）。

### fan 与 ring 是两种排布，`angle_step` 的含义跟着变

这是最容易搞混的一处。`sh_ring(id, 0)`（默认）是 **fan**：`n_angle` 颗按 `angle_step` 逐弹
排开，以基准方向为中心对称展开。第 i 颗的角度是 `base + i×step − (n−1)×step/2`。

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
const NEEDLE: int = 16;   // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
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

`sh_ring(id, 1)` 是 **ring**：`n_angle` 颗自动均分整周，`angle_step` **不再是逐弹增量**，
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

均分是逐颗算 `(i × 65536)/n`，不是"预乘一个整数步长"，余数被均摊掉，所以环精确闭合：最后
一颗与第一颗的间隔和别处一样（差 ≤1 BAM 单位）。`n` 不整除 65536 时也不会攒出一条肉眼可见
的缝。**这正是上面那张 rank 表里手算版做不到的事。**

惯用法是两层错开半个间隔——ring 下 `angle_step` 就是干这个的，写 `32768 / n`
（半个间隔 = 半个 `65536/n`）：

```ecl
const RICE: int = 64;     // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
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

（`(32768 / n) as angle` 里的 `as angle` 是位穿透 cast，不是"转成度"，见 [5 · 三型、字面量与语句](5-types.md)。这里要
的正是位穿透：`32768/n` 算出来的就是 BAM 原值。逐层偏移这一位没有"引擎替你均分"的待遇，
它本来就是你想错开多少就错开多少。）

### `sh_aim` 下 `angle0` 是偏移而不是方向

`sh_aim(id, 1)` 之后，`sh_angle` 的 `angle0` 不再是绝对方向，而是叠在"正对自机"那个方向上的
偏移：`sh_angle(id, 0deg, ...)` 是正打，`sh_angle(id, 15deg, ...)` 是从正对自机的方向再拧
15°（BAM 增大的一侧 = 屏幕上顺时针，见 [`render-contract.md`](../render-contract.md) §2 的朝向
约定）。

**自机方向是 `sh_fire` 那一刻才解析的，不是 `sh_aim` / `sh_angle` 配置那一刻。** 这条值得
明说，因为它正是发射器能成立的前提：如果瞄准角在配置时就定死，"配一遍、循环里反复
`sh_fire`"发出去的三十波会全部瞄向三十波之前自机站的那个位置，这一族就没法用了。实际行为
是每一发都现查自机，跟着它走。

基点是出弹点（含各种偏移之后的那个点），不是 owner 的位置：`sh_offset_abs` 把出弹点挪到
别处时，瞄的是从那个点看自机的方向。

⚠️ **`sh_aim` 从出弹口瞄，`aim_player()` 从 owner 中心瞄——设了 `sh_offset` 时两者不等价。**

```text
sh_offset(0, 30fx, 0fx);
sh_aim(0, 1);                       // 从「中心 + 30」朝自机
   vs
sh_angle(0, aim_player(), 0deg);    // 从「中心」朝自机
```

偏差随偏移量与距离变化，**离自机越近、偏移越大，差得越明显**。严格说 `sh_aim` 更正确——弹是
从枪口出去的，就该从枪口瞄。会踩到这条的是"想自己算瞄准环、所以拿 `aim_player()` 喂
`sh_angle`"、同时又设了 `sh_offset` 的写法：得到的是一个微妙偏斜、且越近越明显的自机狙，
画面上几乎看不出来。

### 四条瞄准路径的解析时机与基点

| 路径 | 什么时候解析 | 从哪个点算 | 瞄谁 | 一个可瞄的都没有时 |
|---|---|---|---|---|
| `aim_player()` | 调用那一刻 | owner 自己的位置（敌 → 敌中心，弹 → 弹自己，无 owner 的关卡脚本 → 世界原点） | 最近的**可瞄**自机 | 回退 1P 的最后坐标 |
| `sh_aim` + `sh_fire` | **`sh_fire` 那一刻** | **出弹口** = owner 位置 + `sh_offset` + 极坐标偏移 | 最近的**可瞄**自机 | 回退 1P 的最后坐标 |
| xformdef 里的 `aim_player` op | 变换游标走到那条 op 的那一帧（可以在飞行途中） | 弹自己**当时**的位置 | 最近的**可瞄**自机 | 整条 **no-op**（弹保持原角度） |
| 弹 setter `aim_at_player(h, off)` | 调用那一刻 | 弹自己的位置 | 最近的**可瞄**自机 | 整条 **no-op**（弹保持原角度） |

四条**全都是"事情发生的那一刻"才解析**，没有一条在声明时定死；**"瞄谁"四条也一律相同**
——从各自的基点看过去、最近的那个**可瞄**自机（缺席或已 game over 的跳过）。剩下两处差别
是有理由的，自己写瞄准逻辑时值得知道：

- **基点**：前两条从射手算，后两条从弹自己算（弹已经飞出去了，从它当前位置瞄才对）。
- **一个可瞄的都没有时**：分野不在"哪条路"，在**这条路能不能拒绝**。`aim_player()` 是查询、
  `sh_fire` 要发弹，两者**必须产出一个角度**，于是回退到 1P 的最后坐标（自机 game over 时
  坐标冻在死亡那一刻）；后两条是**改一颗已经存在的弹**，可以什么都不做，于是保持原角度、
  不报错也不计数。本仓 `MAX_PLAYERS = 2`，所以这一格在单人局里只在"自机已 game over"时
  看得出来。

> ⚠️ **`$player_x` / `$player_y` 不在这张表里**——它们是**坐标读**，恒给 **1P**（`players[0]`）
> 的坐标，不查存活、也不跟着"最近可瞄自机"走。拿它自己算瞄准角（`atan2($player_y - $self_y,
> $player_x - $self_x)`）在单人局与 `aim_player()` 等价，将来有 co-op 就不等价了。要瞄准就用
> `aim_player()`。

（另一处对照：拿 `fire` 写一个三叉自机狙要写三行 `aim_player() ± 12deg`，改成五叉就得重算
中心角；fan + `sh_aim` 的 `sh_count(0, 3, 1)` 改成 `5` 就完事，两者逐位等价。）

### 四条坑

**① 直角偏移与极坐标偏移是相加，不是覆盖；但 `sh_offset` 会清掉 `sh_offset_abs` 的位。**
出弹点 = `基点 + (off_x, off_y) + 极坐标偏移`。`sh_offset` 与 `sh_offset_abs` 写的是同一对
`off_x/off_y`，后写的赢，区别只在基点：前者相对 owner 并清掉绝对位，后者绝对（基点固定为
世界原点）。`sh_offset_rad` 写的是另一对字段，永远叠加上去，且不碰那个绝对位。所以
`sh_offset_abs(0, 300fx, 0fx); sh_offset(0, 10fx, 0fx);` 的净效果是"相对 owner 偏 10"：绝对
模式被第二句关掉了。这是有意设计（两条互为反向），不是 bug。

**② `sh_dist` 是逐颗沿各自角度推，不是整环平移。** 每颗弹出生时沿它自己那颗的角度推 `d`，
所以一个 ring 配上 `dist` 是"半径 d 的圆环出生"，不是"整个环朝某个方向挪了 d"。想要后者请用
`sh_offset`。

**③ 挂弹任务很吃任务槽。** `sh_task(id, sub)` 是"每颗弹派一个任务"，所以
`sh_count(0, 28, 1)` + `sh_task` = 一句 `sh_fire` 吃掉 28 个任务槽，池共 256 个。池满走 P4-a
降级：弹保留、任务丢，不报错、不 Fault。表现是"一环里有几颗静默地没有该有的行为"，很难
debug，因为画面上弹都在。给多颗弹挂任务前先算一下 `n_angle × n_speed × 同时在场的波数` 会不
会顶到 256。弹的自主行为能用 `sh_xform`（xformdef，零任务槽）表达的就别用 `sh_task`。

**④ `sh_xform` 吃的是 xform 段池，账和 `batch` 一模一样。** 配了它以后每颗弹都要一份自己的
段拷贝，于是 `sh_xform` + `sh_count(0, 28, 5)` = 一句 `sh_fire` 吃 140 个段，段池共 2048。
多波同时在场时按 `n_angle × n_speed × 同时在场的波数` 估段，和估任务槽是同一笔账，只是分母
换成 2048。

<details><summary>为什么 shooter 的池账比 batch 更容易失手</summary>

今天的 `batch` 没有 `task` 参数，想给一环弹逐颗挂任务只能写 `for` 循环逐颗 `fire`，写的
时候自然会掂量颗数；`sh_task` 让它变成一句话。

`sh_xform` 的段消耗与 [`xform-ops.md`](../xform-ops.md) 给 `batch` 的段消耗警告是逐字同一件事
——只不过 `batch` 是"一句话传全部参数"，颗数就写在眼前那一行；shooter 把 `sh_count` 和
`sh_fire` 拆到了两处，循环里那句 `sh_fire(0)` 看上去人畜无害。

段满的表现和任务满**不一样**：任务满是"弹在、行为没了"，段满是**这颗弹压根没建出来**——从
满的那一颗起本次开火的剩余部分整个短路（同弹池满），计在 `pool_full[XFORM]`。

</details>

### 什么会 Fault、什么只是静默降级

写 `.ecl` 时值得记住的分界（完整口径见 [`ecl-ops.md`](../ecl-ops.md) 600-660 号表）：

- **静默降级**（no-op + 违约计数，任务继续跑）：槽号 `id` 越界；`n_angle` 或 `n_speed` 为
  0，或两者之积超过弹池容量（整条 `sh_fire` 一颗不发）。这几种最难查，脚本照跑、画面上什么
  都没有。顺带一提 `sh_count` 的两个数各自先钳进 `[0,255]`（不回绕），所以写
  `sh_count(0, 300, 1)` 得到的是 255 路而不是报错。
- **Fault**（任务当场被杀，发 `EVT_TASK_FAULT`）：`sh_fire` 时发现 appearance 越界或落在
  图集空格；`sh_xform` 的区间越界；`sh_task` 的 sub 号不在册、或不是零参 `async sub`。
  这些都在开火那一刻才查——setter 只写字段、不校验，所以错误的行列会指到 `sh_fire` 那一行，
  不是设错的那一行。
- **弹池满**：从满的那一颗起停止本次开火的剩余部分（同 `batch` 的短路），已发的留着。
- **xform 段池满**（只在配了 `sh_xform` 时可能）：和弹池满同样短路，已发的留着，不 Fault。
  这是另一个池、另一个计数器（`pool_full[XFORM]`），别和弹池满混作一谈：段池只有 2048 个，
  而配了 `sh_xform` 的一句 `sh_fire` 一次就吃掉 `n_angle × n_speed` 个（账见坑④）。

## 弹 setter 族——弹在自己的任务里改自己

这九个内建给**挂在弹上的任务**用（`fire` 的 `task` 参或 `sh_task` 派出来的那种）：在弹自己的
任务里改这颗弹的速度、方向、加速度。同名的动作也能写进下面的 `xformdef`，那条路不吃任务槽。

⚠️ **弹 setter 族的 handle 参数是陷阱位。** 九个 setter（`set_speed`/`set_angle`/`turn`/
`set_vel`/`set_ang_vel`/`set_accel`/`set_gravity`/`stop_fx`/`aim_at_player`）的首参
`handle: int` 求值后即丢弃，setter 恒作用于当前任务的 owner 弹，是 `self` 语义。不能借句柄
定向操纵别的弹；owner 不是弹的任务调它 → 任务 Fault。想操纵 `fire(...)` 出来的那颗弹，用
xformdef 或 `fire` 的 `task` 参数挂子任务。

## xformdef（弹变换序列声明）

```ecl
xformdef ARC_SHOT {
    set_speed(1.5fx);
    @20 turn(45deg);
    @20 turn(-45deg);
    set_life(180);
}

const BALL: int = 48; // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const COLOR_CYAN: int = 7;

sub main() {
    _ = fire(BALL, COLOR_CYAN, 0fx, 0fx, 1.0fx, 0deg, ARC_SHOT, none);
}
```

- op 名 = [`xform-ops.md`](../xform-ops.md) 小写助记（`turn`/`set_speed`/`set_ang_vel`/
  `step_speed`…）；`@N` 前缀 = **该槽自己的 wait**，即"这条 op 先执行，再等 N 帧才轮到
  下一条"（细节见下面「`@N` 是后置延迟」）；参数必须是编译期常量（字面量/const/一元负号）。
- **STEP 族（`step_speed`/`step_angle`）物理占 2 槽。** scratch 由编译器自动补，作者按 1 条
  写；物理槽总数 ≤16。`loop`/`end` 不开放（复杂控制流写任务弹；尾部零填充天然 END）。
- 被 `fire(..., NAME, ...)` 引用才占 locals 空间（3 字/物理槽，算进引用它的 sub 的容量账）。

### ⚠️ `@N` 是后置延迟，不是时间标签

`@N op(...)` 读起来很像"到第 N 帧才做这条"——一个**时间标签**，ZUN 原版 ECL 的 `@N` 就是这个
意思。**stg-engine 里不是**：`@N` 解析进的是**这一条 op 自己的 `wait` 字段**
（`parse_xf_slot`，`crates/stg-ecl-compiler/src/lang/parse.rs`），而变换相位是**先发射这条
op、才把 `xform_wait` 设成它的 `wait`**（`advance_cursor`，
`crates/stg-core/src/world/transform.rs`）。所以 `@N` 的真实语义是**后置延迟**：「执行这条
op，然后等 N 帧再走下一条」。

推论：**一段 xformdef 里所有 `wait=0`（不带 `@N`）的 op 会在同一帧连续跑完**，直到撞上第一个
带 `@N` 的 op——那条也在同一帧发射，发射之后才开始停 N 帧。想表达"先做 A、等 N 帧、再做
B"，`@N` 必须挂在 **A** 上，不是 B 上。

对照——`godot/ecl/game/boss_windchime.ecl` 的 `WIND_CHIME` 就是一次真实事故：

```text
// 错写法：@30 挂在 turn 上，读起来像"等 30 帧再转"
xformdef WIND_CHIME_WRONG {
    set_speed(2.0fx);
    @30 turn(90deg);
}
// 实际发生：set_speed 的 wait=0 → 同帧接着发 turn；turn 发射之后才开始等 30 帧，
// 但序列到 turn 就结束了（尾部零填充天然 END），这 30 帧谁都不等——纯粹被浪费。
// 效果：弹一出生就转向 90°，此后再也不会动。实测（harness run --at）：帧 3 已是 angle 90°。
```

```ecl
// 对写法：@30 挂在 set_speed 上，序列变成"设速 → 等 30 帧直飞 → 再转 90°"
xformdef WIND_CHIME {
    @30 set_speed(2.0fx);
    turn(90deg);
}

const BALL: int = 48; // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const COLOR_CYAN: int = 7;

sub main() {
    _ = fire(BALL, COLOR_CYAN, 0.0fx, 0.0fx, 1.0fx, 0deg, WIND_CHIME, none);
}
```

实测：改成对写法后帧 31 仍是 angle 0、帧 32 才转向——`@30` 这才真正延迟到了东西
（本例直接调 `fire`，比 demo 里走发射器少一帧出生延迟，帧号差 1、量级一致）。

发射器的 `sh_xform(id, NAME)` 收的就是这里声明的名字，效果一样，只是"配一遍、每颗弹都带上"
（段消耗账见上面坑④）。

### ⚠️ `sh_task` 与 xformdef 天生错开 1 帧

想让"弹活到第 N 帧、到点自己炸开"这类惯用法成立，一般是两条腿配合：xformdef 里用
`set_life(1)` 让母弹自毁，`sh_task` 派的私有任务同时 `wait` 到那一刻再 `sh_fire`。
**两条腿的起跑基准不同**，直接填同一个 N 一定失败：

| 腿 | 起跑 |
|---|---|
| `sh_xform` 挂的 xformdef | **创建当帧就跑**——那一帧的变换相位（相 4）就处理第一槽 |
| `sh_task` 派的任务 | **出生当帧不跑**，首条语句要到出生后第 1 帧才执行 |

于是任务侧写 `wait(N)` 会比母弹的自毁**晚一帧**追到 `sh_fire`——而那时 owner 已被 `cleanup`
回收，"owner 死 → 任务静默回收"的门禁直接把它拦下：**子弹一颗不出**。而且这是彻底静默的
失败——没有 Fault、没有 `contract_viol`、没有 `pool_full`，画面上只是什么都没发生。

**处置：任务侧的 `wait` 比 xformdef 的自毁延迟少 1。** `godot/ecl/game/boss_mothersplit.ecl`
里就写成 `MOTHER_SPLIT_TASK_WAIT = MOTHER_LIFE - 1`，并用 `harness run --at` 逐帧扫过：
差 1 帧稳定成功、不减稳定失败。

**排查配方**：撞到"子弹一颗不出、也没有任何错"时，`run --at F` 扫自毁那一帧前后各一帧——
看母弹还在不在、子弹有没有出现，一眼就能定位是不是差了这一帧。

### 部分设三兄弟：`set_sprite` / `set_shape` / `set_color`

外观值 = `形 × color_stride + 色`（identity：表索引 ≡ 图集格号 ≡ 池 `sprite` 值，见
[`render-contract.md`](../render-contract.md) §3）。三个 xform op 都改弹当前的外观值，区别
在改哪一维：`set_sprite(形, 色)` 是全设，两维一起换（`fire`/`batch` 内部折叠出的 op 就是
它）；`set_shape(形)` 只换形状，保住当前色位；`set_color(色)` 只换颜色，保住当前形位。

```ecl
xformdef SWAP_LOOK {
    set_color(COLOR_BLUE);        // 保形：不管当前是什么形，只把颜色换成蓝
    @10 set_shape(BALL); // 保色：不管当前是什么色，只把形状换成大玉
}

const OUTLINE: int = 32; // 内容包词表（示例：见 godot/ecl/game/bullets.ecl）
const BALL: int = 48;
const COLOR_BLUE: int = 8;

sub main() {
    _ = fire(OUTLINE, COLOR_BLUE, 0fx, 0fx, 1.0fx, 0deg, SWAP_LOOK, none);
    wait(60);
}
```

⚠️ **部分设不查空格。** `set_shape`/`set_color` 编译期只查值本身合不合法：色号落在
`[0, BULLET_COLOR_STRIDE)`、形状基址是 stride 的整倍数且落在表范围内。它不查"这个形+色组合
在图集里是不是空格"，运行期也不替你兜底——两个解释臂只护 stride 合法性（防除零/溢出），不查
`valid`，撞空格既不报错也不 Fault，弹会悄悄变透明地继续飞。

落到空格 = 该弹变透明，这是设计允许的降级路径，由作者自己负责别把部分设用在会撞空的组合上。
想要"越界就出错"的效果，只有 `fire`/`batch`/`set_sprite` 的两参全设才有这道闸（`set_sprite`
编译期走同一份 `check_shape_color`，含 `valid` 检查，见
`set_sprite_blank_atlas_cell_is_compile_error` 单测）。

⚠️ **稀疏弹型不能盲目轮转全色。** 只做了部分色的弹型（如心弹/蝶弹），
`for i in 0..BULLET_COLOR_STRIDE { ... }` 这类轮转写法在色号跑到空格区间时，`fire`/`batch`
会在编译期或运行期被拒收（两参全设查 `valid`）；换成部分设则不会报错，只会让弹在那几帧变
透明。两种后果都不是作者通常想要的。轮转全色只对满色弹型安全，稀疏弹型要么显式列出可用色，
要么整体避开轮转写法。

<details><summary>为什么部分设不做跨维校验（复审别把它当 bug 修回去）</summary>

这是设计允许的行为，不是漏洞。部分设只改一维，落点还取决于弹当时的另一维——那是运行期状态
（可能来自 `fire` 给的初始外观，也可能来自之前执行过的另一次部分设），编译期看不到那个值，
做不了跨维校验。

曾提议一条"跨形状安全"判据（`set_color(c)` 要求 `c` 在图集里所有弹型上都有图）被人类裁定
否决：图集里只要存在一两个稀疏弹型（某行缺几个色），这条判据就会把那几号色在所有弹型上一起
禁掉，代价远大于收益。（当前内建图集 12 行全满 16 色、没有空格，但这条裁定是针对机制的，
不随某一版美术变化。）

</details>

---

**下一篇** → [5 · 三型与语法](5-types.md)：到这里你已经把 `2.0fx` / `90deg` / `as` 用了一路，
该看它们的完整规矩了。
