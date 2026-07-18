# 技术债与待办清单

> **这是什么**：历次代码复审判定「可延后」的发现，逐条核实后的存活清单。
> **为什么在这**：这些发现原本只活在 `.superpowers/sdd/progress.md`（**git-ignored 的临时账本**），
> 后续者看不到、`git clean -fdx` 一下就没。持久的部分必须入库。
>
> **维护规矩**：解决一条就删一条（别留"已完成"的墓碑，git log 才是历史）。新增的复审 follow-up
> 往这里写，别只写账本。**写之前先核实**——本清单每条都经过代码核对，不是复述当年的复审原文。
>
> 最后核实：2026-07-17（M0-12 道具池 B1 份额核销之后，分支 `m0-12-items`）

---

## A. 有触发条件的（动到对应模块前先做）

> **判据**：某条债一旦满足"下一刀正好要改这块代码，而这块代码没有网"，就升到 A 组、开工前先还。
> bomb 那刀要改 `world/player.rs` 的生死状态机 —— 这正是当初把 GAMEOVER 缺口升到 A 组的理由。

---

## B. 测试覆盖缺口

### B1. P4-a 池满降级：四个写 API 仍零覆盖（道具池份额已还）

`create_bullet` / `create_player_shot` / `create_enemy` / `create_field` 的池满分支
（→ `Handle::NULL` + `diag.pool_full[池id]` + `last_status`）**全无测试**，且金向量也从不触及
（实测稳态：弹 ~375/8192、敌 3/256、field 1/16）。第五个写 API `drop_item`（道具池）的池满分支
已于 M0-12 补测（`step.rs::drop_item_bad_type_and_pool_full`），不再计入本条缺口。

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

### B6. integrate delay 门测试未断言 speed 冻结（accel=0 无判别力）（D3 终审分诊）

delay 门测试只覆盖了 POLAR 弹 `angle`/位置冻结，`accel=0` 时 `speed` 冻不冻结两条路都过——
无判别力。下次动 `integrate` 时补一条 `accel != 0` 的腿，让 speed 冻结成为可判别断言。

### B7. 互斥 debug_assert 无 should_panic 覆盖（D3 终审分诊）

模式位互斥的 debug 断言（若存在类似兜底）没有 `#[should_panic]` 测试触发；需 `pub(crate)`
直写 `flags` 构造出违规状态才能测。`phase_guard` 已有先例（同样是 pub(crate) 直写触发），
可照抄该模式补一条。

### B8. `nearest_aimable_player` 并列取低索引（严格 `<`）无两人等距判别测试（D3 终审分诊）

实现用严格小于（`d2 < bd`）保证并列时取低索引（I4 口径），但没有"两个自机等距"的判别测试
去区分 `<` 与 `<=`。被 co-op 阻塞（`players[1]` 目前恒 `LIFE_ABSENT`），与 B3 同期做。

### B9. `contract_viol` 跨类别双计语义未钉（D4 终审分诊）

同一次调用若同时踩中两类契约违规（例如半径越界 + 变换坏参数），当前实现会各计一次、共 +2。
`clamp_radius` 注释里的"只计一次"约定明说的对象是"同一类别里多个半径字段合并算一次"，没有
覆盖"跨类别是否各自计数"这条轴。现状（跨类别各计一次）站得住，但缺一条测试锁死——不锁的话
下次改动可能悄悄把它并成"整次调用最多计 1"而没人发现是语义变化。**建议**：补一条构造出同一
调用内两类违规同时触发的测试，断言 `contract_viol` 恰 +2。

### B10. 金向量⑨/STEP/LOOP 的"fired-region × LOOP 回跳"角落语义缺测试与文档句（11b 终审分诊）

LOOP 跳回后 `[0, xform_next)` 收缩：活跃 STEP 冻结至重武装、武装反弹弹该窗口 walls 读 0
暂时失效——均确定性且属 spec 字面执行，但缺一条判别式测试与一句设计文档明写这条角落语义。

### B11. 越界 `drop_table` id 分支无直测（M0-12 终审分诊）

`create_enemy` 不验 `drop_table`，调用方可传入越界 id 使其在 settle 趟二触达"视同空表 +
`contract_viol` 计数"的降级分支——P4-b 是宪法级不变量，按纪律该分支应有测试，目前无。

### B12. `credit_item` 的 `lives`/`bombs` u8 进位无饱和（M0-12 终审分诊）

`credit_item` 里 `lives`/`bombs` 字段累加没有饱和上限：刷够 1275 枚生命/炸弹蜡（`u8` 满表下
的极端场景）会在 debug 触发溢出 panic、release 静默回绕——确定性不破（跨平台逐位一致仍成立）
但入账值荒谬。修法：累加改 `saturating_add(1)`，蜡数比较从 `==` 改 `>=` 以抗直写越界。

### B14. `WorldTables::validate()` 角色半径腿只有正向覆盖（M0-17 T1 复审分诊）

`validate_rejects_bad` 的三条负向腿只压 shooter 的 interval/radius/option；角色
`hit_radius`/`graze_radius` 越界的拒绝分支无负向测试（正向由 `tables_v0_validates` 覆盖）。
补一条坏角色半径的拒绝腿即可，与文件加载刀的加载期校验一并做也行。

### B13. 道具近距磁吸 v0 的自机选择与优先裁决序偏离 spec —— 与 B3/B8 co-op 族同批（M0-12 终审分诊）

道具近距磁吸 v0 实现取"升序首个圈内自机"，而非 spec 写的"最近的 ALIVE 自机"——`players[1]`
恒 `LIFE_ABSENT`，两种取法在单机场景下不可观测、无法用当前测试区分。另外 PoC 磁吸与近距圈
两者的优先裁决序 spec 未写明，也是同一刀留下的空白。co-op 出场机制到位（解除 B3/B8 阻塞）后
与它们一并定案；此处先把"spec 偏离"显式记档，避免后续误当 bug 修掉。

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

### C6. `backfill_polar` 的 `isqrt(..) as i32` 理论回绕（D3 终审分诊）

`|v|` 逼近 `Fx` 上限时 `isqrt(len_sq(vx,vy) as u64) as i32` 理论上可回绕为负 `speed`——
确定性无损（跨平台仍逐位一致）、当帧越界回收兜底，纯理论风险。按 P4-c 对称性（引擎自身
bug → debug 帧内断言）补一条 `debug_assert!(sp.raw() >= 0)` 之类的兜底。

### C7. `create_bullet_with_xform` 严格化两件（11b 终审分诊）

STEP `args[1]` 位 24-31 未强制为零（reserved-must-be-zero 前向兼容纪律，未来扩展位若悄悄
非零会被当前实现无声吞掉）；WAIT_SIGNAL 坏通道 / BOUNCE_ARM 坏 n 仅 fire 侧处置（运行期靠
no-op/计数兜底），与 easing id 的 create 期拒收不对称——两条都安全无洞，只是作者体验不一致
（有的坏参 create 时就打回，有的要等 fire 才看见）。

### C8. `slot`/`xf_bullet` 测试助手三处复制（11b 终审分诊）

`step.rs`/`world/transform.rs`/`world/integrate.rs` 各自维护一份结构相同的 `fn slot`
（构造 `XformSlot`）+ `world/transform.rs`/`world/integrate.rs` 各一份 `fn xf_bullet`
（挂变换序列造弹），三处复制。可归拢进 `world.rs` 的 `test_support`（`bullet_at` 已在
那），非阻塞，两可。

### C9. 负 speed 语义全链无测试钉（M0-14 终审分诊）

`speed` 为负（作者直填，或 `speed_step` 为负跨零，或 `ADD_SPEED` 减过头）时
`polar_to_vec` 方向翻转 180°——单发/批量/变换三入口行为一致且确定，但没有任何测试
钉住这个语义。不是 bug（东方语义里"负速=倒飞"甚至有用），但它是接口契约的无声角落：
若未来有人在 `polar_to_vec` 或某入口加"负速钳零"，全套测试仍绿、行为已变。补一条
单测（负速直填 + 批量负步跨零各一断言）即可钉死，M1 ECL 暴露 `create_bullets_batch`
给脚本作者前值得做。

### C10. homing 自机弹单刀设计注记（M0-17 grill 后置）

shottype 表 `Shooter.flags` bit0 已预留 homing。开刀时要拍的唯一悬案是**转向率存放**：
甲案全局常量（所有追踪弹同转向率，零池改动）；乙案 `ShotPool` 加 `turn_rate` 字段
（池布局变更，校验和自动跟上，表达力全）。integrate 加分支走 `nearest_enemy`（现成）。

### C11. WorldTables 文件加载刀的前置三件套（M0-17 遗留占位）

① `content_hash` 字段现恒 0——文件加载落地时做真哈希（A3：与 EclImage 合并进回放头/握手）；
② `World::new` 内引 `TABLES_V0` 喂 spawn（v0 妥协避免百处调用点改签名）——多表时代补
`new_with_tables`；③ 解释器热路径 `timer % interval` 无 interval=0 的 debug 断言（现靠
`validate()` 单测钉 const 表）——外部表可加载后必须加载期强制校验 + 热路径 debug 兜底。

---

## D. 设计层面的已知裂缝

### D1. 自机半径不经写 API —— **M2 表现层接入前应解决**

M0-8 引入 `MAX_ENTITY_RADIUS = 1024px`，在四个 `create_*` 写 API 上双边钳制，使
「六行碰撞的 `(r_active + r_passive)` 裸 i32 Fx 加法不溢出」可证。**但自机侧不走写 API**：
`hit_radius`/`graze_radius` 由 `PlayerState::spawn` 从 `WorldTables::CharacterCfg` 赋值
（M0-17 迁表），上限靠 `WorldTables::validate()` 角色半径腿 + spawn 位等测试钉死
（原 player.rs 编译期断言已随常量迁表退役）。

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

### D4. `define_pool!` 生成的 `alloc`/`free` 是 pub —— 与 `players` 同族问题（D1 姊妹条）

`define_pool!` 生成的 `alloc`/`free` 是 `pub`，和 D1 点名的 `players` 字段 pub 是同一类
"绕写 API"缺口：持 `&mut World` 的外部消费者可以直接 `bullets.alloc()`/`bullets.free()`，
跳过 `create_bullet_with_xform` 等写 API 的契约检查。

**已挡住的一半**：伪造 `transform_head`（例如 `alloc` 后手工填一个越界或指向别处的段号）
本刀已加 P4-b 护栏（`world/transform.rs` 的 `advance_cursor`）——计数 + 序列终止，不 panic。

**未挡住的一半**：直接 `bullets.free(handle)` 释放一颗挂着变换的弹，跳过了
`create_bullet_with_xform` 死亡路径里"弹死还段"那一步——没人再 `xforms.free` 那个段，段
永久泄漏（不是内存不安全，是段池慢性耗尽）。

**修法**：收紧 `alloc`/`free` 可见性，与 D1（`players` 可见性收紧）同期议。
**触发点**：M2（表现层第一次真正持有 `&mut World`）。

---

## E. bomb 那一刀开工前

- **`world/player.rs` 的 `update_players`** 里，`LIFE_DEATHWINDOW` 臂有一句
  `// bomb 救人 stub：本切片无 bomb 输入 → 窗口必耗尽。` —— 那是 M0-7 留的挂点。
- **「被消弹区清掉的弹还算不算 graze？」已答：算**（擦在相位 6 已发生、清弹是相位 7 的事；
  设计明写 graze 独立于中弹）。理由与推导记在 `world/settle.rs` 趟三的注释里 + M0-8 spec。
- bomb 是 `FieldPool` 的**首个真租户**（消弹区已就位，bomb 只需铺一个 `FIELD_CLEAR_BULLETS |
  FIELD_DAMAGE` 的 field）。

---

## F. 长期预留（M0-18 性能审记档，均不动现刀）

### F1. 嵌入式画像三条（单片机移植预留）

基线（`docs/bench-baseline.md`）实证：CPU 不是墙（Cortex-M7 级估 20-40× 慢，真实规模仍 <1ms/帧；
I1 定点+查表+整数 CORDIC 本就是无 FPU 生存姿势，烘焙表天然走 flash），**RAM 才是**
（World 0.92MB > 多数 MCU SRAM）。预留三条：
① **池容量缩编档**：cap 全是 `define_pool!` 编译期常量——弹 2048/xform 512/自机弹 256 档
   估 World ~250-300KB，进 Teensy/ESP32 级；将来做 feature/profile 化，不动架构；
② **`no_std` 触点清单**：stg-core 零外部依赖（防火墙既定），全 crate 唯一 std 触点 =
   `step.rs::World::new` 的堆分配（MCU 上换 static 分配，feature gate）——**新增代码别引入
   新 std 触点**（纪律）；
③ CI aarch64 对拍已证 ARM 整数语义；Cortex-M 同族，金向量上 MCU 对拍是将来最硬的确定性招牌。

### F2. 校验和轻量化——决策窗口在 M3 回放头冻结前

FNV-1a 64 逐字节扫 0.92MB ≈ 1.3-2ms/帧（基线实测最大单项，step 本体的 10-20×）。运行时
本可回避（单机不算、联机 K=20 采样摊薄 ~70µs），故**不紧急**；但若换，窗口在 **M3 回放头/
握手把算法身份冻结之前**（现在换 = 三平台流一起变、零存量破坏；之后换 = bump engine_ver）。
候选：字宽化 mix（wyhash/rapidhash 式 8-16B/步乘法折叠，vendored ~50-80 行，预估 ~10×
到 100-200µs）> vendored xxh64 > 硬件 CRC32（平台可用性参差，仅 32 位）。实现注意：
`#[derive(Checksum)]` 走流式 hasher，字宽化需内部 8B 缓冲；数组元素逐个 `write(4B)` 的
调用开销也在账里，可为 `[i32; N]` 走批量字节路径。

**采样化讨论结论（2026-07-18 grill）**："随机校验"两台机必须采同字节才可比 → 健全形态 =
**确定性分块轮转**（帧号定块，1/16 每帧，16 帧全覆盖，~130µs/帧）。分歧两型：级联型任意
采样 1-2 帧即抓；**休眠型（`facing`/`sprite` 等纯表现字段，P6 特意入哈希）不级联**，检测
延迟 = 覆盖周期。对比既定 K=20 全量采样（摊薄 ~100µs、延迟 ≤20 帧）增益甚微——**不立项**，
仅作"将来要更低分歧延迟"的备选记档。CI/金向量保全量逐帧不动（"首个分歧帧"定位是手术刀）；
版本/mod 不符由 A3 内容哈希在握手拦，不属状态校验和职责。
