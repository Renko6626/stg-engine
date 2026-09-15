# stg-rl env 刀 —— 接收 ECL、批量跑弹幕的强化学习环境（设计，2026-09-15）

> 状态：**设计已拍板，待实施**。
> 来源：RL 路线 A（在 stg-engine 上高速训练 → 迁移到 th06nc / TH18）。前置：经典机体刀（机体 1 =
> `Kit::Classic`，ENGINE_VER 22）。观测契约：`stg-agent-proto` v1（`SPEC.md` / `c/world.h`），
> 与 `renkolab/docs/superpowers/specs/2026-09-10-ai-agent-bridge-design.md` §6 同一定位：stg-engine
> 是契约的**第二个观测生产者**。
> 本刀交付：`stg-rl`（Rust env 核心）+ `stg-py`（PyO3 wheel `stg_rl`）+ 分发 workflow。
> 训练代码 / 特征化 / reward / 训练作业包**不在本刀**（见 §12）。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 首个训练目标 | **单卡 / 单段生存**：mark 中段启动，episode = 撑过这一段或死一次 |
| ② | 模型结构（背景） | 两级：**小模型**输入「局面 + 意图」，一边躲弹一边朝指定方向移动（本阶段训练对象）；**指挥模型**之后训练，给小模型输出意图。意图完全不进引擎 |
| ③ | 观测格式 | **stg-agent-proto 表**（字段名 / 类型 / 偏移逐字相同），env 只出原始定点表 |
| ④ | 特征化 | **模型外的薄 Python 胶水**（训练仓，可随时改），不并入模型、不进 wheel；「取最近 K 颗弹」也在这层 |
| ⑤ | reward | **env 不给标量 reward**，给逐步原始计数（§6），reward 在 Python 组合（含拟人项：贴边、操作抖动等） |
| ⑥ | episode 边界 | **第一次死亡即结束**（跨死亡策略不在本阶段） |
| ⑦ | 开局多样化 | **只随机预热帧**（随机游走），core 不加写自机位置的 API |
| ⑧ | 并行形态 | **单进程**：Rust 专属 rayon 线程池批量 step、释放 GIL，写入调用方提供的 torch pinned 缓冲，零拷贝交 Python，GPU 整批推理。**禁止 subprocess 式 VecEnv** |
| ⑨ | crate 形态 | 方案①：`stg-rl`（纯 Rust）+ `stg-py`（PyO3 abi3 薄壳）；不做进程外 ipc |
| ⑩ | 分发 | 标准 GitHub：CI 出预编译 wheel 挂 Release，集群 `pip install <url>`（仓库将转 public；训练在 AutoDL 等租用 GPU） |
| ⑪ | 表容量 | 不沿用红魔乡 640：stg-engine 后端在 HELLO 声明自己的 cap。bullets **可配 `bullets_cap`，默认 1024，上限 8192**；enemies 256 / items 1024（= 池上限，永不截断）；lasers 恒 0 行 |
| ⑫ | 碎片道具 | proto `AP_ITEM_*` **新增 8 = 残机碎片、9 = 炸弹碎片** |
| ⑬ | 激光 | proto 已有直线激光表；stg-engine 无激光池 ⇒ 恒发空表。训练分布无激光记为已知迁移缺口 |

## 2. 架构

```
 Python（训练仓）                     stg_rl wheel                       stg-core（断层线以下，不改语义）
 ────────────────                     ────────────                       ─────────────────────────────
 alloc_buffers() ─ torch pinned ─┐
 featurize(buf)  ◄── GPU ◄───────┤    stg-py (PyO3 abi3)
 policy(feat) → actions ─────────┼──► VecEnv.step(actions)  ── GIL 释放 ──► stg-rl::VecEnv
                                 │                                          ├ rayon 专属池分块
                                 └──◄ 原地写入 player/enemies/bullets(CSR)… ├ Env × N：World + 起点 + 种子流
                                                                            ├ ImagePool（EclImage 只读共享）
                                                                            └ encode → proto 布局字节
```

- **运行时只有一次拷贝**：pinned 缓冲 → `.to("cuda", non_blocking=True)`。「协议」是内存布局，不是序列化。
  编码成 OBS 记录字节只在写 `.stglog` 时发生（本刀不做写日志，见 §12）。
- `EclImage` 不在 `World` 里（I7），`step` 只读借用 ⇒ N 个 env 共享同一份镜像，不同卡 = 镜像池。

## 3. Python API

```python
import stg_rl

img = stg_rl.compile_bundled("game")                            # 或 compile_sources([(name, src), ...])
buf = stg_rl.alloc_buffers(num_envs=512, bullets_cap=1024, pin=True)   # dict[str, torch.Tensor]
env = stg_rl.VecEnv(
    num_envs=512, threads=32, buffers=buf,
    images={"game": img},
    starts=[stg_rl.Start(image="game", mark=15, rank=2, weight=1.0)],
    frame_skip=1, max_frames=3600, warmup_max=120,
    end_on=["phase_ended", "spell_captured", "spell_failed", "stage_cleared"],
    bullets_cap=1024, seed=0,
)
env.reset()                  # 写全部缓冲
env.step(actions)            # actions: uint32[N]（numpy 或 CPU tensor），写全部缓冲
env.set_start_weights([...]) # 课程学习：改起点采样权重
stg_rl.hello(bullets_cap=1024)   # proto HELLO JSON（str）
stg_rl.OFFSETS               # {"bullets": {"x": (0,"fx"), ...}, ...} 供胶水按字节切片
stg_rl.build_info()          # {"version", "engine_ver", "tables_hash", "git_sha"}
```

- 编译错误抛 `stg_rl.CompileError`（消息 = 带文件名行列的渲染文本）；配置错误（未知镜像名、mark 查无、
  rank 越界、缓冲形状不符）抛 `ValueError`，**在构造期**抛，不在 step 里抛。
- **不做** `gymnasium` 适配（Ruling 4）：训练仓尚未定框架，适配层按框架写才不白写。文档写明：
  禁止再套 `AsyncVectorEnv` / `SubprocVecEnv` / `make_vec_env`——那会把整个 env 复制进子进程、吃掉全部性能。

### 3.1 缓冲布局（`alloc_buffers` 产出，调用方持有）

| 键 | dtype / 形状 | 说明 |
|---|---|---|
| `frame`, `phase` | u32 `[N]` | proto OBS 头 |
| `player` | u8 `[N, 36]` | proto player 行 |
| `enemies` | u8 `[N, 256, 38]` + `enemies_count` i32 `[N]` | 补齐 |
| `bullets` | u8 `[N*bullets_cap, 30]` + `bullets_offsets` i32 `[N+1]` | **CSR**：各 env 行首尾相接，env i 的行 = `[offsets[i], offsets[i+1])` |
| `items` | u8 `[N*1024, 18]` + `items_offsets` i32 `[N+1]` | CSR |
| `lasers_count` | i32 `[N]` | 恒 0（HELLO 仍声明 lasers 表，形状跨后端一致） |
| `bullets_total`, `bullets_dropped` | i32 `[N]` | 场上弹数 / 溢出丢弃数 |
| `events` | i32 `[N, 8]` | §6 |
| `done` | u8 `[N]` | §4.4 |
| `ep_frames`, `warmup_retries`, `start_index` | i32 `[N]` | 刚结束那一局的统计（`done != 0` 时有效） |

- `alloc_buffers(..., backend="torch"|"numpy")`：默认 `"torch"`（pinned，torch 为**可选依赖**，未安装时报错提示改用
  `"numpy"`）；`"numpy"` 分配普通内存，供测试 / CPU 调试（wheel 冒烟不装 torch）。
- 缓冲由调用方持有，Rust 经 numpy 视图（torch 张量走 `.numpy()`）拿 `&mut [u8]`/`&mut [i32]` 原地写；
  构造期与每次 step 前校验形状 / dtype / C 连续。
- CSR 缓冲的 H2D 只需搬 `bullets[:offsets[N]]`（连续前缀，pinned 异步照样生效）。
- torch 不认 numpy 结构化 dtype ⇒ 缓冲一律裸字节，字段抽取在胶水层按 `OFFSETS` 切片 + `.view(torch.int32)`。

## 4. episode 生命周期

### 4.1 起点与种子流

- `starts` 列表 + 权重；每局按权重抽一个起点（镜像 × mark × rank）。机体恒 1（`Kit::Classic`）。
- 种子流：`episode_seed = splitmix64(base_seed ⊕ env_index·K1 ⊕ episode_counter·K2)`（常量钉死，Rust 实现）。
  起点抽样、预热随机游走都从这条流派生 ⇒ **同 base_seed + 同动作序列 ⇒ 缓冲逐字节相同，与线程数无关**。

### 4.2 reset（单个 env）

1. 抽起点与 `episode_seed`。
2. **开局快照缓存**：键 = (镜像, mark, rank)，首次用到时 `World::new_game_at(0, rank, mark, Loadout{character:1, ..default}, &image)`
   生成模板，之后缓存；本局 `template.copy_into(&mut world)`。
3. `world.reseed(episode_seed)`（§8 core 改动）。
4. **随机预热**：抽 `k ∈ [0, warmup_max]`；逐帧喂随机游走动作（9 方向之一，每段持续 `[8, 32]` 帧，**不按 SHOT/BOMB**），
   跑 k 帧。期间若自机进入决死窗口或段落结束事件出现 ⇒ 用 `splitmix64(episode_seed, retry)` 重来（回到第 2 步），
   最多 8 次；耗尽则 k = 0 开局。预热帧不计 events、不计 `ep_frames`。
5. 写观测。

### 4.3 step

每个 env 重复 `frame_skip` 帧：喂 `actions[i] & ACTION_MASK`（位 0–6：UP/DOWN/LEFT/RIGHT/SHOT/BOMB/SLOW；
其余位丢弃）→ `stg_core::step` → 累加 events → 判结束（判到即停，不跑完剩余 skip 帧）。然后写观测。

### 4.4 done 码与自动 reset

| 码 | 含义 | 判据 |
|---|---|---|
| 0 | 继续 | — |
| 1 | 死亡 | 本帧 `EVT_PLAYER_DIED`（决死窗口耗尽后；窗口内 bomb 救回不算） |
| 2 | 撑过这一段 | 本帧出现 `end_on` 中任一事件 |
| 3 | 超时截断 | `ep_frames ≥ max_frames` |

同帧死亡与段落结束并存 ⇒ 1 优先。`done != 0` 的 env **在同一次 step 内自动 reset**，缓冲里的观测已是新局
第一帧；`ep_frames` / `warmup_retries` / `start_index` 写刚结束那局的值。v1 不提供终局前最后一帧观测。

## 5. 观测字段映射（stg-engine 后端）

`obs_timing = "prev-frame-final"`；`field.move_area` = 全场 `[-192,192]×[0,448]`（引擎把自机钳在场内）。
HELLO：`backend = "stg-engine@<ENGINE_VER>"`，`policy = "rl"`，`actions` 声明位 0–6，各表 `cap` 按本节。

**player**：`x,y,hit_radius` ← `PlayerState` 原值；`speed/speed_focus` ← `CharacterCfg.high_speed/low_speed`；
`focus` ← 本帧输入 `BTN_SLOW` 位；`state` ← `life_state` 原值；`lives,bombs,life_frags,bomb_frags` ←
`lives,bombs,life_pieces,bomb_pieces`；`power` ← 原值（0–400 厘火力）；`score` ← u64 **饱和**到 u32；`graze` ← 原值。

**phase 位**：`IN_GAME` 恒 1；`BOMB_ACTIVE` ← `bomb_timer > 0`；`SPELL_ACTIVE` ← 任一 `SpellSlot.active`；
`PLAYER_CONTROLLABLE` ← 自机 ALIVE ∧ 未被 ECL 演出冻结（A 组）。其余 0。

**bullets**（cap = `bullets_cap`）：
- 场上存活敌弹 `alive ≤ cap` ⇒ 全部，**按池索引序**。
- `alive > cap`（溢出保护）⇒ 按全序键 `(到自机平方距离 len_sq, 池索引)` 选最近 `cap` 颗，**输出仍按池索引序**；
  `bullets_dropped = alive − cap`。此规则保证：胶水层若按**当前距离**取最近 K 颗（K ≤ cap），结果与全量严格一致；
  若胶水按其它键（如预测最近接近距离，§12）选取，仅在溢出帧近似成立——训练中 `bullets_dropped > 0` 即应调大 `bullets_cap`。
- `x,y,vx,vy,radius` 原值；`speed = isqrt(vx²+vy²)`、`angle = atan2(vy,vx)`（core 整数 `isqrt` / CORDIC，
  按 proto 派生规则现算，不取池里可能滞后的极坐标）；`flags`：bit0 可碰撞 ← `delay == 0`，bit1 圆形恒 1，
  bit2 已擦 ← `grazed_by & 1`；`state`：1 飞行 / 0 延迟中；`type` ← `sprite`。

**enemies**（cap 256，按池索引序）：`x,y,hp,hp_max` 原值；`hit_w = hit_h = radius`（体碰）；
`hurt_w = hurt_h = hurtbox`（受击）；`flags`：bit0 boss ← 句柄等于某个 `active != 0` 的 `BossUiSlot.enemy`，
bit4 可碰撞 ← `flags & (ENEMY_NO_BODY | ENEMY_DYING) == 0`；`id` ← `pack_enemy_handle`（带 gen 低 15 位）。

**items**（cap 1024，按池索引序，跳过 `magnet_to == MAGNET_PICKED`）：`x,y,vx,vy` 原值；`flags` bit0 ←
`magnet_to != MAGNET_NONE`，bit1 恒 0；`kind`：POWER→1、POINT→2、LIFE_PIECE→**8**、BOMB_PIECE→**9**、STAR→7。

**lasers**：0 行。自机弹不在 Tier 0，不输出。

## 6. events `[N, 8]`（int32，frame_skip 内累加，预热帧不计）

| 列 | 名 | 来源 |
|---|---|---|
| 0 | `died` | `EVT_PLAYER_DIED` 计数 |
| 1 | `graze` | `players[0].graze` 增量 |
| 2 | `score` | `players[0].score` 增量（饱和到 i32） |
| 3 | `enemies_killed` | `EVT_ENEMY_DIED` 计数 |
| 4 | `shot_hits` | `EVT_SHOT_HIT_ENEMY` 计数 |
| 5 | `bombs_used` | `bombs` 减少量（仅统计下降） |
| 6 | `items_picked` | `EVT_ITEM_PICKED` 计数 |
| 7 | `segment_end` | 0 无 / 1 `phase_ended` / 2 `spell_captured` / 3 `spell_failed` / 4 `stage_cleared`（取本步最后一个；不论是否在 `end_on` 里都记） |

列序与名字由 `stg_rl.EVENT_COLUMNS` 导出，胶水层按名取。

## 7. 并行与性能

- `VecEnv` 持**专属** `rayon::ThreadPool`（`threads` 显式，默认 `min(available_parallelism, N)`），不用全局池。
- `step` 内 `py.allow_threads` 释放 GIL；env 与各自缓冲切片是不相交 `&mut`，`par_chunks_mut` 按
  `with_min_len(ceil(N / threads))` 分块（单 env step ~6µs，逐 env 派发调度开销会反客为主）。
- CSR 写入：各 env 先写入自己的定长暂存区（`bullets_cap` 行），并行结束后单线程按 env 序 memcpy 压实并写 `offsets`。
- 文档要求训练侧设 `torch.set_num_threads`，避免与 rayon 抢核。
- 不做 CPU/GPU 流水线重叠（async env），基准显示 GPU 等 CPU 时再议。
- harness 新子命令 `rl-bench`：steps/s × threads（1/2/4/…/64）× num_envs，结果追加 `docs/bench-baseline.md`。

## 8. stg-core 改动（本刀唯一）

`World::reseed(&mut self, seed: u64)`：只写 `body.rng = Pcg32::new(seed, RNG_SEQ)` 与 `seed` 字段。
- 判别测试：对若干 (seed, rank, mark)，`new_game_at(0,…)` + `reseed(seed)` 的 `checksum()` == `new_game_at(seed,…)`，
  并各 step 600 帧逐帧相等。**若 `new_game_at` 在 start_main 期间消费了 RNG 或写入其它 seed 相关状态，
  本测试红 ⇒ 放弃模板缓存，改为每局 `new_game_at`（写进计划的分支）**。
- 文档注明：训练用；回放 / 握手身份仍以开机 seed 为准（`Timeline` 不调用它）。
- 不改布局、不改演化 ⇒ **不 bump `ENGINE_VER`**。

## 9. stg-agent-proto 仓改动（前置，独立小提交）

1. `AP_ITEM_*` 新增 `8 = LIFE_PIECE`、`9 = BOMB_PIECE`（`c/world.h`、`src/stgagent/consts.py`、SPEC 表）。
2. SPEC 写明：表 `cap` 由各后端 HELLO 声明；`AP_MAX_*`（640/256/64/1024）只是 C 结构体默认值（可 `#define` 覆盖），
   不是契约上限；OBS `count` 为 u16。
3. 推到 GitHub（新建仓库），stg-engine 从中拷贝 HELLO 布局夹具。

## 10. 分发

- `crates/stg-py/pyproject.toml`（maturin 构建后端），包名 `stg_rl`，`abi3-py310`。
- 默认内容：build.rs 把 `godot/ecl/game/*.ecl` 按文件名排序以 `include_str!` 嵌进 `stg-rl`；`stg_rl.bundled_sources("game")` 返回 `[(文件名, 源码)]`、`stg_rl.compile_bundled("game")` 直接译成镜像（免去 wheel 包数据拷贝，离线可用性相同；Ruling 1）。
- `.github/workflows/wheels.yml`，触发 = 推 `rl-v*` tag：
  - `manylinux_2_28 x86_64`（maturin-action docker）+ `windows x86_64`；
  - 构建后在干净 venv 装 wheel 跑 `pytest crates/stg-py/tests`；
  - 通过后上传到该 tag 的 GitHub Release。
- 集群安装：`pip install https://github.com/Renko6626/stg-engine/releases/download/rl-v<ver>/stg_rl-<ver>-cp310-abi3-manylinux_2_28_x86_64.whl`。
- `build_info()` 的 `git_sha` 由 build.rs 在构建期注入（无 git 时为 `"unknown"`）。

## 11. 测试

Rust（`cargo test -p stg-rl`）：
- **确定性**：同 base_seed + 同动作序列，`threads = 1` 与 `threads = 8` 各跑 300 步，全部缓冲逐字节相等；换 base_seed 则不等。
- **编码判别**：手摆已知世界（2 弹其一 delay>0 且被擦、2 敌其一 boss 其一 NO_BODY、4 种道具各 1 其一被吸附、一个活跃符卡），
  断言每行每字段字节（互异非零值，防字段错位）。
- **溢出保护**：cap = 4、场上 10 弹 ⇒ 输出恰是距离最近 4 颗、按池索引序、dropped = 6；距离相同者按池索引裁决。
- **派生量**：`speed/angle` 与 core `isqrt/atan2` 对若干向量（含轴向、零向量）一致。
- **reseed 等价**（§8）。
- **生命周期**：预热中被弹 ⇒ 重试且确定、上限 8；死亡 done=1 同步自动 reset；`end_on` 事件 done=2；
  超时 done=3；同帧死亡 + 段落结束 ⇒ 1；frame_skip=3 时 events 累加且判到即停。
- **HELLO 布局对拍**：`hello()` 的 Tier 0 表字段名/类型/偏移/stride 与 proto 夹具逐项相等（cap 除外）。
- **CSR**：offsets 单调、`offsets[N] = Σ 各 env 行数`、行内容与逐 env 编码一致。

Python（`pytest crates/stg-py/tests`，CI 与 wheel 冒烟共用）：
- import / `build_info()` / `alloc_buffers` 形状；reset + 100 步不崩、`done` 出现过 1 或 2（用会死的动作）。
- `stgagent.parse_hello(stg_rl.hello())` 成功，并用其 dtype 解码一段 CSR 行，与按 `OFFSETS` 手切结果一致（stgagent 为测试依赖）。
- 构造期错误：未知镜像名 / mark 查无 / 缓冲形状不符 ⇒ `ValueError`；坏 ECL ⇒ `CompileError` 且消息含行列。

## 12. 已知代价与非目标

### 12.1 胶水层设计要点（训练仓参考，2026-09-15 讨论记录，非本刀交付）

躲弹小模型是局域操作，远处弹意义小 ⇒ 截断在胶水层做（模型不臃肿、RL 好训）。但「局域」按**危险**而非距离度量：
- **选取键 = 预测最近接近距离**：弹与自机按匀速外推（自机速度取「当前意图方向 × 移速」），未来 H 帧内最小距离，
  闭式解 GPU 批量算；取最小 K 颗 + 阈值过滤 + mask。纯「当前最近 K 颗」会漏掉远处高速弹（8px/帧、160px ≈ 20 帧即到）。
- **补粗粒度全局密度图**（8×8 ~ 12×14 格计数，可按速度方向加权）：否则意图目标在远处弹墙后时模型只会贴墙躲；
  路线规划归指挥模型，小模型 = 局部精细 + 全局粗略。
- **结构**：集合编码器（DeepSets / 轻注意力）+ mask，排序无关，K 取 32–128；全部换相对自机坐标；左右镜像增强。
- **截断边界抖动**：把接近距离本身作为特征喂入，边界上的弹权重天然低。
- 敌体（体碰致死）按同一键取少量一并喂入。
- 部署时 C 侧照此复刻（与特征化同一代价）。

- **部署侧要复刻胶水**：特征化与「最近 K 颗」住 Python 胶水，th06nc / TH18 DLL 跑 ONNX 时须在 C 侧同样实现，靠对拍测试守一致。
- **训练分布无激光**（§1⑬）；引擎激光池另开刀（world-design §709）。
- 非目标：训练代码 / 特征化 / reward（训练仓）；**训练作业包**（容器镜像 + 入口 + 输出目录约定，选定作业平台后另开子项目；
  本刀为它提供无交互安装、离线内容、`build_info()`）；`.stglog` 写出；`.stgr` 回放 → 训练轨迹导出器；
  async env；跨死亡 episode；终局前最后一帧观测；非 x86_64 Linux / Windows 以外的 wheel。

## 13. 实施偏差

写计划（`docs/superpowers/plans/2026-09-15-stg-rl-env.md` §Rulings）时对本文档的裁定，逐条落在此：

| # | 偏差 | 理由 |
|---|---|---|
| 1 | 内置内容改用 `stg-rl` 的 build.rs 以 `include_str!` 嵌入（按文件名排序），Python 暴露 `bundled_sources("game")` / `compile_bundled("game")`，替代 §3/§10 的 `bundled_content()` 返回文件系统路径 | 免去 wheel 包数据拷贝，离线可用性相同；`EclImage` 由源码每次编译，无镜像格式冻结债 |
| 2 | `stg-py` 不进主 workspace（根 `Cargo.toml` `exclude` + 自带 `[workspace]`/`Cargo.lock`） | PyO3 cdylib 进 `cargo test --workspace` 会要求 CI 有 libpython，破坏现有三平台闸门 |
| 3 | `pack_enemy_handle` 搬到 `stg_core::enemy::pack_handle` 并公开，syscall 调用它 | enemies `id` 编码（§5）需要同一打包公式，复制一份会漂移 |
| 4 | §3 的可选 `gymnasium` 适配 **v1 不做** | 训练仓尚未定框架，适配层按框架写才不白写；文档写明「禁止套 subprocess vector wrapper」即可 |

