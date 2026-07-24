# PROGRESS —— 进度入口

> 「当前走到哪 / 在干什么 / 下一步」的**唯一权威**；CLAUDE.md / README 的状态一律指到这里。
> 细节不进本文：历史细节归 git log 与 `docs/superpowers/plans/`，技术债归
> [`docs/follow-ups.md`](docs/follow-ups.md)。维护规矩见文末。

## 现在（2026-07-24）

- **位置**：**符卡计器机构落地**——ZUN 忠实分工：符卡**记账归引擎**（`SpellSlot`：计时/
  bonus 衰减/破卡血线自动检测/伤害下钳防打穿/miss·bomb 资格作废/超时/结算入分/事件/req/
  结束自动清弹/boss_ui 自动喂逐卡血条），**控制归脚本**（宣言/弹幕/换卡）。模式随卡生死
  （`spell_bound` + **epoch 代际戳**防槽复用 ABA）+ 三 syscall（`spell_begin/end/timer`）+
  `wait_spell()` 语句糖（保留字化）。rainbow 狗粮化成两行范式，`timer_ui` 轮询删。**A2 拍板
  范围修订**（记账/控制分家，评审记录入 spec §1 + stg-world-design A2 注）。金向量三刀双变
  （字段簇取值平移 ×2 + 狗粮化行为变 ×1，非回归——判别式单测 + storm + 跨平台 CI 守）；
  Critical 槽复用 ABA 已修 + 终审独立复现闭合。
- **在飞**：无。两线仍可真并行：godot 线动 `stg-godot`，RL 线动 `stg-py`（符卡机构给两线
  都添了内容/episode 边界：`EVT_SPELL_*` 是 RL episode 正典信号）。
- **下一阶段候选**（开工前先 grill 定序）：**godot 线**——A1 道具 sprite 列 + A2 bench 重跑
  → 建 `stg-godot` crate（WorldBridge + MultiMesh + 分发器）→ 预测未来/回溯/存档机制刀
  （环形调度插件 + 消费 save_bytes）；**RL 线**——建 `stg-py` crate（PyO3 env：reset/step/
  obs/reward，动作 = BTN 掩码，奖励 = frame_events/EVT_SPELL_*，观测 v1 绑定层拷贝）。
- **待办**：技术债见 [`docs/follow-ups.md`](docs/follow-ups.md)（开工前先读；B16 符卡三小件；B14/B15
  storm 小件；D6 剩四软项 M2 顺手）。乙案与文本 DSL 仍是 modding 扩展点。

## 里程碑史（每条一行，只增不改）

| 日期 | 里程碑 | 一句话 |
|---|---|---|
| 2026-07-24 | **符卡计器机构** | 记账归引擎（SpellSlot 计时/衰减/破卡血线/伤害下钳/结算入分/boss_ui 自动喂）+ 模式随卡生死（spell_bound+epoch 防 ABA）+ 三 syscall + wait_spell 糖 + rainbow 狗粮化；A2 范围修订；金向量三刀双变 |
| 2026-07-23 | **存档+风暴闸（L1/L2）** | SaveBytes derive（Checksum 同源防漏）+ 49B 身份头 + save/load_bytes + storm 恢复重演逐位闸；F2 裁决保 FNV；两线共享底座完工 |
| 2026-07-23 | **WS 查看器** | harness serve/dump——单端口 HTTP/WS + 60Hz 推流 + canvas 页 + 线格式 v1 + 回放转储；通道 A/B 首个交互消费者；坑档 bridge-adaptation-notes.md 开档；金向量逐位不变 |
| 2026-07-23 | **外接前收口刀** | 六路系统审阅 → 快照哨兵+七字段拷贝测试 + tasks/rng/frame/events 封口配读口 + spawn_entry* 表守卫 + ENGINE_VER + 场界 pub；审阅发现批量记档；金向量逐位不变 |
| 2026-07-23 | **通道 B anm call** | RenderReq + reqs 缓冲 + emit_req 三层（API/syscall 27/.ecl RawVal 内建）+ take_requests + settle 敌死请求；断层线双出口齐备，M2 前置全清 |
| 2026-07-23 | **通道 A WorldView** | define_pool! 每字段裸切片 + alive_words + WorldView/view() 单入口 + 五池字段收 pub(crate)；销 D5；金向量逐位不变 |
| 2026-07-21 | **刀 A 可见性收口** | players 字段 + define_pool! alloc/free 收 pub(crate) + set_player_power 写 API + players() 只读种子；销 D1/D4；金向量逐位不变 |
| 2026-07-21 | **C11 资产管线** | owned WorldTables + 规范字节 from_bytes/to_bytes + 真 content_hash + compile 绑定表 + start_main coherence 守卫 + join 防迷路 |
| 2026-07-20 | **Named Entry ABI** | 规范排序 SubId/EntryId、singleton main 保护、安全绑定层、可选调试符号侧载 |
| 2026-07-19 | **M1.9** | ECL 表层语言+编译器——三型/具名函数/$变量/值消费检查；风铃卡 .ecl 化狗粮验收 |
| 2026-07-18 | M1.5 | ECL 读口补齐（self_age/self_hp_max）+ globals 系统段脚本写保护（ZUN 变量表对账驱动） |
| 2026-07-18 | **M1** | ECL 栈机 VM + 协程池 + syscall 沙箱 + builder DSL——彩虹风铃卡入金向量二号 |
| 2026-07-18 | M0-18 | bench 子命令 + 性能基线落档——step 曲线/快照/校验和账；ECL 栈机路线调研定案 |
| 2026-07-18 | M0-17 | WorldTables 骨架全家入驻 + shottype 表通电——逐档弹型/子机/focus 表驱动 |
| 2026-07-18 | M0-16 | 火力定标 0.00-4.00（一格 0.01）+ power_tier 档位取值器 |
| 2026-07-18 | M0-15 | globals+boss_ui 三 API（M1 硬前置）+ 消弹一律转星星（30 分经济回流） |
| 2026-07-17 | M0-14 | create_bullets_batch N×K 网格发射器——环/列/多重环一个原语，ECL 性能面就绪 |
| 2026-07-17 | M0-13 | 敌人 move_to 插值器 + nearest_enemy 查询——ECL syscall 面补全，敌界放宽纪律回收 |
| 2026-07-17 | M0-12 | 道具池：掉落/三源磁吸/行5拾取/四类入账——经济线打通，扩展四步清单 |
| 2026-07-16 | M0-11b | 信号黑板/场界反弹/STEP 缓动——D4 十六 op 齐装，A1 四缝清账 |
| 2026-07-16 | M0-11a | D4 段池+游标+12 op：会照剧本演的弹（LOOP 地板语义/wait 勘误/先段后弹） |
| 2026-07-16 | M0-10 | D3 双表示运动模型：POLAR/CART 模式位 + 九 setter 写 API + 1/16 阈值回填契约 |
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
