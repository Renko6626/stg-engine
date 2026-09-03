# 3 · 敌人

> 这一篇讲敌人的完整生命周期：怎么退场、怎么造、怎么动、怎么死、掉什么，以及关卡脚本怎么隔着
> 敌号观察它。读之前先读 [2 · 任务与时间](2-tasks.md)——敌的行为全挂在任务上，
> 而第一节就是任务与敌之间那条最容易踩的连线。

## ⚠️ 主任务跑完 = 这只敌退场（D9，写敌任务前先读这条）

`spawn_enemy` 的 `task` 参挂上去的那个 sub 是这只敌的**主任务**。它一 `return`（或自然跑到
末尾），引擎立刻把这只敌标 `ENEMY_DYING`，相位 9 回收——ZUN ECL 的"主协程返回即自燃"语义。

- 要敌留在场上 → 主任务不能返回，末尾拿 `loop { wait(1); }` 挂住（或本来就是 `loop{}`
  编排）。**写完一段编排就 `return` = 这只敌当场消失**，这是最容易踩的一脚。
- 要敌退场 → 让主任务自然结束就行，不用把它移到越界线外骗回收。

退场是静默的：不掉道具、不加分、不发 `EVT_ENEMY_DIED`、不发死亡特效请求，连 hp 都不动，满血
退场就是满血。脚本跑完是"退场"不是"被击破"；掉落与记分只属于被击破的那两条路径——被自机打死，
或脚本显式 `die()`（见下面「三条死亡路径对照」）。

**只有主任务有这个效果。** `spawn` 出来的伴生任务、`fire(..., task)` 挂在弹上的任务、
`spell_begin` 的 `pattern` 任务，结束了都只是它自己没了，跟敌的存亡无关；反过来敌一死，
owner 门禁会把整棵 task 树清杀。主任务因 Fault 死也不自燃，那是报错路径，已有
`EVT_TASK_FAULT`。实现在 `ecl::vm::run_tasks` 的 `Exec::End` 分支。

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

那句 `wait(150)` 是重点：`move_to` 是引擎侧插值器，发起后立即返回；主任务不 `wait` 够帧数就
走到末尾的话，敌会在缓动跑完之前退场。`godot/ecl/demo/stage1.ecl` 的杂兵是这段的真实版本。

## 敌的生成与轮询（`spawn_enemy` 的 `task` 参 + `enemy_hp`）

`spawn_enemy` 第 7 参 `task` 与 `fire` 第 7 参**同构**：编译期解析的 async 无参 sub 名，或
字面量 `none`。

非 `none` 时新敌的 owner 三元组落 `(ENEMY, 新敌 index, generation)`，随之解锁 owner 门禁：
`spell_begin`/`move_to`/`self_*` 系列只在这颗新敌自己的任务里才能过闸。旧态下这些调用永远
Fault，纯 `.ecl` 摆不出 boss 正是这条门禁挡的。`none` 时敌照常建成、不派任务，`main_task`
保持 0。

**敌死任务亡**：owner-liveness gate（相位 2）在敌死后的下一相位清杀整棵 task 树。反过来也
成立——主任务跑完这只敌就退场，见上一节。所以 `stage` 侧应该轮询
`enemy_hp` 而不是去猜某个 sub 有没有退出。`enemy_hp(handle) -> int` 是 STAGE 层等 boss/敌死的标准写法：死亡、悬垂、
越界敌号统一返 `-1`（P4-b），槽被另一只敌复用之后旧敌号照样返 `-1`（敌号带 generation）。

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

上面这段只演示"怎么轮询"，不是"boss 该怎么写"：它的 `boss_main` 只 `wait(60)` 就返回了，按
上一节「主任务跑完 = 这只敌退场」那条规则**这只 boss 会在 60 帧后
自己退场**，等待循环随之结束。它按
`godot/ecl/demo/boss_windchime.ecl` 原文精简改写——真实版本的 `boss_main` 跑非符 + 符卡两
阶段、`boss_battle` 的等待循环带 75 秒挂死兜底。**真实关卡编排务必带超时兜底。**

## 敌人运动（`move_to` + 四条速度动词）

五条动词全是 `self` 作用，无句柄参，够不着别的敌。owner 不是敌的任务调它们一律 Fault(0)、
任务当场被杀，和 `die()`/`spell_begin` 同一条门禁。它们全都**发起后立即返回**，真正的推进在
引擎的积分相位里逐帧走，所以要"等它走完"得自己 `wait` 够帧数。

分两层：位置层只有 `move_to` 一条，速度层四条。

| 动词 | 作用 | 插值空间 |
|---|---|---|
| `move_to(dur, x, y, easing)` | 把敌拉到绝对点位（这期间位置由插值器全权决定） | 位置（x/y 各自插） |
| `move_vel(dur, angle, speed, easing)` | 同时改朝向和速率 | **极坐标** |
| `move_vel_xy(dur, vx, vy, easing)` | 同时改两个速度分量 | **笛卡尔** |
| `move_angle(dur, angle, easing)` | 只转向，速率一字不动 | 极坐标 |
| `move_speed(dur, speed, easing)` | 只调速，方向一字不动 | 极坐标 |

`dur == 0` 对五条都是**立即设**，不是"插值 0 帧"，是合法写法、不计违约。它同时是急停键：
若此刻正有一条同层的缓动在跑，`dur == 0` 会把它当场停掉，新值不会在次帧被旧缓动覆盖回去
（速度层四条互相之间如此，位置层的 `move_to(0, …)` 对在飞的位置缓动也如此）。

`easing` 是缓动曲线号 `0..=7`（`0` = 线性）。越界的 `easing` 走 P4-b：整条调用 no-op + 计一次
违约，不 Fault、不钳位。写错了敌就是不动，别指望它退化成线性。`dur` 同理，合法范围 `0..=65535`。

> 这条**从 2026-09-03 起才对所有越界值成立**（`ENGINE_VER` 12→13）。此前 `easing`/`dur` 在
> syscall 边界上是裸截断的，于是 `easing = 256` 会静默变成 `0`（线性）并照常武装，而
> `easing = 264` 落 8、被正确拒掉——**能不能拒取决于越界值模 256 落在哪里**；`dur = -1` 则
> 变成"缓动 65535 帧"。现在两个都在收窄那一步就拒。

### 位置层与速度层谁说了算

- `move_to` 在飞的时候，位置全归插值器，`vx/vy` 一概不参与。速度动词此时照样能跑、照样在改
  速度，只是暂时看不见效果，落地那一刻才接管。
- 没有在飞的 `move_to` 时，位置就是老老实实的 `pos += (vx, vy)`。
- **`move_to` 是一次完整的运动接管。** 它一发起就把此前的速度意图全部作废：不光抹掉"碰过"
  的记号，连正在跑的速度缓动也当场停掉。所以 `move_vel(30, …); move_to(10, …);`
  是"缓到一半被位置命令截胡"，到点就是真停住，不会再飘。
- **到点那帧清不清速，看脚本这一轮有没有碰过速度动词**：没碰过 → 到点清速成 0（老契约：
  到点即停，一字未改）；碰过 → 不清，落地即按那个速度继续飘。

  `move_to` 每次武装都会把"碰过"这个记号抹掉，所以两种写法结果不同：

  ```text
  move_vel(...); move_to(...);   → 到点即停（move_to 在后，记号被抹、缓动被停）
  move_to(...);  move_vel(...);  → 到点继续飘（记号在 move_to 之后被立起来）
  ```

  注意判据是"碰没碰过"，不是"速度插值还在不在跑"。速度那条的 `dur` 常常比位置那条短、先到
  期，那时速度插值早就停了，但刚缓好的速度必须留住。

### ⚠️ `move_vel` 与 `move_vel_xy` 的插值空间不同

两条不是同一个动作的两种写法。拿同一对端点看——从「朝右 5.0」缓到「朝下 5.0」，`dur` 相同，
线性缓动：

- `move_vel` 走**极坐标**：分别插 `speed` 和 `angle`。速率全程恒为 5.0，敌匀速扫出一段弧；
  角度走最短弧（`350deg → 10deg` 是 `+20deg`，不是绕回去的 `-340deg`）。
- `move_vel_xy` 走**笛卡尔**：`vx`、`vy` 各自线性插。中点是 `(2.5, 2.5)`，速率掉到
  `2.5·√2 ≈ 3.54` 再涨回 5.0，速度矢量是走直线穿过去的。线性缓动在这条路上就等于恒定加速度。

要匀速转向（扫描弹幕、绕圈）用 `move_vel`。要恒定加速度、或者要保住一个轴，用
`move_vel_xy`——`move_vel_xy(30, $self_vx, 4.0fx, 2)` 就是 x 分量原样留着、只把 y 缓到 4.0。

`move_angle` / `move_speed` 是 `move_vel` 的"只动一半"，两条都走极坐标空间。

### 相对运动没有 Rel 版动词——用 `$self_*` 组合

引擎不提供 `move_angle_rel` 之类的相对版本。`$self_speed`/`$self_angle`/`$self_vx`/`$self_vy`
读的是活值，在参数位上算一下就是相对量，语义还更透明——相对谁、相对哪一刻，写在脸上。

```ecl
async sub weave() {
    move_vel(0, 90deg, 2.0fx, 0);       // 先给个初速：朝下 2.0
    loop {
        move_angle(60, $self_angle + 15deg, 3);   // 相对转向：每轮再偏 15°
        move_speed(30, $self_speed * 2.0fx, 2);   // 相对加速：速率翻倍
        move_vel_xy(30, $self_vx, 4.0fx, 2);      // 保住 x 分量，只把 y 缓到 4.0
        wait(90);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 40.0fx, 60, 1, 0, 0, weave);
    loop { wait(1); }
}
```

`$self_*` 是**读取即 syscall** 的活值，不是发起动词那刻的快照——速度插值在飞的中途读会读到
中途值。想固定住某个瞬时值就自己 `var` 存一份。

### 典型编排：拉到点位 + 落地继续飘

要落地后继续飘，速度动词必须写在 `move_to` **之后**：

```ecl
async sub dive_and_drift() {
    move_to(40, 80.0fx, 180.0fx, 2);   // 位置：40 帧缓到 (80,180)
    move_vel(20, 90deg, 3.0fx, 0);     // 速度：20 帧缓到「朝下 3.0」——比位置那条早到期
    wait(40);                          // 到点。速度动词碰过 ⇒ 不清速
    loop { wait(1); }                  // 此后每帧 y += 3.0，一路飘出下边界
}

sub main() {
    _ = spawn_enemy(0.0fx, 100.0fx, 100, 1, 0, 3, dive_and_drift);
    loop { wait(1); }
}
```

## 敌人的三条死亡路径与掉落控制

`drop_clear` / `drop_add` / `drop_items` / `die` 四个内建都是 `self` 作用，与弹 setter 族同构：
它们操作的永远是当前任务的 owner 敌，没有句柄参数，也就够不着别的敌。owner 不是敌的任务
（关卡根脚本、`fire` 挂在弹上的任务）调它们一律 Fault(0)、任务当场被杀，和
`move_to`/`spell_begin` 同一条门禁。

**`spell_begin` 的模式任务是敌 owner**——owner 就是宣言那只 boss，与 boss 主任务同一只敌。
所以这四个在模式任务里全都合法：`$self_x`/`$self_y`/`move_to` 能用，`die()` 也能用。"符卡最
后一发打完让 boss 就地阵亡"直接在模式任务末尾写 `die();` 即可。

### 待掉落计数是敌身上的可变状态

每只敌带一份逐类型的待掉落计数，五个类型各一个字节。`spawn_enemy` 的 `drop_table` 参数只是
**生成时的初值**：建敌那一刻把表展开进这份计数，此后再没人读过表号。所以同一张表生出来的
两只敌，可以被脚本各自改成完全不同的掉落。

- `drop_add(type, n)` —— **只增不减**。`type` 直接写引擎常量（`ITEM_POWER` / `ITEM_POINT` /
  `ITEM_LIFE_PIECE` / `ITEM_BOMB_PIECE`，编译器预置注入，不用自己 `const`；`ITEM_STAR`
  也是合法类型号，但星星按设计只由消弹转化产生，别拿它当掉落）。`n` 先钳进 `[0, 255]`
  再累加，负数视同 0——这个内建不做"减掉落"；计数封顶 255 饱和，不回绕。
  坏 `type`（负数或 ≥ 类型数）走 P4-b 降级：整条调用 no-op + 违约计数，不 Fault，脚本继续
  往下跑。写错了不会响亮地炸，只会静默地不掉东西。
- `drop_clear()` —— 五个计数一次清零。要"这只敌什么都不掉"就在它死前调一次。
- 撒出去的顺序恒按类型编号升序，与 `drop_add` 的调用顺序、掉落表的书写顺序都无关。
- `drop_item` 与 `drop_items` 都带随机喷发速度，**消耗模拟 RNG**：它们和 `rand(n)` 共享同一
  颗 PRNG 流，调用顺序会影响后续随机数。做逐帧确定性回放对拍时留意这一点。

### ⚠️ `drop_items()` 吐完不清空——`drop_items(); die();` 掉两份

`drop_items()` 把当前计数照单撒一遍，但撒完不把计数清零。于是：

- `drop_items()` 之后这只敌再死一次（被打死或 `die()`），同一批道具再撒一遍。
- 想只掉一份 → 别在 `die()` 之前调 `drop_items()`；真要"先撒再死"，中间补一句 `drop_clear()`。

保留这个字面语义是照 ZUN 的裁定，不是待修的 bug（引擎侧有测试钉死它）。另外 `drop_items()`
只撒道具，不加分、不发死亡事件、不发死亡特效请求、不标死亡。

### 三条死亡路径对照

| 路径 | 掉落 | 加分（敌的 `score`） | 死亡事件 / 死亡特效请求 | 死后的 `$self_hp` | 触发方式 |
|---|---|---|---|---|---|
| 被自机打死 | ✔ | ✔ | ✔ | ≤ 0（打穿多少是多少，overkill 留负值） | hp ≤ 0（自机弹或消弹区伤害） |
| `die()` | ✔ | ✔ | ✔ | 强制 `min(0)`——满血 boss 也当场归 0 | 脚本显式调用 |
| 主任务跑完（D9） | ✘ | ✘ | ✘ | **完全不动**（满血就还是满血） | 主任务自然 `return` / 跑到末尾 |

前两行是同一份引擎实现，四件事一起发生，而且幂等：已经在死的敌再被 `die()` 一次是 no-op，
不会掉双份。`drop_items()` 那条坑不受此保护，它走的是另一条口子。第三行是"退场"不是"被击破"：
静默消失，什么都不给。

所以：

- 想让自然退场也掉落 → 主任务 `return` 之前自己调一次 `drop_items()`。这只补掉落，仍然不加
  分、不发死亡事件；要那些就得用 `die()`。
- 想让敌就地阵亡 → `die()`。跑完整死亡效果，然后**立即终止本任务**：`die()` 降低成两条指令，
  第二条就是终止，它后面的语句一句都不执行。

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

<details><summary>hp 那一列不是学究：为什么"轮询 enemy_hp 等 boss 死"不能靠 hp 判</summary>

三条路径里只有 D9 让敌带着一身血退场，所以"轮询 `enemy_hp` 等 boss 死"这种编排**不能**靠
hp 判死（D9 退场时 hp 一点没动，可能还是满的；`enemy_hp` 只在槽被**回收之后**才返 -1，
那才是判据）。

绑卡 boss 尤其要留意：`die()` 把 hp 压到 0（压过血线下钳，见 [6 · 符卡与整局编排](6-spell-and-stage.md)），D9 自燃则完全
不碰 hp——两条都照样收卡结算，因为触发的是破卡三路 OR 里的 `ENEMY_DYING` 那一路，不是 hp
那一路。

</details>

### ⚠️ 死了的敌当帧仍参与碰撞，仍能撞死自机

三条路径都只标记不回收：槽要活到相位 8 供表现层读，相位 9 的 cleanup 才收尸。而体碰检测
**不查死亡标记**，这只敌在被回收之前照常算数，自机撞上去照样中弹。

被打死的敌也是这样（死在相位 7，相位 6 已经碰过了），只是 `die()` 的窗口更长：ECL 任务跑在
相位 2，远早于碰撞的相位 6，于是"一只已经宣告死亡的敌在同一帧里撞死了自机"这种场面在
`die()` 路径上明显得多。别把 `die()` 当"立刻从场上消失"使——它是"阵亡"，不是"消失"。

## 数学与查询

### `atan2(y, x) -> angle`

任意向量的方向角。**参数序是 `(y, x)`**，`y` 在前，同 libm 惯例，两位都是 `fx`。这一位最容易
写反：两参同型，写成 `atan2(dx, dy)` 不会有任何编译错误，只会让角度沿 45° 对角线镜像。
`(0, 0)` 返 `0deg`，不报错。

`aim_player()` 只能瞄自机（0 参，基点是自己）；`atan2` 能瞄任意点——瞄某只敌就是
`atan2(ey - $self_y, ex - $self_x)`。

### `dist(dx, dy) -> fx`

**向量的模长，不是两点距离。** 要两点距离自己减：`dist(bx - ax, by - ay)`。开的是真根
（`dist(3.0fx, 4.0fx)` 正好 `5.0fx`），不是平方距离。屏幕尺度上不会溢出，满屏对角线离上限
还有两个数量级；真喂进天文数字的 `fx` 时结果饱和在最大可表示距离，不会回绕成负数。

### `nearest_enemy(x, y) -> int`

离 `(x, y)` 最近的活敌，返敌号；场上无敌返 `-1`。候选是"存活且未在死亡态"的敌，并列时取低
索引，无距离上限。返回值可以直接喂 `enemy_alive(h)` / `enemy_hp(h)` / `enemy_x(h)` /
`enemy_y(h)`——它们是配对的：拿号 → 探活 / 轮询血量 / 读坐标。

### 敌号是不透明句柄

`spawn_enemy` / `nearest_enemy` 返的、`enemy_hp` / `enemy_x` / `enemy_y` / `enemy_alive`
吃的那个 `int`，是引擎给你的不透明句柄，不是数组下标：

- 别猜它的数值，别和 `0` 之外的字面量比，别做算术（`e + 1` 不是"下一只敌"）。唯一有意义的
  取值是 `-1`，意思是无效 / 没有。
- **两个敌号相等 ⇒ 同一只敌。** 它带着 generation，所以敌死、槽被回收、另一只敌落进同一个槽
  之后，你手上那个旧敌号不会变成新那只敌的号：四个读口一律降级（`enemy_hp` → `-1`，
  `enemy_x` / `enemy_y` → `0`，`enemy_alive` → `0`）。
- 它可以存进变量、存进 globals 槽、跨帧带着用；只是别指望它跨局有意义。
- 打包之前（敌句柄打包刀 2026-07-31 以前）敌号只保证"同一个槽"，`enemy_hp(boss)` 有可能在
  boss 死后读到占了它槽的杂兵的血，"等 boss 死"的轮询就此卡住不退——这类 bug 现在不存在了。

### 读坐标前先探活

`enemy_x(handle) -> fx` / `enemy_y(handle) -> fx` 按敌号读坐标，口径同 `enemy_hp`：带
generation，槽被别的敌复用后旧号读到的是降级值。有了这两条，"查最近的敌 → 朝它开火"才接得通。

⚠️ **降级值是 `0`，不是哨兵。** `enemy_hp` 能用 `-1` 表示"这个号没用"，是因为血量天然非负；
坐标没有这个便利——任何 `fx` 取值都可能是真坐标，没有哨兵位可用。所以死槽 / 越界 / 负句柄
一律返 `0`，代价是「那只敌恰好停在原点」与「这个号无效」读起来一模一样。

⇒ **惯例：先 `enemy_alive(e) == 1` 探一下，再读坐标。** `enemy_alive(handle) -> int` 答的是
这个敌号是不是仍指向它当初那只敌，返 `1` / `0`，是探活专用口。

⚠️ **它判的是「这个号还认得那只敌」，不是「还能打」。** 正在死（`ENEMY_DYING`）的敌它照样返
**1**：那只敌的槽要活到本帧末（相位 9）才回收，坐标也仍读得到。这是有意的——读族四条
（`enemy_hp`/`enemy_x`/`enemy_y`/`enemy_alive`）用的是完全相同的判据，谁也不会和谁打架。

⇒ 想要"还能打"的语义，配 `nearest_enemy` 现查一次：**它本身就排除了 dying**（候选 = 存活且
非 dying）。所以从它拿到的号过几帧之后可能已经变 dying，那时应当重查而不是继续用手上那个号
——不是因为号会失效（带 generation，指的一直是同一只敌），而是这两条口的 dying 口径本就不
对称：`nearest_enemy` 排除 dying，读族四条含 dying。

<details><summary>旧探针 enemy_hp(e) != -1 差在哪（overkill 的负血量）</summary>

**旧写法 `enemy_hp(e) != -1` 仍然能用**（引擎的降级值一字未改），但它有一格填不上的缝：
**`-1` 同时是降级值和一个合法血量**。被打穿（overkill）的敌血量是**真实负值**——引擎只
把它压到 `min(0)`、不抹平——所以血量**恰好**是 −1 的那只敌还在屏幕上、还该被瞄，旧探针
却把它读成"号无效"。`enemy_alive` 与血量取值完全无关，没有这条缝。

顺带：`enemy_hp(e) >= 0` 从来就是错的探针，同一个原因——它会把一切被打穿的敌判成无效。

</details>

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
        if e >= 0 && enemy_alive(e) == 1 {
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

（出处：`atan2`/`dist`/`nearest_enemy` 是引擎里早就有、脚本此前够不着的东西，小清洗刀
2026-07-31 通电；`enemy_x`/`enemy_y` 是敌坐标读口刀同日所加，数据本就在敌池里躺着；
`enemy_alive` 是探活读口刀。六个都零新机制。）

---

**下一篇** → [4 · 弹](4-bullets.md)：`fire` / `batch` / xformdef / 发射器族。
