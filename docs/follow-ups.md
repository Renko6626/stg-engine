# 技术债与待办清单

> **这是什么**：历次代码复审判定「可延后」的发现，逐条核实后的存活清单。
> **为什么在这**：这些发现原本只活在 `.superpowers/sdd/progress.md`（**git-ignored 的临时账本**），
> 后续者看不到、`git clean -fdx` 一下就没。持久的部分必须入库。
>
> **维护规矩**：解决一条就删一条（别留"已完成"的墓碑，git log 才是历史）。新增的复审 follow-up
> 往这里写，别只写账本。**写之前先核实**——本清单每条都经过代码核对，不是复述当年的复审原文。
>
> 最后核实：2026-07-15（M0-9 + 补测试之后，main `0c9e42f`）

---

## A. 有触发条件的（动到对应模块前先做）

当前为空。M0-9 复审查出的三条裸奔路径（`commit_death` 的 GAMEOVER 分支、敌人越界回收、
integrate 的 `delay` 门）已于 `0c9e42f` 全部补测试并经变异检验。

> **判据**：某条债一旦满足"下一刀正好要改这块代码，而这块代码没有网"，就升到 A 组、开工前先还。
> bomb 那刀要改 `world/player.rs` 的生死状态机 —— 这正是当初把 GAMEOVER 缺口升到 A 组的理由。

---

## B. 测试覆盖缺口

### B1. P4-a 池满降级：四个写 API **一个测试都没有**（最值得补的一条）

`create_bullet` / `create_player_shot` / `create_enemy` / `create_field` 的池满分支
（→ `Handle::NULL` + `diag.pool_full[池id]` + `last_status`）**全无测试**，且金向量也从不触及
（实测稳态：弹 ~375/8192、敌 3/256、field 1/16）。

P4-a 是宪法级不变量（资源耗尽 → 确定性降级不 panic，计数入校验和），却是四个池零覆盖。
**建议**：一刀补齐四个池满测试 + 一个把某池打满的金向量饱和场景（后者顺带验证"两机同序同丢"）。

### B2. `push_event` 的溢出分支无测试

`world.rs` 的 `push_hit` 溢出（`diag.hits_overflow`）有测试；结构同构的 `push_event`
（`diag.events_overflow`）没有。补一个镜像测试即可（置 `events_len = EVENTS_CAP`，push，
断言未增 + 计数 +1）。

### B3. 多自机 graze 位隔离 —— 被 co-op 阻塞

`grazed_by` 是每自机一位的掩码。「P0 擦到的弹不该置 P1 的位」目前无测试，因为
`players[1]` 在所有场景里恒为 `LIFE_ABSENT`。**需 co-op 出场机制才可测**，届时一并做。

### B4. overkill 测试只断言事件数，未断言 hp

`settle_overkill_two_shots_one_death_event` 断言 `events_len == 1`，但没断言敌人 hp
**没被二次扣减**。加一行 assert 即可，能多抓一类回归（dying 门禁被绕过但事件恰好只发一次）。

### B5. 碰撞的边界相等（`d2 == sum²`）无测试 —— **低价值，可能永远不做**

M0-8 最终复审的分诊：比较两侧都是精确的 i64 Q32.32 同域整数，**边界在数学上已经钉死**，
补测试只是钉住 `<=`（含边界即撞）这个**约定**，而非防任何精度风险。列在此仅为存档；
真要做也就是几行，但别把它当"缺口"焦虑。

---

## C. 代码整洁（低优先，都是两可）

### C1. `world/settle.rs:84,95` —— 两 arm 的门禁 2 行逐字重复

`ROW_SHOT_ENEMY` 与 `ROW_FIELD_ENEMY` 两臂各有一份
`if !self.enemies.is_alive(e) || self.enemies.flags[e] & ENEMY_DYING != 0 { continue; }`。
两处调用点、2 行 —— 抽 `fn enemy_targetable(&self, e) -> bool` 属过早。
**第三个伤害源出现时再抽**（复审与实现者两次独立判断一致）。

### C2. `world.rs` 的 `OOB_MARGIN` 提了 `pub(crate)` 但只有一个消费者

`FIELD_HALF_W`/`FIELD_HEIGHT` 确有两个消费者（`player.rs` 的场界钳制 + `cleanup.rs` 的越界判定），
`pub(crate)` 是对的。但 `OOB_MARGIN` 只被 `cleanup.rs` 用。可沉为 `cleanup.rs` 私有，
也可为「D7 场界几何三件套聚在一处」留着 —— **两个选择都站得住，别为它开会**。

### C3. `NUM_PHASES` 在 release 构建触发 `dead_code`

只在 `phase_enter` 的 `#[cfg(debug_assertions)]` 块里用。CI 的 clippy 跑 debug 故不报，
但 `cargo build --release` 会。修法：`#[cfg_attr(not(debug_assertions), allow(dead_code))]`。
**先于 M0-9 存在**，非拆分所致。

### C4. `push_hit` / `push_event` ~6 行结构重复

两个缓冲、不同元素类型、不同 diag 计数器。两处调用点抽象属过早 —— 与 C1 同款判断。

### C5. `world/integrate.rs` 的部分规格住在 `world.rs`

`field_life_one_lives_exactly_one_frame` / `field_life_n_survives_n_frames` 测的是
「integrate 倒数 ↔ cleanup 回收」的跨相位时序，留在了既不拥有相位 5 也不拥有相位 9 的 `world.rs`。
`step.rs` 有 step 级兜底，故低急。真要动就挪到 `cleanup.rs` 或 `step.rs`（后者拥有跨相位顺序）。

---

## D. 设计层面的已知裂缝

### D1. 自机半径不经写 API —— **M2 表现层接入前应解决**

M0-8 引入 `MAX_ENTITY_RADIUS = 1024px`，在四个 `create_*` 写 API 上双边钳制，使
「六行碰撞的 `(r_active + r_passive)` 裸 i32 Fx 加法不溢出」可证。**但自机侧不走写 API**：
`hit_radius`/`graze_radius` 由 `PlayerState::spawn` 从引擎常量赋值，上限靠 `player.rs`
的编译期断言钉死。

问题在于 **`WorldBody.players` 与 `PlayerState` 的字段都是 `pub`** —— 任何持 `&mut World`
的上层（今天的 harness、将来的 godot/py）都能直接写 `players[i].hit_radius = 30000` 绕过一切。
池的 SoA 数组是 `pub(crate)`，所以"只能走写 API"对**池**是类型系统强制的；对**自机**只是前提。

**这不是理论**：M0-9 复审用一个仓库外的探针实测过 —— 它碰池字段编译失败于
`E0616: field 'invuln' of struct 'EnemyPool' is private`，而自机字段畅通无阻。

**修法**：收紧 `players` 可见性 + 提供只读访问器（表现层要读自机状态），或给自机半径也配写 API。
**触发点**：M2 表现层是第一个真正持有 `&mut World` 的外部消费者。

### D2. 设计与代码的名字漂移：`frame_events` vs `events`

`stg-world-design.md` 通篇（16 处）+ `design_doc.md`（2 处）+ CLAUDE.md 的 P6 都叫 **`frame_events`**；
**代码里的字段是 `events`**（`WorldBody.events` / `events_len` / `EVENTS_CAP` / `push_event` /
`diag.events_overflow`）。拿设计文档去 grep `frame_events`，代码里**一个都搜不到**。

`hits` 两边一致；`reqs` 尚未实现（M2）。

**这不只是审美**：A5 那张表刻意用 `hits`（碰撞命中缓冲）对 `frame_events`（世界大事记）来区分两条
缓冲，`frame_events` 里的 "frame" 正是它的生命周期语义。代码的 `events` 丢了这个区分度。

**修法二选一**（都便宜，但要选一个）：
- 代码 `events` → `frame_events`：纯重命名，`events` 本就 checksum-skip，**金向量校验和零影响**。
  波及 `world.rs`/`world/settle.rs`/`world/player.rs` + harness 探针（无）。
- 或反过来把设计改口径为 `events`（但会丢掉与 `hits` 的对照度，不推荐）。

### D3. 金向量导演的补敌逻辑是计数式补位

`main.rs` 的 `slots.iter().skip(alive)` 只按存活**数量**从头跳，不看哪个槽空。幸存者非首槽时
会把新敌人叠在幸存者身上、同时留一个永久空位。**确定且更病态**（两敌重叠更能压碰撞路径），
诊断场景可接受 —— 但若这个场景被复用到位置敏感的用途，先修这里。

---

## E. bomb 那一刀开工前

- **`world/player.rs` 的 `update_players`** 里，`LIFE_DEATHWINDOW` 臂有一句
  `// bomb 救人 stub：本切片无 bomb 输入 → 窗口必耗尽。` —— 那是 M0-7 留的挂点。
- **「被消弹区清掉的弹还算不算 graze？」已答：算**（擦在相位 6 已发生、清弹是相位 7 的事；
  设计明写 graze 独立于中弹）。理由与推导记在 `world/settle.rs` 趟三的注释里 + M0-8 spec。
- bomb 是 `FieldPool` 的**首个真租户**（消弹区已就位，bomb 只需铺一个 `FIELD_CLEAR_BULLETS |
  FIELD_DAMAGE` 的 field）。
