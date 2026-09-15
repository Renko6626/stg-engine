# RL 卡池写作指南（给写 `.ecl` 的会话）

> 2026-09-15 起草。读者：批量写 RL 训练用弹幕卡的实现者（原创或东方原作转写）。
> 语法与内建函数以 [`ecl-lang.md`](ecl-lang.md) 手册为准，本文只讲**卡进了 RL 训练之后有什么额外约束**。
> 训练侧的整体设计（意图、reward、动作表）在训练仓 spec 里，这里只摘写卡需要知道的部分。

## 1. 卡在训练里怎么被用

训练仓用 `stg_rl.compile_dir(卡目录)` 把每张卡编成一份镜像，然后按权重抽起点开局：

1. **开局**：`new_game_at(rank, mark)`，机体恒为 **1**（经典机体），从场底 `(0, 384)` 出场，火力 0。
2. **随机预热**：先跑 0–120 帧随机游走（9 方向含停住，每段 8–32 帧，不射击不 bomb）。预热中途自机死了、或者出现段落结束事件，
   换个随机数重来，最多 8 次，都失败就不预热直接开始。**预热帧不计入这一局。**
3. **模型接管**：每帧从 18 个动作里选一个（9 方向 × 是否低速）。**SHOT 恒按、BOMB 被屏蔽**——
   机体 1 在训练里没有任何保命手段，只能靠走位。机体 1 也没有停止 / 跳躍。
4. 训练胶水每 2–5 秒在**下半屏**（x ∈ [-192, 192]，y ∈ [224, 448]）随机给一个目标点，
   模型要一边躲弹一边往那儿走；活命绝对优先。
5. **一局结束**（env 的 `done` 码）：

   | 码 | 情形 |
   |---|---|
   | 1 | 自机第一次死亡 |
   | 2 | 出现段落结束事件：`EVT_PHASE_ENDED` / `EVT_SPELL_CAPTURED` / `EVT_SPELL_FAILED` / `EVT_STAGE_CLEARED` |
   | 3 | 本局满 `max_frames`（默认 3600 帧 = 60 s）还没结束——截断，训练上不如 2 干净 |

## 2. 目录与格式

卡池目录住**训练仓**的 `cards/`。训练仓建立之前，先写在 stg-engine 根目录的 `rl-cards/`
（不进 wheel、不进 `build.rs`），建仓时整目录迁走。

```
cards/
  th06_s4_boss_card1/     一张卡 = 一个目录 = 一份镜像
    main.ecl              入口 sub main()；可以再拆多个 .ecl（整目录按文件名排序合并编译）
    meta.toml             卡的元数据（见下）
  orig_ring_spiral/
    main.ecl
    meta.toml
```

- **每张卡自包含**，不做跨卡共享库：`RICE` 这类弹型常量是 `godot/ecl/game/bullets.ecl` 里的**脚本常量**，
  引擎不认，要用就把那几行 `const` 抄进卡里。
- 目录名即卡 id：小写字母、数字、下划线。原作转写用 `thNN_` 前缀，原创用 `orig_`。
- 默认从头开局（`mark = 0`）。一张卡含多个可独立起跳的段时，可以用 `mark(id)` 暴露多个起点，
  并在 `meta.toml` 的 `marks` 里列出（`mark` 的约束见手册第 6 篇）。

`meta.toml`：

```toml
title = "紅符「スカーレットシュート」"   # 人读的名字
source = "th06"                          # "original" 或原作编号 th06 / th07 / …
origin = "Stage 4 Boss 符卡 1，Normal"   # 转写出处；原创写一句设计意图
ranks = [0, 4]                           # 支持的 rank 闭区间（0 Easy … 4 Extra）
marks = [0]                              # 可用起点
time_limit = 1800                        # 段时限（帧）
tags = ["aimed", "ring"]                 # 弹型标签，见 §4
laser_approx = false                     # 原作有激光、这里用弹链近似了 ⇒ true
lower_half_blocked = false               # 下半屏长时间大面积封死 ⇒ true（见 §3.2）
notes = ""
```

## 3. 约束

### 3.1 硬性（不满足训练就会出问题）

1. **段落必须以结束事件收尾。** 用一只 boss 敌 + `phase_begin` / `spell_begin` 带时限，时限到自动发事件。
   **boss 要 `set_invuln(65535)`**：SHOT 恒按，不无敌的话段长会随模型站位变，这张卡的难度就不稳定了。
   确实想按血量收段的，在 `notes` 里写明。
2. **时限 < 3600 帧**，建议 1200–3000（20–50 s）。超过 `max_frames` 的卡永远以码 3 截断。
3. **开场留 ≥ 120 帧缓冲。** 这段时间里别让致命弹到达出生点附近——预热是闭眼乱走，前 2 秒就能打死
   乱走的卡会让预热反复重试，最终退化成每局都从 `(0, 384)` 原地开局，开局多样性就没了。
   段落结束事件同理，不能出现在前 120 帧。
4. **不 bomb、不停止、不跳躍，纯走位能活。** 用 §5 的 `serve` 亲手试玩确认，试玩时别按 X/C。
5. **`run` 零 fault、退出码 0。** 静默 fault 的卡在训练里就是一张空卡。
6. **弹数峰值建议 ≤ 1024**（`run` 输出的「峰值：弹」）。观测表默认 1024 行，超出时丢掉离自机最远的弹，
   模型看不见它们；引擎弹池上限 8192，打满后后续弹直接发不出来。
7. **没有激光。** 引擎没有激光池。原作的激光段要么用密排弹链近似（`laser_approx = true`），要么跳过这张卡。
8. **难度用 `global(GVAR_RANK)` 缩放**（0–4），不是 `$rank`，v1 没有这个变量。

### 3.2 建议（影响训练质量）

- **每局都有变化。** 用 `rand(n)`（世界 RNG，每局重新播种）加随机相位、随机偏角，别把整张卡写死。
  写死的卡会被模型背板。
- **下半屏要有地方可走。** 目标点在下半屏随机，长时间把下半屏大半封死的卡（例如整面慢速弹墙压下来）
  会让「遵从指令」学不到东西。这类卡可以写，但标 `lower_half_blocked = true`，训练侧会降权。
- **难度要有梯度**：同一类弹型出易、中、难几档，方便做课程学习。
- **转写原作时**把 `origin` 写准（作品 / 关卡 / 卡名 / 难度），ZUN 指令对照见
  [`zun-ecl-v2-reference.md`](zun-ecl-v2-reference.md)。
  **东方红魔乡（TH06）有现成的批量转写流水线**：训练仓 `transcribe/`（TH06 → 本引擎的逐指令对照表
  `transcribe/th06/mapping.md`、4 张手转范例卡、dsh worker 契约、验收器 `python -m stgtranscribe.validate`），
  转 TH06 的卡别手写，走流水线。

## 4. 弹型标签（`tags`，可多选，缺了再加）

`aimed` 自机狙 · `random` 随机散弹 · `ring` 环 · `spiral` 螺旋 · `wall` 带缝弹墙 · `curve` 曲线 / 变速 ·
`split` 分裂 · `stream` 连射流 · `dense` 高密度小弹 · `fast` 高速弹 · `mixed` 多种叠加

## 5. 自检流程（每张卡）

```bash
cargo run -p stg-harness -- check <卡目录>                              # 编译
cargo run -p stg-harness -- run <卡目录> --frames <time_limit+300> --rank 0
cargo run -p stg-harness -- run <卡目录> --frames <time_limit+300> --rank 4
# 看四样：退出码 0（无 fault）/ 弹峰值 / 「段结束：」行里 PHASE_ENDED 或 SPELL_* 的帧在时限附近
#        / 时限帧之后弹数归零（说明模式任务已随段退场）
cargo run -p stg-harness -- serve --ecl <卡目录>                         # 浏览器试玩（ssh -L 转发端口 8611）
```

工具的已知限制：
- `run` 不按任何键、机体 0，只能看脚本行为，看不出能不能躲。
- `serve` 固定 rank 2、机体 0。机体 0 和机体 1 的移速、判定、shot 相同，差别只在 X/C 键，**试玩别按 X/C** 就等价。
- Godot 游戏壳只读 `godot/ecl/game/`，不能拿来试玩卡池。

写法本身的坑（`wait` 截断、敌主任务 return 即退场、新任务当帧不跑等）看 [`ecl-lang.md`](ecl-lang.md) 开头的五条；
写卡时可用 `writing-danmaku-ecl` skill。

## 6. 骨架卡（已实测：`check` 通过、`run` 零 fault；stg_rl env 里活过时限以 `done = 2` 结束）

```ecl
// RL 卡池骨架：无敌 boss + 一个非符段，时限到 = EVT_PHASE_ENDED = done 2。
const TIME_LIMIT: int = 1800; // 30 s
const RICE: int = 64; // 弹型号 = 图集行 × BULLET_COLOR_STRIDE（抄自 godot/ecl/game/bullets.ecl）

async sub ring_pattern() {
    sh_reset(0);
    sh_ring(0, 1);
    sh_sprite(0, RICE, 0);
    var base: angle = 0deg;
    wait(90); // 开场缓冲：预热随机游走期间别打死人
    loop {
        sh_count(0, 16 + global(GVAR_RANK) * 4, 1);
        sh_speed(0, 2.0fx, 0fx);
        sh_angle(0, base, 0deg);
        sh_fire(0);
        base = base + 7deg;
        wait(40);
    }
}

async sub boss_main() {
    set_invuln(65535); // 常按 SHOT 打不掉血：段长只由时限决定
    phase_begin(0, ring_pattern, TIME_LIMIT, 0);
    wait_spell();
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 100.0fx, 1000, 0, 0, 1, boss_main);
    loop { wait(600); }
}
```

（本文件在 `docs/ecl-lang/` 之外，里面的 ```ecl 围栏**不会**被 `cargo test` 自动编译；
改骨架时手动跑一次 `check`。）

## 7. 规模提示

每个 `(卡, rank)` 起点在每个训练进程里缓存一份约 1 MB 的开局快照。500 个起点约 0.5 GB，
租用的 GPU 服务器吃得消，但别无节制地把每张卡的五档 rank 都列成起点。
