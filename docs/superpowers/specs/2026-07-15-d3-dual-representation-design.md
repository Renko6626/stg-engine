# D3 双表示运动模型（弹池物理层）—— 设计 spec

> 状态：已过 grill 拍板，待实施计划。上游设计：`stg-world-design.md` D3（模型/字段/setter 集/模式位
> 均已评审定稿），本 spec 只补齐文档留白的实现决策，不重开已决事项。
> 范围：**只做 D3**。D4（段池/游标/17 op/信号/图样表）明确出局，另行设计讨论。

## 目标

让弹会拐弯：实现双表示运动模型——`vx/vy` 积分真相 + `speed/angle` 作者视图缓存 +
`POLAR_FX`/`CART_FX` 连续效果模式位。弹池六个休眠字段（`speed/angle/ang_vel/accel/ax/ay`）
全部通电。无新字段、无布局变化、无新烘焙表。

## 本刀拍板（grill 纪要，2026-07-15）

1. **D3 单独成刀**（milestone 候选名 M0-10）；D4 依赖它但另行讨论。
2. **setter 双层**：`WorldBody` 公开写 API（handle 版，悬垂 → no-op + `contract_viol` 一次，
   沿用既有"多字段返回值 `||`、一次计数"约定）+ `pub(crate)` 索引核（积分循环与将来 D4 op 消费，
   不付句柄查验税）。公开的动机：金向量导演住 stg-harness（外部 crate），必须公开才压得到
   CORDIC/isqrt 回填路径。
3. **回填规则三条**（进确定性契约，改值须过评审）：
   - `speed` 恒回填（isqrt 反正要算）；
   - `angle` 仅当 `speed >= BACKFILL_MIN_SPEED` 时回填，否则保持旧值（弹近停时冻结朝向）；
   - `BACKFILL_MIN_SPEED = Fx::from_raw(4096)`（1/16 px/帧）。取值理由：东方弹速常态
     0.5~8 px/帧，1/16 px/帧视觉等于静止；4096 raw 作 CORDIC 输入仍有 ~12 位有效信号。
4. **金向量就地加戏**（沿 M0-6→M0-8 惯例，校验和流变化无罪——闸门无 committed 基线）。

## 详细设计

### flags 位分配

| 位 | 名 | 状态 |
|---|---|---|
| 0 | `BULLET_CLEARED` | 已有（M0-8） |
| 1 | `BULLET_POLAR_FX` | 本刀新增 |
| 2 | `BULLET_CART_FX` | 本刀新增 |
| 3-4 | 反弹计数 | 预留（D4 `BOUNCE_ARM`），本刀不动 |

**互斥律**：开 `POLAR_FX` 清 `CART_FX`，反之亦然（TH16 `c68 &= ~0x9` 语义）；`stop_bullet_fx`
清两位。只清位不清字段（`ang_vel/ax` 等留陈值，确定性无损）。debug 帧内断言：两位不得同时置。

### 积分相位升级（`world/integrate.rs`，弹循环）

每颗活弹按序：

1. **delay 门**（不变）：`delay > 0` → 只递减，模式效果与移动全部不走；
2. **`POLAR_FX`**：`angle += ang_vel`（BAM 自然回绕）→ `speed += accel` →
   `(vx,vy) = polar_to_vec(speed, angle)`（乘法安全：speed × 单位三角值，
   符合定点乘法规范"至少一操作数 ≤ ~1.0"白名单）；
3. **`CART_FX`**：`vx += ax; vy += ay` → 回填：`speed = isqrt(len_sq(vx,vy))`，
   `angle` 按阈值规则；
4. `x += vx; y += vy`（不变）；
5. `life` 倒数（不变）。

直进弹（两模式位全零）只多付一次位测试，热路径零回归。`speed/accel` 不钳制——
溢出属 P4-c 域（debug 断言兜底，与坐标积分同权）。

### setter 集（公开 handle 版 → `pub(crate)` 索引核成对）

| 公开 API | 语义 |
|---|---|
| `set_bullet_speed(h, speed)` | 写 speed → `polar_to_vec` 刷 `vx/vy` |
| `set_bullet_angle(h, angle)` | 写 angle → 刷 |
| `turn_bullet(h, delta)` | `angle += delta` → 刷 |
| `aim_bullet_at_player(h, delta)` | 瞄最近**存活**自机（平方距离、并列取低索引，I4）+ delta → 刷；无存活自机 → no-op **不计数**（非违约，世界状态使然） |
| `set_bullet_vel(h, vx, vy)` | 写 `vx/vy` → 按回填规则反推极坐标 |
| `set_bullet_ang_vel(h, w)` | 写 `ang_vel` + 开 `POLAR_FX` 清 `CART_FX` |
| `set_bullet_accel(h, a)` | 写 `accel` + 开 `POLAR_FX` 清 `CART_FX` |
| `set_bullet_gravity(h, ax, ay)` | 写 `ax/ay` + 开 `CART_FX` 清 `POLAR_FX` |
| `stop_bullet_fx(h)` | 清两模式位 |

悬垂句柄：全部 no-op + `contract_viol` 一次。setter 是数据写入，任何持 `&mut World` 的
时机可调（导演槽/将来 D4 相位），不新增相位、PhaseGuard 不涉；模式效果的**消费**固定在积分相位。

### 测试清单（判别式为纲）

1. 螺旋：`ang_vel = ω` 的弹跑 N 帧后 `vx/vy` 与 `polar_to_vec(speed, angle₀ + N·ω)` 逐位相等；
2. 沿向加速：`accel` 弹的逐帧位移递增且与手算参考一致；
3. 重力弹：上抛过顶点 `vy` 符号翻转、`angle` 跟随回填（几何可判对错）；
4. 阈值判别：`set_bullet_vel` 到阈值下 → `angle` 冻结；阈值上 → `angle == atan2` 参考值；
5. 互斥律：开 POLAR 后 CART 位清零、反之；`stop_bullet_fx` 清两位；
6. 悬垂句柄：逐 setter no-op + `contract_viol` +1；
7. delay 门：delay 期 POLAR 弹 `angle/speed/位置` 全冻结；
8. 无存活自机时 `aim` no-op 且不计数；
9. scripted 双跑逐帧 checksum 全等。

**变异检验**（合入前）：对调 `polar_to_vec` 的 sin/cos 或对调两模式位 → 上述测试必须变红，
否则测试是瞎的（M0-7 教训）。

### 金向量扩展（就地加戏，600 帧不变）

导演新增三类压力源：每 40 帧一圈螺旋弹（`POLAR_FX`）· 每 90 帧三颗上抛重力弹
（`CART_FX`，抛物线顶点扫过阈值两侧）· 每 75 帧对**最低索引的存活弹**（确定性选取，I4 口径）
轮换 `turn`/`aim`/`set_vel` 骚扰
——让查表/CORDIC/isqrt 在池 churn + 碰撞消弹的病态环境下跨三平台对拍。

### 收尾义务

- `BACKFILL_MIN_SPEED` 定值回写 `stg-world-design.md` D3（补一行，注明本 spec 出处）；
- `PROGRESS.md`：史加一行 + 重写「现在」段；
- `docs/follow-ups.md` 如产生可延后项按规矩入库。

## 验收

判别式单测全绿且过变异检验；金向量三平台 CI 互比全等；clippy/fmt 零告警；
无新 World 字段（校验和结构不动，流因行为变化而变属预期）。
