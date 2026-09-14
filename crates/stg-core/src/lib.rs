//! # stg-core —— 弹幕 STG 引擎的确定性内核（断层线以下）
//!
//! 权威设计见仓库根的 `design_doc.md`（v0.3 总纲）与 `stg-world-design.md`（v1.0 世界层蓝图）。
//! 硬纪律摘要（完整见 `CLAUDE.md`）：
//!
//! - **I1 数值**：唯一标量是定点数 Q16.16（`i32`）；`f32/f64` 不得出现在本 crate。
//! - **I2 角度**：BAM `u16`（一圈 = 65536），三角函数一律查表。
//! - **I3 随机**：PRNG 状态是 `World` 字段，随快照回滚；不碰宿主 RNG。
//! - **I4 顺序**：一切遍历按池索引升序；禁 `HashMap` 等无序容器参与模拟。
//! - **I5 协程**：模拟协程执行状态位于可 memcpy 的扁平内存（自建字节码 VM）。
//! - **I6 时间**：逻辑固定 60 Hz，一切计时用整数帧，step 不接受 delta。
//! - **I7 布局**：`World` 内无指针/引用/堆容器；快照 = 整块字节复制。
//!
//! 断层线靠 `Cargo.toml` 依赖图在编译期焊死，而非自律（见本 crate 清单里的确定性防火墙）。
//!
//! ## Phase 1 脚手架现状
//!
//! 当前仅落地确定性契约的最底层基础设施 —— [`checksum`]（vendored FNV-1a 64，D11）。
//! 数学核 / 池框架 / World 本体 / step 相位随 **M0** 逐模块长肉，全程 TDD + 金向量回归。

// 让 #[derive(Checksum)] 生成的 `::stg_core::…` 绝对路径在本 crate 内解析（serde 同款）。
extern crate self as stg_core;

/// 引擎确定性契约版本。**bump 纪律**：凡改 op 表/syscall 号语义、校验和算法、烘焙表内容、
/// 池 SoA 布局/字段序、step 相位序、**SaveBytes 载荷/叶型编码**（存档字节格式——头形变更
/// 走 `save::SAVE_FILE_VER`，载荷编码变更走本值，见 spec §5）——任何使旧回放/旧对端/旧档
/// 不可对拍的变更——必须 +1 并过评审。
/// 回放头（M3）/联机握手（M4）身份三元组之一（另两个：表 `content_hash`、镜像 `content_hash`）。
///
/// > ⚠️ **下面变更史里的 syscall 号记的都是「当时」的号，一律作废、勿据此查号。**
/// > 号表已于 **2026-07-31 百分区重排**（`3 → 4` 里那个 `SYS_DIE`(61) 现在是 530、
/// > `5 → 6` 里那个 `SYS_ATAN2`(77) 现在是 140，其余同理）。现行号表以
/// > [`ecl::syscall`] 的常量为唯一真相源，呈现面见 `docs/ecl-ops.md`。
/// > **旧号是有意保留的**：变更史记的是"当时为什么 bump"，换成新号就读不出
/// > "`ENGINE_VER 3` 那会儿 54 号确实是 `SYS_CLEAR_BULLETS`"这件事了。
/// > 唯一例外是最后一条 `10 → 11`（重排本身），它拿 `20` 举的例子明写了新旧两侧。
///
/// **1 → 2**（颜色轴刀 T7，2026-07-26）：op 清单新增 `OP_SET_SHAPE`(32)/`OP_SET_COLOR`(33)
/// （`set_sprite` 细化出的"只改形"/"只改色"两个部分设 op）。
///
/// **2 → 3**（ECL parity 刀 Task 2，2026-07-30）：syscall 号表新增 `SYS_CLEAR_BULLETS`(54)
/// （全场清弹 B19）——号表变更同 op 表冻结纪律，同款口径 bump。
///
/// **3 → 4**（敌人死亡效果刀 T3，2026-07-30）——**两条理由，缺一不可**：
/// ① syscall 号表新增 `SYS_DROP_CLEAR`(58)/`SYS_DROP_ADD`(59)/`SYS_DROP_ITEMS`(60)/
///   `SYS_DIE`(61)；
/// ② **敌人池字段布局变更**（本刀 T1）：`drop_table: u16` → `drop_count: [u8; 5]`，
///   `EnemyPool` 的 SoA 字段序/宽度变了 ⇒ SaveBytes 载荷编码随之变化（存档 wire format
///   变更）。T1 落地时未 bump（本刀内的中间状态），身份三元组直到本步才动，故理由记在
///   这里——只写 ① 会让人误以为旧档还能读。
/// **4 → 5**（shooter 刀 Task 1，2026-07-31）——**两条理由**：
/// ① syscall 号表将新增 62–76（`sh_*` 族 setter + `sh_fire`，本刀 T2/T3）；
/// ② **`TaskPool` 布局变更**（本刀 T1，**就是本次提交**）：新增并行数组
///   `shooters: [[ShooterSlot; 4]; 256]`（+45056 B），`SaveBytes` derive 自动把它写进载荷
///   ⇒ 存档 wire format 变化，旧档不可用新版解析。
///
/// **为什么在 T1 就 bump**：布局在本步就变了。上一刀（敌人死亡效果）把 bump 拖到第三步，
/// 结果中间几个 commit 处于"存档格式变了而身份三元组没变"的状态——这次提前。
/// ① 是对**本刀余下两步**的预告，落地时不再二次 bump。
///
/// **5 → 6**（小清洗刀，2026-07-31）：syscall 号表新增 `SYS_ATAN2`(77)/`SYS_DIST`(78)/
/// `SYS_NEAREST_ENEMY`(79)。**只有号表这一条理由**——本刀不动 `World` 布局（尺寸哨兵
/// 未变）、不动存档编码、不动任何既有 syscall 的语义，故旧档形状上仍可读；bump 是号表
/// 冻结纪律的机械要求（同 2→3 那次的单理由 bump 口径）。
///
/// **6 → 7**（敌坐标读口刀，2026-07-31）：syscall 号表新增 `SYS_ENEMY_X`(80)/
/// `SYS_ENEMY_Y`(81)。同上条——**只有号表这一条理由**，不动 `World` 布局（尺寸哨兵未变）、
/// 不动存档编码、不动任何既有 syscall 的语义。
///
/// **7 → 8**（探活读口刀，2026-07-31）：syscall 号表新增 `SYS_ENEMY_ALIVE`(82)。同上条
/// ——**只有号表这一条理由**，不动 `World` 布局（尺寸哨兵未变）、不动存档编码、不动任何
/// 既有 syscall 的语义（`enemy_hp` 的降级口径一字未改，旧探针照旧能用）。
///
/// **8 → 9**（敌句柄打包刀，2026-07-31）：**号表一个没长**，但六条既有 syscall 的
/// **取值编码变了**——`spawn_enemy`(22)/`nearest_enemy`(79) 押的从裸池 index 变成
/// `((gen & 0x7FFF) << 16) | index`，`enemy_hp`(12)/`enemy_x`(80)/`enemy_y`(81)/
/// `enemy_alive`(82) 的判据随之多一条 generation 比对。
///
/// **这比"号表新增"是更硬的兼容破坏**：加号只是让旧引擎跑不了新脚本，改取值语义会让
/// 旧回放/存档在新引擎上**静默走出另一条世界线**（同一个整数在两版里指的不是同一只敌）。
/// 故必须 bump，旧档拒载。`World` 布局与存档编码本身未动（generation 本来就在池里，
/// 尺寸哨兵未变）。
/// **不 bump 的记录**（难度档具名化刀，2026-07-31，`9` 保持不动）：加 `RANK_*` 五个引擎常量
/// 是**编译期词汇**，注入表只影响新脚本的折叠字面量，任何既有镜像的字节码一字未改（用了新
/// 名字的脚本其镜像 `content_hash` 本来就不同，身份三元组的另一头已覆盖）；`new_game_at` 的
/// `rank ∈ 0..=4` 校验只把**此前非法**的输入从"照单全收"改成显式 `Err`，任何合法开局产出的
/// 世界逐位不变，且校验**只在开机 API 上**——`load_bytes` 不校验 rank，故没有一份既有存档
/// 因此不可读。号表/op 表/池布局/`SaveBytes` 编码/相位序全未动。记在这里是为了让"这刀想过
/// 但判定不需要"留痕，免得后人以为漏了。
///
/// **9 → 10**（敌人运动动词族刀 T6，2026-07-31）：**敌池 SoA 布局变更**——新增 12 个并行
/// 数组（`speed`/`angle` 双表示 + 速度插值器 `vel_from_0/1`、`vel_to_0/1`、`vel_t`、
/// `vel_dur`、`vel_easing`、`vel_active`、`vel_space`、`vel_touched`）⇒ **`World` 快照
/// 字节数与 `SaveBytes` 载荷编码都变了**，旧存档/旧回放按新布局解读会走出另一条世界线，
/// 必须拒载。
///
/// **理由是布局，不是号表**：本刀同时新增 syscall 83–90（四条动词 + 四个 `$self_*`），
/// 但号表新增单独只让旧引擎跑不了新脚本（同 2→3 那次的单理由口径，硬度低一档）；
/// 这次真正逼着 bump 的是存档 wire format。
///
/// **10 → 11**（syscall 号表百分区重排，2026-07-31）：号表**取值语义全变**——同一个号在
/// 新旧两版指向**不同的 syscall**（例如 `20` 旧表是 `create_bullet`、新表是 `self_x`），
/// 旧镜像/旧回放按新表解读会**静默走出另一条世界线**，必须拒载。
///
/// **理由不是条数变**——本刀 **74 进 74 出**，一条没增没减、任何 syscall 的语义/参数序/
/// 降级口径一字未改；变的只有号。这比"号表新增"（2→3、7→8 那两次的单理由）**硬一档**，
/// 与 8→9 的"取值编码变了"同侧：加号只让旧引擎跑不了新脚本，改取值语义会让旧产物在新
/// 引擎上悄悄跑出另一件事。`World` 布局/`SaveBytes` 编码/op 表/相位序全未动
/// （尺寸哨兵未变；金向量校验和流逐字节不变，见 spec §9 修订记录三的实测记录——
/// §5 是派发代价那节，别找错）。
///
/// **11 → 12**（`wait` 语义修正，2026-08-01）：**任务调度语义变更**——`OP_WAIT` 从"存 `n`"
/// 改成"存 `n − 1`，且 `n == 0` 不 yield（同帧继续）"。同一份镜像在新旧两版**产出不同的
/// 世界演化**：每个 `wait(n)` 的周期从 `n+1` 变成 `n`，凡自己记帧数的脚本原先一律偏 1/n
/// （`boss_windchime.ecl` 的 600 帧符卡实际走 630 帧）。旧回放逐帧校验和从第一个 `wait`
/// 生效那帧起全线错开、旧存档恢复后的重演也接不上，**必须拒载**。
///
/// **理由是调度语义，不是编码**：`World` 布局/`SaveBytes` 编码/op 表条目/syscall 号表/
/// 相位序全未动（尺寸哨兵未变，`OP_WAIT` 的号与元数也没动）。硬度与 10→11 同侧——变的是
/// **同一个字节序列的含义**，旧产物在新引擎上不报错、只是悄悄走出另一条世界线，这类变更
/// 最需要版本闸挡住。附带代价（作者可见，已入 `docs/ecl-lang.md`）：`wait(0)` 成为真
/// no-op，`loop { wait(0); }` 与截断到 0 的 `wait(65536)` 都变成会被 `FAULT_BUDGET` 杀掉
/// 的死循环——确定性的响亮失败，不是 UB。
///
/// **12 → 13**（运动动词参数收窄，2026-09-03；follow-ups **D19**，人类裁定"收窄成拒收"）：
/// 五条运动动词 syscall（`move_enemy_to` / `move_vel` / `move_vel_xy` / `move_angle` /
/// `move_speed`）的 `dur`/`easing` 从裸 `as u16`/`as u8` 收窄成 `try_from`，越界即 P4-b
/// （`contract_viol` + `BAD_ARGS` + **整条 no-op**）。同一份镜像在新旧两版**产出不同的世界
/// 演化**：`move_enemy_to(30, x, y, 256)` 旧版静默当 Linear 走完整段插值、新版整条不执行，
/// 敌人停在原地——旧回放从那一帧起全线错开，必须拒载。
///
/// **这是"同一个字节序列的含义变了"，与 11→12 同侧**（不是布局、不是编码：`World` 布局/
/// `SaveBytes` 编码/op 表/号表/相位序全未动，尺寸哨兵未变）。旧行为的荒谬之处正是 bump 的
/// 理由：**能不能拒取决于越界值模 256 落在哪里**——`easing = 256` 静默变 Linear，
/// `easing = 264` 却被正确拒掉。`dur = -1` 同理变成"缓动 65535 帧 ≈ 18 分钟"。
///
/// 内容侧零改动（六份 `.ecl` 无一处传越界 `dur`/`easing`），金向量**实测逐字节不变**
/// ——两段场景都压不到这条新路径。
///
/// **13 → 14**（道具池 cap 512 → 1024，2026-09-03；follow-ups **F12** 定案）：**这条是
/// 布局变更**，与 11→12 / 12→13 那两条"含义变了"不同侧——`World` 真的变宽了
/// （WorldBody 977824→989152、World 1137624→1148952，+11 328 B），快照与存档的 wire
/// format 随之改变，旧存档在新引擎上**尺寸就对不上**，是响亮的失败而非悄悄走岔。
///
/// 起因是真内容里的实测：demo 局收卡那一帧（4954）场上 626 颗弹被消，而**消弹转星星是
/// 1:1**（`world/settle.rs` 趟一逐颗调 `spawn_star_at`），512 格的道具池**四个难度档全部
/// 溢出**（Easy 3 / Normal 37 / Hard 67 / Lunatic 104 颗星星没生成 = 丢分）。1024 盖得住
/// 当前内容的弹数峰值（rank 3 为 814）并留余量，代价 +11 KiB（对照弹池 433 KiB 是零头）。
///
/// **金向量必然改变，且形态是本刀的直接指纹**：校验和**哈希全槽、不用 alive 掩码**（P6），
/// 多出来的 512 个空道具槽从**帧 0** 就进哈希 ⇒ 两段场景自帧 0 起全差。这与前几刀"逐字节
/// 不变"的性质不同，是加宽池不可避免的结果，不是行为回归。
///
/// ⚠ **1024 不是结构性保证，只是把线挪远**：弹池 8192 是道具池的 8 倍，1:1 转换天生可能
/// 溢出。溢出时走 P4-a 确定性降级（该颗不生成 + 逐颗计数、循环不短路，判别腿
/// `star_pool_full_counts_every_missing_star`）——**这是已知设计边界，不是待修的债**，
/// 口径写在 `docs/ecl-ops.md` 的 54 号与 `WorldBody::spawn_star_at` 的文档里。
///
/// **14 → 15**（自机能力刀：时间停止 + bomb，2026-09-03）：**布局 + 号表 + 输入词表
/// 三重变更**，任一即足以 bump。① `World` 变宽（`WorldBody.freeze_left` 4 B +
/// `PlayerState.time_stops` 1 B×2）⇒ 快照与存档 wire format 变，旧存档尺寸对不上，
/// 是响亮失败；② syscall 号表新增 `513 add_time_stops` / `560 time_stop_player`；
/// ③ 输入动作词表新增 `BTN_TIMESTOP = 7`（位=0 等价旧行为，故非回放破坏性变更，
/// 但 `actions_vocab_hash` 变）；④ `WorldTables` 新增 `CharacterCfg.bomb`
/// ⇒ 表 `content_hash` 变 ⇒ 身份三元组变。
///
/// **金向量预期改变**（新字段进哈希，同道具池刀的道理）：形态可解释，不是行为回归。
///
/// **15 → 16**（表现契约 v2，2026-09-07）：**布局 + 号表两重变更**。① `World` 变宽：弹池
/// `born_frame: u32`（×8192）+ 敌池 `anm_state_frame: u32`（×256）进校验和与存档；
/// `WorldBody` 新增第四条纯输出缓冲 `vanished`（1024 × 12 B + len，checksum/存档皆 skip，
/// 但结构尺寸变）+ `diag.vanished_overflow`（进校验和）。② syscall 号表新增
/// `430 set_anm_state` / `721 fx_at` / `722 fx_on`；通道 B 引擎段新增 `REQ_FX_AT = 8` /
/// `REQ_FX_ATTACHED = 9`。**金向量预期改变**（两个新字段进哈希，自帧 0 起），行为零改动：
/// `vanished` 不改任何相位决策，两个帧号字段在核内无消费者。
///
/// **16 → 17**（时间机制内核刀，2026-09-07）：**布局 + 词表 + 事件号三重变更**。
/// ① `PlayerState.hit_frame: u32`（×2）进校验和与存档（被 `PlayerState` 尾部 4 B 对齐
/// padding 吃掉，`size_of` 不变——D20 第四次，哨兵仍未响）；② 输入动作词表新增 `BTN_JUMP = 8` /
/// `BTN_REWIND = 9`（位=0 等价旧行为，`actions_vocab_hash` 变）；③ 生命态新增
/// `LIFE_JUMPING = 5`、事件新增 `EVT_REWIND_REQUESTED = 10`。**金向量预期改变**（新字段
/// 进哈希，自帧 0 起）；风铃卡不按新键，行为零改动。
///
/// **17 → 18**（壳子刀·转场协议修正，2026-09-07）：**号表 + 事件号**。syscall 新增
/// `723 stage_clear`（发 `EVT_STAGE_CLEARED = 11` 事实事件，表层 codegen 追发 `WAIT 1`
/// 让出一帧——修掉「`emit_req(REQ_STAGE_CLEAR)` 不让出帧、下一关开头会在挂牌同一帧跑掉」
/// 的缝）。布局未动，金向量逐字节不变（风铃卡不调它）。
///
/// **18 → 19**（壳子刀，2026-09-11）：`PlayerState.continues: u8`（×2，进校验和与存档；
/// 追在 `hit_frame` 之后，`PlayerState` 64→72、`World` +16 B，尺寸哨兵这次响了）+ 输入词表新增
/// `BTN_CONTINUE = 10`（位=0 等价旧行为，`actions_vocab_hash` 变）。金向量：新字段恒 0
/// 但进哈希 ⇒ 预期改变；实测为准。
///
/// **19 → 20**（玩法刀，2026-09-14）：**布局 + 碰撞矩阵 + 号表 + 词表 + 表格式五重变更**。
/// ① `PlayerState` 删 `bomb_phase: u8`/`bomb_timer: u16`/`time_stops: u8`、加 `jump_cd: u16`/
/// `deaths: u8` ⇒ 存档 wire format 变（`size_of` 实测不变，D20 盲区）；② 碰撞矩阵行 8
/// `ROW_STOP_TOUCH`（相位 6/7/9 冻结分支）；③ syscall 513 `add_time_stops` 退役（号不复用）、
/// `add_bombs` 上钳 5；④ 输入词表退役位 7/9、生命态退役 3（`LIFE_RESPAWNING`），死亡改为原地
/// 继续 + `EVT_REWIND_REQUESTED`；⑤ `WorldTables` 删 `CharacterCfg.bomb`（`TABLE_VERSION` 5）
/// ⇒ 表 `content_hash` 变；回放头 `LOG_FILE_VER` 2。**金向量预期改变**；实测为准。
pub const ENGINE_VER: u32 = 20;

pub use stg_derive::define_pool;

/// 最大自机数（共场 co-op 超集，§7.5）。
pub const MAX_PLAYERS: usize = 2;

pub mod boss;
pub mod bullets;
pub mod checksum;
pub mod consts;
pub mod ecl;
pub mod enemy;
pub mod events;
pub mod field;
pub mod input;
pub mod items;
pub mod math;
pub mod player;
pub mod reqs;
pub mod rng;
pub mod save;
pub mod shots;
pub mod spell;
pub mod step;
pub mod tables;
pub mod timeline;
pub mod world;
pub mod xform;

pub use step::{World, step, step_with_director};

// ── Phase 1 M0+ 起逐步落地（占位，勿在 scaffold 阶段实现）──────────────────
//
//   pub mod pool;   // D2 `define_pool!` 宏与六个实体池（SoA + generation 句柄）
//   pub mod world;  // Part III 世界本体（WorldBody 字段、pub(crate) 相位函数）
//   pub mod step;   // P2 组装层宪法顺序（导演槽 + PhaseGuard）
