# 激光池 —— 直线激光一等实体（设计，2026-09-25）

> 状态：**设计已拍板，待写实施计划**。
> 来源：训练仓 `stg-rl-train` 的 brainstorm（2026-09-25）。卡池至今没有激光：转写时整段跳过
> （TH06 s1_mb2 / s1_b3 / s2_b4、第 4 关 11 个 sub、Extra 3 个），`lasers` 表恒为 0 行（follow-ups D23#10）。
> 本刀让引擎能生成带激光的数据，为之后合成训练数据做准备。
> 涉及仓库：**stg-engine**（本刀全部改动）。stg-agent-proto **不改**。训练仓、renkolab、转写流水线的活见 §9 交接。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 保真目标 | **只做直线激光**，参数、三态、判定几何按 th06nc。TH06 全作只有这一种机制（§2）。TH16/18 的曲线激光以后用「一串短线段」复用本池，不在本刀 |
| ② | 转动 | 池里有持续角速度 `omega`，**同时**保留一次性 rotate（原作 88）。引擎求干净，和 th06nc 逐帧锯齿的对齐交给模型侧（§9.2） |
| ③ | 平移 | 可选挂靠到敌人（`anchor` + 偏移），取代原作每帧调一次 `laser_offset` 的写法。敌人死后**脱钩**，原点留在原地 |
| ④ | 判定宽度 | **设多宽就判多宽**：只有一个 `width` 字段，判定半高 = `width/2`，画面宽度也是 `width`。TH06 画出来的宽度是判定宽度的 2 倍（半高 = width/4），这个 gap 由**转写负责**（写 `width = 原作值 / 2`），引擎不内置任何一作的口径 |
| ⑤ | 判定时段 | 只在 state 1（生效）判。按 th06nc 删掉 CC0 在 state 0/2 的判定窗，也不复刻 CC0 预警期判定盒缩成中点的 bug |
| ⑥ | 剩余寿命 | **不提供 `t_left`**，proto 不改。原作的 duration 不可信：飞出去的棒子写的是 9999；Extra 会提前 `laser_cancel`；Sub47 **每帧重建**一条 duration 6 的激光，让「还剩 6 帧」永远成立。这些都是往危险方向错。`t_active`（预警倒计时）可信：ECL 只能 cancel，错也只往安全方向错 |
| ⑦ | 句柄 | `laser()` 返回**带代际的打包句柄**（和 `$self_enemy` 同式）。激光要跨帧操作，只给下标不安全 |
| ⑧ | 擦弹 / 沿线掉星 / 曲线激光 | 本刀不做，记进后续待办（§10） |

## 2. 依据：原作激光只有一种机制

**CC0 源码**（`th06-decomp/src/BulletManager.cpp`，renkolab `engine/bullet/th06nc/01-pools-and-structs.md` §3 已对 NC 一手复核）：
激光存成「射线加上射线上的一段区间」，而不是两个端点：

```
origin = pos,  dir = (cos angle, sin angle)
线段   = origin + [startOffset, endOffset] · dir
每帧：  end += speed;  start = max(start, end − startLength, 0)
        angle、pos 每帧只读、不写（只有 ECL 88/89/90 会改）
判定：  自机转进激光坐标系，钳进盒 [start, end] × [−width/4, +width/4]，dist² < r²
state： 0 预警（不判）→ 1 生效（判）→ 2 收缩（不判）→ 回收；start ≥ 640 也回收
```

**两种形态只是参数取值不同**。参数序依次为 speed, start, end, startLength, width, 预警, 生效：

| 出处 | speed | start→end | startLength | width | 预警 | 生效 | 形态 |
|---|---|---|---|---|---|---|---|
| s1 Sub12 | 0 | 0→500 | 500 | 32 | 30 | 120 | 预警线 → 激活，起来后 `laser_rotate` 每帧 ±0.0083 rad |
| s1 Sub22 | 0 | 0→500 | 500 | 16 | 120 | 60 | 预警线 → 激活，自机狙，不转 |
| s2 Sub27 | 4 | 0→0 | 192 | 6 | 0 | 9999 | 192 px 的棒子沿射线匀速飞出，出屏回收 |
| s4 ×129 | 0 | **64**→500 | 500 | 24 | 23–30 | 54–90 | 预警线，近端 64 px 留空 |
| Extra | 0 | 0/32→216–420 | 有限 | 24 | 60 | 800 | 有限长，挂在芙兰身上转，`laser_cancel` 提前结束 |

**贴图**：etama3 的第 146–153 号精灵，是 8 色的「截面渐变」小方块（横向有渐变，纵向一致）。
画法是 `scaleX = width / 贴图宽`、`scaleY = 长度 / 贴图高`、旋转 `π/2 − angle`，加色混合；
原点另贴一个弹的出生闪光（`SPAWN_BIG_BALL`）。预警线就是同一张贴图压到 1.2 px 宽，
在最后 min(预警, 30) 帧线性长到全宽。收缩时宽度线性缩回 0，`flags` 第 0 位为 1 时改为 alpha 淡出。

**全局清除**：CC0 的 `RemoveAllBullets` / `DespawnBullets` 在清弹时会把 state<2 的激光一并切到 2
（符卡结束、中弹后的宽限期、boss 死亡、清屏道具都走这里）。`laser_clear_all`
**只清敌人自己的指针表，激光本身继续活着**（`EclManager.cpp:530`）。

## 3. 数据：`define_pool! { Laser, cap = 256 }`

容量 256：TH06 用 64，TH16 用 512，这里给以后的曲线分段留出余量。记进 D10 预算表。

| 组 | 字段 | 类型 | 说明 |
|---|---|---|---|
| 几何 | `ox, oy` | `Fx` | 射线原点 |
| | `angle` | `Angle` | |
| | `omega` | `i16` | 每帧转多少 BAM，口径同弹的 `ang_vel` |
| | `start, end, start_len, speed` | `Fx` | 语义同 §2 |
| 外观/判定 | `width` | `Fx` | 画面宽度 = 判定宽度，判定半高 `width/2` |
| | `sprite` | `u16` | 图集外观（形状 + 颜色，折叠方式同弹）。**见修订：实际只存颜色号 0..15** |
| 时序 | `warn, active, fade` | `u16` | 三段的时长（帧）。`warn == 0` 时出生即是 state 1；`fade == 0` 时生效期一结束直接回收 |
| | `timer` | `u16` | 当前 state 内的帧计数 |
| | `state` | `u8` | 0 预警 / 1 生效 / 2 收缩 |
| 挂靠 | `anchor_idx, anchor_gen` | `u16` | 敌人句柄。`anchor_idx = 0xFFFF` 表示不挂 |
| | `ax, ay` | `Fx` | 挂靠偏移 |
| 观测 | `dx, dy` | `Fx` | 本帧原点的实际位移（先例：敌人 dx/dy） |
| | `dang` | `i16` | 本帧实际转角，包括 omega、一次性 rotate、aim |
| | `px, py` / `pang` | `Fx` / `Angle` | 上一帧相位 5 结束时的原点与角度，求差用。出生时等于初值（**见修订：出生当帧被脚本改写时同步**） |
| 杂项 | `flags` | `u8` | 位 0 同原作（收缩方式：0 = 变窄，1 = 淡出），纯表现 |
| | `born_frame` | `u32` | |

- 按 exhaustive `LaserInit` 写满全部字段，配宏全覆写单测（复用槽时写满，是「哈希全槽」的前提）。
- ECL 写 `angle` / `ox,oy` 的操作（rotate / aim / origin）发生在相位 2，早于相位 5 的推进。`dx,dy,dang` 是
  「相位 5 结束时的值 − 上一帧相位 5 结束时的值（`px,py,pang`）」，所以包含本帧的**所有**变化来源，而不只是 omega。

## 4. 每帧行为

### 4.1 推进（相位 5 integrate，在敌人移动之后）

```
若 scene_frozen：整段跳过（同弹）
对每条存活激光（相位 2 的 ECL 修改此时已经生效）：
  若挂靠且敌人仍存活（代际相符）：(ox, oy) = 敌人位置 + (ax, ay)
  否则若挂靠但敌人已失效：清除挂靠（脱钩），原点不动   // 见修订：被杀那一帧敌人仍存活，激光照跟
  angle += omega
  end += speed;  若 end − start > start_len：start = end − start_len;  start = max(start, 0)
  timer += 1；按表切换 state：
     0 且 timer ≥ warn   → 1，timer = 0
     1 且 timer ≥ active → fade == 0 ? 回收 : (2, timer = 0)
     2 且 timer ≥ fade   → 回收
  start ≥ LASER_CULL（640 px）→ 回收
  (dx, dy, dang) = (ox, oy, angle) − (px, py, pang);  (px, py, pang) = (ox, oy, angle)
```

- 时停期间不推进，`dx,dy,dang` 也不更新；时停结束后第一帧的差值包含时停期间 ECL 做的修改（时停时相位 2 是否运行以现有规则为准）。
- 边界：`warn/active/fade` 的「≥」语义和逐帧状态由单测按 §2 三组真实参数钉死（§8）。
  **见修订**：`warn > 0 && active == 0` 不判定任何一帧；`start > end` 时令 `end = start`。

### 4.2 判定（相位 6 collide，碰撞矩阵新增行 9）

| # | 主动 | 被动 | 主动半径 | 被动半径 | 事件 |
|---|---|---|---|---|---|
| 9 | EnemyLaser（仅 state 1） | PlayerHit | `width/2`（线段盒半高） | `player.hit_radius` | PlayerHitByLaser |
| 10 | Field（`FIELD_CLEAR_BULLETS`） | EnemyLaser（state 0/1） | `field.radius` | `width/2` | LaserCanceled |

新原语 `math::geom::seg_box_dist_sq(px, py, ox, oy, angle, start, end, half) -> i64`：

```
(c, s) = sincos(angle)
dx = px − ox,  dy = py − oy                 // Fx；场地内 |dx|,|dy| < 1024，不溢出
along = dx·c + dy·s,  perp = −dx·s + dy·c   // Fx::mul，c、s ≤ 1，安全
qa = clamp(along, start, end),  qp = clamp(perp, −half, half)
return len_sq(along − qa, perp − qp)        // i64 Q32.32，不开根
```

- 命中条件：`seg_box_dist_sq(...) <= r.raw()²`。矩形钳位后比圆，得到的是圆角矩形，与 th06nc `CalcLaserHitbox` 同构。
- 门控和弹一致：`LIFE_ALIVE && invuln == 0`，`scene_frozen` 时不判。
- 行 9 命中后走和弹一样的 `trigger_player_hit`（相位 7 趟二）。
- 行 10：清弹 field 的圆**碰到**激光线段（取 field 圆心代入原语，r = field.radius），就把 state<2 的激光切到 2、`timer = 0`，
  `fade == 0` 时直接回收（**见 §12 第 11 条**：这是相位 7 取消的口径；ECL `lz_cancel` 跑在相位 2，当帧相位 5 即回收）。符卡结束的全屏 field 因此覆盖全部激光，行为和原作的全局清除一致；局部的 `clear_bullets` 只影响碰到的激光。
- **不做**：擦弹（行 2 的激光版）、清除时沿线掉星、时停的 stop-touch（行 8）对激光的扩展。

### 4.3 资源耗尽与违约（P4）

- 池满：`laser()` 返回 -1，`diag.pool_full[POOL_LASER]` 加 1，不 panic。
- 失效句柄（已回收或代际不符）：所有 `lz_*` 调用什么都不做，计数加 1；`lz_alive` 返回 0。
- 参数越界（负宽度、负长度等）：钳到合法值或不做，并计数。具体规则在计划里逐条列出。
  **见修订**：坐标 ±4096、长度/偏移/速度 `[0, 4096]`、宽度 `[0, 2048]`，一次调用多坏字段只计一次 `contract_viol`。

## 5. ECL 表层语法（新族 8xx）

```ecl
let lz = laser(sprite, color, x, y, angle, len, width, warn, active, fade);
    // 出生时 start = 0、end = len、start_len = len、speed = 0（形态一）；返回打包句柄，池满时返回 -1
lz_speed(lz, speed, start_len);   // 形态二：出生后调用，同时把 end 重置为 0，棒子从原点长出去（见修订：end = start）
lz_start(lz, s);                  // 近端留空（第 4 关的 start = 64）
lz_omega(lz, a);                  // 持续转动（每帧多少 BAM）（见修订：低 16 位按位回绕为 i16）
lz_rotate(lz, a);                 // 一次性转一个角度（原作 88）
lz_aim(lz, off);                  // angle = 指向自机的角度 + off（原作 89）
lz_anchor(lz, enemy, ox, oy);     // 挂到敌人身上；enemy 传 -1 表示解除
lz_origin(lz, x, y);              // 直接设置原点（会解除挂靠）
lz_cancel(lz);                    // state<2 时切到 2（原作 92）
lz_alive(lz) -> int               // 原作 91
```

- **见修订**：`laser()` 实际签名为 `laser(color, x, y, ...)`——**只取 `color`、没有 `sprite` 参数**
  （池 `sprite` 字段存颜色号 0..15）；`lz_speed` 令 `end = start`；`lz_omega` 按位回绕不钳位。
- syscall 号落在新族 `8xx`，按 `docs/ecl-ops.md` 的百分区制登记；`builtins.rs` 是唯一权威，改完重跑 `gen-ecl-meta`。
- 原作 85/86 的区别只是「角度是否相对自机」，转写时写成 `laser(..., aim_player() + a, ...)`，不单独做 `laser_aimed`。
- 原作 87 `laser_index` 与 `laser_clear_all` 只是在维护敌人指针表。有了句柄，脚本把句柄存进局部变量即可，转写时去掉这两条。
- 手册新增一篇激光章节，放进 `docs/ecl-lang/` 的合适位置，内容是教学示例（§2 的三种形态各一个）。所有 ```ecl 代码块都被
  `cargo test -p stg-harness` 真编译。

## 6. 观测：Tier 0 填充 `lasers` 表（proto 不改）

| proto 列 | 来源 |
|---|---|
| `x, y` | `ox, oy` |
| `angle, start, end, start_len, speed` | 原样 |
| `half_h` | `width / 2` |
| `omega` | `dang` 换算为弧度/帧（Fx）。换算常数写成定点常量，不在核里用浮点 |
| `vx, vy` | `dx, dy` |
| `t_active` | state 0 时为 `warn − timer`，否则为 0（见 §12 第 10 条） |
| `state` | 0/1/2（和 th06nc 抽取器同一编号） |
| `type` | 0 |

- state 2 的激光也照发，由特征化一侧决定丢不丢。
- 条数超过 `LASERS_CAP = 64` 时，按 `seg_box_dist_sq(自机, 激光)` 取最近的 64 条，平局按池下标，保证确定性。
- 在 `vec_env.rs` 的写行处新增 `write_lasers`，去掉 `lasers_count.fill(0)`。wheel 升为 `stg_rl` **0.3.0**。
- **见修订**：Tier 0 实际新增了 `lasers` 行缓冲（`vec_env` / `stg-py` / Python `_layout`），此前只有计数；
  `t_active` 的预警态公式保持 `warn − timer`（见 §12 第 10 条）。

## 7. 渲染（Godot，通道 A）

- 新增激光图层，每帧从 `view().lasers()` 读取（新增读口）。
- 每条激光一个四边形：截面贴图横向拉到 `width`、纵向拉到 `end − start`，旋转后加色混合；原点贴一个闪光。
- 预警线（1.2 px，最后 min(warn, 30) 帧线性长到全宽）和收缩（变窄或淡出，看 `flags` 位 0）**全在表现层**根据 `state/timer/warn/fade` 计算，核里不存画面状态。
- 图集补一行截面渐变（16 色，按弹的色列排）。`docs/render-contract.md` 补这一层的说明。
  **见修订：截面渐变先由 shader 程序化生成（16 色表从 bullets 图集采样），图集补图留待办**；
  原点闪光本刀不做。
- 画出来的宽度就是判定宽度（裁定 ④），渐变的暗边也在判定内。

## 8. 测试

行为正确性只能靠单测守（金向量只抓跨平台分歧，CLAUDE.md「金向量闸门的能力边界」）。

1. **`seg_box_dist_sq` 判别式单测**：几何取值能区分半高是 `width/2` 还是 `width/4`、钳位下界是 `start` 还是 0、
   along 和 perp 有没有写反、角度符号对不对。做一次变异检验，逐个改错，确认测试会红。
2. **生命周期逐帧对拍**：用 §2 的 s1 Sub12、s1 Sub22、s2 Sub27 三组参数，断言每一帧的 state、timer、start、end，以及是否判定。
   Sub27 要一直跑到出屏回收。
3. omega 与一次性 rotate 叠加后的 `dang`；挂靠跟随敌人、敌人死后脱钩；`lz_origin` 会解除挂靠。
4. 行 10：全屏 field 清掉全部激光；局部 field 只清碰到的；`fade == 0` 时直接回收（**见 §12 第 11 条**：仅相位 7 取消；ECL `lz_cancel` 当帧回收）。
5. 失效句柄的每个 `lz_*` 都不做且计数；池满时确定性降级。
6. 时停期间不推进、不判定。
7. 快照与校验和往返，`copy_into` 同步，哨兵尺寸测试；`LaserInit` 全覆写宏单测。
8. ECL：手册代码块真编译；8xx syscall 的参数序和返回值。
9. Tier 0：激光行内容和换算正确；超过 64 条时按距离取舍，结果确定。
10. 金向量加一张带激光的场景卡（三种形态都覆盖），三平台对拍。

## 9. 交接（本刀不做，写给下游）

### 9.1 转写流水线（stg-rl-train `transcribe/`）
- mapping.md 补激光对照：**宽度减半**（裁定 ④）；循环里每帧（或每 N 帧）转同一个角度的写法改成 `lz_omega`（每 N 帧转 a，就换成 a/N 的连续 omega）；
  每帧重发的 `laser_offset` 改成 `lz_anchor`；去掉 `laser_index`、`laser_clear_all`；85/86 → `laser(... aim_player() + a ...)`。
- 从 `config.toml` 的 `[skip] instructions` 里去掉激光指令，第一批用 s1_mb2、s1_b3、s2_b4 验证，再转第 4 关和 Extra。
- Extra Sub47 的「每帧重建」写法照原样转（每帧 `laser()` + 短 active），不要合并成一条长激光：原作的判定就是这样。

### 9.2 模型（stg-rl-train）
- 激光 token，8 列：线段上离自机最近的点相对自机的位置（2）、这个点的速度（2，线段中间只算垂直于轴的扫动，端点再加上伸缩速度）、`half_h`（1）、
  轴方向（2）、`t_active`（1）。state 2 的激光丢掉。按当前边缘距离取前 K 条（K ≈ 16）。
- 结构：激光有自己的入口 MLP，加上类型嵌入后和弹 token 一起做自注意力；R2 的动作 query 看全部 token；池化仍按类型分开，再拼进 trunk。
  新增参数零初始化，从 R2 热启动；没有激光时，输出和旧模型在数值上几乎一样。
- 部署对齐：th06nc 抽取器用 8 帧平均窗估计 omega 和 vx/vy，ECL「每 N 帧转一次」会产生锯齿。训练时用同样的平均窗（或加噪声）去模拟，
  这件事在模型侧处理，不在引擎。
- 密度图的激光通道先不做，等 token 版跑出来、确认全局信息不够时再加。
- 不用 `t_left`（裁定 ⑥）。

### 9.3 renkolab
- 补 th06nc 激光专项：NC 的 `SpawnLaserPattern` 字段映射、`laser_cancel` 和 `laser_clear_all` 的语义、各清除路径是否和 CC0 一致、`flags` 其余位。
  这是 §2 里 CC0 结论在 NC 上的核对，不阻塞本刀。

## 10. 后续待办（记进 follow-ups）

- 激光擦弹（th06nc 只在 state 1 判，判定盒向外扩 48 px）。
- 清除时沿线每 32 px 掉一个星或点数道具。
- 曲线激光（TH16/18 的 LaserCurve）：用一串共享时间线的短线段复用本池；proto 用 `type` 标成曲线段。
- 时停 stop-touch（行 8）是否扩展到激光，属于玩法问题。

## 11. 版本与文档

- **ENGINE_VER 23 → 24，本刀只 bump 一次**（新增池、相位里新增推进和判定、新增 syscall 族）。旧 `.stgr` 回放随之失效。
- 文档：`stg-world-design.md`（D8 碰撞矩阵加行 9/10、D10 预算、Part IV §1 标注已落地）；`docs/ecl-ops.md` 的 8xx 族；
  `docs/ecl-lang/` 新增激光章节，并同步 `docs/ecl-lang.md` 索引；`docs/render-contract.md` 的激光层；
  `docs/follow-ups.md` 关闭 D23#10，新增 §10 的条目；`docs/rl-card-pool.md` 第 7 条改写；`PROGRESS.md`。

## 12. 实施中的修订（2026-09-25）

实施（Task 1–6）相对本设计的有意偏离，逐条如下；正文相关处已加「见修订」指针。
分歧时**以本节与代码为准**。

1. **`laser()` 只取 `color`，不取 `sprite`**（修订 §5、§3）。激光只有一种截面贴图，颜色就是
   全部外观；池 `sprite` 字段存颜色号 0..15。理由：给同一个东西两个外观参数是没必要的状态。
2. **截面渐变由 shader 程序化生成，图集补图列为后续待办**（修订 §7）。`laser.gdshader` 用
   `UV.x` 画「中心白芯 → 两侧渐暗」，16 色表从 `bullets` 图集采样（近似占位）。原设计"etama3
   第 146–153 号 8 色截面小方块"需美术补图后才谈；见 follow-ups D27#6。
3. **出生当帧（`born_frame == frame`）改几何时同步 `px/py/pang`，`lz_anchor` 立即吸附**
   （修订 §3「出生时等于初值」）。理由：ECL 在出生当帧就改角度/原点/挂靠时，若 `px/py/pang`
   仍停在初值，相位 5 报出的 `dx/dy/dang` 会把「初值 → 出生帧被改写」的整段跳变当成这一帧的
   位移/转角，污染 Tier 0 观测（首帧尖峰）。`world/laser.rs::sync_prev_if_newborn` 实现。
4. **参数上界与钳位计次**（修订 §4.3）：坐标 `ox/oy/ax/ay ∈ ±4096`、长度/偏移/速度
   `start/end/start_len/speed ∈ [0, 4096]`、宽度 `width ∈ [0, 2048]`（= 2×`MAX_ENTITY_RADIUS`）；
   越界双边钳位并计 `contract_viol`——一次 create/写 API 调用里多个字段越界**只计一次**。
   `create_laser` 与 `lz_*` 写 API 均如此（`world/laser.rs`）。
5. **`lz_omega` 按 BAM 模 65536 按位回绕为 `i16`，不钳位、不计数**（修订 §5）。
   `raw as u16 as i16`：反向扫射编码成 >32767 的原始值也能保持反向；钳位会让大负值变正转而反向。
6. **`lz_speed` 令 `end = start`（不是 0）**（修订 §5）。形态二出生后若已 `lz_start` 设过近端
   留空，棒子应从那一点长出去，`end = 0` 会让线段倒退到原点之前。
7. **`warn > 0 && active == 0` 不判定任何一帧；`start > end` 时令 `end = start`**（修订 §4.1）。
   前者：预警结束当帧直接按生效结束处理（`fade == 0` 则立即回收），绝不让相位 6 多判一帧；
   后者：倒置盒没有意义，归一为空线段。
8. **挂靠敌人被杀的那一帧激光仍跟随**（修订 §4.1）：致死的结算是相位 7、敌人在相位 9 才
   回收，而激光在相位 5 跟随——死亡那一帧敌人还在池里，激光读到的是它最后位置；下一帧
   代际不符（或空槽）才脱钩、原点留在原地。
9. **Tier 0 新增 `lasers` 行缓冲**（修订 §6）：`vec_env` / `stg-py` / Python `_layout` 从
   "只有计数"升级为 64 行定长缓冲（proto 不改）；`stg_rl` 升 0.3.0。
10. **Tier 0 的 `t_active` 保持 `warn − timer`**（值域 `warn..0`），生效/收缩态为 0（修订 §6）。
    观测在帧末采集；agent 的动作在下一步生效，而下一步的相位 5 会先切换状态、相位 6 才判定，
    所以 `t_active == 0` 表示「对下一步已经致命」，与 proto「0 = 现在就杀」一致；`state` 列
    区分预警与生效。这一口径与 th06nc 抽取器公式 `+0x270 − timer` 相同（见 renkolab-sysfix
    `mods/th06nc/autoplay/TARGET.md`）。
11. **清弹 field 取消 vs ECL `lz_cancel` 的回收时机**（修订 §4.2、§8 第 4 条）：原文
    「`fade == 0` 时直接回收」只对**相位 7 的清弹 field 取消**成立——取消发生在相位 5 之后，
    `fade == 0` 的激光要到**下一帧相位 5** 才回收；ECL `lz_cancel` 跑在相位 2（相位 5 之前），
    所以被它取消的 `fade == 0` 激光**当帧相位 5** 就回收。

