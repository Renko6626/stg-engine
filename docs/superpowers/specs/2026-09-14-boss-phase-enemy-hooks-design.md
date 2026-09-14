# boss 换段与敌人钩子刀 —— 非符段 / 超时钉血 / 结束读口 / 带参生成 / 判定写口 / 清场 / 半径清弹（设计，2026-09-14）

> 状态：**已落地（2026-09-14）**，实施偏差见 §11。计划 `docs/superpowers/plans/2026-09-14-boss-phase-enemy-hooks.md`。
> 来源：TH18 原版 ECL 迁移差距评估（2026-09-14，22 个原版脚本 × 本仓 ENGINE_VER 20）的 A 类五件——
> 第 1 关直接要用、且用现有 `.ecl` 合成不了的缺口。原版证据取自 `renkolab/local/th18.v1.00a/ecl/*.ecl.txt`
> 与 `th18-leveledit/docs/ECL.md` 先例表（频次口径：22 文件全量；dump 为 CP932，统计须 `grep -a`）。
> 前置：符卡计器刀（spec `2026-07-24-spell-meter-design.md`）、敌人死亡效果刀（`2026-07-30`）、
> 玩法刀（`2026-09-14-gameplay-tools-design.md`）。

## 1. 为什么做

| 缺口 | 原版用量 | 第 1 关谁要 | 现状为什么合成不了 |
|---|---|---|---|
| 非符段（血线+时限，不宣言不计 bonus） | `setInterrupt` 81 | 中 boss「歯車」不算符卡；非符 1/2 | 只能拿符卡冒充：会发宣言请求、出收卡/失败横幅 |
| 超时钉血 | 原版超时 `life = hp_value` | 非符 2「限时是设计核心」（血线 1300→1000，超时后符卡 1 最多多 300 血） | 脚本写不了 hp；超时结算不动血 |
| 结束方式读口 | `$TIMEOUT` 读 154 次、0 次写 | 超时不给奖励 | 只有事件，脚本读不到 |
| 敌任务传参 | `enmCreate*` ≈390 次靠 `$I0–3/%F0–7` 继承 | 编队镜像、使魔拿 boss 号 | typeck 强制 task 位无参；globals 顶替有同帧竞态（新任务次帧才首跑，同帧两只读到同一值） |
| 判定写口 | `setHitbox`/`setHurtbox` 各 111、`setInvuln` 39、`flagSet(3)` 系 ≈34 | 蕾米登场/段间无敌；中 boss 大受击框 | `invuln` 字段、伤害跳过、每帧递减都在，缺写口；半径只在生成时定 |
| 清场 | `enmKillAll` 108（几乎每个段首） | boss 召使魔时换段清场 | 只能每只使魔轮询旗后 return |
| 自身敌号 | `$ID`（st07bs 传给使魔） | 配合带参生成 | 敌任务拿不到自己的号 |
| 清弹不给道具 / 按半径清弹 | `etClear` 150（超时分支）、`Ecl_EtBreak` 扩张消弹波 | 超时清弹 | 清弹区一律转星星，只有全屏 |

## 2. 人类拍板

| # | 议题 | 裁定 |
|---|---|---|
| ① | 非符段怎么表达 | **符卡机构加一位** `SPELL_NONSPELL`，复用计时/血线/模式随段/血条；不宣言、不付分；表层糖 `phase_begin` |
| ② | 超时怎么处理 | **所有超时钉血** + **超时路径的自动清弹不给星**（对齐 ZUN 超时走 `etClear`）；击破路径照旧给星 |
| ③ | 结束方式读口 | `spell_result(slot) -> int`，每槽存最近一次结束方式 |
| ④ | 传参范围 | **只做 `spawn_enemy`**；`fire` / `sh_task` / `spell_begin` 的 pattern 仍只收无参 sub |
| ⑤ | 清场怎么死 | `kill_all_enemies(mode)`：0 静默退场 / 1 走 `die()` 全套 |
| ⑥ | 判定标志 | 无敌只用 `set_invuln(frames)`（自机弹命中本来就穿透不消耗，invuln 期间不掉血也不发命中事件 ≡ 原版不吃弹）；标志位只加 `ENEMY_NO_BODY` / `ENEMY_KILLALL_EXEMPT` 两个 |
| ⑦ | 判定单位 | 单个半径 `fx`，与生成时 12/16 同口径 |
| ⑧ | 清弹接口 | 新增 `clear_bullets_at(x, y, r, stars)`，`clear_bullets()` 不变 |

## 3. 换段机构（`spell.rs` / `world.rs`）

### 3.1 `SPELL_NONSPELL`

`SpellSlot.flags` bit2 = `SPELL_NONSPELL`（值 4）。`spell_begin_internal` 带此位时：

- **照旧**：参数校验、计时 `frames_left`、血线 `hp_threshold` 与伤害下钳、`hp_start`、代际戳 `epoch`、
  模式任务随段生死（`spell_bound`）、`settle_spells` 步骤 5 的 `boss_ui` 自动喂（含 `spell_id` 原样写入）。
- **不做**：不发 `EVT_SPELL_DECLARED`、不发 `REQ_SPELL_DECLARE`。
- **强制**：`bonus_now` / `bonus_floor` / `dec_per_frame` 写 0（传入的 `bonus0` 忽略、不计违约）；
  `SPELL_SURVIVAL` 位被忽略（非符段没有收卡概念，超时就是超时）。
- 资格 `capture_ok` 照常写 1，照常被 `void_spell_captures` / 中弹轮询清 0——对非符段无消费者，保留是为了不分叉代码路径。

表层糖 `phase_begin(slot: int, pattern: sub|none, time_limit: int, hp_threshold: int)`：编译期降低为
`spell_begin(slot, 0, pattern, time_limit, 0, SPELL_NONSPELL, hp_threshold)`，**不新增 syscall**，
与手写这行编译出逐字节相同的 `EclImage`。`wait_spell()` 对两种段通用。

### 3.2 结算：`settle_one_spell(slot, cause)`

参数由「captured + 失败原因」改为**结束方式** `cause`（captured 在函数内按 `capture_ok` 与 flags 推导）：

| `cause` | 常量 | 触发 |
|---|---|---|
| 1 | `SPELL_END_HP` | 破卡自动检测：句柄失效 / `ENEMY_DYING` / `hp ≤ hp_threshold`（含 `die()`、D9 退场、清场杀死绑定 boss） |
| 2 | `SPELL_END_TIMEOUT` | `frames_left == 0` |
| 3 | `SPELL_END_MANUAL` | `spell_end()` |

结算原子包按顺序：

1. **超时钉血**（仅 `cause == 2`）：绑定 boss 句柄有效且未 `ENEMY_DYING` → `hp = min(hp, hp_threshold)`。
   对普通卡、耐久卡、非符段一律执行。
2. **付分与事件**：
   - 普通卡（无 `NONSPELL`）：captured 规则、`players[0].score += bonus_now`、`EVT_SPELL_CAPTURED` /
     `EVT_SPELL_FAILED`（data[1] 的失败原因沿用现值：资格失 1 / 超时 2）、`REQ_SPELL_RESULT` **全部不变**。
   - 非符段：不付分，不发 CAPTURED/FAILED，不发 `REQ_SPELL_RESULT`；发 **`EVT_PHASE_ENDED`（12）**，
     `a_index/a_gen` = 绑定 boss，`x/y` 同现有取法，`data = [spell_id, cause]`。
3. **自动清弹**：除非 `SPELL_NO_CLEAR`，铺全屏清弹区；`cause == 2` 时该区带 `FIELD_NO_STAR`（§6）。
4. **写读口**：`spell_last_result[slot] = cause`。
5. 槽清零、`boss_ui[slot]` 清零（不变）。

### 3.3 读口 `spell_result(slot: int) -> int`

- 新 World 字段 `WorldBody::spell_last_result: [u8; MAX_BOSSES]`（`MAX_BOSSES = 2`）。
  `new_game` 初值 0。**只由结算写**；`spell_begin` 不清它。
- syscall **131** `SYS_SPELL_RESULT`（1xx 读族，紧挨 130 `spell_timer`），owner 无限制。
  返回 0 = 该槽还没结束过 / 1 / 2 / 3。`slot ∉ [0, MAX_BOSSES)` → 返回 0 + `contract_viol` + `BAD_ARGS`（P4-b，不 Fault）。
- 典型写法：

```ecl
phase_begin(0, nonspell2, 1200, 1000);
wait_spell();
if spell_result(0) != SPELL_END_TIMEOUT {
    drop_add(ITEM_BOMB_PIECE, 4);
    drop_items();
}
```

## 4. `spawn_enemy` 带参

### 4.1 表层

`spawn_enemy` 第 7 参（task 位）接受 `name` / `name(实参…)` / `none`。typeck：

- `name(实参…)`：目标须为 async sub，实参个数与类型按其签名逐位核对，规则与 `spawn f(args);` 同。
- `none(…)` 语法错误；`name` 不带括号 = 零实参（须目标本身无参，同现规则）。
- `fire` / `sh_task` / `spell_begin` 的 SubRef 位**仍只收无参 async sub**；带括号写法报错，文案指明「目前只有 spawn_enemy 的 task 位支持带参」。

### 4.2 字节码：原地改 syscall 210 调用约定

压栈序（正序）：`x, y, hp, drop_table, score, sprite, task_sub, arg0 … arg(n−1), argc`。
`sys_spawn_enemy` 弹栈与门禁顺序（**任一门不过都零副作用：不建敌、不占任务槽**）：

1. 弹 `argc`；`argc ∉ [0, LOCALS]` → Fault(2)。
2. 栈内剩余值 < `argc + 7` → Fault(2)。
3. 逆序弹 `argc` 个实参进 `args[0..argc)`，再逆序弹 7 个固定参数。
4. `task_sub < 0`（none）且 `argc > 0` → Fault(0)。
5. `task_sub ≥ 0`：在册、`SubKind::Async`、`param_types.len() == argc`，否则 Fault(0)。
6. 建敌（后续与现实现同）；任务 spawn 成功后把 `args[0..argc)` 写进新任务 `locals[0..argc)`。

拷参逻辑与 `OP_SPAWN` 抽成共用函数（例如 `TaskPool` 上的「spawn 后写实参」助手），两处调用同一实现，
不写两份。新任务的 `spell_bound` / `spell_epoch` 仍为 0（与现口径同）。

**不另开 211**：两条路都是冻结面变更、都要 bump；保留零参 210 只会让两条生成路径长期并存。
现有 `.ecl` 源码零改动，重编后 `argc = 0`。

## 5. 敌判定写口、`$self_enemy`、清场

### 5.1 44x 敌判定族（self-only）

四条共用门禁（同 `move_to`）：owner 非 ENEMY → Fault(0)；owner 句柄悬垂 → no-op + `contract_viol` + `STALE_HANDLE`。

| 号 | 内建 | 行为 | 参数越界 |
|---|---|---|---|
| 440 | `set_invuln(frames: int)` | 覆写 `enemies.invuln`；0 = 取消 | `frames ∉ [0, 65535]` → no-op + `contract_viol` + `BAD_ARGS` |
| 441 | `set_hitbox(r: fx)` | 写体碰半径 `radius`（碰撞行 3） | 复用 `clamp_radius` 钳 `[0, MAX_ENTITY_RADIUS]`，钳了计违约 |
| 442 | `set_hurtbox(r: fx)` | 写受击半径 `hurtbox`（碰撞行 4/7） | 同上 |
| 443 | `set_enemy_flag(flag: int, on: int)` | `on != 0` 置位，否则清位 | `flag` 须为 `ENEMY_NO_BODY \| ENEMY_KILLALL_EXEMPT` 的**非空子集**；含 `ENEMY_DYING` 或未知位 → 整条 no-op + `contract_viol` + `BAD_ARGS` |

敌 `flags: u8` 新位：`ENEMY_NO_BODY = 1 << 1`、`ENEMY_KILLALL_EXEMPT = 1 << 2`（bit0 仍是 `ENEMY_DYING`）。
池布局不变；`EnemyInit.flags = 0` 已满足复用槽写满。

**碰撞行 3**（`collide_body_player`）跳过带 `ENEMY_NO_BODY` 的敌。行 4/7、`nearest_enemy` 不变。

### 5.2 `$self_enemy`

syscall **026**（0xx `$` 变量族，`builtins.rs::ENGINE_VARS` 登记，类型 `int`）：owner 为 ENEMY → 打包敌号
（`pack_enemy_handle`，与 `spawn_enemy` 返回值同编码）；其余 owner → **-1**。
与族内其它变量「非敌读 0」**有意不同**：打包敌号 0 是合法值，读 0 分不清。手册写明。
0xx 是全表唯一与 op 号域重叠的族（follow-ups F6）；26 不是现有 op 号（op 占 20–25、30–33），不新增重叠点，
但 `lang/mod.rs::opcodes_of` 注释里的族清单要补上 026。

### 5.3 `kill_all_enemies(mode: int)`

syscall **531**（5xx 局面族，紧挨 530 `die`），owner 无限制。

- 按敌池索引升序遍历存活敌，**跳过**：调用任务的 owner 敌（owner 为 ENEMY 时）、已 `ENEMY_DYING`、带 `ENEMY_KILLALL_EXEMPT`。
- `mode == KILL_SILENT (0)`：置 `ENEMY_DYING`，不掉落、不加分、不发事件——与 D9 主任务 return 退场同口径。
- `mode == KILL_DIE (1)`：逐只 `kill_enemy`（掉落 + 加分 + `EVT_ENEMY_DIED` + `REQ_ENEMY_DEATH`），同 `die()`。
- 其它 `mode` → 整条 no-op + `contract_viol` + `BAD_ARGS`。
- 被杀敌的任务树由相位 2 owner 门禁次帧清杀（现机制）；被杀的若是符卡槽绑定 boss，当帧相位 7 按 `SPELL_END_HP` 结算。
- 无返回值。

## 6. `clear_bullets_at`

syscall **541**（5xx，紧挨 540 `clear_bullets`），owner 无限制：
`clear_bullets_at(x: fx, y: fx, r: fx, stars: int)`。

- `create_field`：中心 `(x, y)`、半径 `r`（沿用 `create_field` 的 `FIELD_MAX_RADIUS` 钳制与违约计数）、
  `dmg_per_frame = 0`、`life = 1`、`flags = FIELD_CLEAR_BULLETS | (stars == 0 ? FIELD_NO_STAR : 0)`。池满走现有 P4-a。
- 新清弹区标志 `FIELD_NO_STAR = 1 << 2`。settle 趟一：弹照常标 `BULLET_CLEARED`、照常计入 `EVT_FIELD_CLEARED`，
  触发清除的那条命中所属清弹区带 `FIELD_NO_STAR` 时**不调** `spawn_star_at`。一颗弹被多个区覆盖时由 `hits`
  顺序中第一条命中决定（幂等门已保证只处理一次），确定性。
- `clear_bullets()` 不变（全屏 + 给星）。§3.2 超时清弹用「全屏 + `FIELD_NO_STAR`」，`field.rs` 提供对应构造函数，
  与 `fullscreen_clear_field()` 共用同一份几何。
- 扩张式消弹波：脚本每帧调一次、半径逐帧增大（清弹区池 cap 16，每帧一个 life=1 的区不构成压力）。

## 7. 冻结面、号表与横切

- **`ENGINE_VER` 20 → 21**（整刀一次）。理由：syscall 号表增 7 号 + 210 调用约定变更、碰撞行 3 语义、
  符卡结算行为（超时钉血/清弹不给星）、新 World 字段、新事件号。
- syscall：`026` self_enemy · `131` spell_result · `210` 调用约定变更 · `440–443` 敌判定族 · `531` kill_all_enemies · `541` clear_bullets_at。
- 事件：`EVT_PHASE_ENDED = 12`（`events.rs`）。
- 注入常量（`consts.rs` ① 段）：`SPELL_SURVIVAL = 1`、`SPELL_NO_CLEAR = 2`、`SPELL_NONSPELL = 4`、
  `SPELL_END_HP = 1`、`SPELL_END_TIMEOUT = 2`、`SPELL_END_MANUAL = 3`、`ENEMY_NO_BODY = 2`、
  `ENEMY_KILLALL_EXEMPT = 4`、`KILL_SILENT = 0`、`KILL_DIE = 1`。值取自 Rust 侧同名常量，单一来源。
- 新 World 字段 `spell_last_result: [u8; 2]`：derive 自动进校验和；`copy_into` 字段表与存档同步。
  **只有 2 字节，可能被对齐填充吞掉、尺寸哨兵不报**（follow-ups D20 盲区）——实施时人工核对 `copy_into` 字段表，并加一条
  「改该字段 → 校验和变」的直测。
- 桥：导出 `EVT_PHASE_ENDED`、`SPELL_NONSPELL`。壳 `hud.gd`：`hud_spell(0).flags & SPELL_NONSPELL` 时只显示倒计时、不显示卡名。
- 金向量：行为上不走超时路径（符卡时限 3600 > 600 帧窗口），但 210 调用约定改变镜像字节；md5 是否变化在计划中实测，变则重钉并记理由。

## 8. 判别式测试

每条的取值必须能区分对错（CLAUDE.md「金向量抓不了行为回归」）。

1. **超时钉血**：boss hp 1000、血线 300、不打它 → 超时后 `hp == 300`；紧接下一张卡 `hp_start == 300`。对照：打到血线路径 hp 不被额外改写。
2. **超时清弹不给星**：同场景弹若干，超时结算后道具池无新增 `ITEM_STAR`；对照打到血线结算后星星数 = 被清弹数。
3. **非符段**：无 `EVT_SPELL_DECLARED`、无 `REQ_SPELL_DECLARE/RESULT`、score 不变；结束发 `EVT_PHASE_ENDED`，`data == [spell_id, cause]`；
   带 `SPELL_SURVIVAL|SPELL_NONSPELL` 超时仍记 `SPELL_END_TIMEOUT` 且不付分。
4. **`spell_result`**：开局 0；三种结束方式各得 1/2/3；新 `spell_begin` 后读到的仍是上一次结果；槽号越界返 0 且违约 +1。
5. **带参生成**：同一帧连续生成两只敌、实参不同，次帧两只任务各自读到自己的实参（堵 globals 竞态的判别腿）；
   参数个数不符 → Fault 且敌池计数不变；`none` 带参 → Fault；typeck 报错（个数/类型/fire 带参）；零参写法行为不变。
6. **hitbox / hurtbox**：选取距离使「radius 改了才撞 / hurtbox 改了才中」可区分——`set_hitbox` 只改变行 3 结果、不改变行 4/7；`set_hurtbox` 反之。
7. **`set_invuln`**：无敌期间自机弹重叠不掉血、无 `EVT_SHOT_HIT_ENEMY`；N 帧后恢复；`frames = -1 / 65536` no-op 且违约 +1。
8. **`ENEMY_NO_BODY`**：自机与敌重叠不中弹，同时自机弹仍能伤它；`set_enemy_flag(ENEMY_DYING, 1)` 与未知位被拒、flags 不变。
9. **`kill_all_enemies`**：场上 调用者 / 免清 / 已死 / 普通 各一 → 只有普通被杀；mode 0 无事件无加分无掉落；mode 1 事件+分+掉落齐；
   mode 2 no-op 违约 +1；杀绑定 boss 后 `spell_result == SPELL_END_HP`。
10. **`$self_enemy`**：敌主任务读值 == 生成方拿到的返回值；关卡根任务读 -1。
11. **`clear_bullets_at`**：圆内弹被清、圆外（半径外 1px）弹保留；`stars = 0` 无星、`stars = 1` 有星。
12. **表层端到端**：`phase_begin` 降低与手写逐字节相同；带参 `spawn_enemy` 编译运行；`gen-ecl-meta` 防漂移测试绿。
13. **闸门**：fmt / clippy / `cargo test --workspace` / storm / verify-tables / 两冒烟 全绿；`ecl_consts` 注入表与 Rust 常量一致性测试覆盖新常量。

## 9. 文档同步

- `docs/ecl-lang/3-enemy.md`：带参生成、判定写口、`$self_enemy`、`kill_all_enemies`（三条死亡路径表加「清场 mode 0/1」两行）。
- `docs/ecl-lang/4-bullets.md`：`clear_bullets_at`。
- `docs/ecl-lang/6-spell-and-stage.md`：非符段、`phase_begin`、超时钉血与不给星、`spell_result`、常量表。
- `docs/ecl-lang/7-reference.md`：`gen-ecl-meta` 重生成；引擎常量表补行。
- `docs/ecl-ops.md`：号表、210 调用约定、符卡计器节、事件表。
- `docs/zun-ecl-v2-reference.md`：新增「游戏指令迁移对照」段：`setInterrupt`+`$TIMEOUT` → `phase_begin`/`spell_begin`+`spell_result`、
  `setInvuln` → `set_invuln`、`setHitbox`/`setHurtbox` → `set_hitbox`/`set_hurtbox`（ZUN w/h 直径或半径待验）、`flagSet(2)` → `ENEMY_NO_BODY`、
  `enmKillAll` → `kill_all_enemies`、`$ID` → `$self_enemy`、`$I0–3/%F0–7` 继承 → 带参 `spawn_enemy`、`etClear`/`etCancel(r)` → `clear_bullets_at`。
- `stg-world-design.md`：D8 碰撞矩阵行 3 注记 `ENEMY_NO_BODY`；D12 syscall 表补行。
- `docs/follow-ups.md`：记本刀不做的五件（见 §10）。
- `PROGRESS.md`：史加一行，重写「现在」。

## 10. 不做什么（记 follow-ups）

1. `fire` / `sh_task` / `spell_begin` pattern 带参——触发点：内容里出现弹任务或模式需要参数化（`sh_task` 需在发射器槽存实参，涨 TaskPool）。
2. 永久「不吃弹 + 不可锁定」位（ZUN `flagSet(1)` 的锁定语义）——触发点：自机 homing 需要排除某类敌。
3. 一只敌同时挂两个计时器（st07mbs「整体计时 + 卡内计时」）——触发点：Extra 中 boss 类内容。
4. 表层开放 `kill_children` / 按句柄杀任务——开放时须同步处理 `vm.rs` D9 注释所述 `main_task` 清零。
5. `death_script` 通电（`setDeath`）——设计已定形（`stg-world-design.md` 相位 9 挂钩），第 1 关不用。

另：`move_limit`/`move_rand`、激光、ZUN 运动 mode→easing 对照表不在本刀。第 1 关内容脚本由下一刀（内容刀）使用本刀的新能力编写。

## 11. 实施偏差

| # | 计划/spec 原写 | 实际 | 原因 |
|---|---|---|---|
| 1 | `ecl-ops.md` 号表行在文档任务（Task 7）统一补 | Task 2/3 随号同步写入，号表冻结计数 79→85→87 | 冻结面守卫 `ecl_ops_doc_syscall_numbers_match_the_constants` 与 `syscall_table_is_hundred_partitioned_and_unique` 要求号与文档、计数同刀同步 |
| 2 | §9：`clear_bullets_at` 写进 `4-bullets.md` | 写在 `6-spell-and-stage.md` 的 `clear_bullets()` 同节 | 第 4 篇没有清弹节；与 `clear_bullets()` 并列更好找 |
| 3 | §4.2 未提 builder | `stg-ecl-compiler` 的 builder `sys_spawn_enemy` 只追压 `argc = 0`，不暴露带参形态 | 带参只走表层语言；builder 是测试/临时 DSL |
| 4 | — | `step::tests::engine_ver_anchored` 钉版测试随 bump 同步（理由链加 20→21） | 既有「bump 须改本测试」纪律 |
| 5 | §7：金向量 md5 实测为准 | `15a5167c…` → `de70b473ff557cbfb219df78f8f0b495` | `spell_last_result` 恒 0 但进哈希；风铃卡不走超时路径，行为不变 |

