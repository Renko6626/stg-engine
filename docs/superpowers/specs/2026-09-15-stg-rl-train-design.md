# stg-rl-train 第一刀 —— 躲弹小模型的训练仓（设计，2026-09-15）

> 状态：**第一刀已落地**（训练仓 `Renko6626/stg-rl-train`；实施计划 `docs/superpowers/plans/2026-09-15-stg-rl-train.md`）。GPU 验收（§8）待首次上 Vast.ai 回填。
> 来源：RL 路线 A 的训练侧。前置：stg-rl env 刀（spec `2026-09-15-stg-rl-env-design.md`，wheel `rl-v0.1.0`）、
> `stg-agent-proto` `v0.1.0`、RL 卡池写作指南 `docs/rl-card-pool.md`。
> 本文暂住 stg-engine；训练仓 `stg-rl-train` 建立后迁入其 `docs/`（迁移提交注明本文最后一个 stg-engine commit）。
> 本刀交付：**任意有 GPU 的 Linux 机器上 `bash run.sh <config>` 一条命令跑完一次训练**（装依赖 → 训练 → 评测 → 出图 → 打包）。
> Vast.ai 自动化是第二刀，ONNX 导出与 C 侧对拍是第三刀（§11）。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 意图形态 | **目标点**，下半屏均匀随机（§3.3）；训练目标 = 小模型「遵从指令」的能力 |
| ② | 意图生成位置 | **训练仓胶水层**，不进 Rust env、不进 wheel |
| ③ | 指令与活命冲突 | **活命绝对优先**；「风险系数 λ 作条件输入」留作将来——条件向量留扩展位、reward 各项分函数 |
| ④ | 动作空间 | **18 个离散动作**（9 方向 × slow），SHOT 恒按，BOMB 屏蔽；动作表 v1 冻结（§3.2） |
| ⑤ | 遵从 reward | 势函数距离塑形 + 半径 R 内停留奖励；刷新步、终局步不计塑形（§3.5） |
| ⑥ | 意图刷新 | 随机间隔 2–5 s，与是否到达无关，每局开始重抽；模型看不到刷新计时 |
| ⑦ | 训练内容 | **训练仓自带 RL 卡池**，`compile_sources` 编译；写卡另开会话，约束见 `docs/rl-card-pool.md` |
| ⑧ | 评测 | 固定评测集（留出卡 × 固定种子）+ 活命 / 遵从 / 拟人三类指标（§6） |
| ⑨ | 框架 | **照 LeanRL 手写 PPO** + `torch.compile` + CUDA 图；不引 TorchRL（理由见 §12） |
| ⑩ | 本刀范围 | 仓库 + 胶水 + PPO + 评测 + 指标出图 + 开销记录 + `run.sh`；Vast.ai 自动化与 ONNX 不在本刀 |
| ⑪ | 算力平台 | **Vast.ai**（海外网络直连 GitHub / PyPI）；env 吃 CPU，租机须按 CPU 核数筛 |
| ⑫ | 仓库与依赖 | 新仓 `stg-rl-train`（公开）；`uv` + 提交 `uv.lock` |
| ⑬ | 实时看曲线 | `metrics.jsonl` 为权威，同步写 TensorBoard（配置开关，默认开） |
| ⑭ | 可切换性 | 模型 / 特征化器 / 意图生成器 / reward 项全部**注册表 + 配置选名**（§4） |

## 2. 仓库形态与运行

```
stg-rl-train/
  run.sh              一条命令入口（§2.2）
  pyproject.toml  uv.lock  .python-version(3.12)
  configs/
    base.toml         默认超参 / reward 系数 / 意图参数
    smoke.toml        本地 CPU 冒烟：小 N、小模型、少量更新、compile/CUDA 图关
  cards/              RL 卡池（写卡会话交付后迁入）
  eval/splits.toml    评测卡 id × rank × 局数
  src/stgtrain/
    registry.py       注册表（字典 + 装饰器）
    config.py         TOML 读入 + 校验 + 落盘副本
    actions.py        动作表 v1
    intent.py         意图生成器（注册）
    envwrap.py        包 stg_rl.VecEnv：缓冲 / H2D / 意图 / reward / 镜像，吐 GPU 张量
    featurize/        特征化器（注册）：danger_topk_v1.py
    reward.py         reward 项（注册）
    models/           模型（注册）：set_attn_v1.py
    ppo.py            LeanRL 底本的 rollout / GAE / loss / 更新
    evaluate.py       评测
    metrics.py        标量记录：jsonl + TensorBoard
    plots.py          出图（可独立命令行运行，支持多 run 叠加）
    perf.py           分阶段计时 + 后台负载采样 + bench
    checkpoint.py     存 / 读 / 续训
    train.py          入口
  tests/
    fixtures/cards/example_ring/   docs/rl-card-pool.md §6 骨架卡（测试不依赖 cards/）
  .github/workflows/ci.yml         CPU：uv sync --frozen + pytest + 冒烟
```

### 2.1 依赖钉版本

| 依赖 | 来源 |
|---|---|
| `stg_rl` | `https://github.com/Renko6626/stg-engine/releases/download/rl-v0.1.0/stg_rl-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl` |
| `stgagent` | `git+https://github.com/Renko6626/stg-agent-proto@v0.1.0`（动作表键位常量 `BTN_*`） |
| `torch` | 实施时取 PyPI 当时最新稳定版钉死（Linux wheel 自带 CUDA，CPU 上也能跑，本地与远端共用一份锁） |
| `tensordict` | 与 torch 匹配的版本，只用 `tensordict.nn.CudaGraphModule` |
| 其余 | `numpy` `matplotlib` `psutil` `nvidia-ml-py` `tensorboard`；测试 `pytest` |

升级 `stg_rl` = 改 URL + 重锁 + 单独提交；`run.sh` 一律 `uv sync --frozen`，锁不符即失败。

### 2.2 `run.sh`

| 用法 | 行为 |
|---|---|
| `bash run.sh <config> [name]` | 缺 `uv` 则用官方脚本安装 → `uv sync --frozen` → 训练 → 末次完整评测 → 出图 → 打包 |
| `bash run.sh --resume runs/<dir>` | 从该目录 `checkpoints/latest.pt` 续训（配置取 checkpoint 内副本） |
| `bash run.sh --bench [config]` | 扫「线程数 × num_envs」测端到端吞吐，输出推荐配置（§7.3） |

结果目录 `runs/<YYYYmmdd-HHMMSS>-<name>/`：

```
config.toml        生效配置副本
env.json           stg_rl.build_info()、训练仓 git sha（带 -dirty）、torch / CUDA / 驱动版本、GPU 型号、CPU 型号与核数、动作表版本
metrics.jsonl      训练与评测标量（§7.1）
perf.jsonl         开销（§7.2）
tb/                TensorBoard
checkpoints/       latest.pt / best.pt / u<更新数>.pt
eval/<更新数>.json 评测明细
plots/*.png
```

结束后打包为 `runs/<dir>.tar.gz`。本刀取回结果靠手动 `scp`。

## 3. 胶水层

### 3.1 `envwrap.py` 单步

1. 策略在 GPU 出 `action_id[N]` → 镜像局按 §3.2 置换 → 查表得 `buttons` → `.cpu()` → `env.step`（GIL 释放）。
2. 定长缓冲 + `bullets[:offsets[N]]` 前缀 `non_blocking` 拷上 GPU；fx 字段按 `OFFSETS` 切片 `.view(int32) / 65536`。
3. 每 env GPU 状态：指令点 `(x, y)`、刷新倒计时、上一帧 `buttons`、上一帧离点距离、镜像标志、刷新标志。
4. `done != 0` 的 env：缓冲里已是新局首帧观测 ⇒ 状态全部重抽 / 清零；本步 reward 按旧局结算（§3.5）。

坐标系：场界 x ∈ [-192, 192]，y ∈ [0, 448]，y 向下；机体 1 出生点 `(0, 384)`。

### 3.2 动作表 v1（冻结；版本号写入 checkpoint 与 `env.json`）

`action_id = 方向 × 2 + slow`；`buttons = 方向键位 | BTN_SHOT | (slow ? BTN_SLOW : 0)`；BOMB 永不置位。

| 方向 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|---|
| 含义 | 不动 | 上 | 右上 | 右 | 右下 | 下 | 左下 | 左 | 左上 |
| 键位 | — | UP | UP\|RIGHT | RIGHT | DOWN\|RIGHT | DOWN | DOWN\|LEFT | LEFT | UP\|LEFT |
| 镜像 | 0 | 1 | 8 | 7 | 6 | 5 | 4 | 3 | 2 |

- 键位常量取自 `stgagent.consts.BTN_*`，不写魔数。方向顺序与 stg-rl 预热游走表 `WALK_DIRS` 一致。
- 拟人项的原料：`changed = prev ^ cur`；新按下键数 = `popcount(changed & cur)`；shift 切换 = `changed & BTN_SLOW`。
- 部署侧（th06nc / TH18 DLL）照抄本表，第三刀用对拍守一致。

### 3.3 意图生成器 `lower_half_uniform_v1`

- 点：x ∈ [-192+m, 192-m]，y ∈ [224, 448-m]，均匀；`m` 默认 16 px。
- 刷新：倒计时从 [120, 300] 帧均匀抽，按实际帧数扣（`frame_skip` 计入）；每局开始重抽。
- 随机源：独立 `torch.Generator`，由训练基础种子派生；评测用固定种子（§6）。
- 刷新发生的那一步置刷新标志（§3.5 用）。

### 3.4 特征化器 `danger_topk_v1`（输出形状固定，可录 CUDA 图）

**弹**：
1. CSR 行按 `offsets` 铺成 `(N, bullets_cap)` + 掩码；只留 `flags & 1`（collidable）。
2. 自机预测速度 `v_p = unit(指令点 − 自机) × speed`（`speed` 取 player 表高速档）；已在 R 内则 `v_p = 0`。
3. 相对量 `p = b − 自机`，`v = v_b − v_p`；`t* = clamp(−(p·v) / (|v|² + ε), 0, H)`；
   `d_min = |p + v·t*| − radius_b − hit_radius`（边缘最近距离，可为负）。
4. 按 `d_min` 升序取前 `K_b` 颗；`d_min > D_max` 的即便入选也置掩码为空。
5. 每颗特征：相对位置 /192、相对速度 /8、半径 /8、`d_min / D_max`、`t* / H`。

**密度图** `(N, 2, 14, 12)`：32 px 一格；通道 0 = 可碰撞弹计数，通道 1 = 「正朝自机飞来」加权
（`max(0, −p̂·v_b) / 8` 累加）。

**敌**：`flags & 0x10`（collidable）的敌，体碰撞按圆（半径 = `hit_w`，编码器写入的就是体碰撞半径），
同一危险度键取前 `K_e` 个；特征：相对位置、半径、`d_min`、`t*`、是否 boss。

**自机** `player`：位置 /192、focus。**条件向量** `cond`：指令点相对位置 /192、距离 /448、是否在 R 内（将来 λ 追加于此）。

**镜像**：镜像局对全部 x 类特征取反（位置、速度、密度图左右翻转）；动作按 §3.2 置换。
**道具表 v1 不用。**

输出契约（键 → 形状）：`bullets (N,K_b,F_b)` `bullets_mask (N,K_b)` `enemies (N,K_e,F_e)` `enemies_mask (N,K_e)`
`density (N,2,14,12)` `player (N,F_p)` `cond (N,F_c)`。

### 3.5 reward 项（注册；系数在配置 `[reward.terms]`，每项单独记入 metrics）

| 项 | 定义 | 默认系数 |
|---|---|---|
| `death` | `done == 1` 时 −1 | 10.0 |
| `follow_shaping` | `γ·Φ(s') − Φ(s)`，`Φ = −d / 448`，d = 自机到指令点距离；**刷新步与终局步（任意 done）记 0** | 1.0 |
| `hold` | `d < R` 的帧 +1 | 0.01 |
| `segment_survived` | `done == 2` 时 +1 | 0.0 |
| `key_press` | 新按下键数 | 0.0 |
| `shift_toggle` | shift 切换 | 0.0 |
| `edge_hug` | 距场界 < 16 px 的帧 +1（负系数即惩罚） | 0.0 |

- 终局步不计塑形的理由：势函数塑形惯例令终态 Φ = 0，那样离点越远死掉反而白赚 `d/448`；直接置 0 并让 `death` 远大于塑形量级。
- 防自杀：塑形项远离指令点时为负，但一局内累计的负值受势函数差限定（≤ `follow_shaping × 567/448 ≈ 1.27`，567 px 为场内最远两点距离），死亡能「躲开」的也不超过这个量。配置校验断言 `death ≥ 5 × follow_shaping × 1.1`（5 倍余量覆盖上式）。
- `R` 默认 24 px。

## 4. 注册表与接口契约

四张注册表：`models`、`featurizers`、`intents`、`reward_terms`。配置示例：

```toml
[featurize]
name = "danger_topk_v1"
k_bullets = 64
k_enemies = 8
horizon = 60        # H，帧
d_max = 128.0       # px

[model]
name = "set_attn_v1"
d = 64
heads = 4
trunk = 256

[intent]
name = "lower_half_uniform_v1"
margin = 16.0
interval = [120, 300]

[reward]
hold_radius = 24.0
terms = { death = 10.0, follow_shaping = 1.0, hold = 0.01 }
```

- 特征化器实现 `spec() -> dict[键, 形状(去掉 N)]` 与 `__call__(raw) -> dict[键, Tensor]`。
- 模型实现 `requires() -> dict[键, 形状]` 与 `forward(feats) -> (logits[N,18], value[N])`；启动时核对 `requires ⊆ spec` 且形状一致，不符即报错。
- checkpoint 自描述：存注册名 + 完整配置 + 动作表版本；评测、续训、将来导出一律从 checkpoint 重建。
- 加新模型 = 新文件 + `@register` + 新配置，不改 `ppo.py`。

### 4.1 首个模型 `set_attn_v1`

- 弹：逐元素共享 MLP（F_b → d → d）→ 掩码 mean 池化 ‖ 掩码 max 池化 ‖ 一层交叉注意力（`heads` 头，query = `player ‖ cond` 的嵌入）。
- 敌：同构，独立权重。密度图：2 层小卷积后展平。
- 主干：各路拼接 → MLP(`trunk`) → 策略头 18 logits / 价值头 1 标量（编码器共享）。
- 全空掩码必须安全（池化与注意力不出 NaN），单测押运。无动态形状，ONNX 友好。

## 5. PPO（`ppo.py`）

- **底本**：LeanRL `ppo_atari_envpool_torchcompile.py`（MIT）；文件头保留出处与许可。改动只限：envpool → `envwrap`，CNN → 模型注册表，补 done 码处理、日志、checkpoint。rollout / GAE / loss / `CudaGraphModule` 用法照抄。
- **rollout**：GPU 定长缓冲 `T × N`，存特征、动作、logp、价值、reward、done 码。每步「特征化 → 采样 → logp / 价值」整体 compile + CUDA 图；`env.step` 在图外。
- **GAE 与 done 码**：
  - 任何 `done != 0` 都切断优势回传（下一帧已是新局）。
  - `done ∈ {1, 2}`：终止，不自举。
  - `done == 3`：用 `V(obs_t)` 近似末帧价值自举；metrics 记 done=3 占比。占比持续偏高即触发 follow-ups D23#7（env 导出末帧观测）。
- **默认超参**：`N = 2048`，`T = 64`，γ = 0.995，λ = 0.95，4 epoch × 8 minibatch，clip 0.2，价值系数 0.5，熵系数 0.01，梯度裁剪 0.5，Adam lr 3e-4 线性退火，优势归一化。
- **env 默认**：`frame_skip = 1`，`max_frames = 3600`，`warmup_max = 120`，`bullets_cap = 1024`，`threads` 取配置（可由 bench 推荐）。
- **开关**：`compile`、`cudagraphs` 各自可关；CPU 冒烟两者都关。
- **checkpoint**：每 `ckpt_every`（默认 50）次更新存 `latest.pt` 与 `u<k>.pt`：模型、优化器、各随机源状态（torch / 意图 Generator / env 基础种子与局计数）、更新计数、配置、动作表版本。`--resume` 从 `latest.pt` 继续。
  注：env 局内状态不进 checkpoint，续训时 env 重新 reset，这是可接受的非逐字节续训。

## 6. 评测（`evaluate.py`）

- `eval/splits.toml`：评测卡 id、每张卡的 rank 列表、每组局数（默认 32）。训练卡 = 卡池 − 评测卡。
- 独立 VecEnv，env 种子与意图种子固定 ⇒ 不同 checkpoint 可直接比。动作取 argmax（配置可改为采样）。
- 每 `eval_every`（默认 50）次更新跑一轮，训练结束再跑一轮完整评测。
- 指标（按卡、按 rank、总体）：
  - 活命：撑过率（done=2 占比）、平均存活帧数、done=3 占比；
  - 遵从：R 内帧占比、刷新后到达用时中位数（未到达记上限）；
  - 拟人：每秒 shift 切换次数、每秒方向变化次数、贴边帧占比。
- 输出 `eval/<更新数>.json` + 汇总行进 metrics；`best.pt` 按主指标（默认先撑过率、再 R 内占比）另存。
- 回放目测不在本刀（stg_rl 不导出回放，D23#3 / #4）。

## 7. 指标、出图与开销

### 7.1 `metrics.py` / `plots.py`

`metrics.jsonl` 每行 `{"update", "env_steps", "wall", "key": value, ...}`；TensorBoard 同名镜像（`[log] tensorboard = true`）。

记录：
- 回合：reward 各项、回合长度、done 码 1/2/3 占比、存活率、R 内帧占比；
- PPO：policy loss、value loss、熵、approx KL、clip 比例、explained variance、lr；
- 评测：§6 全部指标。

`plots.py`：训练结束自动出 `plots/*.png`；`python -m stgtrain.plots <run 目录或 tar>... [--out dir]` 可对中途拷回的 jsonl 重画、多 run 叠加对比。

### 7.2 `perf.py`

- **分阶段计时**：`env_step`（CPU）、`h2d`、`featurize`、`policy`、`reward`、`update`。每 `perf_sync_every`（默认 20）次迭代做一次带 `cuda.synchronize` 的精确计时，其余迭代不插同步。
- **后台采样**（1 Hz 线程）：CPU 总占用与逐核占用、进程 RSS（psutil）；GPU 利用率、显存、功耗（NVML，无 GPU 时跳过）；`torch.cuda.max_memory_allocated`。
- **汇总**：env-steps/s、端到端 SPS、各阶段时间占比、「GPU 等 CPU」与「CPU 等 GPU」占比，写入 `perf.jsonl` 与 `env.json` 末尾的 `perf_summary`。
- 机器画像写入 `env.json`（§2.2）。

### 7.3 bench（`run.sh --bench`）

- 扫 `threads ∈ {nproc/4, nproc/2, nproc}` × `num_envs ∈ {512, 1024, 2048, 4096}`，每格 10 s。
- 跑完整链路（env + 胶水 + 未训练模型推理，不做更新），只计稳态。
- 输出 `bench.json` + 终端表 + 推荐 `(threads, num_envs)`。

## 8. 测试与验收

**CPU 单测（CI 必跑）**：
- `actions`：键位；镜像置换两次回到原样；镜像后键位左右互换。
- `intent`：取点范围、刷新间隔、done 时重抽、同种子可复现。
- `featurize`：`d_min` 闭式解 vs 逐帧暴力对拍；前 K 选取；无弹时掩码全空；镜像前后一致；输出形状 = `spec()`。
- `reward`：γ = 1 时两次刷新之间的塑形总和 = Φ(段末) − Φ(段首)；刷新步与终局步记 0；三种 done 码；配置校验（`death` 过小时报错）。
- `ppo`：GAE 对手算小例子，覆盖 done = 1 / 2 / 3。
- `registry`：每个注册模型在全空与满掩码输入下前向，形状正确、无 NaN；键或形状不符时报错。
- 确定性：同种子不带学习的 rollout（`tests/fixtures/cards/example_ring`）CPU 上跑两遍，特征与 reward 逐字节相同。

**集成冒烟**：`bash run.sh configs/smoke.toml` CPU 跑数次更新，断言结果目录各文件、plots、tar 生成；`--resume` 能接着跑。

**GPU 验收（首次上 Vast.ai，结果回填本节）**：
- [ ] `run.sh --bench` 跑完，记录机器画像与推荐配置；
- [ ] `compile` / `cudagraphs` 开与关在同一批数据上首次更新的 loss 相对误差 ≤ 1e-4；
- [ ] `base.toml` 连续训练 ≥ 1 小时无错，SPS 与各阶段占比记入训练仓 `docs/perf-baseline.md`（首次验收时新建）；
- [ ] 中途打断后 `--resume` 成功；
- [ ] 评测曲线出图正常。

## 9. 与其它仓的接口

- **stg-engine**：只通过 `stg_rl` wheel（§2.1）。`rl-v0.1.0` 的 `build_info()["git_sha"]` 为 `"unknown"`（CI 构建未取到 git），`env.json` 仍记录 `version` / `engine_ver` / `tables_hash`；修复后发 `rl-v0.1.1` 再升级。
- **stg-agent-proto**：只用 `stgagent.consts.BTN_*`。
- **卡池**：写作约束以 `docs/rl-card-pool.md` 为准；卡池迁入训练仓后，该文档随之迁移或在两边互链。

## 10. 已知风险

- **LeanRL 停更**（最后提交 2025-08）：与当前 torch / tensordict 的兼容性实施首日验证；不兼容处按当前 API 改写并在文件头记录。
- **本地 GPU 不可用**（开发机 `/dev/nvidia*` 仅 root）：GPU 路径首次在 Vast.ai 验证，CPU 冒烟覆盖逻辑但不覆盖 CUDA 图。
- **done=3 自举是近似**：卡池约束（时限 < `max_frames`）使其罕见；由 metrics 占比监控。
- **卡池起步为空**：写卡会话交付前只能用测试骨架卡跑通流程，不代表训练效果。
- **Vast.ai 机器 CPU 差异大**：env 吞吐受 CPU 限制，靠 bench 选配置。

## 11. 非目标与后续刀

- **第二刀 · Vast.ai 自动化**：按 CPU 核数与 GPU 筛机、开机脚本、结果自动上传、可中断实例自动续训。
- **第三刀 · 部署**：ONNX 导出；C 侧复刻特征化与动作表；th06nc `onnx` 后端对拍（renkolab spec §10 第 3–4 步）。
- 指挥模型；λ 条件输入（③）；道具特征；回放导出（D23#3 / #4）；末帧观测（D23#7）；async env（D23#5）；多 GPU；W&B。

## 12. 为什么不用 TorchRL（2026-09-15 调研，torchrl 0.14.0 源码核对）

- collector 逐帧把全部缓冲 stack 进自己的 rollout 缓冲，且不收 CSR 弹表 ⇒ 抵消 env 的原地 pinned 缓冲设计。
- env 内自动 reset 时，末帧观测被填占位值（浮点 NaN、uint8 填 0），超时截断用 `V(obs_t)` 近似且对整数观测不做替换，行为藏在框架里。
- 小版本约每 2–3 个月一次破坏性变更，`torchrl` 与 `tensordict` 须成对锁版本，C++ 扩展须匹配 torch。
- 可单独借用而不引入框架的部分：`tensordict.nn.CudaGraphModule`（本刀采用）。
- LeanRL 实测（其 README）：PPO Atari 从 CleanRL 1022 fps 到 compile + CUDA 图 6809 fps——小网络 RL 的瓶颈在 CPU 调度开销，手写循环同样能拿到。

## 13. 实施偏差

写计划（`docs/superpowers/plans/2026-09-15-stg-rl-train.md` §Rulings）时对本文的裁定：

| # | 偏差 | 理由 |
|---|---|---|
| 1 | 刷新步的遵从塑形不置 0，用旧指令点精确计算；只保留「终局步记 0」 | reward 在刷新之前算、两个距离都对 `prev.target_xy`，本就没有目标突变的跳变；§3.5 表格与 §8 对应单测按此理解 |
| 2 | 到达用时：每局记各刷新段到达帧数的均值（未到达记 `interval` 上限），评测汇总取各局中位数 | 逐段中位数需要在设备上维护变长列表，收益不抵复杂度 |
| 3 | 新增 `cards.py` / `episodes.py` / `bench.py` / `gpucheck.py` | 卡池、逐局统计、测速、GPU 验收各自独立成文件，训练与评测共用逐局统计 |
| 4 | 打包在 `train.py` 里用 `tarfile` 完成，`run.sh` 只装依赖 + 调入口 | 便于测试 |
| 5 | 评测每个 (卡, rank) 开一个 `num_envs = episodes` 的 VecEnv，每个 env 只取第一局 | VecEnv 按权重随机抽起点，无法保证每组恰好 N 局 |
| 6 | 训练仓不加 LICENSE；`ppo.py` 头部保留 LeanRL MIT 许可全文 | stg-engine 同样无 LICENSE，由作者另定 |
| 7 | 续训时 env 种子 = `run.seed + 起始更新号 − 1` | 避免续训重放同一批局（§5 已声明非逐字节续训） |
| 8 | `envwrap._fx` 先压平再 `view(int32)`，存储偏移不整除 4 时 clone | 单行 / 空张量在 PyTorch 看来「连续」，`.contiguous()` 不拷贝，size-1 维步长与偏移 22（bullets `radius`）让 `view` 崩溃；审阅复现后修复 |
| 9 | 密度图：弹恰在内部格线上时左右两格各计一半 | 原 `floor` 分箱让 `density(mirror(obs))` 不等于左右翻转，x = 0 的弹每步都有；这是唯一对称规则（浮点舍入下仍非逐位严格，记待办） |
| 10 | `ppo.py`：LeanRL 的 `Categorical.logits/probs` 只读 property 补丁改为带 setter 的描述符；`agent_inference.requires_grad_(False)` | torch 2.14 下只读 property 让 `Categorical.__init__` 抛错；tensordict 0.14 `to_module` 保留 requires_grad，第二个 minibatch 反向报「图已释放」 |
| 11 | `PPO.load_state_dict` 保留优化器 lr 张量身份 | `Optimizer.load_state_dict` 深拷贝 param_groups，CUDA 图续训时 lr 退火静默失效 |
| 12 | gpucheck：lr = 0、单 minibatch、调用 25 次（> CudaGraphModule warmup 20）后比对；绝对 + 相对混合容差；另比对 grad norm 与策略 entropy / value | 原「1e-4 纯相对误差、32 次 Adam 累积」对正确实现也会失败；剩余盲区：每次输入相同，测不出忽略新输入的重放 |
| 13 | 续训：先截掉 checkpoint 之后的 metrics / perf 行，TensorBoard `purge_step = env_steps + 1`；`best` 优先取 `best.pt`；`total_updates` 不大于 checkpoint 时报错 | 中断后续训会重复写日志行、曲线回跳；purge 取 `env_steps` 会误删 checkpoint 那一步的点 |
| 14 | 负载采样器改流式汇总、停止后不写；评测组局数不足即报错；bench 设 torch 线程数并对线程网格去重；pytest 屏蔽「CUDA 驱动过旧」告警 | 长跑内存无界增长（约 300–400 MB/天）；静默少局；bench 推荐与训练条件一致 |
| 15 | 本刀 `.ecl` 训练卡池仍为空，`cards/` 只有 README；测试卡在 `tests/fixtures/cards/` | 写卡在另一会话进行；GPU 验收前先放至少一张卡 |
