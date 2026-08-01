# 1 · 从零到一个弹幕

> 这一篇从空文件开始，一步一步长出一段能玩的弹幕：一只敌 → 让它动 → 发一颗弹 → 发一圈
> → 放进关卡。每一步都给**完整的、能直接编过的**脚本和"你会看到什么"。
> 读之前不需要读别的篇，只需要写过 C 或 Rust。

## 先把验证环搭起来

写一行验证一行，比写完一屏再调快得多：

```
cargo run -p stg-harness -- check my.ecl
```

通过打印 `OK`、退出码 0；不通过逐条打 `文件:行:列: 说明` + 源行 + `^` 定位。

编过了不等于跑对了。**真跑一遍**看它到底干了什么：

```
cargo run -p stg-harness -- run my.ecl --frames 600
```

打的是逐段采样的**弹数/敌数/活任务数**、全程峰值、末帧，外加诊断计数。⚠️ 脚本被引擎杀掉
（死循环烧穿指令预算、坏参数）时 `run` 会**打出 fault 并退非零码**——这是唯一能看见它的地方，
别的路径一律静默。要验一圈弹到底均没均分，加 `--at F` 打第 F 帧每颗弹的角度（BAM 原值 + 度数）
与速度：

```
cargo run -p stg-harness -- run my.ecl --frames 300 --at 60
```

还有 `--seed S`（换随机种子）、`--rank R`（难度 `0..=4`，默认 2）。想边玩边看就
`cargo run -p stg-harness -- serve --ecl my.ecl`，浏览器开 <http://localhost:8611>，
改完脚本刷新页面即重载。

想在真 Godot 里看，就去替换 `godot/ecl/demo/` 里的关卡脚本（那个目录**整取**、按文件名排序
编译成一个编译单元，所以不是往里加文件——里面已经有一个 `sub main()` 了，加第四个文件会撞名），
再 `cargo build -p stg-godot && godot --path godot`。详见
[`godot/README.md`](../../godot/README.md)。

场地坐标：`x ∈ [-192, 192]`（0 是中轴），`y ∈ [0, 448]`，**y 向下增**——`y = 0` 是画面顶，
自机在底下（[`render-contract.md`](../render-contract.md) §6）。单位是像素，
类型是 `fx`（定点数），所以坐标要写 `96.0fx` 而不是 `96`。

## 先说三件想当然会错的事

写过 C/Rust 的人在这三处最容易按旧习惯理解，而且撞上了不一定报错。

### ① 形状是 Rust 的，不是 C 的

```text
const NAME: int = 1;      var x: fx = 1.0fx;    // 类型写在冒号后
for i in 0..5 { }         loop { }              // 半开区间；loop 是关键字
_ = fire(...);                                  // 弃值绑定
2.0fx / 90deg                                   // 字面量后缀，同 1.5f32
```

跟 C 相同的只有花括号和 `if`/`while` 的形状。写 `int x = 5;` 或 `for (i = 0; i < 5; i++)`
会撞一串编译错误。

标量只有三种：`int`、`fx`（定点小数）、`angle`（角度），**互不隐式转换**，字面量靠后缀分型。
先照着例子写，撞了再去看 [5 · 三型、字面量与语句](5-types.md)——那一篇是完整规矩。

### ② sub 不是普通函数

没有 `struct`/`enum`/泛型/trait，**sub 无返回值**，**禁递归**（直接间接都是编译错误），
局部变量的槽由编译器静态分配。所以"写个辅助函数返回一个角度"这种 C 习惯在这里行不通，
要传值只能走 `globals` 槽（[6 · 符卡与整局编排](6-spell-and-stage.md)）。

### ③ `async sub` 是协程，`wait` 是让出

这一条是整个模型的核心，也是 C/Rust 背景最容易想错的地方：**`async sub` 不是你调用的函数，
是引擎替你逐帧恢复的协程。**

- `spawn f();` 起一条新协程，**立刻返回**——它不等 `f` 跑完，`f` 也不在这一帧跑（新协程
  出生当帧一条指令都不执行，下一帧才首跑）。
- 协程跑到 `wait(30);` 就**让出**：栈、局部变量、执行位置整个冻在那里，引擎每帧的相位 2
  回来看一眼，30 帧后从下一句接着跑。
- 所以 `wait` 不是 sleep，也**不是"让敌人等"**——它只让**这一条任务**等。同一只敌身上另一条
  任务照跑，`move_to` 发起的位移由引擎在积分相位里逐帧推进，跟谁在 `wait` 毫无关系。

和 Rust 的 `async fn` 也不是一回事：这里没有 `.await`、没有 executor、没有 `Future`；
让出点只有 `wait`，恢复由引擎按帧驱动（这是确定性的要求，见 CLAUDE.md 的 I5/I6）。

## 第 1 步：一个能编过的空关卡

```ecl
sub main() {
    loop { wait(1); }
}
```

`sub main()` 是整份脚本的唯一根入口：零参、不能 `async`、整份编译产物里必须且仅有一个。
末尾那句 `loop { wait(1); }` 是"每帧跑一次、永不结束"的挂住写法。

**你会看到什么**：一个空场。`check` 打 `OK`。

（`main` 自己 `return` 不会有灾难——只是关卡脚本没了，世界照常空转。敌人不一样，见第 3 步。）

## 第 2 步：放一只敌

```ecl
sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, none);
    loop { wait(1); }
}
```

`spawn_enemy(x, y, hp, drop_table, score, sprite, task)` 返回一个**敌号**；这里用不上，
所以 `_ = ` 把它显式丢掉（有返回值的调用不消费就是编译错误，见
[5 · 三型、字面量与语句](5-types.md#语句)）。最后那个 `none` 是"不给它挂主任务"。

**你会看到什么**：一只贴图 `1` 的敌停在场地中轴、离顶 96px 处，不动、不发弹、不消失。

## 第 3 步：给它一个主任务——顺带解释为什么 `return` 敌就没了

敌的行为写在它的**主任务**里，就是 `spawn_enemy` 的第 7 参：

```ecl
async sub boss_main() {
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, boss_main);
    loop { wait(1); }
}
```

⚠️ **主任务一 `return`（或自然跑到末尾），这只敌当场退场。** 这是 ZUN ECL 的"主协程返回即
自燃"语义：引擎立刻把它标 `ENEMY_DYING`，相位 9 回收。所以上面写的不是 `async sub
boss_main() { }`——那样敌会在你还没看见它的时候就没了。

反过来说，敌想退场也不用飞到界外，让主任务自然结束就行。完整规则（退场是静默的：不掉道具、
不加分、不发死亡事件）见 [3 · 敌人](3-enemy.md)。

**你会看到什么**：敌停在原地不走了，一直在。

## 第 4 步：让它动

`move_to(dur, x, y, easing)` 把敌在 `dur` 帧内缓动到某个点，`easing` 是曲线号 `0..=7`：

```ecl
async sub boss_main() {
    loop {
        move_to(90, -120.0fx, 96.0fx, 2);
        wait(90);
        move_to(90, 120.0fx, 96.0fx, 2);
        wait(90);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, boss_main);
    loop { wait(1); }
}
```

那两句 `wait(90)` 不是装饰：**`move_to` 发起后立即返回**，真正的位移由引擎在积分相位里逐帧
走。任务不 `wait` 够 90 帧就会立刻执行下一句 `move_to`，把上一条半路截胡——看上去就是"敌只
抖了一下就换方向"。

再说一次：`wait` 等的是**任务**，不是敌人。敌人在这 90 帧里一直在动，动它的是引擎不是脚本。

**你会看到什么**：敌在 `y = 96` 这条横线上左右来回踱步，一趟 1.5 秒（90 帧 ÷ 60Hz）。

## 第 5 步：让它发一颗弹

走位和发弹是两件独立的事，各给一条任务最省心。`spawn` 起来的子任务和主任务共享同一个
owner（这只敌），所以子任务里的 `$self_x`/`$self_y` 读的就是敌自己的坐标：

```ecl
const BALL: int = 48;        // 内容包词表（示例：见 godot/ecl/demo/bullets.ecl）
const COLOR_RED: int = 2;

async sub shoot() {
    loop {
        _ = fire(BALL, COLOR_RED, $self_x, $self_y, 2.0fx, aim_player(), none, none);
        wait(40);
    }
}

async sub boss_main() {
    spawn shoot();
    loop {
        move_to(90, -120.0fx, 96.0fx, 2);
        wait(90);
        move_to(90, 120.0fx, 96.0fx, 2);
        wait(90);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, boss_main);
    loop { wait(1); }
}
```

`fire(shape, color, x, y, speed, angle, xf, task)`：`BALL`/`COLOR_RED` 是**内容包**自己用
`const` 定的弹型与色号（引擎不注册这些名字），`aim_player()` 返回"从自己指向自机"的角度，
末尾两个 `none` 是"这颗弹不挂变换、也不挂任务"。

**你会看到什么**：一只边走边每 40 帧朝自机吐一颗玉的敌。它的弹会跟着它的位置走，因为
`$self_x`/`$self_y` 是**每次执行到那一行时**现读的。

## 第 6 步：发一圈

发环用**发射器**（`sh_*` 族）：先把一组发射参数写进槽里，之后每开一波火只要一句 `sh_fire`。
关键是配置写在 `loop` **外面**——付一次，循环里每波只有一句 `sh_fire`：

```ecl
const BALL: int = 48;
const COLOR_RED: int = 2;

async sub shoot() {
    sh_reset(0);                     // 0 号槽抹回默认，不继承别处的残留
    sh_sprite(0, BALL, COLOR_RED);   // 弹型 + 色号
    sh_ring(0, 1);                   // 整周环：n 颗由引擎均分 360°
    sh_count(0, 16, 2);              // 16 颗 × 2 层速度
    sh_speed(0, 1.2fx, 0.5fx);       // 层速 1.20 / 1.70
    loop {
        sh_fire(0);                  // ← 每一波就这一句
        wait(60);
    }
}

async sub boss_main() {
    spawn shoot();
    loop {
        move_to(90, -120.0fx, 96.0fx, 2);
        wait(90);
        move_to(90, 120.0fx, 96.0fx, 2);
        wait(90);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, boss_main);
    loop { wait(1); }
}
```

那五句 `sh_*` 是**一次性成本**：槽里的参数跨帧留着，所以循环里只剩 `sh_fire(0)`。要下一波不同
就临开火前改一句——`sh_count(0, 24, 2);` 就是 24 路，`sh_speed(0, 2.0fx, 0fx);` 就是提速，其余
五句一个字不用动。boss 一段弹幕开几十次火，这笔账很快就赚回来了。

`sh_ring(0, 1)` 是"整周均分"开关：引擎逐颗算 `(i × 65536)/n`、余数均摊，首尾精确闭合，`n`
不整除 65536 也不会攒出一条缝。**自己拿 `65536 / n` 算步长会掉余数**——真实符卡因此翻过车，
数字见 [4 · 弹](4-bullets.md) 开头那张表。

一次性的角度 × 速度网格也可以用 `batch` 一句发完，代价是角度算术归你：它的 `angle_step` 是
**逐弹增量**，不替你均分整周，整周得自己算 `65536 / n`（角度是 BAM，一圈 65536，`as angle`
是位穿透 cast 不是"转成度"）。三条路什么时候用哪条，见 [4 · 弹](4-bullets.md) 开头的表。

<details><summary>同一个环用 `batch` 写：更短，但那圈是你自己分的</summary>

```ecl
const BALL: int = 48;
const COLOR_RED: int = 2;

async sub shoot() {
    loop {
        var step: int = 65536 / 16;                     // BAM 一圈 65536，16 等分
        _ = batch(BALL, COLOR_RED, $self_x, $self_y,
                  16, 0deg, step as angle,              // 16 个角度，逐弹 +step
                  2, 1.2fx, 0.5fx);                     // 2 层速度：1.2 / 1.7
        wait(60);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, shoot);
    loop { wait(1); }
}
```

发出来的和上面那版是同一张网格——因为 16 恰好整除 65536（`step` 正好 4096），一颗余数都没掉。
换成 28 路就不是了。差别也不在长度（就这一波而言 `batch` 还更短），在于**每次改颗数你都得
重算一遍 `step`，而且没人会提醒你算错了**；而这一版每波都要把十个参数重念一遍，改成"下一波
换个速度"就得再抄一行。

</details>

**你会看到什么**：每秒一发的双层 16 方环，一边踱步一边扩散。到这里你已经能改出自己的弹幕了
——改 `sh_count` 的颗数与层数、改 `sh_speed`，或者把 `wait(60)` 调小。

## 第 7 步：放进关卡

关卡就是 `main` 里的一条顺序流程：先一波杂兵，再让 boss 上场。`mark(N)` 在流程里插一个
"练习模式可以从这里开局"的落点：

```ecl
const BALL: int = 48;
const COLOR_RED: int = 2;
const MARK_BOSS: int = 1;

async sub zako() {
    move_to(90, $self_x, 140.0fx, 2);
    wait(90);
    for i in 0..3 {
        _ = fire(BALL, COLOR_RED, $self_x, $self_y, 1.8fx, aim_player(), none, none);
        wait(25);
    }
    move_to(150, $self_x, 500.0fx, 1);
    wait(150);                       // 等它走完；不等的话敌会在半路上就退场
}

async sub shoot() {
    sh_reset(0);
    sh_sprite(0, BALL, COLOR_RED);
    sh_ring(0, 1);
    sh_count(0, 16, 2);
    sh_speed(0, 1.2fx, 0.5fx);
    loop {
        sh_fire(0);
        wait(60);
    }
}

async sub boss_main() {
    spawn shoot();
    loop {
        move_to(90, -120.0fx, 96.0fx, 2);
        wait(90);
        move_to(90, 120.0fx, 96.0fx, 2);
        wait(90);
    }
}

sub stage1() {
    bgm(1);
    for i in 0..4 {
        _ = spawn_enemy((i * 96 - 144) as fx, 2.0fx, 40, 1, 300, 0, zako);
        wait(15);
    }
    wait(300);
}

sub boss_battle() {
    var boss: int = spawn_enemy(0.0fx, 96.0fx, 500, 1, 1000, 1, boss_main);
    while enemy_alive(boss) == 1 {
        wait(10);
    }
}

sub main() {
    stage1();
    mark(MARK_BOSS);
    boss_battle();
    loop { wait(1); }
}
```

几处值得停一下的：

- `stage1()` / `boss_battle()` 是**普通 `sub`**（不带 `async`），被 `main` 同步调用，
  从 `main` 的时间线上顺序流过去。带 `async` 的那三个只能 `spawn` 或挂在 `spawn_enemy` 的
  `task` 位上，两条路互斥。
- `boss_battle` 靠 `enemy_alive(boss)` 轮询等 boss 死，而不是猜 `boss_main` 什么时候退出
  ——敌号是引擎给的不透明句柄，带 generation，槽被别的敌复用了也不会认错人。
  真实关卡这里还该有超时兜底，见 [3 · 敌人](3-enemy.md)。
- `mark(MARK_BOSS);` 只能写在 `main` 的顶层语句位。正常开局它是个空操作（一步跳过），
  中段启动时宿主可以直接从这里开局，引擎会替你把 `bgm` 这类背景锚点补上。完整规则见
  [6 · 符卡与整局编排](6-spell-and-stage.md)。

**你会看到什么**：一局的骨架——四只杂兵俯冲、三连射、退场，然后 boss 上场打环，打死后
`main` 挂住。真正的 boss 该用**符卡**（引擎替你记计时、bonus 衰减、破卡判定），那是
[6 · 符卡与整局编排](6-spell-and-stage.md) 的事。

## 你现在会的和还不会的

会了：`main` 根入口、`spawn_enemy` + 主任务、`move_to`、`fire` 单发、发射器发环、`spawn`
并行任务、`wait` 的让出语义、`mark` 落点。

还不会（按建议顺序）：`wait` 的精确周期和几条静默截断（[2](2-tasks.md)）、敌的退场/死亡与掉落
（[3](3-enemy.md)）、发射器的 fan/ring/自机狙与池账、弹自己变速转向的 `xformdef`
（[4](4-bullets.md)）、三型的完整规矩（[5](5-types.md)）、符卡与整局编排（[6](6-spell-and-stage.md)）。

---

**下一篇** → [2 · 任务与时间](2-tasks.md)：`sub` 与 `async sub` 的确切分工、`wait(n)` 到底
等几帧、新协程什么时候开始跑。
