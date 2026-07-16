# D4 变换系统（段池 + 游标 + 16 op）—— 设计 spec

> 状态：已过 brainstorm 拍板，待实施计划。上游设计：`stg-world-design.md` D4（三档制取舍 / 段池
> 布局 / 槽格式 / 17 op 清单 / 执行算法 / 信号通道均已评审定稿）。本 spec 补齐文档留白的
> 实现决策与本轮讨论新钉的语义细则，不重开已决事项。
> 依赖：D3 双表示（M0-10，已合入）——瞬时/连续 op 全部是 D3 索引核的薄封装。

## 范围与切分（拍板，2026-07-16）

**两刀、同本 spec、各自实施计划，中间过一次合入门：**

- **M0-11a「会排程的弹」**：`XformSegPool` + `create_bullet_with_xform` + 游标执行器
  （相位 4 `run_transforms` 转正）+ **12 op**：瞬时 7（`SET_SPEED/ADD_SPEED/SET_ANGLE/TURN/
  AIM_PLAYER/SET_SPRITE/SET_LIFE`）+ 连续开关 4（`SET_ANG_VEL/SET_ACCEL/SET_GRAVITY/STOP_FX`）
  + `LOOP`。
- **M0-11b「号令 / 弹墙 / 缓动」**：`WAIT_SIGNAL`（+ `signals` 字段 + `pulse_signal` 写 API）、
  `BOUNCE_ARM`（+ 反弹物理）、`STEP_SPEED/STEP_ANGLE`（+ 插值 tick）。
- **`SPAWN_PATTERN` 出局**：依赖图样描述符表（WorldTables），随那一刀另做；op 号预留。
- **cap 照 D10 原值**：2048 段 × 16 槽 × 12B = 384 KB，可接受（拍板）；实测后调参的门保留。

## op 编号（族号制 v2，2026-07-16 重排拍板；十位=族号、族内留空隙；此后改动=过评审+bump engine_ver，CLAUDE.md 自检 3。有效性按表查：`xform.rs::op_implemented`）

| # | op | args[0] | args[1] | 槽数 | 刀 |
|---|---|---|---|---|---|
| 0 | `END` | — | — | 1 | 11a（零初始化天然终止） |
| 10 | `SET_SPEED` | speed(Fx raw) | — | 1 | 11a |
| 11 | `ADD_SPEED` | Δspeed | — | 1 | 11a |
| 12 | `STEP_SPEED` | target(Fx raw) | frames(低16) \| easing id(高8) | **2** | 11b |
| 20 | `SET_ANGLE` | angle(BAM, 低16位) | — | 1 | 11a |
| 21 | `TURN` | Δangle(BAM as i16 语义) | — | 1 | 11a |
| 22 | `AIM_PLAYER` | Δangle | — | 1 | 11a |
| 23 | `STEP_ANGLE` | target(BAM) | 同上 | **2** | 11b |
| 30 | `SET_SPRITE` | sprite id | — | 1 | 11a |
| 31 | `SET_LIFE` | life 帧数 | — | 1 | 11a（含"到时自爆"用法） |
| 40 | `SET_ANG_VEL` | ω(BAM/帧, i16 语义) | — | 1 | 11a |
| 41 | `SET_ACCEL` | a(Fx raw) | — | 1 | 11a |
| 42 | `SET_GRAVITY` | ax(Fx raw) | ay(Fx raw) | 1 | 11a |
| 43 | `STOP_FX` | — | — | 1 | 11a |
| 50 | `LOOP` | target_slot | count | 1 | 11a |
| 51 | `WAIT_SIGNAL` | ch(0..8) | — | 1 | 11b |
| 52 | `BOUNCE_ARM` | walls 掩码(低4位:左/右/上/下) | n(≤3) | 1 | 11b |
| 60 | `SPAWN_PATTERN` | （预留，不实现） | | | 出局 |

`ARITY` 静态表：`STEP_*` = 1（一个扩展槽），其余 = 0；游标步进 = `1 + ARITY[op]`。
瞬时/连续 op 的执行体 = **D3 同名索引核的薄封装**（`set_speed_at` 等，M0-10 全部就位；
`AIM_PLAYER` 复用 `nearest_aimable_player`）。**未知 op**：P4-b——`contract_viol` + 该弹序列
就地终止（视同 END；两机同样跳过）。

## 段池与创建（11a）

- **`XformSegPool`**：手写特例（不套 `define_pool!`）。段 = 唯一分配单位（16 槽整借整还）；
  分配器 = 空闲位图最低空位（I4 同款确定性）；**无逐槽 generation**——段的生命被弹句柄的
  generation 罩住（弹死段亡，无悬垂段访问面）。
- **数据模块住 `crates/stg-core/src/xform.rs`**（与 bullets/enemy 同辈，池即层）；
  **相位逻辑住 `crates/stg-core/src/world/transform.rs`**（镜像相位骨架命名，相位 4）。
- **`create_bullet_with_xform(init: BulletInit, slots: &[XformSlot]) -> BulletHandle`**：
  - `slots.len() > 16` 或含未知 op → **整体失败** `NULL + BAD_ARGS` 计数（宁缺勿哑，与段池满同款）；
  - **分配顺序：先段后弹**——段满 → `NULL + POOL_FULL(XFORM)`（零副作用）；段到手、弹池满 →
    还段（无 generation，还段零损耗）+ `NULL + POOL_FULL(BULLET)`；
  - 成功：`slots` **拷贝**进段（不足 16 槽的尾部保持零 = 天然 END），`init.transform_head = 段号`
    由本函数覆写（调用方传什么都不算数）；`xform_wait = 0`、`xform_next = 0`。
  - 原 `create_bullet` 语义不变（哑弹路径，`transform_head` 恒被覆写为 `0xFFFF`——防调用方
    伪造段号，P4-b）。
- **还段**：cleanup 回收弹时 `transform_head != 0xFFFF` → 同步还段。快照 `copy_into` 全池拷贝。
- **校验和**：全池入（P6 无例外）；手写 `impl Checksum`（遍历全部槽字节，小端）。
  384 KB @60Hz 的 FNV 吞吐无压力；World 总尺寸 ~450KB → ~834KB，D10 记账更新。

## 游标执行器（11a，相位 4）

照 `stg-world-design.md` D4 伪码逐行落：delay 期跳过 → `xform_wait` 倒数 → 循环
{ 越界或 END → 终止；WAIT_SIGNAL 停驻检查；发射 op；LOOP → 跳转并**本帧到此为止**（护栏）；
`xform_wait = slot.wait`；步进 `1 + ARITY[op]`；wait > 0 → 返回 }。
序列终止的规范动作：`transform_head` 保持（段仍占用直到弹死——终止 ≠ 还段，弹还活着，
段是它的 scratch 遗产；简化生命周期，避免"活弹半路还段"的第二条还段路径）。

## LOOP 语义细则（本轮讨论新钉，三条全进实现与文档）

1. **倒数地板是 1**：authored `count = N` 即循环体总共执行 N 次；`count = 0` 即无限。
   fire 逻辑：`0 → 跳（无限）；1 → 不跳（耗尽，永停在 1）；N → 写回 N-1，跳`。
   耗尽态（1）与无限哨兵（0）永不互撞；倒数状态在槽里，随弹快照、回滚安全。
2. **scratch 重初始化纪律**：一切使用 scratch 的 op（当前只有 `STEP_*`）在**发射时无条件
   重新初始化自己的 scratch**——LOOP 重访 = 自动重新武装，上一轮残骸不可见。
3. **耗尽的 LOOP 不被重入复活**（永停 1）：嵌套循环"外层转一圈、内层重转 N 圈"**做不到**，
   属档三任务弹领地（与"任意条件逻辑归档三"同一条既定取舍线）。平铺多个顺序 LOOP 合法。
   `target_slot ≥ 16` → P4-b：`contract_viol` + 序列终止。

## 信号（11b）

- `WorldBody.signals: [u32; 8]` 新字段（derive 自动入校验和/快照；自检清单第 2 条过账）。
- **存 `frame + 1`，0 = 从未脉冲**——World 是 `alloc_zeroed` 构造，裸存帧号会让第 0 帧的弹
  被幽灵放行；`frame` u32 回绕（60Hz ≈ 2.26 年）不设防，记档即可。
- `pulse_signal(ch)` 公开写 API（导演/将来 ECL syscall 共用）；`ch ≥ 8` → P4-b no-op + 计数。
- `WAIT_SIGNAL` 停驻：`signals[ch] == frame + 1` 才放行——**边沿触发**，只有当帧正停驻在该 op
  上的弹响应（"全场齐转向"符卡语义，零额外状态）。

## 反弹（11b，拍板：位置折返镜像 + 全域一致更新）

- **反弹点 = 场界**（±FIELD_HALF_W / 0..FIELD_HEIGHT），不是 OOB 线。
- `BOUNCE_ARM` fire：`args[1]`（n）钳到 ≤3（越界 P4-b 计数），写入 `flags` 位 3-4（剩余次数）。
- **walls 掩码不进弹本体**（`flags` u8 剩余位不够，且弹本体零新增字段是 D4 既定约束）：
  反弹物理**从弹自有段读 walls**——仅对 `flags` 位 3-4 非零的弹，扫描其段的已发射区间找
  `BOUNCE_ARM` 槽（升序首个为准，I4），读 `args[0]` 低 4 位。武装弹必然有段
  （`BOUNCE_ARM` 只能从段里发射），扫描 O(16) 且只发生在武装弹上，成本可忽略。
- **反弹执行位点**：积分相位（5）位移之后、同帧折返；每帧每轴至多反弹一次。
- **折返数学**：越界量镜像（`x' = 2·墙 − x`，不丢超出部分的位移）；速度按当前模式全域一致：
  `POLAR_FX` 弹镜像 `angle`（垂直墙 `HALF − θ`、水平墙 `0 − θ`，BAM 回绕天然正确）后刷 v；
  `CART_FX` 与哑弹翻对应 v 分量后按 D3 阈值规则回填极坐标。`speed` 不变。
- 每次反弹 `flags` 位 3-4 递减；归零后墙对它失效，越界走正常 OOB 回收。

## STEP 插值（11b）

- 主槽：`args[0]` = target；`args[1]` = frames（低 16）| easing id（高 8，复用 M0-1 八条烘焙曲线）。
  `frames == 0` → 视同瞬时 SET（P4-b 免除：合法退化，不计数）。
- 扩展槽 = scratch：`args[0]` = 起点值（fire 时写当前值）；`args[1]` = elapsed（低 16）| 活跃位（bit 31）。
- **tick 位点**：`run_transforms` 对每个带段活弹，先扫已发射区间 `[0, xform_next)` 内的活跃
  STEP 逐个 tick（升序，I4），再走游标——插值与序列推进**并发**（发射即排程完毕，游标可继续
  走后续 op，这正是"效果并发"的既定语义）。tick：`elapsed += 1`；
  `value = start + easing(t) × (target − start)`（easing 归一化 t ≤ 1.0，白名单乘法）；
  写值走 D3 索引核；`elapsed == frames` → 写终值、清活跃位。
- `STEP_ANGLE` 走**最短弧**：`Δ = (target − start) as i16`（BAM 回绕差值天然最短弧带方向）。

## 金向量与测试

- **11a 加戏**：LOOP 之字弹（`TURN ±` 循环 + 开局 `SET_ACCEL`——"之字加速"招牌并发）+
  `SET_LIFE` 自爆弹；**11b 加戏**：`pulse_signal` 全场齐转（停驻弹群一声令下变向）+ 三墙
  反弹弹 + `STEP_SPEED` 缓动弹。双跑对拍照旧。
- **判别式单测为纲** + 每刀合入前**变异检验 ≥3 个**（惯例）。重点判别对象：LOOP 地板语义
  （count=2 恰执行 2 次）、scratch 重初始化（LOOP 内 STEP 第二轮从新起点插值）、信号边沿
  （非停驻弹不响应、次帧不重复放行）、反弹镜像几何（POLAR 弹反弹后 angle == 镜像参考值）、
  段池满/坏参数整体失败（弹数不变 + 计数）、先段后弹的回滚（弹池满时段不泄漏）。
- **段池专项**：分配确定性（最低空位）、还段后重分配复用、快照往返指纹、checksum 敏感性
  （改任意槽任意字节指纹变）。

## 收尾义务（每刀各自过）

- 设计回写 `stg-world-design.md` D4：op 编号表、END=0、LOOP 三细则、`frame+1` 信号编码、
  反弹语义（walls 从段读）、STEP scratch 布局——均注明本 spec 出处；
- `PROGRESS.md` 史行 + 现在段；`docs/follow-ups.md` 延后项入库；CLAUDE.md 仓库结构树加
  `xform.rs` / `world/transform.rs`；D10 内存预算表更新（World ~834KB）。

## 验收

判别式单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；新 World 字段
（`xforms` 池 + `signals`）derive/手写 Checksum 全覆盖 + 快照往返测试；`cargo tree` 防火墙不变。
