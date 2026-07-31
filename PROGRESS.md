# PROGRESS —— 进度入口

> 「当前走到哪 / 在干什么 / 下一步」的**唯一权威**；CLAUDE.md / README 的状态一律指到这里。
> 细节不进本文：历史细节归 git log 与 `docs/superpowers/plans/`，技术债归
> [`docs/follow-ups.md`](docs/follow-ups.md)。维护规矩见文末。

## 现在（2026-07-31）

- **位置**：**难度档具名化刀**（`feat/rank-enum`）——`GVAR_RANK` 的机制早就通了，这刀只
  **把已有的东西命名并封边**：五个 `RANK_*` 常量入 `consts.rs` ① 段（`0..=4`，四档 +
  Extra 预留位）、`new_game_at` 补值域校验（**拒绝而非钳位**，`RankOutOfRange`，校验在任何
  世界写之前）、`main.gd` 的开机魔数（rank 硬编码 `2` = Hard，此前调用点看不出来）提成
  具名 `const`。**`ENGINE_VER` 判定为不 bump**（理由记在 `lib.rs`）；金向量与 f71d196 对拍
  **逐字节不变**，真工程冒烟绿。收尾补一刀壳层：`bridge.rs` 的 `rank`/`start` 从 `as i32`
  截断改**饱和**（`as` 是模 2³² 回绕，GDScript 传 `2³²` 会截成 `0`＝合法的 Easy，把刚加的
  值域校验整个绕过去），并把 `RANK_*` 转成 gdext `#[constant]`、`main.gd` 改读
  `WorldBridge.RANK_HARD` 而不是手抄一份镜像。
- **在飞**：无。
- **待目验**（卡在"要有头环境"）：B26 余 ②`visible_instances` 断言 + ③ 可玩性目验（打到
  结算/手感/节奏）；shooter 尚无 demo 局消费者，改写 boss 弹幕时顺带看一眼。
- **下一阶段候选**：M3 环形快照回滚 / `stg-py` RL 线 / 背景刀（A4）/ 内容美术期（A9）/
  更多弹型（难度分档只剩"内容真的按档写弹幕"，ZUN `617-625` 那组分档指令不需要——表层
  `if global(GVAR_RANK) >= RANK_HARD` 已等价覆盖）。
- **待办**：见 [`docs/follow-ups.md`](docs/follow-ups.md)（**D15**：敌号只带 gen 低
  15 位 ⇒ ABA 周期 32768，与 D7 一并留给 M5 长跑重估）。

## 里程碑史（每条一行，只增不改）

| 日期 | 里程碑 | 一句话 |
|---|---|---|
| 2026-07-31 | 难度档具名化刀 | **不是加机制,是把已有的东西命名并封边**:`GVAR_RANK` 早就通了(常量已注入/`global(GVAR_RANK)` 可读/`new_game_at` 收 `rank` 并写进它/系统段对脚本只读),缺的只是值域没定义、没有具名档位、真工程那根线埋在魔数里。三件:①五个 `RANK_*` 入 `consts.rs` **① 段**(冻结引擎编号,不随可加载表漂移,同道具类型号的理由),人类裁定**只做四档确定性弹幕**、不做 ZUN 那套"连续 rank + 档位"双轨,`RANK_EXTRA`(4) 写明是**预留位不是第五档**(Extra 关走自己的脚本,通常不靠 rank 分支);②`new_game_at` 补 `0..=4` 值域校验,**取拒绝而非钳位**——`rank` 是回放/握手身份 `(seed, rank, start, loadout, image)` 的一员,悄悄钳过的值会让"同 seed 同 rank 重放"这个契约变得可疑,而开机是宿主一次性调用,当场 `Err` 好过事后翻 `diag`;校验摆在**第一位**、`World::new` 分配之前,判别腿用一枚 `NoRoot` 镜像造"两个错并存"(若校验被摆到 `set_var(GVAR_RANK)`/`start_main` 之后拿到的会是 `NoRoot`),边界**两端都断言**(`-1`/`5` 各 `Err`、`0`/`4` 各 `Ok` 且原样落槽——只测越界侧的话把判据误写成 `1..=3` 也能过);③`main.gd` 开机调用的**九个位置参数里六个是魔数**,rank 硬编码 `2`(=Hard)⇒ demo 一直在跑 Hard 而调用点看不出来,提成 `BOOT_SEED/BOOT_RANK/BOOT_CHARACTER/BOOT_POWER/BOOT_LIVES/BOOT_BOMBS` 六个具名 `const`(**取值一律维持原样**,改成什么是内容作者的事)。**`ENGINE_VER` 判定不 bump 并把理由留痕在 `lib.rs`**:加常量只是**编译期词汇**(既有镜像字节码一字未改,用了新名字的脚本其 `content_hash` 本来就不同)、校验只把此前非法的输入从"照单全收"变成显式 `Err`(合法开局产出的世界逐位不变)、且校验**只在开机 API 上**——`load_bytes` 不校验 rank,故没有一份既有存档因此不可读;号表/op 表/池布局/`SaveBytes` 编码/相位序全未动。`gen-ecl-meta` 实测零 diff(引擎常量不在 `<!-- gen:builtins -->` 生成块里);手册补五行常量表 + 值域 + 一个真编译围栏例(`if global(GVAR_RANK) >= RANK_HARD`);金向量与 f71d196 对拍逐字节不变,真工程冒烟 SMOKE OK。**收尾补一刀壳层**(复审自查发现,不在原任务书范围内):`bridge.rs` 把 `rank`/`start` 的 `as i32` 换成**饱和**——`as` 是模 2³² 回绕,GDScript 传 `4294967296`(=2³²) 会截成 **`0` 这个合法档**、静默以 Easy 开局并返 true,②那道核内校验根本轮不到执行(`start` 同理:越界 mark 号会折回关首);饱和则把越界值钉在 `i32::MIN/MAX`,仍然越界,核内照样响亮失败。判别腿落在**桥级冒烟**(`smoke.gd`,gdext `#[func]` 无从单测):三条负例 `5`/`2³²`/`-(2³²+1)` 各断言返 false **且世界没被动**(frame/power 不变),上沿正例 `RANK_EXTRA`(4) 断言合法;把饱和改回 `as i32` 实证**只有 `2³²` 那条转红**(另两条越界值截断后仍越界,逮不到)。同时把 `RANK_*` 转成 gdext `#[constant]`(同 `BTN_*`/`LAYER_*` 既有先例)、`main.gd` 改读 `WorldBridge.RANK_HARD`——原实现在 `main.gd` 里**手抄了一份五常量镜像**,与 core 之间没有编译期押运,抄错了值域校验也拦不住(除非恰好抄成越界值) |
| 2026-07-31 | 敌句柄打包刀 | 脚本敌号从**裸池索引**改成打包值 `((gen & 0x7FFF) << 16) \| index`,堵掉 ABA:敌死、槽被回收、另一只敌落进同一个槽之后,旧敌号此前**静默指向新那只敌**。**不是加机制,是把已有的信息接上**——`EnemyHandle` 本来就带 generation(`world::nearest_enemy` 返的一直是完整句柄),只是 syscall 边界把它丢了、只押 `index`;故**零新 syscall 号**,只改六处编码口径(产号 22/79 押 `pack_enemy_handle`,消号 12/80/81/82 走同一份 `resolve_enemy_handle`,降级取值一律不变:hp→-1、坐标→0、alive→0,仍不 Fault 不计 `contract_viol`,dying 语义不变)。两条设计裁定各有一条测试押着——(a) **只押 gen 的低 15 位**故打包值恒非负,`-1` 仍是唯一哨兵(押满 16 位会让 `gen>=0x8000` 的敌号变负、与哨兵撞车);(b) **比对两边都要 `& 0x7FFF`**(池的 `generation` 是完整 u16),漏掩码会变成"敌活着但所有读口都说它没了",要跑 32768 次同槽复用才撞得到、正常路径永远绿。判别腿:ABA 主腿(A 占 0 号槽 → dying + 相位 9 回收 → B 落回同一个槽 → 用 A 的旧号读四个口全部降级,外加**反向腿**用 B 的新号读一遍防"全都读不到"的假绿)/ 高 gen 掩码腿(预置 `generation[0]=0xF00C` 造 `gen=0xF00D` 的敌)/ 打包往返 / `-1` 仍无效;外加一条 `.ecl` 源码级 e2e 复刻 `boss_windchime` 的等死轮询(boss 主任务跑完自燃 → 轮询退出 → 杂兵占进 boss 旧槽 → 旧 boss 号仍返 -1)。**打包实际修好了 demo 里一个潜在 bug**:`boss_windchime.ecl:74` 的 `if enemy_hp(boss) < 0` 在打包前会读到占了 boss 槽的杂兵的血(比如 40),轮询卡住不退——此前只靠"boss 段后不再造敌"这条口头惯例兜着。三次变异各自实证:去 gen 比对 ⇒ 只 ABA 主腿与 e2e 转红(读到 B 的血 123 / 杂兵的 40),去掩码 ⇒ 只高 gen 腿转红(活敌读成 -1);`ENGINE_VER` 8→9——**号表一个没长而既有取值语义变了**,这比加号更硬的兼容破坏(旧回放/存档会静默走出另一条世界线,必须拒载);四个读口文档里"句柄复用不可辨"那句话全删,手册补一节**敌号是不透明值**(别猜数值/别做算术,`-1` 是唯一有意义的取值,**两个敌号相等 ⇒ 同一只敌**而不再是"同一个槽");新记 follow-up D15(gen 低 15 位 ⇒ ABA 周期砍半到 32768,与 D7 一并留给 M5 长跑重估);金向量与 7a6945d 对拍**逐字节不变**(`rainbow.ecl` 无 `spawn_enemy`,harness 侧的敌是 Rust 直建不经 syscall),两个冒烟真跑绿 |
| 2026-07-31 | 探活读口刀 | 上一刀（敌坐标读口）留下的**残余缝**收口:syscall 82 `enemy_alive`(ZUN `555 enmAlive` 的对应物)——探活此前只能拿 `enemy_hp(e) != -1` 当探针,而 **−1 同时是降级值和一个合法血量**(overkill 的敌 hp 是真实负值,`settle::kill_enemy` 只 `min(0)` 不抹平),血量恰为 −1 的活敌会被旧探针误判成"号无效";专用口与血量取值无关,判据**逐字照抄**读族三兄弟(`handle >= 0 && idx < CAP && is_alive(idx)`),只是把判据**本身**押出去而不是拿它选一个值;不 Fault、不计 `contract_viol`、owner 无限制。**唯一的语义裁定(人类拍板):判的是「槽有效」,含 `ENEMY_DYING` 的敌返 1,不是「还能打」**——读族四条(hp/x/y/alive)必须用完全相同的三判据,dying 的槽要活到相位 9(坐标仍读得到),四条里单独给一条换判据会让这组口径散掉;配套建议写进手册:`nearest_enemy` **本身已排除 dying**,故"从它拿到的号后来变 dying"应当**重查**而不是继续用。判别腿两条,各经变异实证**只有它**转红——(a) dying 仍返 1(加一条 `& ENEMY_DYING == 0` 后单它一条红,其余全绿,实证"排除 dying"这个最自然的错法确实只有这条测试逮得住)/(b) 与 `enemy_hp` 的判据一致性(三种无效各断言两条口同步 `0 ⟺ -1`,外加"活敌 hp 恰为 −1"那一格证明新口不是 `enemy_hp` 的花哨包装——把实现换成旧探针后单它一条红);`.ecl` 源码级 e2e 新旧**两条并存**(旧的用 `enemy_hp(n) != -1`、新的用 `enemy_alive(n) == 1` 走同一条 snipe 链路,同时绿即证新旧判据在正常路径上等效);手册探活惯例改推 `enemy_alive`、旧写法保留并写明它不够用的那一格;`ENGINE_VER` 7→8(纯号表理由,布局与存档编码未动);金向量与 d137ecd 对拍逐字节不变 |
| 2026-07-31 | 敌坐标读口刀 | 上一刀通电 `nearest_enemy` 后暴露的断头路收口:syscall 80 `enemy_x`/81 `enemy_y` 按敌号读坐标(数据本就在敌池里躺着,缺的只是读口),两条共用一份 `sys_enemy_pos(.., want_y)`;存活判据**逐字照抄** `sys_enemy_hp`(负句柄/越界/死槽降级、`ENEMY_DYING` 仍可读、owner 无限制、不 Fault 不计 `contract_viol`),**只有降级值不同**——`enemy_hp` 能用 -1 当哨兵是因为 hp 天然非负,坐标没这便利(−1 是合法 `fx`),故取 `Fx::ZERO` 并把"敌恰在原点 vs 号无效不可辨"这个歧义写进手册的**探活惯例**(先 `enemy_hp(e) != -1` 再读;⚠️ **不是** `>= 0`——overkill 的敌 hp 是真实负值,`kill_enemy` 只 `min(0)` 不抹平,`>= 0` 会把刚被打穿、槽还在的敌误判成无效);判别腿两条各防一个具体错法——敌放 **(30, −70)** 这个 **x ≠ y** 的位置分别断言(放 (5,5) 则两条派发臂写反完全不可辨,同「圆心重合式测试对半径映射是瞎的」那条推论;负坐标那一半顺带钉死"别把 `Fx` raw 当无符号搬")/ 一条**真编译 + 真 VM** 的 `.ecl` 源码级 e2e 走完 `nearest_enemy → enemy_hp 探活 → enemy_x/enemy_y → atan2 → fire` 并断言**弹池里那颗弹的角度** == `atan2(−80, 60)`(敌放 (60,−80),故"两条读口写反"会让角度落到另一象限;世界侧单测绿证明不了脚本够得着——`nearest_enemy` 的世界侧测试一直绿着而绿的是死代码,上一刀才发现);两条判别腿都经**对调派发臂**的变异实证同时转红;`ENGINE_VER` 6→7(纯号表理由,布局与存档编码未动);手册那条"链路接不通"的 ⚠️ 换成正面完整用法示例(围栏押运真编译);金向量逐字节不变 |
| 2026-07-31 | 小清洗刀 | 「ECL 手感梳理」第五组零碎:三条 syscall 通电——77 `atan2(y,x)`(核里的整数 CORDIC,脚本此前只有 `aim_player` 只能瞄自机)/78 `dist(dx,dy)`(=`isqrt(len_sq)`,Q32.32 开根正好回 Q16.16;**向量模不是两点距离**;不单独暴露 `len_sq`/`isqrt` 是人类裁定——前者返 i64 装不进脚本值域,后者单独没用)/79 `nearest_enemy(x,y)`(**M0-13 建完就没人能调的死代码通电**,世界侧一行未改,返池 index、无敌 -1,口径同 `enemy_hp`);三条判别腿各防一个具体错误实现——`atan2` 两个取值一起断言(两参同型,写成 `atan2(x,y)` 不判型报错只镜像角度)/`dist(3,4)==5.0fx`(漏 `isqrt` 或 Q32.32 当 Q16.16 都立刻红,"结果>0"是空断言)/`nearest_enemy` 场上放**两只**不同距离的敌且近的那只池索引更大(只放一只则"取最近"与"取第一个活着的"不可辨,同「圆心重合式测试对半径映射是瞎的」那条推论);外加一条 `.ecl` 源码级 e2e 证通电(死代码的世界侧测试一直绿,绿的是死代码);`ENGINE_VER` 5→6(纯号表理由,布局与存档编码未动);两条文档订正**先实测再写**——(a)"跨 `.ecl` 无共享 const"作废(多文件是合并后单管线编译,`const` 天然跨文件可见且不受文件序影响,demo 局 `bullets.ecl` 一直这么用)、(b)`wait(n)` 的 u16 截断手册全文没提(`wait(65536)`=0 帧、`wait(-1)`=65535 帧,有测试钉死不是 bug),补进坑清单+语句节;金向量逐字节不变 |
| 2026-07-31 | **Shooter 刀** | 复刻 ZUN ECL `et*` 族的**预存发射参数集**:配一遍→反复开火→改一个字段就是下一波。`ShooterSlot`(44 B=6×Fx+7×u16+4×u8+2 尾部对齐)每任务 4 个,住 `TaskPool` 的并行数组而非 `WorldBody`(P1:world 不知道"任务"存在,+45056 B/world,World 1.03→1.08 MB),`TaskPool::spawn` 复用槽时一并抹默认;15 个内建/syscall 62-76(14 个 setter + `sh_fire` 开火七步,`sh_fire` **无返回值**故可裸语句 D-8);ZUN 九值 aimmode 枚举塌成 `aimed`/`ring` **两个正交布尔**(D-6,mode 4/5 冗余因 ring 下 `angle_step` 本就是逐层偏移);两处招牌语义各有判别腿——fan **以基准方向为中心**对称展开(改颗数不用重算 `angle0`)/ring **逐颗算 `(i×65536)/n`** 余数均摊故精确闭合(预乘写法留 16 BAM 的缝,而"角差之和==65536"是空判据、逐颗值与相邻极差才有判别力);复审逮住一条 Critical——`angle_step` 存 `Angle`(u16),开火侧居中公式 `((n−1)·step)/2` 的除 2 不与 mod 65536 交换,零扩展让**负步长×偶数路**整把扇形偏 180°(形状仍对,只看"相邻差 step"的测试全瞎),修法 `as i16 as i32` 回到本仓家规(`batch` 形参本就是 i16);`sh_task` 是**每颗弹派一个任务**⇒`sh_count(28)` 一句话吃 28 个任务槽(池 256),池满弹保留任务丢,是 shooter 新引入的压力面故写进手册坑清单;`ENGINE_VER` 4→5(存档载荷变);金向量 T1 因池布局漂一次(哈希全槽⇒空世界即变),T2/T3/T4 逐字节不变;新记 follow-ups D13(随机 aimmode 消耗世界 RNG,抽取序进校验和,要单独 spec)/D14(`sh_fire` 与 `batch` 的池满计数口径与校验先后序两处差异,**都进校验和,别顺手对齐**) |
| 2026-07-30 | **敌人死亡效果刀** | 三件:掉落从"生成时定死的表索引"迁成敌身上的 `drop_count:[u8;5]` 逐类型可变计数(表号退化成 `spawn_enemy` 的生成参数,建敌时展开,撒落恒按类型编号升序)+死亡效果提成 `world::settle::kill_enemy()`(幂等门禁/`hp.min(0)`/敌死强制把 `enemies.score` 记进自机 0——此前 score 是纯装饰字段)+四个 syscall 58-61 `drop_clear`/`drop_add`/`drop_items`/`die`(表层 `die()` 由 codegen 降低成 `SYS`+`OP_KILL_SELF` 两条指令故立即终止本任务;道具类型常量入 consts ①段;`ENGINE_VER` 3→4);两条人类裁定用测试钉死(`drop_items` 吐完不清空⇒`die()` 前调掉双份/dying 敌当帧仍体碰仍撞死自机);关键判别力:`(type=5,n=3)` 逮参数写反、满血 `die()` 逮 `min(0)` 与无条件置 0、D9 自燃在 hp 远高于血线时判 `hp_break` 三路 OR 里的 `ENEMY_DYING` 那路(删该路即红,实测);金向量两段漂移各有独立归因与对照实验——T1 池布局(哈希全槽⇒空世界即变,frame 0 起全差)/T2 死亡加分(frame 29 起才分岔,不死敌的二号场景零漂,注释掉加分即逐位复原),T3/T4 逐字节不变;新记 follow-ups D12(击杀分硬编码自机 0,联机线要重访) |
| 2026-07-30 | **ECL 复刻刀** | 三项挡着写真实关卡的 ECL 缺口:敌主协程返回即自燃(D9,挂 vm::run_tasks 的 Exec::End 分支+任意终止路径清 main_task 的别名防护,demo 杂兵退场点从越界 y=760 改回回收线内 y=500;判别腿=不掉道具/不发 EVT_ENEMY_DIED)+clear_bullets() 全场清弹(B19,复用 FieldPool 铺 life=1 全屏消弹区,消弹转星星与 EVT_FIELD_CLEARED 白送;判别腿=星星池真出 3 颗)+add_lives/add_bombs/add_power 三个账面增量 setter(B20,saturating_add 后双边钳,power 钳 POWER_MAX=400 非 u16::MAX;裸+与错上限两处变异实证);syscall 号表 54-57、ENGINE_VER 2→3;ecl-lang.md 手写三块语义(D9 单列一节改敌任务默认心智)+ecl-ops.md 号表追四行;销 follow-ups 三条(D9/B19/B20);金向量全程逐字节不变 |
| 2026-07-27 | **P4 覆盖刀** | 宪法级不变量的零覆盖分支补齐:四池满降级(B1)+push_event 溢出(B2)+overkill 断言 hp(B4)+越界 drop_table(B11)+task 号支路①③两族(B25);修两处真缺陷:credit_item 的 u8 饱和(B12,debug 曾 panic)+storm --saves 0 假绿(B14);每条经变异检验证判别力;销 follow-ups 七条 |
| 2026-07-27 | **真美术 + 命中特效** | 原作弹片切 12 弹型×16 色图集(切图工具/gen_atlas 移除弹层防覆盖/cell 16 零留白)+词表按行序登记+色名三轮测量定稿(暗亮对×5 + 暖色梯度×4,名字迁就受众)+弹朝向补四分之一圈(贴图头朝上 vs BAM 0 指右,两轮变异实证)+自机弹命中走 events(EVT_SHOT_HIT_ENEMY,坐标取弹不取敌心)+桥面 frame_events() 读口(八种事件一次开放);B23 销(画面目验不镜像) |
| 2026-07-26 | **弹幕颜色轴刀** | 弹型×颜色二维图集(表长成 12×16 整齐矩形/identity/空格掩码)+ECL 两参糖(编译器折叠,字节码零改)+编译期三判据(先分别校验再折叠,防写反)+color_stride 进表与词表归内容包(mod 对等)+部分设两新 op(OP_SET_SHAPE/OP_SET_COLOR,只改一维,stride 编译期从表写入槽,ENGINE_VER 1→2)+图集 16×12 占位上下不对称;金向量因 sprite 重排整体平移 |
| 2026-07-26 | **Godot 场景刀** | A5 乙案 `spawn_enemy` 7 参+`enemy_hp`/渲染契约收口/真 Godot 工程竖切(场景树/四层 MultiMesh/请求分发器/HUD)/demo 局(杂兵+风铃卡 boss)/双冒烟(桥级+真工程级);M2 全落地;金向量全程逐位零平移(证据归 git 历史) |
| 2026-07-25 | 前置债务刀 | boss_ui 结算清扫(B16②)+D6 四字段收口(49 处迁移)+register_layer 换 buffer 判据+冒烟大扩(B18 可达面)+D8 乙案+新 A5(enemy-owned 任务语言缺口=场景刀设计输入) |
| 2026-07-25 | 文档整理 | CLAUDE.md 结构树/命令/里程碑追新 + follow-ups 对账(销 B17 顺手补两断言/C17 归位 C 组/A3·B18 追注 new_game_at·anchors 现实) + D10 杂项行记锚点四字段 + ecl-lang check 目录用法 |
| 2026-07-25 | **整局流程刀** | 方案A拍板(整局一World一镜像)+compile_units多文件+mark中段启动(垫片/标记表/自动补偿)+Loadout/new_game_at+表现锚点四字段(bgm/bg/bg_phase声明式)+转场挂牌协议;金向量因锚点字段整体平移(判别式护行为) |
| 2026-07-24 | **编辑体验刀** | check 诊断环 + builtin 元数据(doc/param_names)→ gen-ecl-meta 两 sink(JSON/VS Code 扩展/ecl-lang.md 生成段)+ ecl-lang.md agent 优先重构(坑清单/debug 循环/例子可编译押运) |
| 2026-07-24 | **stg-godot 桥刀** | 工具链 1.94/gdext 0.5.4/三纯模块/冻结面/headless 冒烟；M2 桥半程通 |
| 2026-07-24 | 前置小刀 | A1 表列 sprite/正典 boot new_game/bench 第三轮 |
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
