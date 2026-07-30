# ECL 复刻刀（D9 / B19 / B20）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐三项挡住真实关卡内容的 ECL 能力：敌主协程返回即自燃（D9）、清弹 builtin（B19）、三个账面增量 setter（B20）。

**Architecture:** 三项都尽量**零新机制**——D9 挂在 `run_tasks` 既有的任务终止分支上；B19 复用现成的 `FieldPool` 消弹区（连"消弹转星星"都是白送的）；B20 照 `add_score` 的 5x 族口径原样加三个。

**Tech Stack:** Rust 1.94.0（edition 2024）/ `stg-core`（断层线以下，纯整数）/ `stg-ecl-compiler`（表层语言）

**背景：** 这三条同属「需要复刻的 ECL 功能」类——它们不是内部质量债，是**挡着写真实关卡**的功能缺口。D9 尤其：`stg-world-design.md` 与两处代码注释都承诺了"敌主协程返回即自燃是 ZUN ECL 语义"，但全仓没有任何代码路径实现它，demo 杂兵只能靠把退场点设到越界线外（`y=760`）来假装消失。

## Global Constraints

- **I1–I7**：`stg-core` 内不得出现 `f32`/`f64`、系统时钟、宿主 RNG、`HashMap`/`HashSet`。
- **P4 三铁律**：新增的降级路径按 (a) 资源耗尽→确定性降级不 panic / (b) 调用方违约→安全结果+计数 处置。
- **两条人类裁定**（不得偏离）：
  - **B19 复用 `FieldPool`**，消弹转星星（现成行为，别绕开）。不做护盾帧——那是 bomb 那刀的职责。
  - **B20 取增量形态 `add_*`**，不做 `set_*`（绝对赋值的唯一确定场景已被 `Loadout` 收编）。
- **syscall 号表是冻结契约**（`syscall.rs` 模块文档："编号即契约；冻结纪律同 op 表"）。本刀 append-only 占 **54/55/56/57**，不动既有号。
- **`ENGINE_VER` 2 → 3**：新增 syscall 号 = 号表变更，与 T7 新增 xform op 时 bump 的口径一致（新编译器产出的镜像跑不了旧引擎，`ENGINE_VER` 正是拦这个的）。
- **金向量不得漂移**：`rainbow.ecl` 的两个敌任务（`patrol`/`windchime_pattern`）都是 `loop{}` 永不返回，D9 对它无影响；B19/B20 是纯新增、不被现有脚本调用。**每个任务结束前跑一次 `golden` 与 base 对拍，必须逐字节相同**——这是本刀"零行为漂移"的硬证据。
- 每个任务结束前必须全绿：
  ```bash
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- **commit 结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **分支**：`feat/ecl-parity`。

## File Structure

| 文件 | 改什么 | 任务 |
|---|---|---|
| `crates/stg-core/src/ecl/vm.rs` | `run_tasks` 终止分支挂自燃 + 别名防护 | T1 |
| `crates/stg-core/src/enemy.rs` | `main_task` doc 改口径（**它从此有消费者了**） | T1 |
| `crates/stg-core/src/world.rs` `world/cleanup.rs` | 两处"承诺"注释改成事实陈述 | T1 |
| `godot/ecl/demo/stage1.ecl` | 杂兵退场点改回场内（不再靠越界兜底） | T1 |
| `crates/stg-core/src/ecl/syscall.rs` | `SYS_CLEAR_BULLETS`(54) + 三个 `SYS_ADD_*`(55/56/57) | T2 / T3 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | 四个表层内建 | T2 / T3 |
| `crates/stg-core/src/lib.rs` | `ENGINE_VER` 2→3 | T2 |
| `docs/*` `PROGRESS.md` | 销三条 + 文档收口 | T4 |

---

### Task 1: D9 敌主协程返回即自燃

**Files:**
- Modify: `crates/stg-core/src/ecl/vm.rs`（`run_tasks` + 测试）
- Modify: `crates/stg-core/src/enemy.rs`（`main_task` doc）
- Modify: `crates/stg-core/src/world.rs:106`、`crates/stg-core/src/world/cleanup.rs:6`（注释）
- Modify: `godot/ecl/demo/stage1.ecl`

**Interfaces:**
- Consumes: `EnemyPool.main_task`（存的是**任务槽号 + 1**，0 = 无；`syscall.rs:642` 回填）、
  `ENEMY_DYING`（`enemy.rs`）、`run_tasks` 的三条终止路径
- Produces: 敌主任务自然 `End` → owner 敌被标 `ENEMY_DYING`（相位 9 cleanup 回收）

- [ ] **Step 1: 读清三条终止路径与别名风险**

`run_tasks`（`vm.rs:427`）里任务终止有多处：
- owner 存活门禁 → `tasks.kill(i); continue;`（owner 已死，本条与我们无关）
- `spell_bound` 门禁 → 同上
- 脚本号不在册 → `kill` + fault 事件
- `exec` 返回 `Exec::End` → `kill`（**这就是"自然返回"，我们要挂的点**）
- `exec` 返回 `Exec::Fault(code)` → `kill` + fault 事件

**别名风险（必须防）**：`main_task[e]` 存的是槽号+1、**不带 generation**。场景：敌 E 的主任务（槽 5）因 `Fault` 被杀 → E 仍活着 → E 后来 spawn 的子任务恰好落进槽 5（`first_free()` 取最低空位）→ 该子任务自然结束 → 若只比 `main_task[E] == 5+1` 就会**误杀 E**。

**防法**：在**任意**一条终止路径上，若 `i` 恰是 owner 敌记的主任务槽，就把 `main_task` 清零；自燃只在 `Exec::End` 这一条上触发。清零后槽 5 再被复用也不会匹配。

- [ ] **Step 2: 写失败测试（四条判别腿）**

在 `vm.rs` 的 `mod tests` 加。**判别力要点**：自燃是"静默退场"，不是"被击破"——必须断言**不掉道具、不加分、不发 `EVT_ENEMY_DIED`**，否则实现者照抄 `damage_enemy` 也能让第一条绿。

```rust
    /// D9：敌主协程自然返回 → owner 敌被标 ENEMY_DYING（相位 9 回收）。
    /// **静默退场**：不掉道具、不加分、不发 EVT_ENEMY_DIED——与伤害致死路径的判别腿。
    #[test]
    fn enemy_main_task_returning_self_destructs_quietly() {
        // 镜像：一个立刻 END 的零参 Async sub 当敌主任务
        // 敌用 drop_table=1（非空表：内建表 1 号有 POWER×2 + POINT×1）+ score=100,
        // 这样"若走了 damage_enemy 路径"会立刻显形（掉 3 颗道具、加 100 分、发事件）。
        // ...构造照 vm.rs 既有测试的 test_world()/async_image() 惯例...
        // 断言：
        //   enemies.flags[e] & ENEMY_DYING != 0      —— 自燃了
        //   items.iter_alive().count() == 0          —— 不掉道具
        //   players[0].score == score_before         —— 不加分
        //   frame_events 里无 EVT_ENEMY_DIED         —— 不发死亡事件
        //   tasks.is_alive(slot) == false            —— 任务本身也没了
    }

    /// 判别腿：**非** main_task 的敌属任务结束 → 敌不死（否则 fire 的伴生任务一结束敌就没了）。
    #[test]
    fn non_main_enemy_task_ending_does_not_self_destruct() { /* ... */ }

    /// 判别腿：Fault 结束 **不**自燃——只有自然返回才是"跑完了"，报错是另一回事
    /// （且已有 fault 事件 + 计数）。
    #[test]
    fn enemy_main_task_faulting_does_not_self_destruct() { /* ... */ }

    /// 别名防护：主任务 Fault 后 main_task 清零 → 同槽复用的子任务结束不误杀 owner。
    #[test]
    fn main_task_slot_reuse_does_not_spuriously_self_destruct() { /* ... */ }
```

> 四条的世界/镜像构造**照抄 `vm.rs` 既有测试**（`test_world()`、`async_image()`、
> `spawn_enemy` 等）。构造不出"同槽复用"的第四条时，可用 `pub(crate)` 直写
> `main_task` 造出该状态——本模块测试可直接摸池字段（既有测试有先例）。

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core self_destruct`
Expected: 第一条 FAIL（敌没被标 DYING——这正是 D9 的缺口）。

- [ ] **Step 4: 实现**

`vm.rs::run_tasks`，把三条 `kill` 路径收敛成"先算 is_main、再按路径处置"。建议形态：

```rust
// D9：敌主协程返回即自燃（ZUN ECL 语义）。`main_task` 存槽号+1、不带 generation，
// 故**任意**终止路径都要清零——否则该槽被同 owner 的子任务复用后，子任务结束会误杀 owner。
// 自燃只在自然 `End` 上触发：Fault 是错误路径（已有 fault 事件），不该顺手把敌收走。
let is_main = t.owner_kind == OWNER_ENEMY
    && body.enemies.main_task[t.owner_index as usize] == i as u32 + 1;
```

`Exec::End` 分支：

```rust
            Exec::End => {
                tasks.slots[i] = t;
                tasks.kill(i);
                if is_main {
                    let e = t.owner_index as usize;
                    body.enemies.main_task[e] = 0;
                    // **静默退场**，不走 damage_enemy：脚本跑完是"退场"不是"被击破"——
                    // 不掉道具、不加分、不发 EVT_ENEMY_DIED（ZUN 口径）。相位 9 cleanup
                    // 见 ENEMY_DYING 即回收。
                    body.enemies.flags[e] |= crate::enemy::ENEMY_DYING;
                }
            }
```

`Exec::Fault` 分支只清零、不自燃。owner 门禁与 spell 门禁两条路径上 owner 已死/槽已换，清零可做可不做——**若做，注意那两条 `continue` 在 `is_main` 计算之后**。

- [ ] **Step 5: 跑测试确认四条全过**

Run: `cargo test -p stg-core -- self_destruct main_task_slot_reuse`

- [ ] **Step 6: 三处注释改口径**

- `crates/stg-core/src/enemy.rs` 的 `main_task` doc：**它从此有消费者了**。上一刀（B25）我写的"全仓目前无消费者读它"现在是错的，必须改：写明 `run_tasks` 的 D9 自燃判据会读它，且它存的是槽号+1、终止时清零。
- `crates/stg-core/src/world.rs:106` 与 `world/cleanup.rs:6`：两处"M1 起敌人主协程返回即自燃"从**承诺**改成**事实**，并指到实现位置（`vm.rs::run_tasks` 的 `Exec::End` 分支）。

- [ ] **Step 7: demo 杂兵改回场内退场**

`godot/ecl/demo/stage1.ecl` 的 `zako_dive` 现在把退场目标设到 `y=760`（超过
`FIELD_HEIGHT + ENEMY_OOB_MARGIN = 704` 的回收线）来假装消失。D9 落地后不需要这个技巧了
——改成一个**场内**的合理退场点（比如飞到 `y≈500` 附近，仍在越界线内），靠任务跑完自燃收尾。
这同时是 D9 的**端到端证明**：改完跑真工程冒烟，杂兵仍应正常出现与消失。

改的时候把原来那句"靠越界兜底"的注释一并改掉（它记录的是 workaround，现在不需要了）。

- [ ] **Step 8: 全绿 + 金向量对拍 + 冒烟**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
# 金向量零漂移（rainbow.ecl 的敌任务都是 loop{} 永不返回，D9 对它应无影响）
git stash && cargo run -q -p stg-harness -- golden --out /tmp/g-base.txt && git stash pop
cargo run -q -p stg-harness -- golden --out /tmp/g-head.txt && diff -q /tmp/g-base.txt /tmp/g-head.txt && echo "GOLDEN 逐字节相同 ✔"
bash godot/smoke/run-smoke.sh
```

> `git stash`/`stash pop` 那两步是为了拿到改动前的 golden。若嫌绕，也可用
> `git worktree add /tmp/base-wt HEAD` 在干净副本里跑 base 再对拍——**别在主 checkout 上
> `git checkout` 文件**（上一刀吃过亏：`git checkout` 清临时代码时把实现一起撤了）。

- [ ] **Step 9: 提交**

```bash
git add crates/stg-core/src/ecl/vm.rs crates/stg-core/src/enemy.rs \
        crates/stg-core/src/world.rs crates/stg-core/src/world/cleanup.rs godot/ecl/demo/stage1.ecl
git commit -m "$(cat <<'EOF'
feat(ecl): 敌主协程返回即自燃(D9)——把注释里的承诺变成代码

stg-world-design 与两处代码注释从 M1 起就承诺"敌主协程返回即自燃是 ZUN ECL 语义",
但全仓没有任何路径实现它:唯一置 ENEMY_DYING 的地方是伤害血线。demo 杂兵只能靠把
退场点设到越界线外(y=760)假装消失,原地驻守型敌根本没法写。

挂在 run_tasks 的 Exec::End 上,**静默退场**:不走 damage_enemy,故不掉道具、不加分、
不发 EVT_ENEMY_DIED(脚本跑完是"退场"不是"被击破")。Fault 不自燃(错误路径另有 fault
事件)。别名防护:main_task 存槽号+1 不带 generation,故任意终止路径都清零——否则该槽
被同 owner 的子任务复用后,子任务结束会误杀 owner。

demo 杂兵退场点改回场内,靠自燃收尾(D9 的端到端证明)。金向量逐字节不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: B19 `clear_bullets()` + `ENGINE_VER` bump

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`SYS_CLEAR_BULLETS = 54` + 实现 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（表层 `clear_bullets`）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 2 → 3）

**Interfaces:**
- Consumes: `WorldBody::create_field`、`FieldInit`、`FIELD_CLEAR_BULLETS`、
  `FIELD_RADIUS_FULLSCREEN`（`field.rs:21`，值 400，**注释里写明就是给全场消弹算好的**）
- Produces: `SYS_CLEAR_BULLETS: u16 = 54`；表层 `clear_bullets()`（0 参、无返回）

- [ ] **Step 1: 写失败测试**

在 `syscall.rs` 的 `mod tests`：

```rust
    /// B19：`clear_bullets()` 铺一个覆盖全场、存活 1 帧的消弹区。
    /// **消弹转星星是白送的**（M0-15：settle 趟一对每颗被消的弹原位转一颗星星）,
    /// 故断言"弹没了"**和**"星星出现了"——后者是"真走了 FieldPool 那条路"的判别腿
    /// （若实现者绕开 field、自己写个循环把弹 free 掉，星星那条立刻红）。
    #[test]
    fn clear_bullets_lays_fullscreen_field_and_converts_to_stars() {
        // 造几颗散在场内不同位置的敌弹 → call(SYS_CLEAR_BULLETS) →
        // 手动跑 collide + settle（照本模块既有测试的相位驱动惯例）→
        //   bullets 全部带 BULLET_CLEARED / cleanup 后为 0
        //   items 里出现同等数量的星星（ITEM_STAR）
        //   frame_events 里有 EVT_FIELD_CLEARED，data[0] == 弹数
    }

    /// P4-a：field 池满 → 确定性降级（不 panic、不 Fault，计 pool_full[POOL_FIELD]）。
    #[test]
    fn clear_bullets_field_pool_full_degrades() { /* 先把 FieldPool 灌满再调 */ }
```

- [ ] **Step 2: 跑测试确认失败**（`SYS_CLEAR_BULLETS` 不存在）

- [ ] **Step 3: 实现 syscall**

`syscall.rs` 号表 5x 族尾部（50-53 已用）：

```rust
/// 全场清弹（B19；0 参、无返回）。铺一个覆盖全场、`life=1` 的 `FIELD_CLEAR_BULLETS`
/// 作用区——**复用现成的消弹区机制**，故"每颗被消的弹原位转一颗星星"（M0-15）与
/// `EVT_FIELD_CLEARED` 都是白送的，引擎侧零新机制。
///
/// 关底转场（`REQ_STAGE_CLEAR` 挂牌前）是首个真实消费者。**不给护盾帧**——那是 bomb
/// 那刀的职责（bomb = `FIELD_CLEAR_BULLETS | FIELD_DAMAGE` + 自机无敌）。
/// P4-a：field 池满 → `create_field` 自身的降级（NULL + 计数），本 syscall 不 Fault。
pub const SYS_CLEAR_BULLETS: u16 = 54;
```

派发分支：置场心 `(0, FIELD_HEIGHT/2)`、半径 `FIELD_RADIUS_FULLSCREEN`、`life: 1`、
`flags: FIELD_CLEAR_BULLETS`、`dmg_per_frame: 0`。`FieldInit` 是 exhaustive（无 `Default`），
字段要写齐——**确切字段名以 `field.rs` 的 `define_pool!` 为准**。

- [ ] **Step 4: 表层内建**

`builtins.rs` 照 `add_score` 的形状加（`params: &[]`、`ret: None`），并更新同文件里那两处
穷举名单断言（`BUILTINS.len()` 与"应无返回值"名单）。

- [ ] **Step 5: `ENGINE_VER` 2 → 3**

`crates/stg-core/src/lib.rs`。理由写进该行上方注释：**syscall 号表变更**——`syscall.rs`
模块文档明写"编号即契约；冻结纪律同 op 表"，而 T7 新增 xform op 时同样 bump 过，口径一致。
`step.rs` 有 `engine_ver_anchored` 一类的锚点测试，同步更新。

- [ ] **Step 6: 全绿 + 金向量对拍**

同 T1 Step 8 的做法（纯新增 syscall，无脚本调用它，golden 必须逐字节不变）。

- [ ] **Step 7: 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): clear_bullets() 全场清弹(B19) + ENGINE_VER 2→3

复用现成 FieldPool 消弹区:铺一个覆盖全场、life=1 的 FIELD_CLEAR_BULLETS 区,
"每颗被消的弹原位转一颗星星"(M0-15)与 EVT_FIELD_CLEARED 都是白送的,引擎侧零新机制。
人类裁定:不做护盾帧——那是 bomb 那刀的职责。

判别腿断言"星星出现了"而不只是"弹没了"——绕开 field 自己写循环 free 弹的实现会红。

syscall 号表变更 → 按 T7 新增 op 的同款口径 bump ENGINE_VER。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: B20 三个账面增量 setter

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`SYS_ADD_LIVES/BOMBS/POWER` = 55/56/57 + 实现 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`

**Interfaces:**
- Consumes: `players[0].lives`/`bombs`（`u8`）、`power`（`u16`，上限 `crate::items::POWER_MAX = 400`）
- Produces: 三个 syscall（各 1 参 `delta`、无返回），表层 `add_lives`/`add_bombs`/`add_power`

- [ ] **Step 1: 写失败测试**

**人类裁定取增量形态**（`add_*` 而非 `set_*`）：绝对赋值的唯一确定场景（开局装备）已被
`Loadout` 收编。三条测试，每条覆盖**正增 / 负减 / 上钳 / 下钳**四腿：

```rust
    /// B20：`add_lives(d)` 增量记账，双边钳位（P4-b），不回绕不 panic。
    #[test]
    fn add_lives_clamps_both_ends() {
        // +1 正常增；-1 正常减；从 0 再 -1 → 仍 0（下钳）；
        // 从 u8::MAX 再 +1 → 仍 u8::MAX（上钳）
    }
    #[test]
    fn add_bombs_clamps_both_ends() { /* 同构 */ }
    /// `power` 上限是 POWER_MAX(400，= 显示 4.00)，不是 u16::MAX——判别腿。
    #[test]
    fn add_power_clamps_to_power_max_not_u16_max() { /* 从 POWER_MAX 再 +1 → 仍 400 */ }
```

- [ ] **Step 2: 跑测试确认失败** → **Step 3: 实现**

派发分支照 `SYS_ADD_SCORE`（`syscall.rs:329`）的形状。钳位用 `i32` 中间量再钳回：

```rust
        SYS_ADD_LIVES => {
            let d = pop(task)?;
            let p = &mut ctx.body.players[0];
            p.lives = (p.lives as i32 + d).clamp(0, u8::MAX as i32) as u8;
            Ok(())
        }
```

`power` 的上钳是 `crate::items::POWER_MAX`（400），**不是 `u16::MAX`**。

三条号表注释里都写明：**增量形态是人类裁定**——绝对赋值场景已被 `Loadout` 收编，
故不开 `set_*`（记录裁定，防将来有人"补全"成四件套）。

- [ ] **Step 4: 表层内建 ×3** + 更新 `builtins.rs` 的两处穷举名单断言

- [ ] **Step 5: 全绿 + 金向量对拍 + 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): add_lives/add_bombs/add_power 三个账面增量 setter(B20)

人类裁定取**增量**形态:绝对赋值的唯一确定场景(开局装备)已被 Loadout 收编,
故不开 set_*(裁定写进号表注释,防将来被"补全"成四件套)。与 add_score 同族口径:
1 参、无返回、双边钳位(P4-b)。power 上钳 POWER_MAX(400) 而非 u16::MAX。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 文档与销账

**Files:**
- Modify: `docs/follow-ups.md`（销 D9 / B19 / B20 三条）
- Modify: `docs/ecl-lang.md`（生成段重跑 + 手写节补语义）
- Modify: `docs/ecl-ops.md`（syscall 号表追四行）
- Modify: `PROGRESS.md`

- [ ] **Step 1: 重跑生成器**

```bash
cargo run -p stg-harness -- gen-ecl-meta
```
四个新内建的签名会自动进 `ecl-meta.json` / VS Code 扩展 / `ecl-lang.md` 的生成段。

- [ ] **Step 2: `ecl-lang.md` 手写节**

生成段只有签名，**语义要手写**：

- `clear_bullets()`：清的是**敌弹**，每颗原位转一颗星星（不是白清）；关底转场用；
  不给无敌帧。铺的是 1 帧的全场消弹区，故**当帧生效**。
- `add_lives`/`add_bombs`/`add_power`：增量、双边钳位；`power` 单位是厘火力（100 = 1.00）；
  **没有 `set_*`**，开局装备走菜单侧的 `Loadout`。
- **D9 要单独写一段**（这是脚本作者最该知道的行为变化）：敌的主任务（`spawn_enemy` 的
  `task` 参那个）**跑完就等于敌退场**——静默消失，不掉道具不加分。要让敌留着就 `loop`，
  要它退场就让主任务自然结束。这条改变了"敌任务写法"的默认心智，**必须**进手册。

> `ecl-lang.md` 的围栏示例是**真编译**的（harness 有 `every_ecl_fenced_example_in_doc_compiles`），
> 你写的任何 `.ecl` 例子都要能过。

- [ ] **Step 3: `docs/ecl-ops.md`** syscall 号表追 54/55/56/57 四行。

- [ ] **Step 4: 销三条**

`docs/follow-ups.md` 的规矩是「解决一条就删一条」——D9 / B19 / B20 **整条删除**。
**删前逐条核实**（本仓规矩：写之前先核实）：
- D9 → 自燃实现在？四条测试在？三处注释改口径了？demo 杂兵改回场内了？
- B19 → syscall + 内建 + 两条测试在？
- B20 → 三个 syscall + 三个内建 + 三条测试在？

顺带更新文件头「最后核实」行。**若某条没做全，不要删它**，报告里说明。

- [ ] **Step 5: `PROGRESS.md`** 史加一行 + 「现在」段重写（≤10 行，保留 B26 余量那条现状）。

- [ ] **Step 6: 全绿收口**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

---

## 自审记录

- **覆盖**：D9（T1）/ B19（T2）/ B20（T3）/ 销账（T4）。
- **两条人类裁定**已写进 Global Constraints 与对应任务：B19 复用 FieldPool + 不做护盾帧；
  B20 取 `add_*` 不做 `set_*`。
- **本刀的判别力策略**：与上一刀（P4 覆盖刀，测已存在实现）不同——**这三条测的都是新行为**，
  所以走标准 TDD（先红后绿）即可，不需要变异检验。但 D9 的第一条与 B19 的第一条各埋了一个
  **反向判别腿**（"不掉道具/不加分/不发事件"、"星星要出现"），防止实现者用错误的路径
  （抄 `damage_enemy` / 绕开 FieldPool）也能让主断言变绿。
- **金向量零漂移**是本刀的硬不变量，每个任务都要对拍——D9 改的是模拟行为，若 golden 变了
  说明它意外影响了现有场景（`rainbow.ecl` 的敌任务应当都是 `loop{}` 永不返回），要查清原因
  而不是直接接受新流。
