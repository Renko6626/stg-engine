# ECL Shooter（预存发射参数集）—— 设计（2026-07-31）

> 状态：已拍板，待写实施计划。
> 权威度：低于 `design_doc.md` / `stg-world-design.md`；与两者冲突处以两者为准，本文只记本刀决策。
> 出处：「ECL 手感梳理」（2026-07-31）列出的四条硬缺陷之一（收益最大的一条）。
> 姊妹刀：时间标签（**已否决**，见 `2026-07-31-ecl-time-labels-design.md`）；难度分档、敌人运动
> 动词、小问题清洗各自独立，**不在本文范围**。

## 1. 问题

我们的 `fire`/`batch` 是**一次性调用**——8~10 个参数每发都要重写一遍。demo 里能直接看出来：

```ecl
_ = batch(RICE, color, $self_x, $self_y, ways, base, astep, 1, speed, 0fx);
```

十个参数里只有 `color` 和 `speed` 在变，其余八个每轮重复。想改一个字段就得把整串重打。

**而且 `fire` 与 `batch` 不是包含关系，中间有个能力缺口：**

| | 网格（N×K） | xform | 挂弹任务 |
|---|---|---|---|
| `fire` | ✘（单发） | ✔ | ✔ |
| `batch` | ✔ | ✘ | ✘ |
| ZUN 的弹幕管理器 | ✔ `count1/count2` | ✔ `etEx` | —（走 `funcSet`） |

所以今天想发"带 xform 的一环弹"**根本写不出来**，只能 `for` 循环逐颗 `fire`。

## 2. ZUN 的做法

ZUN 的 `et*` 族（600-641，共 41 条）是一套**弹幕管理器**：`etNew(etId)` 重置编号槽 →
一堆以 `etId` 打头的 setter 逐项配 → `etOn(etId)` 开火。改一个字段再 `etOn` 一次就是下一波。

关键结构（数据源：Priw8 的 ECL 文档 `js/ecl/ins.js`；助记名取自 ECLjs 仓库的 `th17.eclm`。
本仓 `docs/zun-ecl-v2-reference.md` 只录了 VM 级 ins_0-93，游戏指令段是本次新查的）：

| ins | 语义 |
|---|---|
| `600 etNew(etId)` | 重置管理器 `etId` 为默认 |
| `601 etOn(etId)` | 用管理器 `etId` 的属性开火 |
| `602 etSprite(etId, type, color)` | 弹型 + 颜色 |
| `603 etOffset(etId, x, y)` | 相对偏移 |
| `604 etAngle(etId, angle1, angle2)` | 两个角 |
| `605 etSpeed(etId, speed1, speed2)` | 两个速度 |
| `606 etCount(etId, count1, count2)` | 两个数量（= N×K 网格） |
| `607 etAim(etId, aim)` | 瞄准模式（枚举，见 §5） |
| `608 etSound(etId, sound1, sound2)` | 两个音效 |
| `609-612 etEx*` | 弹变换表（= 我们的 `xformdef`） |
| `614 etCopy(dst, src)` | 复制管理器（其文档自陈 "partially broken"） |
| `626 etOffsetRad(etId, angle, radius)` | 极坐标偏移，**与 603 叠加** |
| `627 etDist(etId, dist)` | 出生后沿当前角度先位移 |
| `628 etOffsetAbs(etId, x, y)` | 绝对偏移 |
| `617-625` | 按 rank/难度设 speed/count 的六条专用版 |

`(count1, count2)` × `(angle1, angle2)` × `(speed1, speed2)` 这三对，正是我们 `batch` 的
N×K 网格结构——**两边的发射器参数集本就同构**，差别只在"每发重传"还是"存起来"。

## 3. 决策（人类裁定，实施不得偏离）

| # | 决策 |
|---|---|
| **D-1** | 状态按**任务**键，每任务 **K=4** 个编号槽；无分配、无句柄、无泄漏 |
| **D-2** | 参数集取 `fire` ∪ `batch`（网格 + xform + 挂弹任务），顺带补上 §1 的能力缺口 |
| **D-3** | 位置存 **owner 相对偏移**（默认 `(0,0)` = owner 位置），三种偏移叠加 |
| **D-4** | `etSound` 归并进通道 B，存**请求 id** 而非音效号；只收 sound1，**sound2 不做** |
| **D-5** | 极坐标偏移（`626`）与出生位移（`627`）**都进 v1** |
| **D-6** | ZUN 的九值 aimmode **塌成两个布尔** `aimed` / `ring`；4/5 冗余，**6/7/8 随机模式不做** |
| **D-7** | **fan 以基准方向为中心对称展开**（默认语义，非可选）；ring 不居中 |
| **D-8** | `sh_fire` **无返回值** |
| **D-9** | 砍 `614 etCopy` |
| **D-10** | 难度分档（`617-625`）**不在本刀**——归「难度分档」独立一刀 |

## 4. 归属与存储（D-1）

### 4.1 为什么必须住 ECL 层

CLAUDE.md 的 P1：*world 不 import 任何 ECL 类型、不知道"任务"存在*；`World.tasks` 之所以
物理上住组装层，正是为了满足 memcpy 快照又不破坏这条。

shooter 按任务键 ⇒ **天然是 ECL 层的东西，不能放 `WorldBody`**。放 `TaskPool` 做并行数组：

```rust
pub struct TaskPool {
    pub(crate) slots:    [Task; TASK_CAP],
    pub(crate) shooters: [[Shooter; SHOOTERS_PER_TASK]; TASK_CAP],   // K = 4
    pub(crate) alive:    [u64; NW],
}
```

放这儿还白捡一件事：**槽复用时的重置有现成挂点**（`TaskPool::spawn`），这正是本仓
「复用槽写满」硬规则要求的。

### 4.2 成本（无条件付）

I7（`World` 内无堆容器、快照 = 整块字节复制）+ 校验和**哈希全槽不掩码**
⇒ 这 45 KB 的**分配、memcpy、哈希三样都与当前活着几个任务无关**。

| | 现状 | 本刀后 |
|---|---|---|
| World | 1.03 MB | **1.08 MB**（预算 1.3 MB，余量仍有 ~220 KB） |
| 16 帧快照环 | 14.7 MB | 15.4 MB |
| 单次快照耗时 | 173–463 µs | +约 15 µs |

### 4.3 为什么不用全局编号槽

全局 `[Shooter; 32]` 只要 1.4 KB，而且"编号槽靠作者约定"是本仓**已有的家规**
（`globals` 分系统段/自由段、`boss_ui` 编号槽、符卡槽 0..`MAX_BOSSES`）。

**但它有个 ZUN 没有的问题**：ZUN 的 `etId` 是**每敌独立作用域**的，两个 boss 都用 0 号互不
干扰；全局槽做不到——一屏杂兵各自用 0 号就会互相踩。除非约定"杂兵只用 `fire`/`batch`、
shooter 专供 boss"，而那是脆的。45 KB 买"永远不撞车"，在这个预算里划算。

## 5. 字段（44 B）

| 字段 | 类型 | 默认 | ZUN 对应 |
|---|---|---|---|
| `appearance` | u16 | 0 | `602 etSprite`（形×stride+色，同 `fire` 的折叠糖） |
| `off_x, off_y` | Fx×2 | 0 | `603 etOffset` |
| `polar_ang, polar_r` | Angle, Fx | 0 | `626 etOffsetRad`（**叠加**在上一条之上） |
| `dist` | Fx | 0 | `627 etDist` |
| `angle0, angle_step` | Angle×2 | 0 | `604 etAngle` |
| `speed0, speed_step` | Fx×2 | 0 | `605 etSpeed` |
| `n_angle, n_speed` | u8×2 | **1, 1** | `606 etCount` |
| `flags`（`aimed` / `ring` / `abs_offset` 三位） | u8 | 0 | `607 etAim` |
| `xform_off, xform_cnt` | u16, u8 | 无 | `609-612 etEx` |
| `task_script` | u16 | 无 | （ZUN 走 `funcSet`） |
| `on_fire_req` | u16 | 0 | `608 etSound` 的 sound1 |

合计 42 B → 对齐 **44**（`Fx` = i32 故 4 字节对齐）。

`n_angle/n_speed` 默认 **1×1** 而非 0：刚重置的 shooter 开火发**一颗**弹，是个有意义的退化，
而不是"什么也不发"这种要 debug 半天的静默。

**三处"怎么清掉"必须明确**（否则是二义的）：

- `sh_offset(id, x, y)` **清 `abs_offset` 位**；`sh_offset_abs(id, x, y)` 置位。两者写的是同一对
  `off_x/off_y` 字段，只是解释方式不同——**后写的赢**。
- `sh_offset_rad(id, angle, 0fx)` 即取消极坐标分量（半径 0）。它与上面那对**永远叠加**，
  不存在"覆盖"关系。
- `sh_xform(id, none)` / `sh_task(id, none)` 清除挂接，与 `fire` 的 `none` 同款写法。

## 6. `aimed` / `ring`：九值枚举塌成两个布尔（D-6）

TH8 的 `ins_96`–`104` 把 aimmode 枚举逐条写明了（`607` 那条描述偷懒没展开）：

| mode | 名称 | `ang1` | `ang2` |
|---|---|---|---|
| 0 | aimed fan | 相对自机方向的**偏移** | **弹间**夹角 |
| 1 | unaimed fan | 绝对方向 | 弹间夹角 |
| 2 | aimed ring | 偏移 | **层间**夹角 |
| 3 | unaimed ring | 绝对方向 | 层间夹角 |
| 4 / 5 | offset aimed / unaimed ring | 同上 | 同上 |
| 6 | random angles | 最大方向 | 最小方向 |
| 7 / 8 | random speeds / angles+speeds | —— | `spd1/spd2` 转义为 max/min |

这其实是个**扁平化的叉积**，塌成两个正交布尔：

| 布尔 | 作用 |
|---|---|
| **`aimed`** | `angle0` 是**相对自机方向的偏移**（而非绝对方向）。ZUN 文档里 aimed 模式写 "aim **offset**"、unaimed 写 "aim **direction**"，差别就在这 |
| **`ring`** | `n_angle` 颗**自动均分整周**，`angle_step` 转义成**逐层**偏移；否则 fan（`angle_step` 逐弹） |

**ZUN 的 4/5（offset ring）冗余**——ring 模式下 `angle_step` 本就是逐层偏移，"错开半步"就是
作者写 `sh_angle(id, base, (32768 / n) as angle)`，不必单开模式。

**ring 必须逐颗算 `(i × 65536) / n_angle`**，把余数均摊、**精确闭合**。demo 现在手算
`65536/28 = 2340`，28 颗只铺满 65520，收尾留 16 BAM 的缝。

### 随机模式（6/7/8）不做

它们消耗**世界 RNG**——而 RNG 是 `World` 字段、随快照回滚，**消耗顺序直接进校验和**（I3）。
这不是加个字段的事，是要把"每颗弹抽几发、按什么序"钉死成契约。且"随机散布"本身是独立特性，
与 shooter 的核心价值（预存参数集）无关。**记 follow-up。**

## 7. fan 居中（D-7）

**fan 以基准方向为中心对称展开**——奇数路正中那颗**正对**基准方向，偶数路基准方向落在
**中间两颗之间**。这正是奇偶路自机狙都能自然工作的原因，也是 ZUN 的默认语义。

```
fan:   angle_i = 基准角 + i·angle_step − ((n_angle − 1)·angle_step) / 2
ring:  angle_i = 基准角 + (i × 65536) / n_angle + j·angle_step
```

奇数路时 `(n−1)` 为偶数、除 2 精确；偶数路时截断半个 BAM 单位（1/65536 圈，可忽略且确定）。

**后果：改颗数不用重算 `angle0`。**

```ecl
sh_aim(0, 1);
sh_count(0, 5, 1);
sh_angle(0, 0deg, 10deg);   // 5-way 自机狙,正中一颗正对
sh_count(0, 4, 1);          // 改 4-way,angle0 不动
sh_fire(0);                 // 自机方向落在中间两颗之间
```

**ring 不居中**——整周均分本就无"中心"可言（整环转多少都是同一个环），基准角就是第 0 颗的位置。

**"瞄准自机旁边"是白送的**：aimed 模式下 `angle0` 就是偏移，`sh_angle(0, 15deg, ...)` 即打在
自机右侧 15°。

## 8. 声音归并进通道 B（D-4）

核心里不该有"声音"这个概念，它只转发。所以字段存的是**一个请求 id**：

`on_fire_req ≠ 0` 时，开火后发
`emit_req(on_fire_req, [原点x, 原点y, appearance, 实际创建数, 0, 0])`。

- 表现层拿它当音效、枪口闪光、震屏都行——**核心不需要知道**
- 不用新增引擎保留 req id（脚本用 `REQ_SCRIPT_BASE = 64` 以上的自定义号）
- **ZUN 的 `516 playSound`（独立播音指令）我们已经有了**——就是现成的 `emit_req`，不加新内建
- 容量无压力：`REQS_CAP` 每帧 256 条、满了走 P4-a 丢弃计数；20 个 shooter 每帧都开火才占 8%

**ZUN `etSound` 的 sound2 不做**：那是"该管理器发出的弹**做变换时**"触发，逐弹逐变换、量级
完全不同，且要 xform 段 VM 能发 req = op 表变更。

## 9. 表层面：15 个内建（syscall 62–76）

```
sh_reset(id)                   重置该槽为默认
sh_sprite(id, shape, color)    两参折叠,同 fire/batch 的颜色轴糖
sh_offset(id, x, y)            相对 owner
sh_offset_abs(id, x, y)        绝对(置 abs_offset 位)
sh_offset_rad(id, angle, r)    极坐标,叠加在 sh_offset 之上
sh_dist(id, d)                 出生后沿各自角度推 d
sh_angle(id, angle0, step)
sh_speed(id, speed0, step)
sh_count(id, n_angle, n_speed)
sh_aim(id, on)
sh_ring(id, on)
sh_xform(id, xf)               xformdef 名或 none
sh_task(id, sub)               async sub 名或 none
sh_req(id, req_id)             开火时发的请求,0=不发
sh_fire(id)                    开火,无返回值
```

`sh_fire` **无返回值**（D-8）：本语言要求值必须消费，有返回值就得写 `_ = sh_fire(0);`，
而开火是循环里最高频的语句。池满走 P4-a 计数（同现有 `batch`），脚本本来也无从处置。

**砍 `614 etCopy`**（D-9）：其 ZUN 文档自陈 "partially broken"，且 K=4 下手写几行 setter 不费事。

### 典型用法

```ecl
async sub windchime() {
    sh_sprite(0, RICE, COLOR_RED);
    sh_ring(0, 1);
    sh_count(0, 28, 5);
    sh_speed(0, 1.0fx, 0.25fx);
    sh_xform(0, WIND_CHIME);          // ← 今天的 batch 做不到
    var base: angle = 0deg;
    loop {
        sh_angle(0, base, 0deg);      // 只改一个字段
        sh_fire(0);
        base = base + 7deg;
        wait(20);
    }
}
```

## 10. 开火的七步（顺序即契约）

1. **owner 位置**——同 `$self_x/$self_y` 口径：敌→敌坐标，弹→弹坐标，关卡→`(0,0)`
2. **原点** = `(abs_offset ? (0,0) : owner位置)` + `(off_x, off_y)`
   + `(cos(polar_ang)·polar_r, sin(polar_ang)·polar_r)`
3. **基准角** = `aimed ? 指向自机的角(从原点算) + angle0 : angle0`
   ——**在开火那一刻解析**，不是 `sh_aim` 时。无存活自机时的取值**沿用既有 `aim_player`
   内建的口径**，本刀不另立规矩
4. **网格**，序照抄 `create_bullets_batch`：**角度外层、速度内层**
   （`for i in 0..n_angle { for j in 0..n_speed { … } }`）
   - 角：见 §7 的 fan / ring 两式
   - 速：`speed_j = speed0 + j·speed_step`
5. **dist**：第 (i,j) 颗的生成位置 = 原点 + `(cos(angle_i), sin(angle_i))·dist`
   ——**逐颗方向不同**，不是整体平移
6. 每颗挂 `xform`（`xform_off`/`xform_cnt`）与 `task_script`
7. `on_fire_req ≠ 0` → 发请求（§8）

### 为什么开火循环住 ECL 层

本想扩 `create_bullets_batch` 加 `dist`，但撞上 P1——world **不知道任务存在**，没法逐颗挂
`task_script`（这正是 `fire` 的挂任务发生在 ECL 层的原因）。所以开火循环住 ECL 层，逐颗调
既有的 `create_bullet_with_xform`。

**结果是整个 shooter 对 world 层纯加法，一个既有 API 都不动。** 代价是网格循环有了第二份
实现——所以 §12 那条等价测试从"顺带的"升级成**必需的**。

## 11. P4（逐条对齐既有口径，不自创）

| 情形 | 处置 | 对齐谁 |
|---|---|---|
| `id ≥ 4` | no-op + `contract_viol` + `STATUS_BAD_ARGS` | 全部 15 个内建 |
| `n_angle`/`n_speed` = 0，或乘积 > `BulletPool::CAP` | 不发 + `contract_viol` | `create_bullets_batch` |
| xform 参数非法 | 不发 + `contract_viol` | 同上 |
| 弹池满 | 逐颗降级 + `pool_full[POOL_BULLET]`，已建的保留 | 同上的 `'grid` 短路 |
| 任务池满 | 逐颗降级 + `pool_full[POOL_TASK]`，**弹保留** | `sys_create_bullet` |
| `sh_task` 的 sub 号非法 | **Fault**，零副作用 | `sys_create_bullet` 的"先验后建" |
| owner 类别 | **无限制**（敌 / 弹 / 关卡都能发） | `emit_req` |

## 12. 测试

### 最硬的一条：带居中补偿的等价

```
shooter_fan(n, base, step)  ≡  batch(n, base − (n−1)·step/2, step)
```

（在 `task`/`dist`/`polar`/`aim`/`ring` 全默认时。）

一次押住**网格序、坐标算法、速度递增、以及居中公式本身**。补偿写错、或实现忘了居中，都会红。
这条之所以必需，见 §10 末——网格循环有第二份实现。

### 三条判别腿，每条防一个具体的错误实现

| 测试 | 防什么 |
|---|---|
| `dist` **逐颗沿各自角度**位移 | 错误实现把整环朝同一方向平移——圆心重合式测试对它是瞎的 |
| `sh_offset` + `sh_offset_rad` 同设 → **相加** | 错误实现让后设的覆盖先设的（ZUN 明写 stacks） |
| 自机移动后再 `sh_fire`，基准角随之变 | 钉"开火那一刻解析 aim"，防实现成"`sh_aim` 时就把角算死" |

### 其余

- **ring 精确闭合**：`n_angle = 28` 时 28 颗铺满整 65536、无缝（防照抄 demo 的 `65536/n` 预乘）
- 每个 setter 的"只改这一维、其余不变"
- `sh_reset` 恢复全部默认
- **任务槽复用后 shooter 是默认值**（「复用槽写满」纪律）
- `id` 越界（参数化一条覆盖 15 个）
- `on_fire_req` 的 `args[3]` 是**实际**创建数而非请求数（池满时要能区分）
- 池满逐颗降级、不 panic、计数

## 13. 破坏面

- **`ENGINE_VER` bump**（syscall 号表新增 62–76）
- `TaskPool` +45 KB → 世界尺寸哨兵更新；快照 / 存档格式变
- **金向量逐字节不变**（纯新增，无脚本调用它们）——**两个方向都要验**（"该同的同"）
- **零 world API 改动、零 op 表改动**
- 文档：`docs/ecl-lang.md` 新节（含 §12 三条判别腿对应的坑）、`docs/ecl-ops.md` 号表 15 行、
  `docs/bench-baseline.md` 内存账续表

## 14. 明确不做

- ZUN 的随机 aimmode 6/7/8（§6 末，消耗世界 RNG，独立特性）→ follow-up
- ZUN 的 `617-625` 难度分档专用 setter（D-10）→ 归「难度分档」独立一刀
- `614 etCopy`（D-9）
- `etSound` 的 sound2（§8 末）
- `sh_fire` 的返回值（D-8）
- 扩 `create_bullets_batch` 或任何既有 world API（§10 末）
- 迁移 demo / 金向量脚本到 shooter 写法——留给作者按需改，不在本刀制造无谓 diff

## 15. 与其余手感缺口的关系

「ECL 手感梳理」列了四条硬缺陷，本刀做的是收益最大的一条。其余：

- **时间标签**——**已否决**（`2026-07-31-ecl-time-labels-design.md`）：会系统性地让 ECL 变复杂，
  而日常写法用 `wait` 就够。
- **难度分档**（ZUN `rankI3/F3`、`diffI/F`、`diffWait`，及每条指令的 `rank_mask`）——独立一刀。
  与本刀有一处**明确的接口**：ZUN 的 `617-625` 是"按难度设 shooter 的 speed/count"，
  那一刀落地时会以 `sh_speed_rank(id, …)` 之类的形式**扩本刀的 setter 面**，而不是另起机制。
- **敌人运动动词只有 `move_to` 一个**（ZUN 的 400-447 共 48 条）——独立一刀。
- **小问题清洗**：暴露 `atan2`/`isqrt`/距离（核里已有、脚本够不着）、接上 `nearest_enemy`
  死代码（`world.rs:923` 有实现有测试、无 syscall）、订正 `ecl-lang.md` 那条已过时的
  "跨 `.ecl` 文件无共享 const"限制（**已实测否定**）、补 `wait` 的 u16 截断坑记
  （`wait(65536)` → 0，手册全文未提）。
