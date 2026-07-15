# PROGRESS —— 进度入口

> 「当前走到哪 / 在干什么 / 下一步」的**唯一权威**；CLAUDE.md / README 的状态一律指到这里。
> 细节不进本文：历史细节归 git log 与 `docs/superpowers/plans/`，技术债归
> [`docs/follow-ups.md`](docs/follow-ups.md)。维护规矩见文末。

## 现在（2026-07-15）

- **位置**：Phase 1 · M0 世界层，推进至 **M0-9**（+输入词表注册化 · 容量约定 v2：32 动作位+32 预留）；main 全绿（单测 110 · clippy · 金向量 · 烘焙表全验）。
- **在飞**：无。
- **下一步候选**：bomb（`FieldPool` 首个真租户，闭合自机能力）· 敌人 AI + `move_to` 插值器 ·
  道具池 · 变换系统（D4）· 直上 M1 ECL（世界层 API 已够它调）。
- **待办**：技术债见 [`docs/follow-ups.md`](docs/follow-ups.md)（开工前先读）；最值得的一刀是
  B1——四个 `create_*` 的池满降级分支（宪法级 P4-a）零测试。

## 里程碑史（每条一行，只增不改）

| 日期 | 里程碑 | 一句话 |
|---|---|---|
| 2026-07-15 | M0-9 | world.rs 1593→546 按相位拆成 world/ 五模块；补三条裸奔路径测试；技术债清单入库 |
| 2026-07-15 | M0-8 | `FieldPool` 通用消弹区（碰撞行 6-7 + 结算趟一消弹）；bomb 只差铺一个 field |
| 2026-07-15 | M0-7 | `EnemyPool` + 碰撞矩阵 D8 四行 + 结算 D9 三趟 + 自机生死状态机；金向量扩成碰撞诊断场景 |
| 2026-07-14 | M0-6 | 输入抽象 + 自机移动/发弹 + `ShotPool`；金向量自机边走边打 |
| 2026-07-14 | M0-4/5 | WorldBody + 11 相位 step + PhaseGuard + 整块快照；纯弹幕金向量上线 |
| 2026-07-14 | M0-3 | 池框架 `define_pool!`（存活掩码即分配器，exhaustive `Init`） |
| 2026-07-14 | M0-2 | 字段级校验和 `#[derive(Checksum)]`（stg-derive 防漏，skip 须给理由） |
| 2026-07-14 | M0-1 | 定点数学核：Fx / Angle / 查表三角 / CORDIC / isqrt / easing + 烘焙表纪律 |
| 2026-07-14 | M0-0 | Phase 1 骨架：workspace + 三平台 CI 对拍 + 依赖防火墙 |

## 维护规矩

- **时机**：milestone 合入 main 时**必须**更新（史加一行 + 重写「现在」段）；
  交班时发现「现在」段不符事实，顺手刷新。
- **防膨胀**：「现在」段**重写不追加**、≤10 行；史每条恰一行。想写细节 = 写错了地方
  （细节 → git log / plans，待办细目 → follow-ups.md）。
