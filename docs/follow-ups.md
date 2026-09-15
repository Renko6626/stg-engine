# 技术债与待办清单

> **这是什么**：历次代码复审判定「可延后」的发现，逐条核实后的存活清单。
> **为什么在这**：这些发现原本只活在 `.superpowers/sdd/progress.md`（**git-ignored 的临时账本**），
> 后续者看不到、`git clean -fdx` 一下就没。持久的部分必须入库。
>
> **维护规矩**：解决一条就删一条（别留"已完成"的墓碑，git log 才是历史）。新增的复审 follow-up
> 往这里写，别只写账本。**写之前先核实**——本清单每条都经过代码核对，不是复述当年的复审原文。
>
> **玩法刀（2026-09-14）销 1 条、新记 1 条**：销 **F19**（时停键占 D——停止合并后 X = 停止，
> D/V 退场）；新记 **F25**（2P 同帧双死只认领一条遡行请求）。
>
> **壳子刀（2026-09-11）销 2 条**：**A8**（GAME OVER 已是独立覆盖页 + 续关 `BTN_CONTINUE` 世界内
> 落地、计分按东方惯例，深度流程债兑现）/ **F20**（`effects.clear_after(F)` 按出生帧清理，遡行落地
> 改走它）。**F23** 未动（自机仍单张图，`player.png` 32×32 只有一格，等美术给帧）。
>
> **表现纪律刀（2026-09-11，纯文档）新记 F20–F23**（fx 按出生帧清理 / 敌人退场残影 / 动画时长
> 单一来源 / 自机动画帧 + 三件套命名对齐）；规矩本体在 `render-contract.md` §0 第 6 条与 §0.5。
>
> **M3 时间机制内核刀（2026-09-07）新记 F15–F19**（影子层只有弹 / 跳过帧的 B 请求丢弃口径 /
> `Boot::Snapshot` 的 log 不可从头重放 / 键位 D 临时 / 回放头无脚本身份哈希）；**D20 追第四次**
> （`PlayerState.hit_frame: u32` 被尾部 4 B padding 吃掉）；**A7 追注**（遡行落地 = 一次读档，
> `_sync_anchors` 的第二个真消费者已落地）；余晖拖尾（策划案 7.5）记进 F16 一并。
>
> 最后核实：2026-09-07（表现契约 v2 刀收口：B18/B26 随首次有头目验整条销——`visible_instances`
> 有真值、可玩性四件人眼确认；A9 ⑤ 销；F4 部分销改余项；新记 F14）。
> 上一次：2026-07-26（Godot 场景刀收口：A5 整条销——乙案 `spawn_enemy` task 参/`enemy_hp`
> 已落地，练习模式定式移一句进 `docs/ecl-lang.md`；B18 收窄为独苗（`visible_instances`）；
> B22 销（两处 smoke 脚本已对称修复）；B23 补全另两对实测 UV 配对；B16①③④ 复核触发点仍未到，
> 措辞刷新；新记 A6-A9/B24-B25/D9/F4——场景刀本刀发现的债：`.ecl` 导出 PCK 过滤坑/宿主读档口
> 缺失/GAME_OVER 三态裁定/演出打磨小件四包（以上四条 A 组）、冒烟 `enemy_seen` 判别盲区/
> `spawn_enemy`&`fire` task 号三拒绝支路零测试（以上两条 B 组）、敌"主协程返回即自燃"承诺
> 未落地为代码（D9）、逐帧全缓冲上传性能账未记（F4））
>
> 终审全分支修复波追记（2026-07-26）：A8 措辞刷新（残机耗尽最小处置已落地，深度流程降级
> 待办）；A9 补第⑤件（effects.gd 三处）；F4 数值订正（468.75KB→468KB整=479232B）；新记
> **B26**（B18/B23/DoD 可玩目验三件套合并单，均卡在"首个有 GPU/X 环境"）。
>
> 弹幕颜色轴刀收口追记（2026-07-26）：B23/B26①追注占位图集已改上下不对称（判决程序不变，
> 只是不必再临时加测试格）；C11 追注 `color_stride` 已进表、乙案符号段仍开放；C14 追注
> `APPEARANCE_*` 系 ② 段符号已随本刀退场（`APPEARANCE_MEDIUM`/`APPEARANCE_STAR=3` 两处
> 历史举例订正为当前状态），注入面现为"①⧺②(空)⧺表派生 `BULLET_COLOR_STRIDE`"；新记
> **B27**（`OP_SET_SHAPE` 缺近 `i32::MAX` 溢出判别式测试，与 `OP_SET_COLOR` 不对称）、
> **C22**（`tables.rs::validate` 的 ② join 校验循环随 ② 段清空而空转，机制保留但非活防护）、
> **D10**（部分设运行期不查 `valid`，编译期空格闸只覆盖 `.ecl` 源码路径，直接构造
> `XformSlot` 的未来消费者不受保护）。
>
> 最后核实：2026-07-27（P4 覆盖刀销账：逐条核实后整条删除 **B1**〔四个 `create_*` 池满
> 测试〕/**B2**〔`push_event` 溢出测试〕/**B4**〔overkill hp 断言〕/**B11**〔越界
> `drop_table` 测试〕/**B12**〔`credit_item` `saturating_add`+`>=` 修复与测试〕/**B14**
> 〔`storm --saves 0` 守卫+去重+测试〕/**B25**〔`spawn_enemy`/`fire` task 号支路①③四条
> 测试，`main_task` doc 注释已随该刀补在 `enemy.rs`〕，七条均已在代码里核实落地）。
>
> 最后核实：2026-07-30（**ECL 复刻刀**销账：逐条核实后整条删除 **D9**〔`vm::run_tasks` 的
> `Exec::End` 分支已自燃 + 四条测试 + `enemy.rs`/`world.rs`/`cleanup.rs` 三处注释改口径 +
> demo 杂兵退场点改回场内〕/**B19**〔`SYS_CLEAR_BULLETS`=54 + 表层 `clear_bullets()` +
> 两条测试；语义拍板"直接消 + 消弹转星星"，不做护盾帧——那归 bomb〕/**B20**〔55/56/57 三个
> syscall + `add_lives`/`add_bombs`/`add_power` 三个内建 + 四条测试；形态拍板为**增量**
> `add_*`，`set_*` 不做，绝对赋值归 `Loadout`〕。三条的语义均已写进 `docs/ecl-lang.md`
> 手写节与 `docs/ecl-ops.md` 号表。）
>
> shooter 刀收口追记（2026-07-31）：新记两条，均为**本刀有意不做/有意不对齐**的记档，不是
> 缺陷单——**D13**（ZUN 随机 aimmode `6`/`7`/`8` 不做：消耗世界 RNG，抽取发数与顺序直接进
> 校验和，要单独一份 spec 钉死）、**D14**（`sh_fire` 与 `create_bullets_batch` 的两处口径
> 差异：弹池满不做剩余批量补计 / 退化网格与 appearance 的校验先后序相反——**两处都影响
> 进校验和的值，别"顺手对齐"**）。本刀顺带删掉 `builtins::folds_shape_color`（两个真消费者
> 已转投 `fold_start`，它只剩测试在调且答的是没用的那一半）。
>
> shooter 刀**整支复审修复波**再追一条（2026-07-31）：**C23**（`SHOOTERS_PER_TASK` 没作为
> C14 引擎常量注入 ⇒ 手册与所有脚本硬编码 `0..=3`，而它每个兄弟都是注入的）。同波顺带
> 订正、**不入清单**（已改完，不留墓碑）的四处：`ecl/task.rs` 模块文档的容量数字过时 ·
> `bench-baseline.md` 文件头"贴在旧表上方"与实际的向下追加自相矛盾 ·
> `ecl-ops.md`/`ecl-lang.md`/`xform-ops.md` 三处漏枚举 `sh_fire` 的 **xform 段池**压力 ·
> `ecl-ops.md` 75 号补记 `sh_req` 为何与 `emit_req` 收窄口径不同（钳 vs no-op）。
>
> 敌坐标读口刀收口（2026-07-31）：销掉 C12⑤ 里上一刀（小清洗刀）自己追记的那条小缺口
> ——`SYS_ENEMY_X`(80)/`SYS_ENEMY_Y`(81) + 同名表层内建落地，`nearest_enemy → 探活 →
> 读坐标 → atan2 → fire` 已有一条 `.ecl` 源码级 e2e 押着（`codegen.rs`）。**只删这一句**：
> C12⑤ 原文点名的另两项（`last_status`/`attract_all_items`）与"`SPAWN` owner 来源枚举
> 简化为恒继承"仍未做，条目保留。`ENGINE_VER` 6→7（纯号表理由）。
>
> 敌人死亡效果刀 T1 追记（2026-07-30）：新记 **D11**——掉落从"生成时定死的表索引"迁成
> "敌身上按类型计数的可变状态"后，掉落表的**条目顺序不再影响任何东西**（撒的顺序由
> `spill_drops` 的类型升序决定），而 `validate()` 不要求条目升序 ⇒ 非升序的内容包表会
> **静默**改变 RNG 消耗顺序，无任何检查会红。

> **道具池刀（2026-09-03）销账 1 条**：**F12**——判决程序跑完，**结论与条目里的假设相反**。
> 条目猜的是"长跑 + 零拾取导致池积压（harness 场景性质，无害）"，实测反证：`run` 的 item
> 列在**每个采样点都是 0**，峰值只出现在**帧 4954 那一帧**（509/512），~450 帧内回落到 0
> ⇒ 不是慢慢垫满，是**一帧之内被打满**，与自机吃不吃道具无关（那些格子在"能被吃"之前就
> 已经分配失败了）。真因：帧 4954 是收卡消弹，弹 626 → 0、道具 0 → 509，而**消弹转星星是
> 1:1**（`world/settle.rs` 趟一逐颗调 `spawn_star_at`），弹池 8192 而道具池 512。**四个难度
> 档全部命中**（Easy 3 / Normal 37 / Hard 67 / Lunatic 104）——条目写的"Lunatic 下"只是数字
> 最大的一档。真人局同样会中：那一刻场上有多少弹由脚本和难度决定，不由玩家操作决定。
> 处置由人类拍板取 **1+3**：cap 512 → 1024（`ENGINE_VER` 13→14，World +11 328 B，金向量
> **预期改变**——哈希全槽不用 alive 掩码，多出的 512 个空槽从帧 0 就进哈希）+ 把"消弹转星星
> 是 best-effort、道具池是它的限流器"写成正式口径（`spawn_star_at` 文档 / `ecl-ops.md` 540 号
> / 手册 `6-spell-and-stage.md` 与 `8-errors.md`）。**不取 2**（给转换设上限）——那动的是玩家
> 看得见的经济，为一个罕见边界不值得。顺带补 `star_pool_full_counts_every_missing_star`：
> `spawn_star_at` 文档明写"逐颗计数、循环不短路"而此前无测试，判别力=灌满池后消 3 颗弹须
> 恰好 +3（短路或整批只计一次都给 +1）。**1024 不是结构性保证只是把线挪远**,这句连同
> 溢出后的 P4-a 处置一起进了文档,故 F12 整条删除、不留残余单。

> **瞄准口径统一刀（2026-09-03）销账 1 条**：**F8**——引擎唯一的"瞄谁"口径落成
> `WorldBody::aim_target`（`world/motion.rs`），`aim_player()`(120) 与 `sh_fire`(6xx) 两条
> 改走它，`nearest_aimable_player` 的两个旧消费者（xform op / 弹 setter）一行未改。
> 口径由人类拍板：**只统一"瞄谁"，"一个可瞄的都没有时"按路径性质分**——必须产出角度的
> （查询/发射）回退 1P 最后坐标，能拒绝的（弹上 setter）保持 no-op。**单人局逐位不变**
> （`players[1]` 恒 ABSENT ⇒ 两种口径给出同一个人），金向量 md5 与 base 相同、`ENGINE_VER`
> 不动，修的是 co-op 下"P1 已 game over、P2 还活着时朝尸体喷"。顺带把 **B8** 记的并列取低
> 索引缺口补成判别式测试（等距反向摆位，`<` 写成 `<=` 当场红）——B8 条目随之删除。
> `$player_x`/`$player_y`（010/011）**有意不改**：它是坐标读、不是瞄准原语，恒给 1P，
> 这条已写进 `docs/ecl-ops.md` 与 `docs/ecl-lang/4-bullets.md`。

> **技术债零风险批（2026-09-03）销账 17 条**，逐条在代码里核实过、金向量逐字节不变
> （`17fe7e32…` 与 base 相同）：**A10**〔`wait` 常量越界编译期闸 + 两条判别式测试〕/
> **B6**〔delay 门补 `accel≠0` 腿——原版对 speed 冻结无判别力〕/**B9**〔`contract_viol`
> 跨类别各计一次 + 顺带钉住两条路径验证序相反〕/**B10**〔LOOP 回跳收缩 fired-region ⇒
> walls 读 0 一帧，判别式测试 + `xform-ops.md` 语义句〕/**B27**〔`OP_SET_SHAPE` 近
> `i32::MAX` 对称腿〕/**C3**+**F7**〔相位常量族按 `#[cfg(debug_assertions)]` 整体裁，
> 复核发现是 5 条 warning 不是 1 条〕/**C6**+**C20③**〔两处 `isqrt` 窄化补 P4-c debug 护栏〕/
> **C9**〔负 speed = 倒飞，三入口逐位相反数 + 批量负步跨零〕/**C20①②**〔`Angle::FULL_TURN`
> + 派生 `Ord` 是线性序非环形序的警示〕/**C21**〔池布局账目实测重算 + **池尺寸哨兵**押运，
> 弹池 625KB→433KB、敌池 `~94B`→105B〕/**C23**〔`SHOOTERS_PER_TASK` 进 ① 段注入〕/
> **D2**〔`events` → `frame_events`，设计与代码同名〕/**D11**〔掉落表条目严格升序进
> `validate`〕/**D18**〔`builtins::ENGINE_VARS` 成唯一真相源，`gen-ecl-meta` 加第三个
> sink，手册那张表改成生成段，扩展补上 `$` 补全与 hover〕/**F9**〔`FAULT_NAMES` 进 core，
> harness 不再抄第二份〕/**F10**〔`sh_task` 与 xformdef 差 1 帧并进 `4-bullets.md` 主干〕。
>
> **D19 紧随其后单独一刀销账**（2026-09-03，人类裁定"收窄成拒收"）：五条运动动词的
> `dur`/`easing` 统一走 `try_from`，越界即 P4-b 整条 no-op；`ENGINE_VER` 12→13
> （同一份镜像在新旧两版产出不同世界演化，旧回放必须拒载）。内容侧零改动，金向量实测
> 逐字节不变（两段场景压不到这条新路径）。两条变异实证：改回裸截断 / 拒收改成钳位，
> 各自转红。

> **自机能力刀（2026-09-03）销账**：整节删除 **E. bomb 那一刀开工前**——三条逐条核实
> 已兑现（`try_bomb` 已接 deathbomb 救人路径 / graze 与消弹的口径注释原样保留、未被本刀
> 触碰 / bomb 已成为 `FieldPool` 的首个真租户，铺 `FIELD_CLEAR_BULLETS | FIELD_DAMAGE`
> 两条 field）。本刀新记 **D20**——`world_size_sentinel_guards_copy_into_field_list`
> 这条尺寸哨兵本刀又被对齐 padding 吃掉三次真实字段新增（`freeze_left`/`time_stops`/
> `prev_input`）而未响，`PlayerState` 复审实测只剩 1 字节空档，下一个 `u8` 还会重演；
> 真正逮住这三次改动的全程是金向量而非这条哨兵，候选修法（押字段数 / 押存档 payload
> 长度）记在条目里，留人裁定。道具池第二压力入口（bomb 消弹×120帧 1:1 转星星 + 全屏
> 吸取）已实测：灌到 rank-3 峰值量级（约 814 颗弹）、真起一发 bomb、跑满整段效果时长，
> `diag.pool_full[POOL_ITEM]` 全程为 0（`bomb_at_rank3_peak_bullet_count_does_not_overflow_item_pool`，
> `crates/stg-core/src/world/player.rs`）——1024 的道具池对当前内容仍有约 210 格
> （~20%）余量，**未触发**需要人类裁定 cap 的场景。

---

## A. 有触发条件的（动到对应模块前先做）

> **判据**：某条债一旦满足"下一刀正好要改这块代码，而这块代码没有网"，就升到 A 组、开工前先还。
> bomb 那刀要改 `world/player.rs` 的生死状态机 —— 这正是当初把 GAMEOVER 缺口升到 A 组的理由。

---

### A3. `new_game` 硬绑内建 `TABLES_V0`——owned 表路径到来时补 tables 参数化姊妹入口（前置小刀终审分诊，2026-07-24）

正典 boot `World::new_game`（step.rs）内部走 `World::new` → 内建 `TABLES_V0`，签名不收
`tables`。v1（内建表唯一）自洽；但当 C11 资产管线的 owned 表（`from_bytes` 载盘、
`content_hash` LIVE）成为消费路径时，这些消费者只能退回 `new_with_tables + set_var +
start_main` 三步——"唯一正典入口"的防分歧保证对该路径有洞。（2026-07-25 追注：正典
完全体现为 `new_game_at`，它同样硬绑 `TABLES_V0`——character 越界检查/重 spawn/标记表
跳转全走内建表——本条的 tables 参数化届时应落在 `new_game_at` 身上、`new_game` 链式
委托。）**触发点 = owned 表消费者出现**（C11 资产管线刀 / 任何 mod 表加载）。

### A4. 背景 STD 式 mini-VM——表现层解释器 + 文本格式（整局流程刀 spec §7 记档，2026-07-25）

机器模型已拍板采纳 ZUN STD 式（`goto label @ time` 同时设 ip 和时钟，纯控制流静态、任意帧
状态可解析求出）；本刀只交付 §4 表现锚点四字段契约（`bgm_id`/`bg_id`/`bg_phase`/
`bg_phase_frame`），解释器本体（连同其文本格式）**不做**——它是**表现层资产**：住
`stg-godot`/Godot 侧，可用浮点、不进校验和、不占任务槽，断层线判据同 F1。**触发点 = Godot
场景刀真做背景演出时**：落地时钉死"嫁接 phase 分段保寻位"这条约定——背景脚本按
`bg_phase` 切段，段内 wait/loop/jump 随便，跨段转移只认 `bg_phase` 边沿；寻位配方
`local_t = 世界帧 - bg_phase_frame`，从该段入口重新解析执行到 `local_t`（段是无记忆的，
读档/中段启动后历史丢失也能重建；变长 boss 段 = 段尾无限 loop、phase 切换破环）。详见
`docs/superpowers/specs/2026-07-25-game-flow-midstart-design.md` §7。

### A6. `.ecl` 是非 Godot 原生资源扩展名——导出 PCK 须显式 `include_filter`（场景刀 T7 记档，2026-07-26）

`godot/ecl/game/*.ecl`、`crates/stg-godot/smoke/*.ecl` 对 Godot 编辑器/导出器而言是未注册
资源类型的普通文本文件；`_boot` 早前的内置源回退分支已在 T6 删除（demo 目录已实存，静默
降级判为雷），现在纯靠 `DirAccess.open("res://ecl/game")` 读磁盘文件，读不到就是硬失败（`push_error` +
`return false`），没有任何兜底。本刀只跑 headless/编辑器内路径，从未真正导出过 PCK——
Godot 默认导出规则按已注册资源类型 + 场景引用链收集文件，`.ecl` 两头都不占，大概率被
默认导出规则漏掉。**触发点 = 第一次做 `godot --export` 打包**：导出预设（`export_presets.cfg`）
必须给 `res://ecl/` 显式加 `include_filter`（如 `*.ecl`），否则打出的包在真机上找不到关卡
脚本，且是那种"编辑器里/命令行 `--path` 跑正常，导出包一运行就崩"的隐蔽失败模式。

### A7. 宿主读档口未建——`main.gd` 无 `load_state` 路径（场景刀 T7 记档，2026-07-26）

`bridge.rs` 的 `load_state`/`save_state` 桥面口子（C17）已就位，但 `main.gd` 从未调用
`load_state`——本刀只有 `new_game_at`（含中段启动）一条开局路径，没有"读一份存档继续"的
UI/按键。`_sync_anchors`（双表示规矩：电平追平 hud/bg）目前的唯一调用点是 `_boot` 之后，
职责被 `new_game_at` 路径顺手覆盖了（新开局本就要对表一次）；它真正**不可替代**的场合是
`load_state` 成功之后——读档不像开局，没有 `bgm`/`bg`/`bg_phase` 声明式脚本语句重新跑一遍，
必须靠 `_sync_anchors` 读 `anchors()` 把 HUD/背景电平拉回存档那一帧的状态。**触发点 = 宿主
侧真正要做"继续游戏"/存档菜单时**，届时 `_sync_anchors` 直接复用，缺的只是调用点与 UI。
（追注 2026-09-07：时间机制内核刀的**遡行落地**就是这条路径的第二个真消费者——`main.gd`
`_finish_rewind` 已调 `_sync_anchors()`，证明它在"非开局"场合确实不可替代；`load_state` 的
UI/按键仍未建，本条保留。）

### A9. 演出打磨小件五包——spec 写了、实施未接（场景刀 T7 记档，2026-07-26；终审补第⑤件）

`docs/superpowers/specs/2026-07-25-godot-scene-design.md` §7 的请求分发表比 `main.gd`
`_wire_requests` 实际落地的处理器多写了几笔，均属可玩性不受影响的表现层打磨：
① `REQ_SPELL_RESULT`——spec 写"取得/失败横幅 + bonus 飘字"，`main.gd` 只有横幅，没有
`effects.gd` 式的 bonus 数字飘字；② `REQ_BGM`——spec 写"HUD 曲名标签 + 日志（无音频资产）"，
`main.gd` 只更新了标签，没有日志行；③ `hud.gd::refresh` 的 `p.is_empty()` 早退分支只
`return`，不清空 `boss_bar`/`spell_l`——若某帧 `hud_player()` 返回空（如未开局态被意外调用），
boss 条/符卡行会残留上一次刷新的陈旧值而非归零；④ `hud.gd` 的 `banner`（横幅，位于
`Vector2(120, 200)`，无 `word_wrap`/宽度限制）与右栏 HUD 面板（`x≥424`）之间没有互斥/换行
处理，长文案（比如更长的符卡名/中文结算文案）视觉上可能压到右栏。（原 ⑤ `effects.gd` 三件——`_Ring` 的 free 后重绘、`drain(arr)` 无类型、
重开不清特效——已随表现契约 v2 刀 2026-09-07 重写 `effects.gd`/`dispatcher.gd` 整体销掉。）
余下四条都不阻塞可玩性，**触发点 = 内容与美术期**顺手一并做。

## B. 测试覆盖缺口

### B3. 多自机 graze 位隔离 —— 被 co-op 阻塞

`grazed_by` 是每自机一位的掩码。「P0 擦到的弹不该置 P1 的位」目前无测试，因为
`players[1]` 在所有场景里恒为 `LIFE_ABSENT`。**需 co-op 出场机制才可测**，届时一并做。

### B5. 碰撞的边界相等（`d2 == sum²`）无测试 —— **低价值，可能永远不做**

M0-8 最终复审的分诊：比较两侧都是精确的 i64 Q32.32 同域整数，**边界在数学上已经钉死**，
补测试只是钉住 `<=`（含边界即撞）这个**约定**，而非防任何精度风险。列在此仅为存档；
真要做也就是几行，但别把它当"缺口"焦虑。

### B7. 互斥 debug_assert 无 should_panic 覆盖（D3 终审分诊）

模式位互斥的 debug 断言（若存在类似兜底）没有 `#[should_panic]` 测试触发；需 `pub(crate)`
直写 `flags` 构造出违规状态才能测。`phase_guard` 已有先例（同样是 pub(crate) 直写触发），
可照抄该模式补一条。

### B13. 道具近距磁吸 v0 的自机选择与优先裁决序偏离 spec —— 与 B3 co-op 族同批（M0-12 终审分诊）

道具近距磁吸 v0 实现取"升序首个圈内自机"，而非 spec 写的"最近的 ALIVE 自机"——`players[1]`
恒 `LIFE_ABSENT`，两种取法在单机场景下不可观测、无法用当前测试区分。另外 PoC 磁吸与近距圈
两者的优先裁决序 spec 未写明，也是同一刀留下的空白。co-op 出场机制到位（解除 B3 阻塞）后
与它们一并定案；此处先把"spec 偏离"显式记档，避免后续误当 bug 修掉。

### B15. `storm_short_gate` 是全套件最重单测(~10.6s debug,终审实测知会)

240 帧 × 3 点的短版闸占 harness 测试总时长 ~全部;ARM CI 更慢。暂不动作;若 CI 墙钟成
问题,减帧或拆 `#[ignore]` 全量版 + 轻量常开版。

### B16. 符卡机构三小件（符卡计器刀终审分诊，2026-07-24）

**已还**（2026-07-25 前置债务刀）：② **boss_ui 结算不清致 ≤1 帧陈旧闪**——`settle_one_spell`
（`world.rs`）结算原子里紧随 `spells[slot]` 清零同步清 `boss_ui[slot]`，卡结算后下一卡相位 7
自动喂前不再残留旧卡 active/spell_id/timer 一帧陈旧值。

① **脚本读自机资源 syscall（原 G1）**：`PlayerState` 有 lives/bombs/score/graze 全套但无读
syscall——被符卡机构溶解后从"符卡前置"降为独立小件（bonus 结算引擎付、收卡事件世界产出,
脚本不再需要轮询资源判 miss）。真做花式收卡条件（如"无擦弹收卡"）时再加一族读号。
③ **ZUN 分段衰减曲线**：v1 线性衰减（begin 时整除定格 dec_per_frame）;真做关卡内容嫌糙再
升级为 ZUN 分段（快衰段+慢衰段+地板），参数已隔离在 begin 计算,不动 syscall 接口。
④ **负 threshold 与 ENEMY_DYING 分支**：syscall 已拒 `threshold<0`（脚本到不了负值）;
`hp_break` 的三路 OR 中 ENEMY_DYING 一路对 threshold≥0 实为死码（=0 冗余、>0 被下钳挡）,仅
threshold<0（world API 白盒可达）承重——保留作"非 damage_enemy 死亡路径"的防御,记档非债。

**场景刀复核（2026-07-26）**：①③④触发点均未到——demo 局收卡（风铃卡）走系统默认结算，未写
任何"无擦弹"一类花式收卡条件；符卡衰减曲线仍是 v1 线性，demo 内容量级未觉得"嫌糙"到需要
升级 ZUN 分段；④本非债、性质不变。三条措辞与代码现状一致，未改动。

---

### B21. mark 自动补偿的 `visited` 防环在"同一 sub 被同步调用两次"场景下与纯线性执行序背离（整局流程刀批审 Minor-1，2026-07-25）

`scan_mark_compensation`/`walk_mark_scan`（`crates/stg-ecl-compiler/src/lang/codegen.rs`）
沿 `main` 的同步调用链顶层线性展开时，用 `visited: BTreeSet<&str>` 保证每个 sub 全程**只
走一次**（防环，I4 同款确定性容器纪律）。病态场景：同一个 sub 在 `main` 顶层被同步调用
**两次**，且两次调用之间还有一条顶层锚点声明——例如 `common(); bgm(5); mark(1); common();
mark(2);`（`sub common() { bgm(9); }`）。`visited` 在第一次遇到 `common` 时就把它标记为
已访问，第二次调用点**不会**被重新展开，于是 `mark(2)` 处注入的补偿值仍是第一次展开时
算出的"最新值"（`bgm=5`），而不是"真按顺序回放到这里"会得到的值（`bgm=9`，来自第二次
`common()` 调用）——扫描顺序与**纯线性执行序**背离。影响评估：注入的值不是垃圾/未定义
状态，是脚本里确实声明过的某个值（只是不一定是最近那次）；同一份脚本永远编译出同一张
标记表，确定性/回放/跨平台一致性不受影响；真实整局脚本几乎不会把同一个 sub 在同一条同步
调用链里连续调用两次（ZUN 式关卡是"顺序调不同的关卡 sub"，不是"重复调同一个"）。**修法
方向**：把 `visited` 的粒度从"全局只访问一次"收紧为"按调用点/调用路径重扫"，让同一 sub
在不同调用点各自贡献一次"最新值"快照——真实脚本出现这种写法、或想让补偿更贴近直觉时再做。

### B23. ~~`layer.gdshader` 图集选格 UV 垂直朝向嫌疑~~ —— **已判决：不镜像，无需修改**（2026-07-27）

**判决结论**：真美术上屏后画面正常，UV 朝向没有问题，`layer.gdshader::fragment()` 的
`uv = (cell + UV) / vec2(grid_cols, grid_rows)` 保持原样，**候选修法（`1.0 - UV.y`）不要采纳**。

**证据**：颜色轴刀接入原作弹片（`godot/assets/bullets.png`，12 弹型 × 16 色）后，格内图元
本身就是强上下不对称的——arrowhead 有朝向、kunai 是三角带**下方**小圆、laserhead 是端头
圆帽。作者在有 GPU/X 的环境跑了真工程，画面无异常。若 shader 真把每格上下镜像，这批图元
会立刻显出"苦无的圆跑到上面去了"一类的错位，不可能看不出来。这比原先设计的判决程序
（往占位图集塞一张上红下蓝的测试格）覆盖面更大——整屏所有弹型同时受检。

**留档理由**：原条目记录的静态观测（`QuadMesh.get_mesh_arrays()` 读出的顶点/UV 配对看似
会导致镜像）本身没错，错的是由它外推到"最终采样结果"——中间的 NDC/视口变换/`CanvasItem`
坐标系翻转把它抵消了。**教训**：CPU 侧网格数据不能直接外推 GPU 光栅化结果，这类问题只能
上真渲染器判。

### B24. 冒烟①`enemy_seen` 判别不辨 boss/杂兵——负控实证（demo 局刀 T6 复审残余缝隙，2026-07-26）

`main.gd::_run_smoke` 的 `enemy_seen` 断言（逐帧扫 `puppets()` 木偶喂料找非默认位置；表现契约 v2 起敌层退役，此前扫 `LAYER_ENEMIES` 缓冲）
证明"敌真的在动"，但不区分动的是 `stage1` 杂兵还是 boss——**负控制实测**：临时把
`main.ecl` 整段替换成 `sub main() { bgm(1); loop { wait(600); } }`（不调 `stage1()`/
`boss_battle()`，即"main 只剩 bgm+loop"）会被真实抓到（`SMOKE FAIL`）；但若只清空
`stage1()` 内部逻辑（`zako_dive` 全删）而保留 `main` 的调用序，`boss_battle()` 几乎立即
执行，boss 出生点 `y=96.0` 本身就让 `enemy_seen` 命中，判别面被 boss "顶包"，这类更典型
的"只删杂兵波次"回归**不会**被当前断言捕获（换个说法：只要 `boss_battle` 仍在 240 帧窗口
内被摸到，`enemy_seen` 就不关心杂兵段死活了）。修法方向：加一条按坐标排除 boss 的判据
（如 `|ox|>40`——demo boss 固定 `x=0` 出生，杂兵三波在场界左右两侧出生/俯冲）或改按
敌实例数量（杂兵波次期望瞬时敌数 >1，boss 单敌）判别。**触发点 = 下次真要收紧这条冒烟
断言，或者杂兵/boss 内容有实质变化时**（权衡：复杂度 vs 真实手误极少"连节奏一起清空"，
当前不算阻塞）。

### B28. 擦弹特效未接——事件化须先定聚合口径（命中事件刀记档，2026-07-27）

自机弹命中已走 `EVT_SHOT_HIT_ENEMY`（逐命中发，见 `render-contract.md` §3.5），**擦弹刻意
没跟着做**：擦弹频率高一个量级（满屏弹幕贴身可以每帧上百次），逐条发必撑爆 `EVENTS_CAP=512`
——与 `EVT_FIELD_CLEARED` 当初被迫聚合是同一个约束。真要做时的选项：①每自机每帧聚合一条
（带本帧擦弹数，表现层自己撒火花）；②不发事件，宿主从 `hud_player` 的 graze 计数做帧间差分
（零引擎改动，但只有计数没有位置）。**触发点 = 真要做擦弹演出时**，届时先定口径再动手。

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

### C4. `push_hit` / `push_event` ~6 行结构重复

两个缓冲、不同元素类型、不同 diag 计数器。两处调用点抽象属过早 —— 与 C1 同款判断。

### C5. `world/integrate.rs` 的部分规格住在 `world.rs`

`field_life_one_lives_exactly_one_frame` / `field_life_n_survives_n_frames` 测的是
「integrate 倒数 ↔ cleanup 回收」的跨相位时序，留在了既不拥有相位 5 也不拥有相位 9 的 `world.rs`。
`step.rs` 有 step 级兜底，故低急。真要动就挪到 `cleanup.rs` 或 `step.rs`（后者拥有跨相位顺序）。

### C7. `create_bullet_with_xform` 严格化两件（11b 终审分诊）

STEP `args[1]` 位 24-31 未强制为零（reserved-must-be-zero 前向兼容纪律，未来扩展位若悄悄
非零会被当前实现无声吞掉）；WAIT_SIGNAL 坏通道 / BOUNCE_ARM 坏 n 仅 fire 侧处置（运行期靠
no-op/计数兜底），与 easing id 的 create 期拒收不对称——两条都安全无洞，只是作者体验不一致
（有的坏参 create 时就打回，有的要等 fire 才看见）。

同族补两对（2026-07-23 系统审阅）：`SYS_CREATE_BULLET*` 的 `appearance` 越界 → Fault，而
`SYS_DROP_ITEM` 的 `item_type` 越界 → 静默 NULL+计数；`batch` 负计数 `max(0)` 钳零不计数，
而 `xform_cnt` 负数 → Fault。同为"坏枚举/负参"，脚本作者遇到的响度不可预测——归并本条一起裁。

### C8. `slot`/`xf_bullet` 测试助手三处复制（11b 终审分诊）

`step.rs`/`world/transform.rs`/`world/integrate.rs` 各自维护一份结构相同的 `fn slot`
（构造 `XformSlot`）+ `world/transform.rs`/`world/integrate.rs` 各一份 `fn xf_bullet`
（挂变换序列造弹），三处复制。可归拢进 `world.rs` 的 `test_support`（`bullet_at` 已在
那），非阻塞，两可。

### C10. homing 自机弹单刀设计注记（M0-17 grill 后置）

shottype 表 `Shooter.flags` bit0 已预留 homing。开刀时要拍的唯一悬案是**转向率存放**：
甲案全局常量（所有追踪弹同转向率，零池改动）；乙案 `ShotPool` 加 `turn_rate` 字段
（池布局变更，校验和自动跟上，表达力全）。integrate 加分支走 `nearest_enemy`（现成）。

### C11. WorldTables 资产管线（已还，2026-07-21）

**已还**：`WorldTables` owned 化（内部 `&'static` 切片 → `Box`，`TABLES_V0` 为
`LazyLock`）；规范字节格式 `to_bytes`/`from_bytes`（i32/u16 小端，无 float，`TableLoadError`）
+ 真 `content_hash`（vendored FNV-1a64，加载时自校验）；`tables_v0.bin`（840B）由 harness
从 `build_tables_v0().to_bytes()` 烘焙、committed，`verify-tables` 逐位对拍，`stg-core` 经
`include_bytes!` + `from_bytes` 加载，`content_hash` 现为 LIVE 值（`0x3ac258d4031d2ced`）；
`compile_for_table(src, file, &WorldTables)` 把表 `content_hash` 焊进 `EclImage`
（`compile` 委派绑定 `TABLES_V0`，签名不变）；`World.tables_hash`（`new_with_tables` 记录，
入校验和，随快照复制）+ `start_main` coherence 守卫（`image.content_hash != tables_hash` 时
拒绝，`TaskStartError::TableImageMismatch`，任一侧为 0 视为未绑定放行，只在启动时查一次）；
harness 端到端自证：从磁盘加载 `tables_v0.bin` 跑一遍（挂弹+移动），校验和与内建路径一致。

**未挡住的口子**：
- `WorldTables::from_bytes` 按**文件里的计数**直接 `Vec::with_capacity(count)`——对**可信**表安全（body
  哈希先验，损坏 → `HashMismatch`；唯二调用者是 committed `tables_v0.bin` 与 harness 对它的测试）。但
  **蓄意构造**的文件（自洽 hash + `count = u32::MAX`）会在读循环撞 `Truncated` 之前触发数 GB 预分配 →
  OOM abort，一条非确定性 panic 路径（与 P4-a 张力）。**加载不可信 mod `.bin` 之前必补**：分配前用
  剩余缓冲长度给每个 count 封顶（终审 2026-07-21 分诊）。

**仍留给未来（modding，非本刀范围）**：
- **`EclImage` 离线序列化格式**（2026-07-23 审阅拍板记档）：M2 走"启动时编译 `.ecl` 源码"
  （harness 同款，`compile_for_table`）；镜像字节格式**不冻结**，离线分发/加载等 modding
  需求真出现再定——过早冻结 = 白背一份格式兼容债。
- **乙案**——表自带 `[(name,id)]` 符号段，让非 Rust/mod 作者自定义外观词表，v1 仍用
  `consts.rs` 里手写的 ② 符号名字。**追注（颜色轴刀，2026-07-26）**：表已带 `color_stride`
  字段（`WorldTables.color_stride`），乙案的**符号段**本身仍未落地——`consts.rs` 的 ②
  段现已清空，弹型名/色名改由内容包自己的 `.ecl` 用 `const` 声明（见 C14、
  `docs/ecl-lang.md`"引擎常量"节），mod 作者靠这条路径已经能给外观取名，乙案不是
  "唯一出路"了；乙案真落地后，这条 `const` 声明的路径可以退位给表自带的机读符号段。
- **文本 DSL** 给表本身当作者格式——v1 仍是 Rust 里建 + 烘焙字节，无文本源。
- 多角色 / 多道具类型表（v1 固定 `characters` 长度 1、`item_cfg` 长度 `ITEM_TYPE_COUNT`）。

### C12. ECL 后续小件四包（M1 T5 分诊）

① `spawn_task_now`（当帧 worklist drain 版）——design_doc §4.3 既定后期可选，真实用例窄；
② 帧相对寻址指令族（sub 可重入）——locals 共享是 v0 拍板，重入需求出现时纯增量加；
③ vm.rs 顶部 `#![allow(dead_code)]` 毯（T1 遗留）与 `run_tasks` 的 256 槽逐位扫描措辞
   （复审 Minor：正确性无碍，可改按字跳空；`KILL_CHILDREN` 同款 256 槽扫、预算只计 1 条
   ——终审 Minor，病态脚本群最坏 ~16.8M 探测/帧，纯性能项）；④ 敌 appearance 表
   （`spawn_enemy` syscall 现走直参，弹的 appearance 表已建——对称化留内容需要时）；
⑤ spec syscall 清单三项**有意未入号表 v1**（终审 Minor 补记）：
   `last_status`/~~`nearest_enemy`~~/`attract_all_items`（号表 v2 候补），`SPAWN` 的
   "owner 来源枚举操作数"简化为恒继承——现实现更简且够用，记录在案防"悄悄丢"。
   **`nearest_enemy` 已还**（小清洗刀，2026-07-31）：`SYS_NEAREST_ENEMY`(79) + 同名表层
   内建，世界侧 `world::nearest_enemy`（M0-13 起的死代码）一行未改，只是通了电。**剩两项
   （`last_status`/`attract_all_items`）仍是候补**。

### C13. ECL 表层语言（M1.9 已落地）——销账与遗留

**已还**（M1.9，2026-07-19）：①表达式即参数 ✔（类型化内建 + 运行时表达式直通）；
②值消费静态检查 ✔（`_ =` 显式丢弃）；③repeat 限制 ✔（真 `for`/`while`/`loop`）；
④单位字面量 ✔（`1.5fx`/`90deg`/`bam`）；⑥引擎状态变量化 ✔（`$` 前缀，随机数变量
仍拒绝）。**Named entry ABI**（2026-07-20）：⑧强制 `sub main()` + singleton 生命周期 ✔；
⑨async sub 自动注册为公共 named entry ✔；⑩普通 sub 为 CallOnly（不注册 entry）✔；
⑪CALL/SPAWN 操作数为规范 `SubId`，运行时执行 SubKind 检查 ✔；
⑫安全绑定层（`start_main`/`spawn_entry`/`spawn_entry_named`）替代裸 `spawn_task` ✔；
⑬编译器可选调试符号侧载（`DebugInfo::Full`），`EclImage` 本身不变 ✔。
**遗留**：⑤时间标签 `+N:` 糖（未进 v1，用户拍板显式 wait；备胎保留）；
⑦`jnz` 收编（纯密度优化，降低模板实证不需要）。

### C14. .ecl 跨语言常量引用缺失（M1.9 T4 复审分诊，三 Minor 共同根因）

**已还**（常量注入半，2026-07-21）：`.ecl` 源码现可引用 Rust 侧命名常量——`stg-core`
`engine_consts!` 注册表（`crates/stg-core/src/consts.rs`）对每行同时生成 Rust `pub const`
与注入表 `ENGINE_CONSTS`，编译器 `lang::compile` 默认把它们当"第 1 行前预声明的 const"
注入类型检查命名空间（脚本重声明同名报错"与引擎常量重名，不能重新声明"，见
`typeck/consts.rs`）。原先三份并行的 const 求值/运算规则（typeck 内联折叠、codegen
`eval_const_arg`、`typeck::matrix` 的 `mulf_const`/`divf_const` 等）收编成 `lang::const_eval`
（求值）+ `lang::type_rules`（运算矩阵/cast 白名单），旧 `typeck/matrix.rs`、
`typeck/intents.rs` 已删。金向量场景 `rainbow.ecl` 去魔数示范：风铃摆环固定外观
`fire(1, ...)` → `fire(APPEARANCE_MEDIUM, ...)`（`var appearance = i % 4` 那类有意轮转
全表的写法保留不换）；校验和不变（纯换书写不换值，两次 golden 对拍 + 编辑前后对拍均逐位
相同）。详见 `docs/ecl-lang.md`"引擎常量"节、
`docs/superpowers/specs/2026-07-21-ecl-const-injection-design.md`。**追注（颜色轴刀，
2026-07-26）**：`APPEARANCE_MEDIUM` 一类 ② 段符号已随颜色轴刀退场——`fire`/`batch`
现收两参（`shape`/`color`），`rainbow.ecl` 已改写成 `fire(OUTLINE, COLOR_CYAN, ...)`
这类调用，`OUTLINE`/`COLOR_CYAN` 是脚本自己用 `const` 声明的内容包词汇（真美术接入后
词表已换成 LASER/ARROWHEAD/OUTLINE/BALL/… 十二行，见 `godot/ecl/game/bullets.ecl`），不再是
Rust 侧注入的引擎常量；上面这句"去魔数示范"描述的是它当时（2026-07-21）的写法，读到本条
时若查 `consts.rs` 找不到 `APPEARANCE_MEDIUM`，不是回归，是这条追注记录的迁移。

**coherence 不变量**（本刀记档，留 C11 焊死）：注入的引擎常量在字节码里落为**字面量值**
（非名字），故存在一条语义前提——**同一份常量来源必须同时喂"编译期注入"与"运行期查表"**。
当前 v0：`ENGINE_CONSTS` 由 `consts.rs` 单一权威生成，`WorldTables`（v0 静态 `TABLES_V0`）
与之同源（如 `APPEARANCE_STAR=3` 既是注入给编译器的值、也是 `TABLES_V0.appearances` 的
索引），二者天然一致，靠的是"同一个宏调用/同一份源文件"这条结构性保证，不是机制性校验。
**C11 表文件加载落地后风险浮现**：若注入常量取自表 A、却拿表 B 跑，appearance id 可能
错位且**三平台一致地错**——金向量闸门只抓跨平台分歧、抓不了这种"一致地错"（见 CLAUDE.md
"金向量闸门的能力边界"）。Spec 2 拍板令 `EclImage`/`WorldTables` 记录各自的 `content_hash`，
运行期比对拦截。**追注（颜色轴刀，2026-07-26）**：`APPEARANCE_STAR=3` 这个举例已过期——
② 段现是空切片，`consts.rs` 里不再有任何 appearance 符号。同一条 coherence 责任现由
表派生常量 `BULLET_COLOR_STRIDE` 承担（`compile_with_options` 里 `i32::from(t.color_stride)`
直接从绑定的 `WorldTables` 读值），注入面因此变成"①（结构常量）⧺②（表符号词汇，现空）
⧺ 表派生一项"；`BULLET_COLOR_STRIDE` 比旧例子更强——它的值直接来自同一个 `WorldTables`
实例本身，不是"同一个宏调用"这种结构性巧合，`content_hash` coherence 守卫覆盖的正是
这条穿线关系换表就换值。

**C11 已焊死**（2026-07-21）：`compile_for_table` 把绑定的 `WorldTables.content_hash` 盖进
编译产物 `EclImage.content_hash`；`start_main` 拿它与 `World.tables_hash` 比对，不符即拒绝
（`TaskStartError::TableImageMismatch`）。见 follow-ups C11 条。

**仍开放**：T1 复审四 Minor（parser 无递归深度护栏/链式比较左结合未钉/`@70000` wait
越界测试缺/`i32::MIN` 字面量不可拼写）——与常量注入无关，未在本刀清。

### C17. 桥壳四处打磨(桥刀终审分诊,2026-07-24;整局流程刀追一项,2026-07-25)

**已还**(2026-07-25 前置债务刀):②`save_state()` 未开局分支已补 `warn_once`(`W_NO_GAME`),
与 `load_state` 对称,GDScript 侧现可靠 warning 区分"空存档/未开局";③`hud_boss` 已改走
`g.world.view().boss_ui()`(`WorldView::boss_ui()`),与 `hud_player`/`hud_spell` 读口一致。

`bridge.rs`:①`ping()` 测试助手留在生产冻结面(无害欠整洁);④`new_game_at`(bridge.rs)的
loadout 参数是 `character`/`power`/`lives`/`bombs` 四个平铺标量,非 Dictionary——v1 装备维度
少(四件)尚可承受,将来若长出更多装备维度(如子机类型/初始道具)时考虑 Dictionary 化。下次动
壳时与①一并顺手打磨。

### C22. `tables.rs::validate` 的 ② join 校验循环现空转、无测试覆盖（颜色轴刀 T4 清空 ② 段的残余，2026-07-26）

`WorldTables::validate()` 里 `for c in crate::consts::TABLE_SYMBOLS { if (c.value as usize)
>= self.appearances.len() { return false; } }`（`tables.rs:316-320`）是 C14 记的那道
"② 表符号必须落在 appearances 合法行内"的 FM1 防线；颜色轴刀把 `consts.rs` 的 ② 段清空
（弹型名归内容包，见 C14/C11 追注）后，`TABLE_SYMBOLS` 是空切片，这个循环体永远不执行——
机制还在，但当前不是一条活防护。原有两条覆盖它的测试
（`validate_rejects_table_symbol_without_appearance_row`/
`builtin_appearances_exactly_cover_table_symbols`）已随 ② 段清空一并删除（`tables.rs`
`mod tests` 里留了说明注释）。复审判定**保留循环本身可接受**——② 段将来重新长出行（如
道具类型符号）时机制自动生效，删掉它反而是白扔一次未来要重写的代码——但记档防止将来
复审者看到"零覆盖的校验逻辑"误判为遗留 bug 想删掉它。**触发点 = ② 段再长出符号**
（如道具类型符号表落地）时，记得给这条 join 校验重新配一对正/负测试，别让它继续裸奔。

## D. 设计层面的已知裂缝

### D3. 金向量导演的补敌逻辑是计数式补位

`main.rs` 的 `slots.iter().skip(alive)` 只按存活**数量**从头跳，不看哪个槽空。幸存者非首槽时
会把新敌人叠在幸存者身上、同时留一个永久空位。**确定且更病态**（两敌重叠更能压碰撞路径），
诊断场景可接受 —— 但若这个场景被复用到位置敏感的用途，先修这里。

### D7. 池 generation u16 回绕的理论 ABA（2026-07-23 系统审阅新记）

`define_pool!` 的 `generation` 是 u16、`first_free()` 恒取最低空位——池内最热槽必是低索引，
65536 次复用后回绕，**跨帧长期持有句柄**的外部消费者理论上可撞句柄别名（旧句柄"复活"指向
新实体）。全仓现无触发路径（模拟内句柄短命,`Handle::NULL` 哨兵不依赖全零),但 M5 headless
高频 churn 长跑（RL 训练百万帧级）量级上够得到。**触发点**：M5 开工前过一遍 churn 估算，
必要时 gen 扩 u32（池账 +2B/槽）或文档写死"外部句柄不得跨 >N 帧持有"。
（知会，原 D6 尾注，2026-07-25 前置债务刀随 D6 收口挪存于此：`TaskPool` 的 `impl Default`
仍可外部构造空池——但 `tasks` 字段已封,无注入路径,终审 2026-07-23 判无动作必要,记此防将来
误判为漏网。）

### D15. 脚本敌号只带 generation 的**低 15 位** —— ABA 检测周期 32768（敌句柄打包刀，2026-07-31）

`syscall.rs::pack_enemy_handle` 把敌号编成 `((gen & 0x7FFF) << 16) | index`，**只押 15 位**。
理由是打包值必须恒**非负**：`-1` 是四个读口（100/110/101/102/103 那族，即
`enemy_hp`/`nearest_enemy`/`enemy_x`/`enemy_y`/`enemy_alive`）唯一的"无效/没有"哨兵，
押满 16 位会让 `gen >= 0x8000` 的敌号变成负数、与哨兵撞车。

**代价**：同一个池槽复用 **32768** 次之后，`generation & 0x7FFF` 回绕，旧敌号会重新"认领"
占了那个槽的新敌——即 D7 那条池级 ABA 在脚本敌号这一层被**砍掉一半周期**。
（D7 是 u16 满周期 65536，且说的是引擎内 `EnemyHandle`；本条只针对**跨 syscall 边界**的
那个 `i32` 编码。）

**现在够不着**：一局 STG 里同一个敌槽复用 32768 次意味着单槽产出 3 万只敌，远超实际；而且
脚本持有敌号通常只跨几十帧。**触发点** = M5 headless 长跑（RL 训练百万帧级）真跑起来之后，
与 D7 一并重估。届时的修法：要么把 index 收窄到 8 位（敌池 cap 256，只需 8 位）给 gen 腾出
23 位，要么改用两个 `i32`（脚本侧要配对存，很难看）。

**弹 / 道具句柄仍是裸 index**（`create_bullet`(200) 押 `BulletHandle.index`、`drop_item`(220) 押
`ItemHandle.index`）。今天**物理上够不着**这个 ABA 面，不是"危害小"——引擎侧根本没有解析口：
弹的九个 setter 全走 `self_bullet_handle(task)` 从 owner 三元组取句柄，脚本传的首参 pop 完
就丢；道具句柄没有任何内建吃它。**触发点 = 谁要给它们加读口**（`bullet_x`/`item_type` 之类）：
**加之前先照 `pack_enemy_handle` 打包**，别照 `create_bullet` 现在的返值口径重犯一次。

### D16. 敌人运动的四条非目标（`spec §4.4`，敌人运动动词族刀 T6 记档，2026-07-31）

本刀只做**匀速/线性缓动的速度层**，ZUN `move` 400-447 族剩下的四类明确不做，各随触发点：

1. **高阶轨迹**——`moveCircle`(408) / `moveEllipse`(420) / `moveBezier`(425) / `moveCurve`(434)。
   另一个量级（要么每帧算参数方程、要么存控制点），单独立项。**届时未必照抄 ZUN**：仓里已有
   `xform` 变换段那套"按时间轴发 op"的机器，圆周运动完全可能是段序列而不是新动词。
   ⚠️ 参数方程走大数坐标时的定点坑见 [`fixed-point-corners.md`](fixed-point-corners.md)。
2. **敌人的连续效果**——`ang_vel`/`accel`/`ax`/`ay`，即弹身上早就有的 `BULLET_POLAR_FX` /
   `BULLET_CART_FX` 两个模式位。本刀的插值器是**有终点、有时长**的（`dur` 帧后自己解除
   武装），连续效果是**无终点**的（每帧加一个增量直到被改），两者是不同的机制、不是同一个
   东西的两种参数。触发点 = 高阶轨迹立项时一并评估（`xform.rs` 的 `7x 笛卡尔族` 至今仍标着
   "预留，若将来立项"，是同一笔账）。
3. **`moveEnm`(432，对齐到另一只敌) / `moveRand` / `moveLimit`**。`moveRand` 尤其要留神：
   它消耗世界 RNG，抽取序直接进校验和（同 D13 那条随机 aimmode 的账），做之前先定 spec。
4. **句柄版的 `enemy_speed(e)` / `enemy_angle(e)`**（读**别人**的速度）。位置那边
   `$self_x` 与 `enemy_x(e)` 两套都有，速度这边先只做 `$self_*`——读别人的**位置**有明确
   用途（瞄准/聚集/跟随），读别人的**速度**暂时想不出非它不可的场景，而 `nearest_enemy`
   返的敌大多是拿来打的不是拿来跟的。真需要时补两个 syscall 号即可，**不影响本刀任何设计**
   （敌池里 `speed`/`angle` 本来就逐敌存着，缺的只是读口，同 `enemy_x`/`enemy_y` 当初的形状）。

另有一条已裁定不做且**不留触发点**：笛卡尔单轴动词 `move_vx`/`move_vy`——
`move_vel_xy(30, $self_vx, 4.0fx, 2)` composed 已等价且更通用。

### D17. `vel_touched` 现有 **5 个写点**——加新的位置动词必须同样清它（敌人运动动词族刀 T6 复审记档，2026-07-31）

到点清速的判据是黏滞位 `vel_touched`（spec §6.3）：四条速度动词各置 1（`set_enemy_vel_polar`
/`set_enemy_vel_cart`/`set_enemy_angle`/`set_enemy_speed`），`move_enemy_to` **武装时归 0**
——共 5 个写点，全在 `world/motion.rs`。语义靠"位置动词武装 = 一次重新表态"这条约定维持。

**缝在哪**：将来若加**新的位置动词**（`move_to_rel`、圆周轨迹落点、任何会置 `mv_active` 的
东西），它**必须同样把 `vel_touched` 清 0**，否则 spec §6.3 那张表的第三行（"速度动词在前、
位置动词在后 ⇒ 到点仍清速"）在新动词上就破了：残留的 `vel_touched=1` 会让新动词到点时不
清速，敌人落地后带着一个脚本早已不打算要的速度飘走。

**现有测试盖不到**：`move_to_rearm_resets_touched_so_arrival_clears_again` 押的是
`move_enemy_to` 这一个入口，新动词自带一条新路径，那条测试对它是瞎的。**没有编译期强制**
（不像 `Init` 那种 exhaustive 结构体），只能靠这条记录 + 新动词自带一条同款仲裁腿测试。
**触发点 = 下一个位置动词落地时**（大概率是 D16 的高阶轨迹那刀）。

### D10. 部分设运行期只护 stride、不查 `valid`——编译期空格闸只覆盖 `.ecl` 源码路径（颜色轴刀 T6 记档，2026-07-26）

`OP_SET_SPRITE`/`OP_SET_SHAPE`/`OP_SET_COLOR`（`world/transform.rs::fire_op`）三个解释臂
只做一条运行期护栏——stride/参数是否会导致除零或溢出（P4-b：坏参数计 `contract_viol` 后
no-op；算术上 `wrapping_add` 防近 `i32::MAX` panic）——**从不索引
`WorldTables.appearances[..].valid`**。空格闸（拒收"落到图集空格的组合"）只活在编译器
前端（`lang::atlas::{check_shape_color, check_shape_only, check_color_only}`，挂在
`lang::codegen` 的 `OpFold2`/`OpWithStride` staging 阶段）。这对 `.ecl` 源码路径足够——
一切 `fire`/`batch`/`set_sprite`/`set_shape`/`set_color` 调用都先过这条编译期闸——但
**任何绕开 `lang::compile` 直接构造/反序列化 `XformSlot` 的路径都不受这条闸保护**。目前
全仓没有这样的路径（唯一产出 `XformSlot` 的是编译器 codegen 与手写测试助手），所以不是
活漏洞，只是"编译期闸的覆盖面比看起来窄"这条事实需要记档，防止将来有人以为空格在任何
路径下都造不出来。**触发点 = 出现直接构造/反序列化 `XformSlot` 的消费者时**（候选：
mod 提供的二进制 xform 段格式、M4 rollback 对端镜像重放）——届时需要在运行期臂补一条
`valid` 校验，或明确记录"信任构造方已经过编译期闸"这条前提由谁来担保。

### D12. 死亡加分硬编码给自机 0——"谁打死谁得分"在联机/多人下不存在（2026-07-30 记档）

`world::settle::kill_enemy` 里那句 `self.players[0].score = ...saturating_add(bonus)` 的
自机号是**写死的 0**。上游拿不到别的号：`damage_enemy(e, dmg, tables)` 的签名里就没有
"是谁打的"——`shots` 池明明有 `owner: u8` 字段，但 settle 趟二的 `ROW_SHOT_ENEMY` 臂只从
命中记录里取了 `damage`，`owner` 一路没往下传；`ROW_FIELD_ENEMY`（消弹区伤害致死）那条更是
连概念上的"击杀者"都要重新定义（field 的属主是谁？bomb 是谁放的？）。

本刀按裁定就取自机 0，与既有 `SYS_ADD_SCORE`（`syscall.rs`，同样硬编码 `players[0]`）同口径，
单人下完全正确。**注意对照**：道具入账 `credit_item(p, ..)` **不**是这样——它带自机号参数，
拾取者是谁就记给谁。所以联机线上要统一的是"击杀分"与"脚本 `add_score`"这两处，不是全仓。**触发点 = co-op / rollback 联机真的上线时**
（M4 那条线，与 B3 多自机 graze 位、B13 磁吸自机选择同批）：届时要一次性
决定"击杀归属"的口径——是把 `owner` 从 `hits` 一路穿到 `kill_enemy`，还是干脆改成共享分数。
在那之前别单独修这一处，改一半会让四处记分口彼此不一致。

### D13. 随机 aimmode（ZUN 的 `6`/`7`/`8`）不做——它们消耗世界 RNG，消耗序直接进校验和（shooter 刀 T4 记档，2026-07-31）

ZUN 的 `607 etAim` 是个九值枚举，本刀（D-6）把它塌成 `aimed`/`ring` 两个正交布尔
（`sh_aim`/`sh_ring`，syscall 640/641），**塌得下的只有 `0-5` 那六个**。剩下三个是另一类东西：

| ZUN mode | 语义 | 参数转义 |
|---|---|---|
| `6` | random angles | `ang1`/`ang2` 转义成方向的 max/min |
| `7` | random speeds | `spd1`/`spd2` 转义成速度的 max/min |
| `8` | 两者 | 同上两条一起 |

**为什么不是"加个 flag 位"就完事**：随机散布要抽的是**世界 RNG**（I3：PRNG 状态是 `World`
字段、随快照回滚），于是"每颗弹抽几发、按什么顺序抽"就不再是实现细节，而是**进校验和的
契约**——写错一次，回放/rollback/三平台对拍全部分岔，而金向量闸门只互比三平台、抓不到
（`CLAUDE.md`"金向量闸门的能力边界"）。要钉死的至少三条：① 抽取发生在网格循环的哪一步
（每颗一发？角度与速度各一发？）；② `6`/`7`/`8` 三种模式下的抽取**发数与顺序**互相之间是
什么关系；③ `n_angle`/`n_speed` 与随机的交互（随机模式下 `angle_step` 已被转义成 min，
那"居中"还成不成立）。这是一份独立的小 spec，不该塞进 shooter 刀的尾巴。

且"随机散布"本身与 shooter 的核心价值（**预存参数集**）正交——`rand_range` 已经在手，
今天想要随机弹幕的脚本可以自己 `for` 循环逐颗 `fire`，只是写起来啰嗦。

**将来的形状**（若做）：`flags` 里加 `SH_RAND_ANGLE`/`SH_RAND_SPEED` 两位（`ShooterSlot`
的 `flags: u8` 现只用了低 3 位，有余量；且槽内还有 2 字节尾部 padding，加字段不涨槽宽——
见 `step.rs` 哨兵注释），`angle0/angle_step`、`speed0/speed_step` 在置位时转义成 min/max，
开火循环里逐颗抽。**触发点 = 第一张真需要随机散布的符卡**；届时先补 spec 钉死抽取序，
再动代码，并且要 bump `ENGINE_VER`（`flags` 语义变化 ⇒ 存档载荷解释变化）。

### D14. `sh_fire` 与 `create_bullets_batch` 的两处口径差异——**都影响进校验和的值，别"顺手对齐"**（shooter 刀 T3 复审记档，2026-07-31）

网格发弹在本仓有**两份实现**：`world.rs` 的 `create_bullets_batch`（world 层）与
`ecl/syscall.rs` 的 `sys_sh_fire`（ECL 层）。第二份是必须的——P1 下 world 不知道"任务"
存在，逐颗挂 `task_script` 只能在 ECL 层做。两份实现在**主干上逐位等价**（有等价测试
`shooter_fan_matches_batch_with_centering_compensation` 押运），但有两处**已知**不同：

**① 弹池满时不做"剩余批量补计"。** `batch` 在短路时把剩余额度**批量**记进
`diag.pool_full[POOL_BULLET]`，使计数与"逐颗试"严格等价；`sh_fire` **只短路、不补计**，
故弹池满时它记的是 **1** 而不是"剩余颗数"。这不是偷懒：`batch` 敢批量补计，是因为它在
循环**之前**就把 xform 内容验过一遍（`WorldBody::xform_args_valid`），进循环后唯一的失败
因就是池满；`sh_fire` **够不着那个判据**（`xform_args_valid` 是 `WorldBody` 的私有关联
函数），它的失败因至少两种——池满（记 `pool_full[对应池]`）与坏 xform 内容
（`create_bullet_with_xform` 记 `contract_viol`）——照抄批量补计会把**坏 xform 导致的失败
误计成 `pool_full[POOL_BULLET]`**。

⚠️ **`diag` 的计数值进校验和**（P6，`diag` 无 skip）。所以"为对齐 `batch` 而顺手修好"
会**静默改掉一个入校验和的值**：回放/存档不兼容、三平台仍一致所以金向量闸门照绿。真要
对齐，正确做法是把 `xform_args_valid` 提权成 `pub(crate)` 谓词、循环**前**验一次再批量
补计——那是一处 world API 改动，与 shooter 刀 spec §13"零 world API 改动"冲突，故本刀没做。
**触发点 = 下一次有正当理由改 world API 时**（或有人真的在意两条路径的 `pool_full` 口径
一致），届时连同 `ENGINE_VER` 一起处理。

**② 校验先后序相反。** `sys_sh_fire` 是"退化网格 → appearance"，`sys_create_bullets_batch`
是"appearance → 退化网格"。可观察后果：`n_angle=0`（或 `n_speed=0`）**配一个坏
appearance** 时，`batch` 会 **Fault**（先撞 appearance），`sh_fire` 只 **no-op + 违约计数**
（先撞退化网格短路返回）。两条单独看都符合各自的 P4 处置，只是**同一份坏参数在两条路径上
的结局不同**。实现忠实照抄了计划骨架的顺序，不算违约，但**手册里不能写"与 `batch` 同
口径"**——现在不是。改哪一边都会动金向量（Fault 与 no-op 的世界演化不同），同样要
bump `ENGINE_VER`。

### D20. `world_size_sentinel_guards_copy_into_field_list` 对齐 padding 是盲区——三次真实字段新增全被吃掉未响（自机能力刀 Task 9 记档，2026-09-03）

这条哨兵（`step.rs`）押的是"改 `WorldBody`/`World` 的 `size_of` 就逼你走一遍四件套清单
（`copy_into`/checksum/D10 容量预算/存档格式）"。本刀**恰好三次**给它出题，**三次都绿**：
`WorldBody.freeze_left`（`[u16; 2]`，4 B）、`PlayerState.time_stops`（`u8`，1 B）、
`PlayerState.prev_input`（`u32`，4 B）——三处新增分别落进了既有的对齐空隙，`size_of`
前后一字节不差，测试本该报警的时刻，它睡得很沉。

复审手工摊开 `PlayerState` 布局：64 字节里排到第一个 `u64` 成员前有 6 字节空档，本刀
先后插 `time_stops`（吃 1，剩 5）、`prev_input`（吃 4，剩 1）——**只剩 1 字节了，
下一个 `u8` 还是会被吃掉、`size_of` 还是不会动**。`step.rs` 自己的哨兵注释已经在
`ShooterSlot`（44 B、留 2 字节尾部 padding）那道题上记过同一个盲区——这是**第二次**
在同一份文件里踩到它，不是孤例。

真正逮住这三次改动的，全程是**金向量**：checksum derive 按字段全量入、不看对齐，
新字段进哈希与 padding 是否被吃光无关——`size_of` 测的是布局的副作用，金向量测的是
字段本身，二者不是同一件事，这条哨兵能守住的只是"布局意外变宽"这个真子集。

**第四次（时间机制内核刀，2026-09-07）**：`PlayerState.hit_frame: u32` 落在 `graze: u32`
之后到 8 对齐的 4 B 尾部 padding 里，`size_of` 仍 64、哨兵仍绿——D20 原文说"只剩 1 字节"
指的是 `score` 之前的空档，尾部还有 4 B 没算。逮住它的是金向量 md5 变 + `vocab_hash_pinned`。
候选修法①（押字段数）此刻就能拦住这四次里的每一次。

**候选修法**（本条只记录、不实现，由人裁定）：①把 `assert_eq!` 的对象从
`size_of::<WorldBody>()`/`size_of::<World>()` 换成**字段数**（如 derive 宏生成的
`FIELD_COUNT` 常量，或手数一份清单常量）——加字段必挂，不看是否被 padding 吃掉；
②改押**存档 payload 长度**（`SaveBytes` 序列化后的字节数，逐字段写出、天然不含
padding）而非内存内 `size_of`——这与四件套里"④ 存档格式"那一项同源，更贴近哨兵
真正想守的东西。两案都要连带更新测试里长年累积的"逐刀追记"注释风格。

---

### D21. boss 换段与敌人钩子刀的五条非目标（spec §10 记档，2026-09-14）

1. **`fire` / `sh_task` / `spell_begin` pattern 带参**——触发点：内容里出现需要参数化的弹任务或模式。
   `sh_task` 要在发射器槽里存实参（涨 `TaskPool`，过 D10）；`fire` 与 `spell_begin` 可照 210 的调用约定直接做。
2. **永久「不吃弹 + 不可锁定」位**（ZUN `flagSet(1)` 的锁定语义）——触发点：自机 homing 需要排除某类敌。
   现状无敌走 `set_invuln`（上限 65535 帧），`nearest_enemy` 不看它。
3. **一只敌同时挂两个计时器**（TH18 st07mbs「整体计时 + 卡内计时」）——触发点：Extra 中 boss 类内容。
   现状一只敌只能绑一个符卡槽；伴生计时任务 + `spell_end` 可近似，但结算记 `SPELL_END_MANUAL`。
4. **表层开放 `kill_children` / 按句柄杀任务**——开放时须同步处理 `vm.rs` D9 注释所述 `main_task` 清零（`OP_KILL_CHILDREN`
   绕过 `run_tasks`，被杀子任务若恰是某敌主任务会留下陈旧槽号）。任务池无逐槽 generation，按句柄杀有 ABA，要先设计。
5. **`death_script` 通电（ZUN `setDeath`）**——设计已定形（`stg-world-design.md` 相位 9 挂钩：扫 `EVT_ENEMY_DIED`、
   派生 `sub(x: fx, y: fx)`、owner = STAGE），第 1 关用不到。

### D22. 经典机体刀的非目标（spec §7 记档，2026-09-15）

机体 1（`Kit::Classic`）只保证核内行为。以下未做，**触发点**各自写明：
1. Godot 壳 / 桥 HUD 不适配机体 1（冷却条、停止文案、bomb 演出）——壳里选机体 1 能跑但表现未校。触发点：想在 Godot 里人肉玩经典机体。
2. `BombField` 只有圆；激光形 bomb 须改碰撞矩阵行 6/7。触发点：迁移目标作的 bomb 形状影响训练。
3. 机体 1 复用机体 0 火力 / 移速，不对齐任何原作机体。触发点：迁移验证发现差异显著。
4. 场底重生无入场动画 / 不可操作帧（原作有）。触发点：同上。
5. `STOP_STOCK_MAX` 两套件共用（原作 bomb 上限常为 8）。触发点：同上。
6. 发弹分派仍按 `character_id` 硬分支（`world/player.rs` `0 | 1 =>`），不在表里。触发点：加第 3 个机体——漏改会静默不开火；届时考虑把 shot 解释器选择挪进 `CharacterCfg`。

### D23. stg-rl env 刀的非目标（spec §12 记档，2026-09-15）

本刀只交付「Rust 批量 env + PyO3 wheel + 分发 workflow」。以下未做，**触发点**各自写明：
1. **训练代码 / 特征化 / reward** 住训练仓（reward 含拟人项），不进 wheel。触发点：训练仓立项（框架定了才有形状，故 Ruling 4 连 `gymnasium` 适配都不做）。
2. **训练作业包**（容器镜像 + 入口 + 输出目录约定）。触发点：选定作业平台（AutoDL 等）后另开子项目；本刀只备好无交互安装、离线内容、`build_info()`。
3. **`.stglog` 写出**。触发点：训练需要落轨迹做离线分析 / 模仿学习时。
4. **`.stgr` 回放 → 训练轨迹导出器**。触发点：要用真人回放喂 RL / 行为克隆时。
5. **async env（CPU/GPU 流水线重叠）**。触发点：基准显示 GPU 等 CPU——本刀 `rl-bench` 平台期见 `docs/bench-baseline.md`，先不动。
6. **跨死亡 episode**。触发点：训练目标从「单段生存」升级为整关（需与玩法刀的遡行/偏差值语义对齐）。
7. **终局前最后一帧观测**（v1 的 `done != 0` 缓冲已是新局第一帧）。触发点：loss/credit assignment 需要 death frame 时。
8. **非 x86_64 Linux / Windows 以外的 wheel**（无 macOS / aarch64 wheel）。触发点：训练机或部署端出现这两类机器。
9. **部署侧复刻胶水**：特征化与「最近 K 颗」住 Python 胶水，th06nc / TH18 DLL 跑 ONNX 时须在 C 侧同样实现。触发点：迁移到目标作时，靠对拍测试守一致。
10. **训练分布无激光**：引擎无激光池 ⇒ HELLO 恒发空 lasers 表。触发点：迁移验证显示激光是主要差距；届时引擎激光池另开刀（world-design §709）。
11. **`rl-bench` 默认 workload 不代表密弹**（每 env ~12.5 弹、2000 步 6656 次 reset）；需加密弹档位（预热到弹幕展开 / 高 rank / 不死动作）才能测到编码与压实真实开销，并区分 T6 修复后 32/64 线程回落是负载噪声还是并行 scatter 开销。触发点：训练吞吐成为瓶颈时。
12. **`VecEnv::step` / `reset` 每步现场 `collect` 一个 `Vec<Work>`**（~N 项引用结构，`build_work`）：串行段内的小分配，默认 workload 下占比可忽略；复用它要处理跨步借用的生命周期，改动面大于收益。触发点：密弹档位基准（第 11 条）显示串行段成为瓶颈时。

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

### F2. 校验和轻量化——**已裁决:保 FNV 冻结(2026-07-23)**

**裁决**（存档字节格式/恢复重演风暴闸刀，2026-07-23）：保 FNV-1a64，不换。依据——校验和
无任何**在线逐帧消费者**（单机不算、联机 K=20 采样摊薄 ~70µs(bench 实测 1414µs/20;新账 1.56ms/20≈78µs,A2 续表时以重跑为准)、CI/storm 皆离线跑）；换字宽
mix 只省离线工具的运行耐心，却要为此重新 bless 金向量、多背一刀改动。`ENGINE_VER = 1` 的
身份语义**即含 FNV-1a64**这一算法选择——真要换算法，走 bump `ENGINE_VER` + 过评审的正规
流程，不是本刀的份内事。活口：若 M4 网络实测采样成本超预算，再重启本条讨论。以下原分析
正文保留作裁决依据。

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

### F3. 查看器泛化（serve 的将来扩展点，2026-07-23 记档）

v1 只跑彩虹风铃卡固定场景。两个自然延伸，各随触发点：`--ecl <path>` 任意脚本预览
（触发 = .ecl 创作流真开动——写卡即看，编译错误回显进页面）；回放文件播放/逐帧步进
（触发 = M3 回放调试，线格式 v1 直接可复用为 dump 格式）。多客户端/TLS/断线续联不做
（测试工具本分）。

### F4. 逐帧全缓冲上传账——余"前缀上传"一件（场景刀 T7 记档，2026-07-26；表现契约 v2 刀 2026-09-07 部分销）

已落地两件：① 壳侧 `multimesh_set_custom_aabb` 钉成场界矩形，绕过 Godot 对全部实例重算
AABB（godot-proposals #957 指认的最大 CPU 坑）；② 桥持 `PackedFloat32Array` 经 `as_mut_slice`
原地编码，省掉 `Vec` → Packed 那次拷贝（`multimesh_set_buffer` 内部那次无法省）。
弹层旋转同刀改查核内 `sincos` 表，不再每弹调 libm。

**余项**：`bridge.rs::step_frame` 仍对每个已注册层无条件整块上传 `cap × 12 × 4 B`（三层
bullets 8192 / shots 1024 / items 1024 ⇒ 10240 × 48 B ≈ **480 KB/帧**，60 Hz ≈ 28 MiB/s，
与场上实体数无关），`docs/bench-baseline.md` 仍无这条账。方向：按 `n × FLOATS_PER_INSTANCE`
前缀上传（**Godot 尚不接受 undersized buffer**，proposal #957 未落地；且 `register_layer`
的长度判据要一起改）或脏检测。**触发点 = profiler 指认时**；目前 llvmpipe 有头跑 demo 无感。
### F5. `bench` 的场景**全部**传 `EclImage::empty()`——量不了任何 VM/syscall 改动（**已还，2026-08-01**）

**已还**（bench 饱和档刀，2026-08-01）：`bench` 尾部追加了**四个饱和档**，其中三个跑真脚本
（`crates/stg-harness/scenes/bench.ecl`，三个 named entry 走 `spawn_entry_named`）：

- **`ecl 任务 256`** —— 打满协程调度：`TASK_CAP` 个任务每帧全部 resume，零弹零敌。
- **`syscall 密`** —— 就是本条点名要的那一档：60 个任务 × 254 条 syscall/帧 =
  **15 240 条 `OP_SYS`/帧**，`ecl::syscall::dispatch` 成了可测比例（实测 ~408 µs/帧）。
  下次再动号表/派发实现，跑 `bench` 前后对比这一行就有意义了。
- **`shooter 大环`** —— 打满 `sh_fire` 的网格循环 + 逐颗 `create_bullet`。
- （第四档 `敌插值 256` 不用 ECL，压的是敌人运动的笛卡尔回填热路径。）

`run_measured` 的空镜像从函数体里提成了参数，原八档传 `&EclImage::empty()`——**逐位不变**
（两版各跑 120+60 帧后八档 World 校验和全等，用 `main` 的 worktree 对拍过）。数据与口径见
`docs/bench-baseline.md` 末节。**下面留作墓碑的是原始记录**（别拿它当现状）：

> **【墓碑：原始记录，2026-07-31】** `bench_ladder`/`bench_mix` 四类场景（哑弹 ×4 /
> xform ×3 / 全混合）都显式传 `EclImage::empty()`（源码里还带着"本刀无脚本场景：显式传空
> 镜像（零任务零成本）"的注释），所以 **`bench` 一条 `OP_SYS` 都不执行、一个 ECL 任务都不
> 跑**。当初这是对的（M0 期没有 VM），现在它变成了一个**沉默的覆盖缺口**：任何动
> `ecl::vm`/`ecl::syscall`/协程调度的刀，跑 `bench` 前后对比都只会得到热漂移噪声，而那个
> "无差异"看起来像是结论。
>
> 号表百分区重排刀（2026-07-31）就撞在这上面——spec §5 点名要求用 `bench` 量派发代价，
> 实际只能另外外挂一个临时 crate 才量得出来（做法与数据见 `docs/bench-baseline.md`）。
>
> **触发点 = 下一次真要量 VM 性能时**。方向：给 `bench` 补一档 `ecl-*` 场景——最省事的是
> 复用 `compile_rainbow_image()`（现成的风铃卡）再叠一档纯 syscall 压力脚本。
>
> （实际落地时**没有**复用风铃卡：那是一局"真实凑巧的帧"，而 bench 要的是最烂情况，
> 见下一段裁定。改成了专门写的三个饱和入口。）

### F6. `0xx` 族与 op 号域**重叠**的后果清单——已知两处，撞到第三处再考虑挪族（syscall 号表重排刀 T2 记档，2026-07-31）

百分区重排把 `1xx`–`7xx` 全推到 100 以上、与 op 号空间（`u8`，现最大 60 = `OP_SYS`）永久错开，
**但 `0xx` 族（`$` 引擎变量）取值 000–032，整族 12 条仍落在 op 号域内**，逐条撞号：

```
frame=0=OP_END      player_x=10=OP_PUSHI   player_y=11=OP_PUSHL   self_x=20=OP_ADD
self_y=21=OP_SUB    self_vx=22=OP_MUL      self_vy=23=OP_DIV      self_speed=24=OP_MOD
self_angle=25=OP_NEG  self_hp=30=OP_MULF   self_hp_max=31=OP_DIVF  self_age=32=OP_SINB
```

> spec `2026-07-31-syscall-renumber-design.md` §3 那句"syscall 全部推到 100 以上后两个号空间
> 永久错开"**对 `0xx` 族不成立**，已在该 spec §9 修订记录二留痕。

**已知后果两处**（都不是运行期缺陷——字节流里 opcode 与操作数是**位置区分**的，`ARITY` 驱动
PC 推进，操作数永远不会被当成 opcode 解码，**运行期不存在歧义**）：

1. **`lang/mod.rs::opcodes_of` 的扫描假阳性**：`code().contains(&(OP_X as u32))` 裸扫字会把
   `OP_SYS 20` 的操作数字误当成一条 `OP_ADD`。**注意这条与 `0xx` 无关也成立**——`PUSHI 20` /
   `POPL 20` / `JMP 20` 的操作数同样长得像 op，裸扫字**结构上**就不可靠。挪族消不掉它。
2. **`builtins.rs` 的 `is_op` 标错不再响亮失败**：重排前 `sin`/`cos` 漏标 `is_op` 会发出
   `OP_SYS 32`，而 32 当时不是任何 syscall ⇒ 运行期 `FAULT_BAD_OP`；重排后 32 是
   `SYS_SELF_AGE`，同一失误变成**静默押一个任务龄**——比原来更坏。
   **已用真判据补上**：`builtin_dispatch_kind_matches_what_the_field_holds`（按表查
   `ops::op_implemented` / `syscall::syscall_implemented`，与编号怎么排无关）。

**现在不挪 `0xx` 到 `8xx`**，理由两条：

- 那个"响亮失败"**从来不是设计出来的不变量**，只是稀疏编号的**巧合**。为保住一个巧合去改一张
  刚冻结、刚 bump 过 `ENGINE_VER` 的表，本末倒置。
- `0xx` 与 `resolve_engine_var` 白名单一一对应，是全表最自解释的一格；挪到 `8xx` 会让"`$` 变量
  住 0 号段"这个直觉丢掉，而换来的只是消掉上面第 2 条（第 1 条根本消不掉）。

**触发点**：若将来撞到**第三处**由这个重叠引发的真实麻烦，再把 `0xx` 整族挪到 `8xx`（届时
又是一次冻结面变更 + `ENGINE_VER` bump，且两条守卫测试仍然有效、与号无关）。撞到了往这条底下追加。

### F13. `checksum_report()`（D11 承诺的逐字段哈希清单）从未实现——desync 只能定位到"有东西变了"（时停设计刀记档，2026-09-03）

`stg-world-design.md:631`（D11）承诺 `#[derive(Checksum)]` 有**两种输出**：合并的
`checksum()` 与 debug 的 **`checksum_report()` —— 逐字段哈希清单 `[(字段路径, u64)]`**，
原文称它是"字段级方案相对整块哈希的杀手级红利"（desync 时 CI 自动改跑 report 模式，直接
定位"`bullets.vx` 在第 N 帧分歧"）。**只有前者实现了**：全仓 grep 无 `fn checksum_report`，
`docs/superpowers/plans/2026-07-14-m0-2-checksum-derive.md:273` 当年就把它列进"未覆盖
（明确留后）"，但**此后没进过任何待办清单**，于是对后来者不可见——本条就是把它入库。

**后果不是"少个功能"，是排障成本**：现在任何"整块校验和不相等"的测试（跨平台闸门、
恢复重演风暴闸、以及时停设计里那条"全场静止下除恒跑字段外不得有任何变化"的主测）红了
之后，都只能告诉你**有东西变了**，说不出是哪个字段——只能靠人二分。字段数越多越贵。

**触发点**：① 第一次真的撞上跨平台 desync（M3/M4 那条线，届时它从"方便"变成"刚需"）；
② 或者谁写了第二条依赖整块校验和相等的测试，被排障成本咬到。在那之前它只是一笔记账。
**不是时停那一刀的前置**——那条主测不依赖它，只是有它会红得更好看。

### F14. `puppets()`/`vanished()` 每帧构造 Dictionary-of-PackedArrays（表现契约 v2 刀记档，2026-09-07）

桥面 `puppets()` 每帧新建 8 条 Packed 数组 + 1 个 Dictionary，`vanished()` 4 条；壳侧
`_update_puppets` 再逐列取用。常态 <50 敌、bomb 帧 ~800 行 vanished，实测 llvmpipe 有头无感。
若 profiler 指认，方向是桥持复用的 Packed 数组按 `resize` 写前缀、或改成一条交错的
`PackedFloat32Array`（同三层缓冲的 stride 思路）。**触发点 = profiler 指认时**。

### F11. xformdef 的 `@N` 记号读法与语义相反——本刀已补文档，语义/记号本身要不要改留待评审（docs 刀，2026-08-02）

`@N op(...)` 里的 `N` 解析进的是**这一条 slot 自己的 `wait` 字段**
（`parse_xf_slot`，`crates/stg-ecl-compiler/src/lang/parse.rs:337`），而变换相位是**先发射
这条 op、才把 `xform_wait` 设成它的 `wait`**（`advance_cursor`，
`crates/stg-core/src/world/transform.rs`）。所以 `@N op()` 的真实语义是「执行 op，然后等 N
帧再走下一条」——**后置延迟**。但记号读起来像「到第 N 帧才做这条」——一个**时间标签**，
ZUN 原版 ECL 的 `@N` 就是那个意思。**读法与语义相反**，本仓至少已有两名作者（含一个不带任何
提示、独立在本仓写弹幕的 agent）在这一处摔倒：

1. **demo 的 `WIND_CHIME`**（`godot/ecl/game/boss_windchime.ecl`，已修）：原写
   `set_speed(2.0fx); @30 turn(90deg);`，两条在出生同一帧全跑完，弹一出生就转了 90° 并再也
   不动，`@30` 什么都没延迟。
2. 上面 F10 记的母弹分裂卡踩坑是同一处记号的另一种摔法：`xformdef { @110 set_life(1); }`
   想让母弹活 110 帧，结果 `@110` 被解析进 `set_life` 自己的 wait、`set_life` 创建帧就立即
   发射，母弹一帧就死。F10 已经把这次事故记进内容侧的处置；这里补的是**面向所有 xformdef
   作者的文档**，不局限于母弹分裂卡那一处。

**已做（本刀）**：[`docs/ecl-lang/4-bullets.md`](ecl-lang/4-bullets.md) 补了「`@N` 是后置延迟，
不是时间标签」一节（含 `WIND_CHIME` 错/对写法对照、`harness run --at` 实测帧号）；
[`docs/xform-ops.md`](xform-ops.md) 的 `wait` 一条加了指回那节的一句话。**只补文档，零风险**，
但记号本身仍反直觉，下一个人还会栽。

**候选处置（留待评审，本刀不做）**：

1. **只补文档**（本刀做的）——零风险，但记号仍反直觉，下一个人还会栽。
2. **改语义**：`@N` 变前置延迟（parser 把 `N` 挂到前一条 slot）。符合直觉，但**改变所有
   既有 xformdef 的行为** ⇒ 金向量变、`ENGINE_VER` bump、要重审每一份内容。
3. **换记号**：保留后置语义，把 `@N op()` 改成读起来就是后置的写法（如 `op() @N;`）。
   破坏面是语法而非语义。

**裁定**：先走 1，把 2/3 留给单独评估——这是冻结面的语义问题，不该顺手改。**触发点** = 谁要
动 xformdef 的语法或语义。

### F15. 影子层（観測）只有弹——敌人木偶的未来态不做（时间机制内核刀记档，2026-09-07）

`preview(n)` 只把影子世界的**弹层**上传到影子 MultiMesh（`bridge.rs`）；影子里的敌人走位、
自机弹、道具都不画。策划案 7.6「未来态的弹幕以线框叠加」只说了弹幕，v1 照此。要看敌人未来
走位得给木偶再备一套 256 个 Sprite2D（或把敌人也改 MultiMesh），喂料从 `Timeline::shadow()`
的 `puppets` 编码。**触发点 = 有符卡靠敌人走位而非弹幕来"读未来"时**。

### F16. 余晖拖尾（策划案 7.5「子弹保留半秒历史位置」）——环里就是历史（时间机制内核刀记档，2026-09-07）

策划案要的"平时是拖尾、遡行时沿余晖倒退"在本刀没做。数据两条路都现成：① `Timeline::ring_get`
逐帧读弹位置（30 帧 × 遍历弹池，壳侧攒成拖尾实例）；② 壳侧自己留最近 N 份弹层实例缓冲
（`multimesh_get_buffer` 回读或桥再出一条只读口），约 30 × 480 KB。倒放期间 `view_ring`
本身就是"沿余晖退回去"，拖尾只是把那条历史平时也画出来。**触发点 = 美术期**；余晖时长与
`REWIND_DEPTH` 联动（策划案 7.5 注）。

### F17. 跳躍快进丢弃中间帧的通道 B 请求与 `vanished`——有意口径（时间机制内核刀记档，2026-09-07）

`main.gd` 在同一 tick 连续 `step_frame` 走完缺席的 N 帧，只对最后一帧跑 `_after_step`，
中间帧的 `take_requests()`/`vanished()`/`frame_events()` 未取即被下一 step 的 `begin` 清空
（缓冲是帧内私有的）。她不在场，「线框态瞬时对齐为实体」，丢掉合题；`render-contract.md`
§5.6 已写成口径。若演出期想要"落地时补一次那 30 帧里的爆炸"，方向是 `Timeline::advance`
返回值里攒事件、或壳侧逐步 `_after_step` 但静音。**触发点 = 演出打磨**。

### F18. `Boot::Snapshot` 出身的 `InputLog` 不可从头重放（时间机制内核刀记档，2026-09-07）

`load_state` 之后 timeline 重建，log 头记 `Boot::Snapshot { world_checksum }`，
`Timeline::replay` 对它返 `BootNotReplayable`——log 只能落盘/展示，`harness replay` 也拒。
练习模式（策划案 2.9：手动存档点 + 长时间线）要把快照本体嵌进 log 头（或 log 引用一份
`.stgw` 存档文件），格式 v1 的 boot tag 已留了扩展位。同族缺口：**回放头的"镜像哈希"是
`EclImage.content_hash` = 表 coherence 哈希，不是脚本身份**——换一份脚本、同一张表，头照样过，
只会在重放中途分歧或撞 `UnexpectedRewindRequest`；存档头同款局限。脚本身份哈希（源码 FNV
进 `EclImage`）是练习模式/回放分享前要补的。**触发点 = 练习模式刀 / 回放文件对外分享**。

### F21. 敌人退场动画（detach / 残影）未做（表现纪律刀记档，2026-09-11）

现在敌死 = 木偶当帧隐藏 + `REQ_ENEMY_DEATH` 爆炸环。ZUN 的"宿主死亡时给 VM 发 interrupt 播完
退场再自杀"在这里应是：壳在 `EVT_ENEMY_DIED{a_index, a_gen}` 时把该木偶的当前帧与位置转成一条
带 `(index, gen)` 的 oneshot 残影行，木偶节点立即释放给复用。数据齐全、纯壳侧。**触发点 =
美术给出退场帧**。预测回滚下"残影又复活"的取消逻辑归 M4。

### F22. 动画时长的单一来源——`ENEMY_ANIM` / `FX_LIFE` 迁内容数据文件双生成（表现纪律刀记档，2026-09-11）

「等施法动画播完再开火」要 ECL 能引用动画时长常量，而时长现在只住 GDScript
（`content_tables.gd`）。方向：内容数据文件（JSON/TOML，住 `godot/` 或 `content/`）→
`gen-ecl-meta` 式双 sink：ECL 注入常量（`ANM_LEN_*`，进 ① 段或内容包 `const`）+ GDScript 表。
核拿到的是常量，动画改了时序不会悄悄错位。**触发点 = 第一张依赖动画时长的符卡**。

### F23. 自机动画帧未做 + 三件套命名对齐（表现纪律刀记档，2026-09-11）

自机现在是单张 Sprite2D，无 `sprite`/`anm_state`。策划案 7.3 要待机 4 帧 + 左右移 6 帧；核里
`facing` 已有，壳侧按 `(facing, 输入方向, 帧号)` 选帧即可，**核零改动**。顺手做两件：①敌池
`anm_state`/`anm_state_frame` 与背景 `bg_phase`/`bg_phase_frame` 命名对齐成 `*_phase` /
`*_phase_frame`（`ENGINE_VER` 不动——改名不改布局；存档/校验和按字段序不按名字）；②`WorldView`
给敌 / 自机 / 背景三类同形状的三件套读口。**触发点 = 壳子刀接自机动画帧时**（render-contract
§0.5「存储是 SoA 分散的，抽象是统一的」）。


### F24. 表现层 2× 基准（1280×960 / 弹幕域 768×896 / 自机 64×96）——世界坐标不动（美术交接包记档，2026-09-12）

工程现在 640×480 窗口、弹幕域 SubViewport 384×448、自机占位 32×32，全部按原作 1× 走。
setting §7.3 与 `docs/art-brief.md` 定的资产规格是**严格 2×**：窗口 1280×960、弹幕域 768×896、
自机点阵 64×96、图集网格翻倍（16→32、32→64）。**场界 384×448 世界单位不动**（`FIELD_HALF_W`/
`FIELD_HEIGHT` 是核常量，进校验和），只在表现层乘 2：`playfield.gd` 的 viewport/world_root/`CELLS`、
`layer.gdshader` 的格子尺寸、`project.godot` 窗口、HUD 布局。定点→浮点边界仍在消费端
（render-contract §6）。**触发点 = 自机点阵委托落地前**——没有 2× 基准，64×96 的帧在游戏里验不了。
顺手销 F23 的自机动画帧接入（帧表同一份）。

### F25. 同帧多个自机死亡只认领一条遡行请求——2P 前必须处理（玩法刀核心复审记档，2026-09-14）

`Timeline::advance`（`timeline.rs`）每帧只 `find` 第一条 `EVT_REWIND_REQUESTED`。旧语义下未认领的请求
零副作用；玩法刀起 `commit_death` 在世界侧已扣残机/加偏差值，两个自机同帧死亡时第二个的代价会随
快照恢复被整体抹掉（落地只对第一个调 `rewind_landed`）。现状 `players[1]` 只在测试里 spawn，是潜伏
问题。**触发点 = co-op / M4 联机前**：落地侧按帧内全部请求逐个 `rewind_landed`，`Cut` 记多个自机（回放
格式变更）。与 B3/B13 同属 co-op 族。
