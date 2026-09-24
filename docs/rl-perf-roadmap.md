# RL 训练吞吐：引擎侧待办（2026-09-24 调研）

> **来源**：训练仓 `stg-rl-train` 在 Magnus A100 上做分阶段计时后，对本仓做了三路只读调研
> （VecEnv 批处理与线程 / 单 env 模拟热点 / 观测布局），结论汇总于此。训练仓的计时数据见
> `stg-rl-train/docs/perf-baseline.md`。
> **已落地**（`rl-v0.1.1`，commit `4d95cd9`）：VecEnv 分块修复、`tick_steps` 遇 END 即停，实测见
> `docs/bench-baseline.md` 末节。本文只记**还没做**的。
> **拍板（2026-09-24）**：引擎尚未正式发布，**改变校验和可以接受**（旧 `.stgr` 回放随之失效，
> 按 ENGINE_VER 评审规矩 bump 即可）。字节布局也不是不能动——目标是性能与可维护性。

## 背景数字（A100、2048 env、每次 PPO 更新 64 步，更新总长约 1.94 s）

| 训练侧阶段 | 秒/更新 | 与本仓的关系 |
|---|---:|---|
| `env_step` | 约 0.55 | 本仓 `VecEnv::step`（`rl-v0.1.1` 预计约减半，待 Magnus 实测） |
| PPO 更新 | 约 0.55 | 与本仓无关 |
| `h2d`（拷贝 + 解码 + 敌人速度 + 意图状态） | 约 0.48 | 主要由观测布局决定（下文 §1） |
| 特征化 + reward + 策略前向 | 约 0.30 | 与本仓无关（已录 CUDA 图） |

单线程下 2048 env 一步约 39 ms：模拟 30.6（其中自动 reset 7.2——一次 reset 含 `copy_into` 与最多
120 帧 warmup，约 1 ms，普通一步约 7 µs）、写观测 7.5、弹/道具行拷贝 1.35。模拟本身不慢；
训练侧的瓶颈是逐个小算子的发射开销（每个约 30 µs），所以**减少训练侧要做的算子数**比压单 env 的微秒更值钱。

## 1. RL 专用观测缓冲（最大一件，跨三仓，需先出 spec）

**关键事实**：部署端模型从不读 Tier 0 字节——`stg-agent-proto/c/sa_model.c:46-134` 直接从
`ap_world_t` 的浮点字段构造模型输入。Tier 0 只是线协议与日志格式。训练与部署真正要对齐的是
**模型输入签名**（`sa_model.h`），不是字节布局。

**训练侧实际消费的**（其余全是线协议 / 日志用的）：
- player：x、y、hit_radius、speed（高速值）、focus
- bullets：x、y、vx、vy、radius，按 `flags & 1`（有判定）取舍
- enemies：x、y、hit_w、boss、vx、vy，按存活与 `flags & 0x10` 取舍；id 只用来差分速度
- 逐 env：done、events[8]、ep_frames、start_index；计数：bullets_offsets、enemies_count、bullets_dropped

**现状的浪费**：Q16.16 与 BAM 迫使训练侧在 GPU 上逐字段按字节解码（bullets 的 radius 在偏移 22，
不对齐，还得 clone）；bullets 的 speed/angle（atan2 + isqrt 记忆）、items 每步编码而训练从不读；
enemies 按 256 行整表上传（每步 19 MiB，实际存活最多约 61 只），count 之后是陈旧行；7 次零碎小拷贝。

**提案（选项 a：另开 RL 缓冲，Tier 0 不动）**：
- 一整块 pinned 内存，只有 4 字节字：全局头（m_bullets、m_enemies、n、版本）+ 每 env 头
  （f32 自机五项 + i32 done/ep_frames/start_index/events[8]/各计数）+ 弹行 24 B（f32 x,y,vx,vy,r +
  i32 目标下标）+ 敌行 28 B（f32 x,y,hit_w,boss,vx,vy + i32 目标下标）；只导出有判定的行。
- 定点转浮点用 `(raw as f32) * (1.0 / 65536.0)`——与训练侧 `_fx` 同两步 IEEE 运算，逐位一致。
- **敌人速度由引擎算**：每个 env 按池槽号存上一帧坐标与 generation，O(E)；规则照抄训练侧
  `enemy_velocity`（`(cur − prev) / frame_skip`、瞬移守卫 `max|Δ| ≤ 16·frame_skip`、reset 后首步为 0、
  上一帧坐标取全部存活敌人含无判定的）。顺带消掉一个边角：`pack_handle` 只留 15 位 generation
  （`enemy.rs:94`），回绕后 0 号槽的 id 会是 0，训练侧 `id != 0` 守卫永远对不上它。
- `VecEnv(..., tier0=false)` 训练时跳过 Tier 0 编码（顺带省掉本仓写观测 7.5 ms 的大头）；
  `replay_gif` 与诊断脚本仍用 `tier0=true`。`env_base` 构造参数让批量评测各组直接写全局下标。
- **估算（未实测）**：每步上传约 25 MiB → 5~8 MiB，9 次拷贝 → 1 次，训练侧每步约 110 个小算子 →
  约 10 个；训练侧 h2d 约 0.48 → 0.15 s/更新。还能把解码与特征化一起录进 CUDA 图（固定形状的
  staging 缓冲 + 设备端 `arange < m` 掩码）。前两次同类估算都偏乐观 2~3 倍，以 A/B 为准。
- **一致性保证**：① 迁移测试：新缓冲 == 旧 Tier 0 解码 + 参照 `enemy_velocity`，逐位，覆盖自动 reset、
  第 5 关咲夜瞬移卡、frame_skip=2；② 跨语言测试：引擎写 `.stglog`，proto 现有的 C-fill 转储
  （`tests/test_model_parity.py`）对它构造输入，与 RL 缓冲比到 1e-6；③ `check_model_parity` 照旧。
  模型输入语义不变 ⇒ 不 bump GRAPH_VERSION、不动动作表，旧 checkpoint 仍可用；wheel 小版本升级，
  `build_info` 加 `rl_layout`，训练侧 import 时断言。
- **选项 b（改 Tier 0 本身）不推荐**：改字段类型违反 `stg-agent-proto/SPEC.md` 三后端字段逐字相同的规则，
  逼出 proto v2；th06nc / th18 的抽取器都得补敌人速度；日志读取器要兼容 v1/v2；f32 在 256 px 外
  存不下 Q16.16 的精确值，日志失去整数精确性；训练侧仍要为线协议字段付列收集的代价。
- 实施顺序：引擎（`write_model_obs`、按槽速度状态、arena、`RL_COLUMNS`、`tier0`/`env_base`、Rust 测试）
  → 训练仓新解码路径（开关控制）+ 逐位迁移测试 + A100 A/B → proto 的 SPEC「模型输入 v1」一节 +
  日志对拍测试 → 训练侧把解码并进 rollout 图、删旧路径 → 发 wheel，训练热路径不再走 Tier 0。
- 并入此项：`compact()` 改为「先数、串行前缀和、再并行直接编码进最终切片」，省掉每 slot 的暂存区
  （共约 98 MB）与每步约 0.25~0.3 ms 的行拷贝（估算，分块修复后约占一步的 12%）。

## 2. 没有活跃 STEP 的弹整段跳过 `tick_steps`

`tick_steps` 遇 END 即停之后，th06_s6_b6 仍约 30% 时间在变换。给弹的 `flags` 加一位（bit 5~7 空闲）：
STEP 触发时置位（`transform.rs:150-175`），某帧扫完没有活跃 STEP 时清零，无位直接跳过。
估算变换密集卡再快 15~25%，其余卡约 0。`flags` 进校验和 ⇒ 校验和变化（已拍板可接受）。

## 3. ECL 虚拟机：加载时一次性校验

`ecl/vm.rs:112-131` 每条指令都重查预算、`op_implemented`、`code.get` 与操作数越界；在 th06_s2_w01
上这几行约占 VM 指令的 25%。改为加载镜像时校验（op 已实现、操作数在界内、跳转目标合法），
两个预算计数器并成一个倒数。估算 VM 时间 −10~20%，道中卡 −8~15%（猜测）。
要守住：`FAULT_BUDGET` 仍在第 1025 条指令触发；坏 op 的 fault 从运行时挪到加载时——只影响畸形镜像，
但属于语义变化，走评审。

## 4. 道中卡每帧轮询（内容 + 新内建）

如 `stg-rl-train/cards/th06_s2_w01/main.ecl:8-17`（`oob_guard`）与 `:64-74`（脚本里手动积分速度，
再 `move_vel` + `wait(1)`）：每只敌每帧约 50 条 ECL 指令，全场每帧上千条。道中卡 ECL 解释占单 env
时间的 55~84%。引擎加「进场后出界即删」标志、加速度走 `move_vel` / 变换，可去掉大部分。
估算最慢的约 10 张道中卡快 2~3 倍（猜测）。要求转写保持忠实，新内建需要一刀 spec；
改卡走训练仓 `transcribe/` 流水线。

## 5. CART_FX 极坐标惰性化

`world/motion.rs:56-71`：带重力的弹每帧跑 CORDIC `atan2` + `isqrt` 回填极坐标，th06_s6_b10 上约占
25% 指令。惰性计算会改变存储的 speed/angle ⇒ 校验和与快照变化（已拍板可接受）。只影响少数卡，
估算约 20%；需要设计评审，确认没有读者依赖每帧回填的值。

## 6. reset / warmup 成本

reset 占模拟时间的 5~25%（随机策略下；会活的策略 reset 更少）。`env.rs:374-390` / `step.rs:153`：
冷 `copy_into` 1.19 MB 的 World 约 100 µs、每次 warmup 重试都要一次，其余是 0~120 帧 warmup。
可选：只复制到各池的高水位；或把 warmup 交给 RL 层。两者都碰「全槽哈希 / reseed 等价」保证。
另一条是训练配置调低 `warmup_max`，约砍半 reset 成本，但改变训练看到的开局分布——训练侧决定。

## 附：运维层面（不改本仓）

- **单插槽运行**：2048 个 World 共 2.4 GB，远超 L3，且都由主线程在 `VecEnv::new` 首次写入 ⇒ 落在一个
  NUMA 节点。28 线程跨两插槽比单插槽的总工作量多约 25%。训练进程应绑在一个插槽上。
- **线程数不超过 CPU 配额**：rayon 线程空转多烧 25~47% CPU；在有 cgroup 配额的租用机上这会吃掉配额
  （很可能是 255 线程比 63 线程慢 5 倍的原因，未在那台机器上验证）。
- **小项**：去掉每步的 `Vec<Work>` 与 `compact` 里的四个 `Vec`（约 0.05~0.1 ms 串行，约 3%）；
  合并两次 `pool.install`（≤ 0.05 ms，猜测）。
- **测过、没用**：`run_tasks` 里就地读 `wait` 代替复制约 440 B 的 `Task`（校验和相同，无可测收益）；
  fat LTO + `codegen-units=1` + `target-cpu=native`（噪声内）。
