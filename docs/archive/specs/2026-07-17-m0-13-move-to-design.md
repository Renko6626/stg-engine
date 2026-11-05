# M0-13 敌人 move_to 插值器 + nearest_enemy 查询 —— 设计 spec

> 状态：已过 grill 拍板，待实施计划。上游设计：`stg-world-design.md` D5（`mv_*` 八字段与
> "integrate 敌人趟插值器优先"语义定稿）、D7（nearest_enemy 为 homing/ECL 预定的世界查询助手）。
> 定位：**M1 ECL 前的 API 补全刀**——`move_to` 是杂鱼脚本的第一句话。

## 拍板纪要（grill，2026-07-17）

1. **到点即停**：完成帧写精确终点 + **清零 `vx/vy`** + `mv_active = 0`。插值期**完全优先**
   （无视速度场，不积分 `vx/vy`；`invuln`/`hit_flash` 计时照常）。
2. **敌人越界系统性放宽**：新 `ENEMY_OOB_MARGIN = 256`（x ∈ [−448, +448]、y ∈ [−256, 704]）
   替换敌人回收对共用 64px 边距的引用；弹/自机弹/道具维持 64px 紧边界。**回收主导靠纪律**：
   M0 = 导演/测试显式杀 + 大边界兜底防泄漏；M1 起 = **敌人主协程返回 → 敌人自燃**
   （ZUN ECL 标准语义，与"owner 失效则任务死亡"同绳两头——forward pointer，本刀不实现）。
   无 `mv_active` 豁免特例——mover 与 cleanup 解耦。
3. **homing 缓议**：本刀只落 `nearest_enemy` 查询基建；`flags.HOMING` + 角色模块转向留待
   角色差异化立项。

## move_to 语义（细则）

- **API**：`pub fn move_enemy_to(&mut self, h: EnemyHandle, x: Fx, y: Fx, dur: u16, easing: u8)`
  - 悬垂句柄 → P4-b（`contract_viol` + `STATUS_STALE_HANDLE`，no-op）；
  - `easing >= 8` → `BAD_ARGS` 拒绝 no-op + 计数（与 STEP create 期拒收同款先例）；
  - `dur == 0` → **瞬移**（合法退化不计数：直接写 `x/y`、清 `vx/vy`，不置 `mv_active`）；
  - 生效：`mv_from = 当前位置`、`mv_to = (x,y)`、`mv_t = 0`、`mv_dur = dur`、`mv_easing`、
    `mv_active = 1`。**进行中重下 = 覆盖重启**（from 取当前位置——绝对插值下即 `x/y` 字段现值）。
  - 目标点**不钳制场界**（场外目标 = 飘出退场编排，配合大边界语义）。
- **integrate 敌人趟**（`mv_active != 0` 分支替代 `pos += vel`）：
  `mv_t += 1`；`t = (mv_t << 16) / mv_dur`（i64 整数除法）；`e = ease(easing_from_id(mv_easing), t)`；
  逐轴 `pos = from + e × (to − from)`（**绝对插值**每帧从 from 重算不累积误差——STEP 同款纪律；
  e ≤ 1.0 白名单乘法，i64 中间量）；完成帧（`mv_t == mv_dur`）按拍板①收尾。
- **`easing_from_id` 唯一居所化**：现为 `world/transform.rs` 私有 fn——本刀迁至
  `math::easing::from_id(id: u8) -> Easing`（`pub(crate)`，越界 fallback `Linear` 兜底确定性），
  transform.rs 与 integrate 共用，消一处即将出现的重复。

## nearest_enemy 契约（十条，grill 定稿）

`pub fn nearest_enemy(&self, x: Fx, y: Fx) -> Option<EnemyHandle>`

① 纯查询零副作用（`&self` 类型层保证，任意相位可调）；② 候选 = 存活且非 dying；
③ 度量 = `len_sq` i64 平方距离不开根；④ 并列 = 升序遍历严格 `<` → 低索引（I4）；
⑤ 空集 → `None` 不计数（合法世界状态非违约）；⑥ 返回带 generation 句柄（P1）；
⑦ 无距离上限（限程变体留将来）；⑧ 输出 = World 状态纯函数（回滚一致）；
⑨ 入参任意点不绑自机；⑩ 判别测试四口径：三敌取最近（非圆心重合摆位）/ 等距取低索引 /
dying 被跳过（次近当选）/ 空场 None。

## 大边界实现

`world.rs` 常量区加 `pub(crate) const ENEMY_OOB_MARGIN: i32 = 256;`；`cleanup.rs` 敌人回收
判据改用敌人专用越界函数（弹/shot/item 的 `out_of_bounds`(64px) 不动）；既有测试
`cleanup_frees_out_of_bounds_enemy` 边界值更新（有意识行为变更，理由进 commit）；cleanup.rs
模块文档补"敌人=大边界兜底、主导靠纪律（M1 协程自燃）"一句。

## 金向量与测试

- **金向量就地加戏**：导演补位敌人改为**场外出生（y = −100，旧界必死点）+
  `move_enemy_to` 飘入原位（40 帧 `QuadOut`）**——插值/easing/大边界三条新路径一次入流；
  敌人到位时序改变 = 场景演化变（无基线，预期）。
- **判别式单测**：插值中点与 `ease` 参考逐位等 / 完成帧精确到点 + `vx/vy` 清零 + `mv_active`
  清 / `dur=0` 瞬移 / 进行中重下覆盖（from = 当前位置）/ 悬垂计数 / `easing≥8` 拒 /
  大边界两侧（y=500 存活【旧界必死】、y=720 被收）/ nearest 四口径。
- **变异检验 ≥3 候选**：完成帧不清 `vx`（到点漂移）→ 完成测试红；插值 from/to 对调 →
  中点参考红；nearest 严格 `<` 改 `<=` → 等距低索引测试红。

## 收尾义务

`stg-world-design.md` D5 回写（到点清零 / 敌人大边界 + 纪律回收 forward pointer——注明本 spec）；
`PROGRESS.md` 史行 + 现在段；CLAUDE.md 无结构变化不动；follow-ups 如产生延后项按规矩入库。

## 验收

判别式单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；无新 World 字段
（`mv_*` 八字段 M0-7 起已入校验和/快照——本刀纯通电）。
