# 引擎第二刀 —— 敌人速度一等字段 + 三项模拟热点（设计，2026-09-24）

> 状态：**设计已拍板，待实施**。
> 来源：训练仓 `stg-rl-train` 在 Magnus A100 上做性能调研后，引擎侧的待办记在 `docs/rl-perf-roadmap.md`。
> 本刀从中取 §2、§3、§5 三项，再加一条训练仓提出的缺口：敌人没有速度字段。
> **不在本刀**：roadmap §1（RL 专用观测缓冲，属于新子系统，之后单独开刀）、§4（道中卡内建）、§6（reset / warmup）；
> 部署侧（th06nc 抽取器、`sa_model.c`）的敌人速度也不在本刀，见 §7。
> 涉及仓库：stg-engine（主体）、stg-rl-train（观测解码）、stg-agent-proto（只改 SPEC 文字与 stride 常量）。

## 1. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 本刀范围 | 敌人速度、§2 跳过没有活跃 STEP 的弹、§3 VM 在加载时校验、§5 CART_FX 极坐标改为惰性计算。RL 专用缓冲之后再做 |
| ② | 敌人速度的语义 | **新增池字段，记录本帧的实际位移**，由引擎维护。不改 `vx/vy` 的语义（那是脚本作者可读写的积分状态），也不在 stg-rl 里外挂差分 |
| ③ | 部署侧 | 本刀只动引擎和训练仓。C 侧暂时保留差分，记一条待办（§7） |
| ④ | CART_FX 角度 | 惰性计算：读取时回填。角度的含义从「最后一次速度够快的那一帧的方向」变为「最近一次被读取、且当时速度够快时的方向」 |
| ⑤ | 版本 | 四项合成一刀，**ENGINE_VER 只 bump 一次（22 → 23）**，旧 `.stgr` 回放随之失效（roadmap 已拍板接受校验和变化）。wheel 升为 `stg_rl` 0.2.0 |
| ⑥ | 字段名 | 池字段叫 `dx` / `dy`；Tier 0 敌人行的字段叫 `vx` / `vy` |
| ⑦ | 坏镜像 | 加载时拒绝，编译直接失败。不做「降级为运行时 fault」的兼容模式 |

## 2. 实施顺序与验证方法

先做三项纯引擎内部的改动，每项单独提交：§3 跳过 STEP → §4 VM 加载时校验 → §5 惰性极坐标。然后是 §6 敌人速度，最后改训练仓（§6.4）。ENGINE_VER 的 bump 和 changelog 条目放在第一个会改变校验和的提交里，后面的提交不再重复 bump。

每项提交前都要做一次**前后对比**（同 `docs/bench-baseline.md` 分块修复一节的做法）：
- `stg-harness golden` 的输出；
- `stg-harness run <card>` 的输出，覆盖训练仓 `cards/` 下全部卡，按帧逐行比较。

各项允许出现的差异：

| 项 | 允许的差异 |
|---|---|
| §3 跳过 STEP | 只允许 checksum 列变化，轨迹必须逐字节相同 |
| §4 VM 加载时校验 | checksum 与轨迹都必须不变：合法镜像的行为不变，而训练卡池全部合法。若出现差异，说明有卡依赖运行时 fault，必须停下来查 |
| §5 惰性极坐标 | checksum 变化；`run` 输出里 CART_FX 弹的 speed / angle 列可以变化（取决于那一帧是否被 materialize），位置列必须不变 |
| §6 敌人速度 | checksum 变化；Tier 0 敌人行多出两列；其余不变 |

性能收益在全部完成后统一在 A100 上 A/B（沿用训练仓 `magnus/phase_probe.sh`），结果补进 `docs/bench-baseline.md` 与 roadmap。本刀不预设收益门槛：roadmap 的估算（§2 变换密集卡快 15–25%，§3 VM −10~20%，§5 少数卡约 20%）只作参考。

## 3. 跳过没有活跃 STEP 的弹（roadmap §2）

**现状**：每颗带变换序列的弹，每帧都要 `tick_steps` 扫一遍自己的变换槽，找 `STEP_ACTIVE`（扩展槽 `args[1]` 的第 31 位）。大多数弹身上根本没有进行中的 STEP。

**改动**：
- 弹的 `flags` 第 5 位定义为 `BULLET_STEP_LIVE`（`bullets.rs`，第 5–7 位目前空闲）。
- `fire_op` 在 `frames > 0`、写入 `STEP_ACTIVE` 的同时，置上 `BULLET_STEP_LIVE`（`transform.rs:175` 附近）。LOOP 重新触发也走 `fire_op`，不需要另外处理。
- `run_transforms` 看到 `BULLET_STEP_LIVE == 0` 就不调 `tick_steps`，其余照旧（`advance_cursor` 照常跑）。
- `tick_steps` 扫完一遍后，如果没有任何一个扩展槽的 `STEP_ACTIVE` 还在，就清掉 `BULLET_STEP_LIVE`。这包括 STEP 在本帧刚好结束的情况。

**不变量**：`BULLET_STEP_LIVE == 0` ⇒ 这颗弹所有已触发的扩展槽里都没有 `STEP_ACTIVE`。反方向不要求：位多置了，只是多扫一次，下一次扫描会清掉。
- 启动 STEP 的途径只有 `fire_op` 一处，所以不会出现「STEP 活着但位没置」的情况。
- 存档、`copy_into` 恢复时，这一位与变换槽一起恢复，两者保持一致。旧存档靠 ENGINE_VER 拒收。

**测试**：
- 新增：STEP 进行中这一位为 1；STEP 结束那一帧清零；LOOP 重新触发后再次置位；两个 STEP 重叠时，要等后一个结束才清零；时停期间不变。
- 现有 `transform.rs` 的 STEP 测试（约 696–810 行）全部照过。
- 前后对比：轨迹逐字节相同。

## 4. VM 加载时校验（roadmap §3）

**现状**：`ecl/vm.rs:107-131` 每执行一条指令都要检查预算、`op_implemented`、取指越界和操作数越界。加载闸 `EclImage::try_from_parts`（`image.rs:262-463`）只查表的一致性，不查代码本身。

### 4.1 加载时校验器

在 `try_from_parts` 末尾新增 `validate_code`：从每个 sub 的 `code_entry` 起，按 `ARITY` 线性扫描，直到遇到下一个 sub 的入口或代码末尾。先收集全部指令边界，再逐条检查：

| 检查项 | 对应的原运行时 fault |
|---|---|
| opcode 已实现（`op_implemented`），头字的高 24 位必须为 0 | BAD_OP |
| 操作数没有越出 `code` 末尾 | PC_OOB |
| JMP / JZ 的目标 `< code.len()`，且落在某条指令的边界上 | 原来会等到下一次取指才报 PC_OOB，或者把操作数误当作 opcode 解码 |
| CALL 的目标是合法的 SubId，且 kind 为 CallOnly | BAD_OP |
| SPAWN 的 SubId、kind、arity 正确，且 `argc ≤ 64` | BAD_OP / STACK |
| PUSHL / POPL 的下标 `< 64` | BAD_OP |
| SYS 编号满足 `syscall_implemented` | BAD_OP |

- 校验失败时返回新的 `ImageError` 变体，带上 sub 名、pc 和原因。编译器侧（`ImageBuilder::build`）直接把它作为编译错误抛出。
- 各 sub 之间的边界：sub 的末尾允许「落出代码末尾」，这在运行时照旧报 PC_OOB，校验器不要求 sub 必须以 END 结尾。
- 高 24 位要求为 0：它们现在注释为「留作将来的掩码」，编译器从不写入。要求为 0 是给将来留余地，可以在校验里再放开。

### 4.2 留在运行时的检查

- **取指越界（PC_OOB）必须保留**：`World::load_bytes` 会原样恢复存档里的 task pc，而镜像哈希只校验表的一致性，不证明是同一份脚本。
- 栈深度与下溢（包括 SPAWN 的 `sp < argc`）、调用深度、RET、除零、syscall 内部依赖栈值或 owner kind 的检查（`syscall.rs:1569-1580`、`1732-1744`），继续在运行时检查。
- **修正（Task 2 实施时拍板，覆盖上一段草案）**：循环头只删 `op_implemented` 这一次查表——未实现 op 的 `ARITY` 恒为 0，会照原样落进 `match` 的 `_ =>` 默认臂返回 `FAULT_BAD_OP`，行为不变。**取指越界与操作数越界这两个检查必须保留**（不能跟着 `op_implemented` 一起删）：`World::load_bytes` 会原样恢复存档里的 task pc，镜像哈希只证明"表一致"，证不了"pc 落在这份脚本的合法指令边界上"——跨镜像存档或手改过的存档能把 `task.pc` 带到任意位置，循环头若不再检查操作数越界，`ctx.code[opnd_start]` 就可能真的越界 panic，直接违反 P4（调用方违约 → 确定性安全结果，不 panic）。CALL / SPAWN / PUSHL / POPL / SYS 各自的运行时检查同样保留——它们只在各自的 op 分支里跑（不在每条指令的公共路径上），与静态校验器各自独立地校验同一件事，互不依赖、也不冲突。

### 4.3 预算合成一个倒数

进入 `exec` 时，`n = min(TASK_BUDGET, *ctx.budget)`。每执行一条指令前先检查 `n == 0 ⇒ Fault(FAULT_BUDGET)`，然后 `n -= 1`。退出时（包括 WAIT、END、fault 三种情况），把实际用掉的条数从 `*ctx.budget` 中扣除。

两条现有语义保持不变，并用现有测试守住：
- 单个 task 恰好在第 1025 条指令处报 `FAULT_BUDGET`（`budget_boundary_1024_ok_1025_faults`）。
- 全局余额在 task 运行中途耗尽时，该 task 报 fault，事后 `budget == 0`（`global_budget_exhaustion_also_faults`）。

另外，全局余额在进入 `exec` 前就已经是 0 时，task 被静默跳过，这条语义在 `run_tasks` 里，不受影响。

### 4.4 测试与文档

以下 vm 测试原本断言「运行时 fault」，改为断言「加载时被拒」，理由相同：
- `unknown_op_faults`
- `pc_out_of_bounds_on_empty_code_faults`：空代码仍在运行时报 PC_OOB，保留
- `truncated_operand_is_pc_oob_fault`
- `pushl_popl_bad_index_faults`
- `sys_op_bad_syscall_number_faults`
- `spawn_argc_over_64_faults`
- `spawn_bad_script_id_faults`

vm 测试直接把 `code: &[u32]` 交给 `VmCtx`，不经过镜像。为此加一个测试辅助函数：先跑 `validate_code`，通过了再执行。

`ecl/mod.rs:40-130` 的 fuzz smoke 改为：随机字节**要么在加载时被拒，要么能安全运行完**（不 panic；fault 只能是留在运行时的那几类）。

需要同步修改的文档：
- `docs/ecl-ops.md` 的 fault 表（342–349 行）和 25、55–60 行（CALL / SPAWN 目标 kind 不对，改为在加载时报错）；
- `docs/ecl-lang/8-errors.md` 23–39 行、60 行；
- `crates/stg-harness/src/run.rs:43-44` 的提示文字；
- VM 设计文档 `2026-07-18-m1-ecl-vm-design.md`：在文末追加一条修订记录，不改原文。

## 5. CART_FX 极坐标惰性化（roadmap §5）

**现状**：`integrate_bullets` 对 CART_FX 弹每帧先 `v += a`，再调 `backfill_polar`（`motion.rs:56-72`），做一次 CORDIC `atan2` 和一次 `isqrt`。速度总是回填；只有 `speed ≥ BACKFILL_MIN_SPEED`（1/16 px/帧）时才更新角度，否则角度保持原值。

**改动**：
- 弹的 `flags` 第 6 位定义为 `BULLET_POLAR_STALE`。
- CART_FX 的积分分支，以及反弹的 CART 分支（`integrate.rs:332/349` 附近），只更新 `vx/vy`，然后置上 `BULLET_POLAR_STALE`，不再调 `backfill_polar`。
- 新增 `materialize_polar(i)`：若这一位为 1，就调 `backfill_polar(i)`（规则不变：速度总是回填，角度只在够快时更新），然后清掉这一位。
- **以下每一处调用点，都要先 `materialize_polar`**（依据调研，逐一核对）：
  - 变换 `ADD_SPEED`（`transform.rs:107`），以及 STEP 读取起始值（`transform.rs:168-172`）；
  - 极坐标 setter：`set_speed_at` / `set_angle_at` / `turn_at` / `aim_at_player_at`，以及它们内部经 `refresh_vel_from_polar` 读取另一个分量的路径（`motion.rs:25`、`118-237`）；
  - 模式切换：`SET_ACCEL` / `SET_ANG_VEL` 切到 POLAR 之前、`STOP_FX` 离开 CART 之前（`motion.rs:31-50`）；
  - ECL 读取 `$self_speed` / `$self_angle`（`syscall.rs:535-557`、`741/745`）。这里的 `ctx.body` 是只读的，所以要么把 materialize 提到 syscall 分发之前做，要么让读取方按同样规则算出值、不回写。实现时选前者：语义最干净，读到的值与之后写入的值一致。
- 世界外的读取方不回写 World：
  - Godot 渲染（`stg-godot/src/frame.rs:107-110`）：对 `BULLET_POLAR_STALE` 为 1 的弹，用 `vx/vy` 直接算朝向。渲染不进校验和。
  - harness 的 dump（`stg-harness/src/run.rs:200-201`）：同样按规则现算，保证输出稳定、与是否被读取过无关。
- Tier 0 编码本来就从 `vx/vy` 重新算，不读存储的 speed / angle，不受影响。

**语义变化（已拍板接受）**：只要一颗弹从上一次 materialize 到这次读取之间，速度一直 ≥ 阈值，读到的值与每帧回填**逐位相同**。只有「期间速度曾低于阈值、之后又回升」时，角度才可能不同：现在存的是「最后一次够快时」的方向，改后是「上一次被读取时」的方向。

**测试**：
- `integrate.rs:517-540` 原本断言每帧回填，改为断言 materialize 之后的值。
- 新增对拍测试：一颗抛物线 CART_FX 弹，速度全程在阈值以上，逐帧读 `$self_angle`，结果与旧的每帧回填逐位相同。
- 新增：弹速降到阈值以下再回升，惰性结果按新规则取值（锁定新语义）。
- 新增：进入 POLAR 模式 / STOP_FX 之前一定已经 materialize，切换之后的第一帧位移与旧实现相同。
- 前后对比：位置列必须不变。

## 6. 敌人速度（一等字段）

### 6.1 引擎

**现状**：敌人池里的 `vx/vy` 是速度积分器的状态，**不等于每帧的实际位移**：
- `move_to` 插值期间，位置由插值器重新计算，`vx/vy` 保持旧值；
- 到站的那一帧位置吸附到终点，但只有 `vel_touched == 0` 时才清速；
- `move_to(dur=0)` 瞬移时 `vx` 被清零；
- 时停时整个积分被跳过，位移为 0，`vx` 保持原值。

训练侧因此在 `envwrap.enemy_velocity` 里按 id 匹配、用坐标差分，还要加一道 `≤ 16·frame_skip` px 的瞬移守卫。

**改动**：
- 敌人池（`enemy.rs:24`）新增 `dx: Fx, dy: Fx`，语义是**本帧积分阶段（phase 5）中位置的变化量**。
- 在 `integrate_enemies` 里，每只敌人进入循环体时记下 `x0, y0`，出循环时写 `dx = x − x0`、`dy = y − y0`。这样可以自然覆盖以下情况：
  - 普通积分：`dx = vx`；
  - `move_to` 插值与到站吸附：如实计入；
  - 瞬移（`move_to(dur=0)`，发生在 phase 2）：**不计入**，所以瞬移那一帧的 `dx` 只含 phase 5 的位移，不需要另加标记；
  - 出生：phase 2 出生、同一帧 phase 5 积分，第一帧就拿到真实值。
- 时停帧：`integrate_enemies` 被整体跳过（`integrate.rs:31-32`），在跳过的分支里把所有存活敌人的 `dx/dy` 清零。
- `EnemyInit` 的初值为 0。23 处穷尽构造 `EnemyInit { .. }` 的地方（分布在 9 个文件中）都要补上这两个字段。
- 池字段进入校验和与 SaveBytes，池大小哨兵（`step.rs:2350-2364`）要跟着更新。

### 6.2 Tier 0

- 敌人行步长 38 → **46**：新增 `vx @38`、`vy @42`，类型 i32 Q16.16，单位 px/帧，值取自 `dx/dy`。`frame_skip > 1` 时取观测前最后一帧的值（不是 k 帧平均）。
- `layout.rs` 的偏移表和 HELLO 里的字段表同步加这两项；`tests/fixtures/proto_v1_hello.json` 更新。HELLO 仍为 `"proto":1`：SPEC 规定在行尾追加字段不需要升版本（`SPEC.md:376-379`）。
- `encode.rs:200`、`vec_env.rs:57/451` 里写死的 38 改为读 `ENEMIES.stride`。
- 镜像：Tier 0 不做镜像，训练侧对 `vx` 取负，和现在一样。

### 6.3 stg-agent-proto

- `SPEC.md` 敌人行（216–240 行）追加 `vx` / `vy` 两个字段，写明语义：本帧实际位移，瞬移不计入，时停为 0；`frame_skip > 1` 时取最后一帧。
- `c/sa_layout.h` 里的 `SA_ENEMY_STRIDE` 改为 46。
- `sa_encode.c`：追加的两列先写 0，并注释「部署侧速度仍由 `sa_model.c` 差分得出，见 stg-engine follow-ups」。
- 核对 `.stglog` 读取器（`obs.py`）的步长是否取自 HELLO（调研表明是），不用改代码。
- 按「三个后端字段逐字相同」的规则，th06nc 的 HELLO 也应当声明这两列。本刀让 th06nc 写 0，缺口记入 §7 的待办。

### 6.4 训练仓（stg-rl-train）

- `envwrap` 从 `stg_rl.OFFSETS` 读取敌人的 `vx` / `vy`，和其他定点字段一样除以 65536。
- 删除 `enemy_velocity`，以及它依赖的 id 匹配、prev 缓冲和瞬移守卫常量。镜像时对 vx 取负的逻辑保留（`envwrap.py:566`）。
- import 时断言 `stg_rl >= 0.2.0`，给出清楚的错误提示。
- 测试：
  - `tests/test_envwrap.py:139, 241-242` 写死的 38 改为读步长；
  - 新增新旧对照：在一张不含瞬移的卡上、`frame_skip = 1`，新字段与旧差分函数的结果逐位相同（旧函数只留在测试里当参照）；
  - 新增：瞬移那一步，新字段只含 phase 5 的位移；出生那一步速度非零。
- `experiments.md` 记下语义变化：出生帧有值、小于 16 px 的瞬移不再当作速度、`frame_skip > 1` 取最后一帧。`GRAPH_VERSION` 不变，旧 checkpoint 仍可加载。
- `perf-baseline.md` 的待办列表同步更新。

## 7. 本刀不做、记为待办

在 `docs/follow-ups.md` 新增条目：

1. **部署侧敌人速度仍是差分**：`sa_model.c:88-131` 按 id 差分，带 `≤ 16` px 的守卫；th06nc 的 HELLO 里 vx/vy 两列写 0。与训练的差异：出生帧（部署为 0）、小于 16 px 的瞬移（部署会当成速度）。**触发点**：下次动 th06nc 抽取器，或者实机迁移显示敌人相关的差距。届时要查 TH06 的敌人结构能否直接读到每帧位移。
2. **`pack_handle` 的 15 位 generation 回绕**（已有记录，`follow-ups.md:525`）：训练侧不再按 id 匹配，这个问题对训练已经无关；C 侧的差分仍会受影响，但 C 侧本来没有 `!= 0` 的守卫。随第 1 条一起处理。
3. roadmap 里剩下的 §1、§4、§6 维持原状。本刀完成后，在 roadmap 里把 §2、§3、§5 标为已落地，并写明版本号。

## 8. 风险

- **§4 校验器太严，拒掉了合法镜像**：训练卡池 84 张、scenes、Godot 启动脚本，全部要编译一遍，确认没有新的编译错误。这一步作为 §4 提交的验收条件。
- **§5 漏掉某个极坐标读取点**：后果是读到陈旧的 speed / angle，行为出现漂移。防线有两道：调研表逐项核对，以及 debug 构建下的断言——`speed()` / `angle()` 的读取器在 `BULLET_POLAR_STALE == 1` 时 `debug_assert!` 失败（需要把直接的字段访问改成走访问器，或者至少在已知的读取点加断言）。
- **校验和变化让回归难以发现**：所以每项按 §2 的差异表逐项做前后对比，不把四项混在一起比。
