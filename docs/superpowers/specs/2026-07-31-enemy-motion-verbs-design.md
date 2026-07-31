# 敌人运动动词族 —— 设计（2026-07-31）

> 2026-07-31 brainstorm 拍板。对标 ZUN ECL 的 `move` 族（400–447）。
> 「ECL 手感梳理」清单的最后一项；前四项（死亡效果 / shooter / 数学面 / 敌人读口）已落地。

## 1. 问题

敌人当前只有**一条**运动动词 `move_to(dur, x, y, easing)`，且它只覆盖「位置 × 绝对 × 缓动到点」
这一格。三个缺口：

**① 匀速积分路径是死代码。** `integrate.rs:104` 有 `x += vx; y += vy` 这一支，敌池也有
`vx/vy` 字段，但全仓**非测试代码里写这两个字段的地方只有三处、全是清零**（`move_to` 到点清、
`dur==0` 硬停清）。没有任何 syscall 能写它，`spawn_enemy` 的七参里也没有速度。这条分支永远
在加零 —— 敌人只会「点到点缓动」，不会「匀速飘过屏幕」，而后者是杂兵最常见的运动。
（`cleanup.rs:125` 的出屏测试是 Rust 侧直接写 `vy` 造的场景，所以世界层测试一直绿着，
绿的是脚本够不着的代码。同 `nearest_enemy` 那次——建完了没接线。）

**② 敌人速度对脚本全黑。** 位置有 `enemy_x`/`enemy_y`，血有 `enemy_hp`，速度一个读口都没有。

**③ 没有速度的过程版本。** 弹有 `STEP_SPEED`/`STEP_ANGLE`（D4 变换 op，带 8 条缓动曲线），
敌人一条都没有。

## 2. 对标：ZUN 的 move 族

TH17 eclmap 实表（`404 moveVel` 等，签名取自 Priw8 的 `ins.js`）：

```
400 movePos   (x,y)          401 movePosTime  (time,mode,x,y)
402 movePosRel               403 movePosRelTime
404 moveVel   (r,spd)        405 moveVelTime  (time,mode,r,spd)
406 moveVelRel               407 moveVelRelTime
440 moveAngle (r)            441 moveAngleTime
444 moveSpeed (spd)          445 moveSpeedTime
408 moveCircle  420 moveEllipse  425 moveBezier  432 moveEnm  434 moveCurve …
```

三轴笛卡尔积：〔量〕×〔绝对 / `Rel`〕×〔瞬时 / `Time`〕。`*Time` 变体统一多 `(time, mode)`
两参，**`mode` 就是缓动曲线号**——与我们 `move_to` 的 `easing` 是同一个东西。

**关键一条**：`404 moveVel` 收的是 **`(r, spd)` = 角度 + 速率，不是笛卡尔 `(vx, vy)`**。
它正是 ①那条死分支的写口：脚本给极坐标，引擎存笛卡尔，中间一个 `polar_to_vec`。

**我们比 ZUN 干净的一处**：ZUN 把「瞬时」与「插值」拆成两条指令（400/401、404/405…），
我们的 `move_to` 已用 **`dur == 0` 退化成瞬时** 把两格合并（`world.rs:491` 的「瞬移=硬停」
契约）。沿用这个惯例，ZUN 的八条指令折成我们的四条动词。

## 3. 三条拍板

> 下面三小节各自是一次独立的取舍。**为免混淆，此处不用「甲案/乙案」指代**——
> brainstorm 过程中每个问题各有一组甲/乙，编号在三个问题之间并不通用。

### 3.1 敌人速度**照弹补双表示**（人类拍板）

敌池加 `speed: Fx` + `angle: Angle`。`vx/vy` 仍是**积分真相**，`speed/angle` 是**作者视图**，
双向同步同弹：正向 `polar_to_vec`，反向 `backfill_polar`（含 `BACKFILL_MIN_SPEED` 阈值——
低速时冻结朝向，防 CORDIC 吐垃圾角）。

**否决的对案（单一真相）**：只存 `vx/vy`，极坐标写口进来立刻转掉。省字段、无同步纪律，但
`move_angle`（只转向、保持速率）每次都要 `isqrt(len_sq)` 反算当前速率，有精度损失；且角度
插值只能在笛卡尔空间做，**走的是直线不是弧**，拿不回「该往哪边转」。

**理由**（人类原话）：「敌人和子弹的速度逻辑相同，也会为将来设计圆形轨迹有帮助。」
绕圆本质是「速率恒定、角度匀速转」——极坐标里是一条 `ang_vel`，笛卡尔里得每帧重算两分量。

### 3.2 位置插值与速度插值**分层**而非互斥（人类拍板）

位置插值**只接管位置**；速度插值照跑（改 `speed`/`angle` 或 `vx`/`vy`），只是这期间不驱动
位置；位置插值一结束，那个已调好的速度立刻生效。

**否决的对案（全互斥）**：任一写口清掉另一个插值器。规矩更硬，但表达不了这个常见编排：
*杂兵从屏幕外被拉到 (100, 80)，同时速度缓动到「朝下 3.0」，到点后不停顿地继续往下飘走。*
互斥方案下这要写成两段 `wait` 拼接，中间必然有一帧停顿。

**连带的契约变更**（见 §6.3）：`move_to` 到点**不再无条件清速**。

### 3.3 `move_vel_xy` 的插值走**笛卡尔**空间（人类推翻控制器提案）

控制器原提案是「`move_vel_xy` 的插值也转成极坐标做」，理由是极坐标插值更便宜、且视觉上
几乎总是想要的。**人类推翻**，理由：那样 `move_vel_xy` 就退化成 `move_vel` 的语法糖，
没有存在的理由——「如果我想要极坐标插值，我干脆就用 `move_vel`」。

采纳。且笛卡尔插值有它独占的手感：**线性缓动的笛卡尔速度插值 = 恒定加速度**（重力/坠落），
极坐标插值做不出来（极坐标匀速转向时速率恒定，是「绕」不是「坠」）。

两种插值针对**同一个量**（速度），不可能同时在飞 ⇒ **一个插值器 + 一个空间判别位**，
不是两组字段。

## 4. 动词与读口

### 4.1 五条动词（四条新增）

| 动词 | 参数 | `dur == 0` | `dur > 0` | ZUN 对应 |
|---|---|---|---|---|
| `move_to` | `dur, x, y, easing` | 瞬移 + 硬停 | 位置插值 | 400 / 401 |
| `move_vel` | `dur, angle, speed, easing` | 立即设 | 极坐标插值 | 404 / 405 |
| `move_vel_xy` | `dur, vx, vy, easing` | 立即设 + 回填 | **笛卡尔**插值 | （无直接对应） |
| `move_angle` | `dur, angle, easing` | 只转向、保持速率 | 最短弧插值 | 440 / 441 |
| `move_speed` | `dur, speed, easing` | 只调速、保持方向 | 速率插值 | 444 / 445 |

`move_to` 已存在，本刀不改其签名与既有语义（唯一变更是 §6.3 的到点清速条件化）。

`move_angle` / `move_speed` 走**极坐标空间**，另一分量填当前值。

角度插值走**最短弧**，直接继承 `transform.rs:255` 的既有实现：

```rust
let delta = (to as u16).wrapping_sub(start.raw()) as i16;  // 带方向的最短弧
```

### 4.2 读口两条

`enemy_speed(e)` / `enemy_angle(e)` —— 打包敌号，存活判据**逐字照抄** `enemy_x`/`enemy_y`
（同一个 `resolve_enemy_handle`），降级返 `0`。补上 §1② 那个空白。

**降级值 `0` 是有歧义的**（速率 0 = 静止的活敌，角度 0 = 朝右的活敌，都是合法取值），
同 `enemy_x`/`enemy_y` 返 `Fx::ZERO` 的既有歧义。**探活仍走 `enemy_alive(e)`**，不要拿
读口取值当探针——这条已是手册里的既定惯例（探活读口刀，2026-07-31），本刀只是再加两个
适用它的读口，不新增歧义种类。

### 4.3 不做相对版本（人类拍板：「rel 暂时意义不大，先不做」）

ZUN 有一整列 `Rel`（402/406/442/446…），等于动词数翻倍。不做。有了 §4.2 的读口，相对移动
可以**精确地**composed 出来：

```
move_angle(60, enemy_angle(self) + 15deg, 3)
```

读发生在**调用时刻**，正好是插值起点，语义与 `moveAngleRelTime` 一致。同上一刀处理敌人位置
的路子（给读口 `enemy_x`/`enemy_y`，没做相对版 `move_to`）。

### 4.4 明确的非目标

- **高阶轨迹**（`moveCircle` 408 / `moveEllipse` 420 / `moveBezier` 425 / `moveCurve` 434）——
  另一个量级，单独立项。仓里已有 `xform` 变换段那套机器，届时未必照抄 ZUN。
- **敌人的连续效果**（`ang_vel`/`accel`/`ax`/`ay`，即弹的 `POLAR_FX`/`CART_FX`）——本刀不做。
  圆形轨迹立项时一并评估；`xform.rs` 的 `7x 笛卡尔族` 至今仍是「预留，若将来立项」。
- **`moveEnm`（432，对齐到另一只敌）**、`moveRand`、`moveLimit`。

## 5. 数据模型

敌池（`enemy.rs`，`define_pool!`）增量：

```rust
// §3.1 的作者视图（vx/vy 仍是积分真相）
speed: Fx, angle: Angle,

// 速度插值器（极坐标/笛卡尔共用一组，vel_space 决定四个载体槽怎么读）
vel_from_0: i32, vel_from_1: i32,
vel_to_0:   i32, vel_to_1:   i32,
vel_t: u16, vel_dur: u16,
vel_easing: u8, vel_active: u8, vel_space: u8,
```

**为什么是裸 `i32` 载体而不是 `Fx`**：极坐标空间要装 `(speed: Fx, angle: Angle)`，笛卡尔空间
要装 `(vx: Fx, vy: Fx)`。把 `Angle`（u16）塞进 `Fx` 字段是 newtype 破坏（违 I1/I2 的类型纪律）。
裸 `i32` 是**载体**不是标量，按 `vel_space` 重解释——这正是 `xform` 槽 `args: [i32; 2]` 按 op
重解释的既有先例。

`vel_space` 取值：`VEL_SPACE_POLAR = 0` / `VEL_SPACE_CART = 1`。

**`EnemyInit` 是 exhaustive 的**（`define_pool!` 编译期强制"复用槽写满"），新字段必须进
`EnemyInit`、进 `spawn_enemy` 的建敌路径、进全部测试助手。初值全零：`speed = 0`、
`angle = 0`、插值器全零（`vel_active = 0` 即无插值在飞）。**`spawn_enemy` 的参数面不变**——
本刀不给它加初速参数，建完敌之后脚本自己调 `move_vel` 即可。

**尺寸账**：+29 B/敌 × 256 ≈ 7.3 KB，World 从 ~1.1 MB 涨 0.6%。字段预算不是本刀的约束
（一组插值器 22 B/敌 = 5.6 KB），故设计不为省字段扭曲。D10 预算表更新，`step.rs` 的世界尺寸
哨兵 `EXPECTED` 同步。

## 6. 相位与仲裁

### 6.1 `integrate` 相敌人段的新结构

```
① 若 vel_active：推进速度插值一帧
     polar 空间 → 插 speed/angle（最短弧）→ polar_to_vec 刷 vx/vy
     cart  空间 → 插 vx/vy            → backfill_polar 反算 speed/angle
② 若 mv_active：位置插值接管位置（不读 vx/vy）
   否则：       x += vx; y += vy
```

①**永远跑**，②决定位置归谁——这就是 §3.2 的分层。遍历仍按池索引升序（I4）。

**插值器的生命**：`vel_t` 到达 `vel_dur` 的那一帧写精确终值并清 `vel_active`。位置插值器
（`mv_active`）的既有语义不变。

**重新武装**：四条速度动词中任意一条在**已有速度插值在飞**时被调用 → **无条件重新武装**
（从当前值起算，覆写 `vel_from_*`/`vel_to_*`/`vel_t`/`vel_dur`/`vel_easing`/`vel_space`），
不排队、不叠加。同 `STEP_*` 的 "scratch 无条件重初始化（LOOP 重访 = 自动重新武装）"
（`transform.rs:156`）。**空间可在重新武装时切换**：笛卡尔插值途中调 `move_vel` 即切回极坐标，
起点取切换时刻的**当前** `speed`/`angle`（双表示恒同步，故两个空间的当前值任何时刻都有效）。

### 6.2 插值的数值纪律

**绝对插值**：每帧从 `from` 重算，不累积误差；**终帧写精确终值**（不吃插值舍入）。两条都照抄
`transform.rs::tick_one_step` 的既有做法。`dur == 0` 是合法退化 = 瞬时 set（同 `STEP_*` 与
`move_to` 两处先例）。

**笛卡尔空间每帧要回填**：不回填的话 `enemy_speed`/`enemy_angle` 两个读口会读到陈值、说谎。
代价是 `isqrt` + `atan2`，但**只对正在做笛卡尔插值的敌付费**。

### 6.3 契约变更：`move_to` 到点清速条件化

现状（`integrate.rs:81`）：位置插值到点无条件 `vx = vy = 0`（「到点=硬停」）。

新规：**仅当 `vel_active == 0` 才清速**。有速度插值在飞就保留它的成果，落地即接管。

判据用「有速度插值在飞」而非「脚本设过速度」——前者是引擎自己看得见的状态，不需要额外
记脏位。

既有测试 `integrate.rs:692` 的 `assert_eq!(vx, ZERO, "到点清速")` 拆成两条（§7）。

## 7. 测试

**招牌判别式（整刀的核心）**——同时钉死「两条动词真的不同」与「笛卡尔没被实现成极坐标」：

```
两者都从「朝右 5.0」插到「朝下 5.0」，取 t = 0.5 那帧断言速率：
  move_vel     → 5.0    （匀速扫弧，极坐标）
  move_vel_xy  → 3.54   （直线穿过，中途掉速）
```

一条测试逮住两个错法：笛卡尔实现写成了极坐标；`move_vel_xy` 被做成 `move_vel` 的糖
（后者正是控制器初稿差点设计进去的错，见 §3.3）。

其余：

- **最短弧**：跨 0° 缝插值（350° → 10° 应走 **+20°** 而非 −340°）。
- **分层仲裁**：位置插值 + 速度插值同时在飞 → 到点**不清速**且速度立刻接管；
  再来一条**无**速度插值的 → 仍清（守住原契约）。两条都要，只留一条则判据写反也能过一半。
- **双表示同步双向**：`move_vel` 后读 `vx/vy`；`move_vel_xy` 后读 `enemy_speed`/`enemy_angle`。
- **单轴保持另一轴**：`move_angle` 后速率不变、`move_speed` 后方向不变
  （取 `angle ≠ 0` 且 `speed ≠ 1.0` 的值，否则"另一分量填错"不可辨）。
- **`dur == 0` 退化**：五条动词各自的瞬时路径。
- **P4 降级**：`easing >= 8` → `contract_viol` + no-op（同 `world.rs:486`）；owner 非 ENEMY
  → Fault（同 `move_to` 现状）。
- **读口**：判据与 `enemy_x`/`enemy_y` 一致的三种无效（负号/越界/死槽）。
- **`.ecl` 源码级 e2e**：杂兵被拉到点位、同时缓动到朝下，落地继续飘走（§3.2 那个编排）。
- **死代码通电的正面证据**：一条走 `move_vel` 后纯靠 `x += vx` 位移的断言——
  §1① 那条分支此前永远在加零，世界层单测绿的是够不着的代码。

**变异验证**：招牌判别式与仲裁两条腿各做一次变异，实证「只有它转红」。

## 8. 错误与降级（P4）

- 五条动词全部 **self-only**：owner 非 ENEMY → **Fault**（同 `move_to` 现状，不是静默降级）。
- `easing >= 8` → `contract_viol` + no-op（同 `world.rs:486` 既有做法）。
- `speed` 为负 → **不拦**。负速率就是反向，与弹一致（`set_speed_at` 不校验符号）。
- 读口两条：无效敌号 → 返 `0`，**不** Fault、**不**计 `contract_viol`（同 `enemy_x`/`enemy_y`）。
- 笛卡尔目标 `(0, 0)` → 速率归零、**朝向保持不变**（`BACKFILL_MIN_SPEED` 的既有规则，
  防 CORDIC 在零向量上吐垃圾角）。

## 9. 兼容性

**`ENGINE_VER` 必须 bump** —— 池布局变了（快照字节数变、`SaveBytes` 编码变），比号表新增
硬得多：旧存档/旧回放按新布局解读会走出另一条世界线，必须拒载。

**金向量预期会变**（本刀与前几刀的不同）：`rainbow.ecl` 场上有敌人，敌池布局一变，逐帧
校验和流全变。**基线需重新生成**，不能拿「逐字节不变」当验收判据。跨平台对拍闸
（`determinism-gate`）照常——它比的是三平台之间，不是与历史基线。

## 10. 文档

- `docs/ecl-lang.md`：运动一节补四条动词 + 两个读口；写明 `move_vel` 与 `move_vel_xy` 的
  **插值空间不同**（这是最容易踩的一格）；相对移动的 composed 写法给例（§4.3）。
  围栏示例真编译，`every_ecl_fenced_example_in_doc_compiles` 押运。
- `docs/ecl-ops.md`：六个新 syscall 号。
- `stg-world-design.md` D5/D10：敌池字段表 + 容量预算。
- `docs/fixed-point-corners.md`：笛卡尔插值每帧回填的 `isqrt`/`atan2` 精度账（若实现期发现
  有值得记的坑）。
- `PROGRESS.md`：史加一行 +「现在」段重写。
- `docs/follow-ups.md`：§4.4 的非目标逐条记为 follow-up（高阶轨迹 / 敌人连续效果 / `moveEnm`）。
