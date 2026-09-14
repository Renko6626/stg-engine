# ZUN ECL V2 系统指令表（本地参考副本）

> **来源**：[Priw8 的 ECL 文档](https://priw8.github.io/)（数据文件
> `js/ecl/ins.js` / `js/ecl/vars.js`，**TH13（神灵庙）表**），2026-07-18 抓取整理。
> 社区逆向成果，归功 Priw8 及 thtk 社区；此副本仅作我们设计时的对照参考。
> **对照结论**（详见 follow-ups C13 增补）：七项 M1 拍板无一需推翻；三条手感增量
> （time label 糖/引擎状态变量化/jnz）已入表层语言需求。

## 系统指令 ins_0 - ins_93（VM 级；游戏指令住 300+/400+/500+/600+ 高段，不录）

参数签名：`m`=sub 号 `o`=跳转目标 `S`=整数 `f`=浮点；每指令二进制头带
time/rank_mask/param_mask 字段（时间轴混编 + 难度过滤 + 栈引用替换——我们三者皆无，见对照）。

| # | 助记 | 参数 | 语义 | 我们的对应 |
|---|---|---|---|---|
| 0 | nop | — | 空指令 | —（无需求） |
| 1 | loop | — | 回到当前调用栈顶（sub 开头无条件循环） | DSL `loop_forever`（JMP 合成） |
| 10 | ret | — | sub 返回 | `RET 5` |
| 11 | call | m | 调 sub（可带参，拷入被调栈帧） | `CALL 4`（无参传递——locals 共享，拍板 4） |
| 12 | goto | o | 无条件跳 | `JMP 2` |
| 13 | jz | o | 弹栈，零则跳 | `JZ 3` |
| 14 | jnz | o | 弹栈，非零则跳 | **无——C13⑦ v2 候补** |
| 15 | callAsync | m | 异步调 sub（开协程） | `SPAWN 50` |
| 16 | callAsyncId | m,S | 异步调 + 赋脚本自定 id | SPAWN 返回句柄压栈（等价能力） |
| 17 | killAsync | S | 按 id 杀最近 async sub | 句柄 + kill（绑定层） |
| 18-20 | （异步管理，未考据明） | | | — |
| 21 | killAllAsync | — | 杀本敌全部 async sub | `KILL_CHILDREN 52`（差异：ZUN 按敌归属，我们按 parent 直系） |
| 23 | wait | S | 停 %1 帧 | `WAIT 1`（取栈顶低 16 位）—— **2026-08-01 起才真的相等**：此前我方 `wait(n)` 的周期是 n+1，这一行宣称的等价是无声失真的（见 `design_doc.md` 的 `wait n` 勘误）。这份逐条对照当时的沉默同意，正是那个差一活了一年的一环 |
| 24 | waitf | f | 浮点亚帧等待 | **永不进**（I6 整数帧） |
| 30 | printf | … | 调试打印 | —（诊断走事件/校验和） |
| 40 | stackAlloc | S | 栈指针抬 %1 字节（帧局部变量区） | 无——locals 定长 64 共享（拍板 3/4） |
| 42 | pushI | S | 压整数 | `PUSHI 10` |
| 43 | popI | S(var) | 弹入变量 | `POPL 12` |
| 44 | pushF | f | 压浮点 | **永不进**（I1；定点走同一整数栈） |
| 45 | popF | f(var) | 弹浮点入变量 | 同上 |
| 50/51 | addi / addf | — | 栈加（int/float 双生） | `ADD 20`（单型 wrapping） |
| 52/53 | subi / subf | — | 栈减 | `SUB 21` |
| 54/55 | muli / mulf | — | 栈乘 | `MUL 22` / `MULF 30`（Q16.16） |
| 56/57 | divi / divf | — | 栈除 | `DIV 23` / `DIVF 31`（除零 Fault，MIN/-1 回绕） |
| 58 | modi | — | 栈模 | `MOD 24` |
| 59/60 | eqi / eqf | — | == | `EQ 40` |
| 61/62 | nei / nef | — | != | `NE 41` |
| 63/64 | lti / ltf | — | < | `LT 42` |
| 65/66 | lei / lef | — | <= | `LE 43` |
| 67/68 | gti / gtf | — | > | `GT 44` |
| 69/70 | gei / gef | — | >= | `GE 45` |
| 71/72 | noti / notf | — | 逻辑非 | EQ+PUSHI 0 合成（未设专条） |
| 73 | or | — | 逻辑或 | 合成（比较后算术）；位运算族整体未设——需求出现再收编 |
| 74 | and | — | 逻辑与 | 同上 |
| 75/76/77 | xor / bitor / bitand | — | 位运算 | 未设（v2 空隙可收） |
| 78 | decPush | var | **压变量再自减**（循环计数原语） | DSL repeat 用 PUSHL/SUB/POPL/JZ 合成——ZUN 一条搞定，v2 候补可议 |
| 79/80 | sin / cos | — | 栈顶浮点三角 | `SINB 32 / COSB 33`（BAM 查表定点） |
| 81 | polar→cart | … | 角度+半径 → (x,y) | 世界侧 `polar_to_vec`（syscall 内部用；脚本层无直通） |
| 82 | normAngle | var | 角度归一化 | BAM u16 天然回绕，无需求 |
| 83/84 | negi / negf | — | 取负 | `NEG 25`（wrapping） |
| 85 | distSq | … | %1 = %2²+%3² | 世界侧 `len_sq`（脚本层无直通——需求出现走 syscall） |
| 86 | dist | … | 开根距离 | 核内禁开根比较（I1 平方距离），脚本要距离再议 |
| 87 | angleToPoint | … | 两点方位角 | `SYS 120 aim_player_angle`（特化版）；通用版候补 |
| 88 | sqrt | — | 栈顶开根 | `isqrt` 核内有，脚本未暴露 |
| 89 | mulAssign | … | %1 = %2×%3 | 合成 |
| 90 | rotatePoint | … | 点绕角旋转 | 合成/候补 |
| 91 | tween | … | 变量 %2 在 %3 帧内按模式 %4 从 %5 变到 %6 | **注意**：ZUN 有脚本变量级缓动！我们的缓动住 xform（弹）与 move_to（敌）；脚本变量 tween 无对应——表层语言需求候补 |
| 92 | tweenBezier | … | 带控制点 tween | 同上 |
| 93 | randPoint | … | 随机点入 (%1,%2) | ECL 循环 + rand_range 合成（路线甲：原语不内建 RNG） |

## 特殊变量（负 ID 引擎寄存器，TH10+ 口径）

| ID | 含义 | 我们的对应 |
|---|---|---|
| -10000 | 随机整数（**每次引用即抽号**） | **拒绝**——显式 `SYS 150 rand_range`（消耗序必须显式，C13⑥） |
| -9999 / -9998 | 随机 float [0,1) / [-π,π] | 同上 + I1 禁浮点 |
| -9995 / -9994 | 敌（自身）x / y | `SYS 20/21 self_x/y` |
| -9991 / -9990 | 自机 x / y | `SYS 10/11 player_x/y` |
| -9988 | 敌出生以来帧数 | `SYS 32 self_age`（M1.5 落地）。**注语义偏离**：ZUN 这条只对敌有意义
  （"敌出生以来"）；我们量的是**任务**出生以来的帧数（`frame - task.born_frame`），对全部
  owner 种类（含 STAGE）均有意义，零新状态（复用既有 `Task.born_frame`）——代价是"任务"与
  "owner 实体"生命周期不严格重合时两者会分叉（如任务在 owner 存活期间被脚本重新 `SPAWN`
  出一份新的，新任务的龄从 0 重新计，不继承旧任务/owner 实体的年龄），详见 `ecl-ops.md`
  该条目。 |
| -9960 | rank 数值（-1024..1024，连续难度！） | `globals` 约定槽（拍板 5；ZUN 双轨实证变量式是刚需） |
| -9959 | 难度档（E0/N1/H2/L3/EX4） | 同上 |
| -9954 | 敌当前 HP | `SYS 30 self_hp` |
| （无对应 ZUN 特殊变量） | 敌上限 HP | `SYS 31 self_hp_max`（M1.5 新增，无 ZUN 对照——引擎侧
  补齐 `self_hp` 的自然邻居，非对照驱动；非敌 owner 恒 0，同 `self_hp` 误用策略） |

## 结构性差异备忘（设计对照的核心五条）

1. **单层扁平指令空间 vs 分层**：ZUN 系统指令与游戏指令（敌/弹/boss 操作，300+ 高段）同住
   一个编号空间、直捅引擎；我们拆 op 表（纯 VM）+ syscall 白名单（`SYS 60` 单口沙箱）。
2. **时间轴混编**：ZUN 每指令带 time 字段、sub 有时钟（`+N:` 标签文化）；我们纯 WAIT
   命令式——表层语言以 `+N:` 糖回归手感（C13⑤）。
3. **rank_mask 每指令 + param_mask 栈引用替换**：均不采（难度走变量、引用走显式 push）。
4. **int/float 双生全家桶**：I1 之下我们单 i32 + 定点变体，指令数减半。
5. **无预算无沙箱**：ZUN 死循环即卡死、坏指令即 crash；我们双层预算 + 六类确定性 Fault +
   fuzz 实证——rollback/联机时代的必需品，ZUN 无此需求。

## 游戏指令迁移对照（boss 换段与敌人钩子刀，2026-09-14）

> 来源：TH18 原版 22 个 ECL 的迁移差距评估（spec `docs/superpowers/specs/2026-09-14-boss-phase-enemy-hooks-design.md` §1）。
> 只列本刀补上口子的那几条；其余 et*/move*/anm* 的逐条判定见该评估。

| ZUN（TH18） | 原版用量 | 我们 | 注意 |
|---|---|---|---|
| `setInterrupt(slot, hp, t, "sub")` + 读 `$TIMEOUT` | 81 / 154 | `phase_begin` 或 `spell_begin` + `wait_spell()` + `spell_result(slot)` | **结构不同**：ZUN 到线/到时抢占式跳进处理子程序，我们在主任务里顺序编排；ZUN 非符与紧跟的符卡共用一条血、段间 `lifeSet` 重灌，我们用一池总血 + 逐段递降血线 |
| 超时时 `life = hp_value` | — | 引擎自动：超时 `hp = min(hp, 血线)` | 迁移时删掉手写重灌 |
| `$CAPTURE` / `$MISS_COUNT` / `$BOMB_COUNT` 段首复位 | 81 / 68 / 68 | 引擎自动 | 原版只写不读，迁移时删 |
| `setInvuln(n)` | 39 | `set_invuln(n)` | — |
| `setHitbox(w,h)` / `setHurtbox(w,h)` | 111 / 111 | `set_hitbox(r)` / `set_hurtbox(r)` | 同名同义（hitbox 撞自机、hurtbox 挨打）；ZUN `w` 是直径还是半径待验 |
| `flagSet(2)` / `flagClear(2)` | — | `set_enemy_flag(ENEMY_NO_BODY, 1/0)` | 按用法推断的位义，未见 exe 实证 |
| `flagSet(32)` | 36 | `set_enemy_flag(ENEMY_KILLALL_EXEMPT, 1)` | exe 实证：bit5 不被 `enmKillAll` 清 |
| `enmKillAll()` | 108 | `kill_all_enemies(KILL_SILENT 或 KILL_DIE)` | ZUN 被杀者掉不掉落未核实，脚本自己选 |
| `$ID` | 1 | `$self_enemy` | 非敌读 -1 |
| `$I0–3` / `%F0–7` 生成继承 | ≈390 次生成 | `spawn_enemy(..., sub(实参…))` | 实参拷贝、非共享；ZUN 同一敌的 async 共享这 12 格，我们要共享走 globals 自由段 |
| `etCancel(640)` | 137 | `clear_bullets()` | 转星星 |
| `etClear(640)` | 150 | `clear_bullets_at(0.0fx, 224.0fx, 400.0fx, 0)` | 不转星星；超时收段引擎已自动这么清 |
| `etCancel(r)` 扩张消弹波 | 2 | 每帧 `clear_bullets_at(x, y, r, 1)` 加大 `r` | — |
| `movePosTime(0, 0, 0.0f, 0.0f)` 段首 | 常见 | `move_to(0, $self_x, $self_y, 0)` | **别照抄成 `move_to(0, 0.0fx, 0.0fx, 0)`**：原版是取消插值，我们的 dur=0 是瞬移 |

