# 弹变换 op 速查表（D4）

> **这是什么**：弹幕作者视角的变换系统参考——op 编号、效果、参数语义一张表。
> **权威来源**（冲突时以它们为准）：`crates/stg-core/src/xform.rs` 常量（编号即契约）·
> `docs/superpowers/specs/2026-07-16-d4-transform-design.md` · `stg-world-design.md` D4。
> **铁律**：op 编号已冻结、随金向量进回放/联机契约——增删改编号 = 过评审 + bump engine_ver
> （CLAUDE.md 自检清单第 3 条）。

## 单位与编码约定

- 每槽 12B：`{ wait: u16, op: u8, _pad: u8, args: [i32; 2] }`。`args` 是裸 i32，每个 op
  自定义语义（I1 之下 Fx 与 i32 同位宽，"整数槽 vs 浮点槽"的区分不存在）。
- **速度/加速度**：Fx raw（Q16.16）——`65536 = 1.0 px/帧`。常用值：`32768`=0.5、`131072`=2.0。
- **角度**：BAM，语义住 args 低 16 位（一圈 65536）。**坐标系 y 向下为正**：
  `0`=右(+x)、`16384`=下(+y)、`32768`=左、`49152`=上。负增量写负 i32 字面量（op 层 `as u16` 回绕）。
- **wait**：发射本 op 后**恰等 `wait` 帧**再执行下一槽；`wait=0` 同帧连发。
- 序列 ≤16 槽；不足部分自动补零 = 天然 `END`。

## op 表

| ID | 名 | args[0] | args[1] | 槽数 | 效果 | 状态 |
<!-- 族号制 v2：十位=族号（1x 速率 / 2x 角度 / 3x 状态 / 4x 连续 / 5x 控制 / 6x 派生 / 7x 预留笛卡尔族） -->
|---|---|---|---|---|---|---|
| 0 | `END` | — | — | 1 | 序列终止（零初始化段天然 END） | ✅ M0-11a |
| 10 | `SET_SPEED` | speed (Fx raw) | — | 1 | 置速率，回填 `vx/vy` | ✅ |
| 11 | `ADD_SPEED` | Δspeed (Fx raw) | — | 1 | 速率增量（可负） | ✅ |
| 12 | `STEP_SPEED` | target (Fx raw) | frames (低16) \| easing id (高8) | **2** | 限时缓动到目标速率 | 🚧 M0-11b |
| 20 | `SET_ANGLE` | angle (BAM) | — | 1 | 置朝向，回填 | ✅ |
| 21 | `TURN` | Δangle (BAM，可负) | — | 1 | 相对转向 | ✅ |
| 22 | `AIM_PLAYER` | Δangle (BAM) | — | 1 | 瞄最近可瞄自机 + 偏移（可瞄 = 非 ABSENT 非 GAMEOVER；无可瞄自机 → 静默 no-op） | ✅ |
| 23 | `STEP_ANGLE` | target (BAM) | 同上 | **2** | 限时缓动到目标角（最短弧） | 🚧 M0-11b |
| 30 | `SET_SPRITE` | sprite id | — | 1 | 换贴图 | ✅ |
| 31 | `SET_LIFE` | 寿命帧 | — | 1 | 重设寿命（"到时自爆"惯用法） | ✅ |
| 40 | `SET_ANG_VEL` | ω (BAM/帧, i16 语义) | — | 1 | 开 `POLAR_FX`（清 CART）——旋转弹 | ✅ |
| 41 | `SET_ACCEL` | a (Fx raw/帧²) | — | 1 | 沿向加速，开 `POLAR_FX`（清 CART） | ✅ |
| 42 | `SET_GRAVITY` | ax (Fx raw/帧²) | ay (Fx raw/帧²) | 1 | 笛卡尔加速，开 `CART_FX`（清 POLAR）——重力/漂移 | ✅ |
| 43 | `STOP_FX` | — | — | 1 | 清两模式位（连续效果全停） | ✅ |
| 50 | `LOOP` | target_slot (0..16) | count | 1 | 游标跳回 target；count 语义见下 | ✅ |
| 51 | `WAIT_SIGNAL` | ch (0..8) | — | 1 | 停驻等信号脉冲（边沿触发，"全场齐转向"） | 🚧 M0-11b |
| 52 | `BOUNCE_ARM` | walls 掩码（低 4 位：左/右/上/下） | n (≤3) | 1 | 反弹待命（场界折返镜像） | 🚧 M0-11b |
| 60 | `SPAWN_PATTERN` | pattern_id | Δangle | 1 | 按图样描述符表发一批子弹 | 📋 预留（随图样表另立一刀） |

槽数 = `1 + ARITY[op]`；双槽 op 的第二槽是引擎 scratch（作者写 0 占位即可）。

## 关键语义（踩过坑的都在这）

- **LOOP 计数**：`count = 0` 无限；`count = N` 循环体**总共执行 N 次**（不是跳 N 次）。
  耗尽后永停 1、外层重入**不复活**——嵌套循环做不到，要复杂控制流去写任务弹（档三）。
  护栏：每帧至多跳一次，`wait=0` 的循环体每帧恰推进一轮。
- **模式位与游标正交**：连续效果（4x 族）是持久开关，游标只负责在排程点开/关/改参。
  "之字 + 加速" = 循环里 TURN + 开局一次 SET_ACCEL——单游标天然并发。
  40/41 互相兼容（同属 POLAR），与 42 互斥（置一清另一）。
- **delay 期**（出现延迟）变换不走、不移动、不判定。
- **失败语义**：创建时 >16 槽或含未知 op → `create_bullet_with_xform` **整体失败**返回
  NULL（宁缺勿哑，弹和段都不产生）；段池满同。运行期撞未知 op / LOOP target 越界 →
  `contract_viol` 计数 + 该弹序列就地终止（弹本体照常飞）。
- **序列终止 ≠ 弹死**：END 之后弹继续按当前状态飞；段随弹死（cleanup）才归还。

## 示例（金向量实况，`stg-harness/src/main.rs`）

**无限之字加速弹**（槽 0/1 只执行一次，LOOP 只圈 2..4）：

```text
[0] SET_SPEED  wait=0   args[65536, 0]     // 1.0 px/帧
[1] SET_ACCEL  wait=0   args[1638, 0]      // 0.025 px/帧²，持久生效
[2] TURN       wait=20  args[16384, 0]     // +90°
[3] TURN       wait=20  args[-16384, 0]    // -90°
[4] LOOP       wait=0   args[2, 0]         // 跳回槽 2，无限
```

**定时自爆弹**：

```text
[0] SET_SPEED  wait=45  args[131072, 0]    // 2.0 px/帧，等 45 帧
[1] SET_LIFE   wait=0   args[1, 0]         // 寿命置 1 → 同帧倒数归零 → 当帧回收
```

## 消费入口

- **世界侧（现在）**：`WorldBody::create_bullet_with_xform(init, &[XformSlot]) -> BulletHandle`。
- **ECL 侧（M1）**：丙方案 locals 区间引用——每槽 3 字，`word0 = (wait << 16) | (op << 8)`、
  `word1/2 = args`；16 槽 = 48 字 ≤ Task.locals[64]。
