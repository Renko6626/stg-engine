# 按局碰撞规则

ENGINE_VER32 起，开局使用 `CollisionRules` 选择 `LEGACY` / `MODERN` 和 `graze_enabled`。默认是 `LEGACY`、擦弹开启。选择独立于角色装备和弹型贴图，开局后不提供动态切换接口。

设距离平方为 d²，对象半径为 b、玩家判定半径为 p：

| 路径 | LEGACY | MODERN |
|---|---|---|
| 普通敌弹命中自机 | d² < (b+p)² | d² < b²+p² |
| 圆形敌人体碰 | d² < (b+p)² | d² < b²+p² |
| 敌弹擦弹 | d² < (b+p+20)² | d² < b²+(p+max(40,b/2.5))² |

全部使用 Q16.16 坐标与 i64 平方计算，不开根。现代大弹的 b/2.5 按 raw×2/5 取整数商；这是原生公式的定点实现，不承诺与原作浮点舍入逐位等价。边界严格小于，与此前引擎的包含等号不同。

`LEGACY` 的圆弹公式以 TH06NC 为参照，`MODERN` 以 TH18 为参照。它们不是覆盖全系列的完整模拟器。原作核查见 renkolab `engine/player/collision-comparison.md`。

## 有意识保留的差异

- 同帧命中与擦弹独立判定，可以同时发生；不复刻 TH18 的互斥返回协议。
- 敌人体不计擦弹，不增加周期计数或单独开关。
- 敌人体沿用现有圆形半径。本次没有新增 TH06NC 的 AABB 尺寸或 TH18 的 OBB 宽高/角度；经典 TH06 的盒形弹判定也未实现。
- 无敌、生命状态、延迟弹及时停仍按现有引擎门禁处理；无敌期间不检测擦弹，未还原原作在此处的区别。
- 激光、自机弹伤敌、消弹区和时停触碰消弹保持各自现有算法，不套用这两种公式。
- 玩家判定半径由角色表及既有写口决定；选择 MODERN 不会自动将半径改成 3。

## 关闭擦弹

`graze_enabled=false` 跳过擦弹阈值计算、碰撞记录、已擦标记与计数，致死判定正常运行。模式与开关在碰撞遍历外分派，循环使用常量参数。

RL 保留观测和事件布局，擦弹增量为 0。`PlayerState.graze_radius` 不再控制上述两档的擦弹公式，但继续作为既有道具拾取范围的一部分；关闭擦弹不会将该值清零，也不会关闭拾取。

## 使用

Python：

```python
env = stg_rl.VecEnv(
    num_envs=16, threads=4, images=images, starts=starts,
    collision_mode="MODERN", graze_enabled=False,
)
```

Rust：使用 `stg_core::collision::{CollisionMode, CollisionRules}`；`World::new_with_rules` / `World::new_game_at_with_rules` 和 `Timeline::new_game_at_with_rules` 接受规则，原有入口使用默认值。RL 的 `EnvConfig.collision_rules` 在每次开局、预热及自动重置时保留。

Godot：`Play` 的 `collision_mode`、`graze_enabled` 导出属性在开局时传给 `WorldBridge.new_game_with_rules`。回放使用日志内的规则，不使用当前宿主选项。

规则进入校验和、快照、存档和回放。回放格式升级至 3；ENGINE_VER32 会拒绝旧引擎存档/回放，旧模型也不能据此视为已验证兼容。表格式和 RL 观测布局不变。
