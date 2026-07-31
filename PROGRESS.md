# PROGRESS —— 进度入口

> 「当前走到哪 / 在干什么 / 下一步」的**唯一权威**；CLAUDE.md / README 的状态一律指到这里。
> 细节不进本文：历史细节归 git log 与 `docs/superpowers/plans/`，技术债归
> [`docs/follow-ups.md`](docs/follow-ups.md)。维护规矩见文末。

## 现在（2026-07-31）

- **位置**：**小清洗刀**（`feat/ecl-cleanup`）——「ECL 手感梳理」第五组零碎收口：syscall
  77/78/79（`atan2`/`dist`/`nearest_enemy`）把核里现成的东西通电（`nearest_enemy` 的世界
  实现自 M0-13 起是死代码，一行未改），零新机制、不动 `World` 布局；`ENGINE_VER` 5→6
  （号表纪律）；两条文档订正（跨文件 `const` 可见 / `wait` 的 u16 截断）。金向量与
  89d4860 对拍**逐字节不变**。
- **在飞**：无。
- **待目验**（卡在"要有头环境"）：B26 余 ②`visible_instances` 断言 + ③ 可玩性目验（打到
  结算/手感/节奏）；shooter 尚无 demo 局消费者，改写 boss 弹幕时顺带看一眼。
- **下一阶段候选**：难度分档（ZUN `617-625`）/ M3 环形快照回滚 / `stg-py` RL 线 /
  背景刀（A4）/ 内容美术期（A9）/ 更多弹型。
- **待办**：见 [`docs/follow-ups.md`](docs/follow-ups.md)（本刀销 C12⑤ 的 `nearest_enemy`，
  同处新记：按敌号读坐标的读口未暴露，"查最近的敌→朝它开火"接不通）。

## 里程碑史（每条一行，只增不改）

| 日期 | 里程碑 | 一句话 |
|---|---|---|
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
