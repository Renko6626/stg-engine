# 场景刀前置债务刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清掉 Godot 场景刀会直接踩到的五条债:B16②(boss_ui 结算不清)、D6(四字段写口收口)、
B18 可达部分(register_layer 校验换判据 + 冒烟判别大扩)、C17②③(save_state 对称化/hud_boss 走 view)、
D8 乙案(蓝图追认降级)+ 新债 A5 记档(enemy-owned 任务语言面缺口——场景刀设计输入)。

**Architecture:** 全部改动不触碰模拟演化路径——**金向量全程逐字节零平移**(每任务收尾断言,基线
`.superpowers/gameflow/golden-task2.txt`)。B16② 的清扫在金向量窗口内不可达(两段金向量零结算,
有测试明写);D6 是纯可见性收口+路由改道;桥/冒烟改动全在断层线以上。

**Tech Stack:** Rust 1.94.0 workspace;Godot 4.6.3 headless 冒烟(dummy renderer)。

## Global Constraints

- **金向量零平移**:每任务收尾 `cargo run -q -p stg-harness -- golden --out /tmp/g.txt && diff /tmp/g.txt .superpowers/gameflow/golden-task2.txt` 零差异。
- **P4-a/P4-b 口径**、I1-I7 全持有;FFI 无 panic 律(壳层错误 = no-op + false + **去重**日志)。
- **实验判决(已亲测,Godot 4.6.3 headless dummy renderer)**:`multimesh_get_instance_count` 恒 0、
  `allocate_data` 后 `get_buffer` 尺寸 0;**仅 `set_buffer` 之后 `get_buffer` 完整往返**
  (98304 浮点逐位)。`multimesh_get_visible_instances` 恒 0 → visible 数**不可** headless 断言,如实标注。
- 机械调整许可:类型/方法实名以仓库为准,披露;语义决策不许改。
- commit 尾附:`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 每任务 fmt/clippy(-D warnings)净。本刀**不改 builtins.rs**(无 gen-ecl-meta 重跑)。

---

### Task 1: B16② boss_ui 结算清扫(stg-core)

**Files:**
- Modify: `crates/stg-core/src/world.rs`(settle_one_spell,783-836)
- Test: 同文件测试区(仿既有符卡结算测试基建)

**Interfaces:** 无新 API。行为变更:符卡结算原子末尾同步清 `boss_ui[slot]`。

背景(探查钉死):结算只清 `spells[slot]`(world.rs:835),boss_ui 一字段不碰;次帧起
`settle_spells` 因 `spells[slot].active==0` 整槽跳过(spell.rs:88)——**无后续卡的场景
(rainbow 即是)boss_ui 永久冻结旧值**,不止"≤1 帧"。相位时序无竞态:清(帧 S 相 7)→
可能的新卡开(S+1 相 2)→ 喂(S+1 相 7),`continue` 分支与步骤 5 同帧互斥。

- [ ] **Step 1: 失败测试**(找到既有符卡结算测试——`settle_one_spell`/`hp_break`/超时路径在
  spell.rs 或 world.rs 测试区有基建,仿其构造:开卡→触发结算):

```rust
#[test]
fn spell_settle_clears_boss_ui_slot() {
    // 仿既有结算测试基建:boss_set 喂入非零 boss_ui + spell_begin_internal 开卡,
    // 跑到 hp_break 或超时结算;断言:
    // ① 结算发生的当帧之后,boss_ui[slot] == BossUiSlot::default()(全字段:active/spell_id
    //    /timer_frames/hp_ratio/phase_left/enemy 六项逐一断言,不许只看 active——判别式纪律);
    // ② 再多跑 3 帧(无新卡),boss_ui[slot] 仍 default(冻结回归:此前会永久保留旧值)。
    // 反向对照:结算前一帧 boss_ui[slot].active == 1(证明测试真走到了非零态,非圆心重合)。
}
```

- [ ] **Step 2: 确认红**(现实现不清 → ①②断言失败)。
- [ ] **Step 3: 实现**——`settle_one_spell` 末尾(world.rs:835 `self.spells[slot] = ...default()` 之后)加:

```rust
        // B16② 清扫(2026-07-25):结算原子同步清公告板——否则 spells 清零后整槽被
        // settle_spells 首行跳过,boss_ui 冻结旧值(无后续卡时无界陈旧)。次帧若新卡
        // 开(相2),相7 步骤5 自动喂真值,清→开→喂按相位全序串行,无竞态。
        self.boss_ui[slot] = crate::boss::BossUiSlot::default();
```

- [ ] **Step 4: 全绿 + 金向量零平移**(证据:两段金向量窗口零结算——场景一无 ECL,场景二
  time_limit 3600≫600 帧且 hp 打不穿,`rainbow_scene_reaches_steady_state` 断言 active 恒 1;
  该测试必须继续绿)。
- [ ] **Step 5: 提交**:`fix(core): 符卡结算同步清 boss_ui 公告板——修无后续卡时的无界陈旧(B16②)`

---

### Task 2: D6 收口——四字段 pub(crate) + WorldView 四读口 + 全迁移面(跨 crate)

**Files:**
- Modify: `crates/stg-core/src/world/view.rs`(四读口)
- Modify: `crates/stg-core/src/world.rs`(四字段可见性)
- Modify: `crates/stg-godot/src/bridge.rs:269`(hud_boss 改走 view——兼 C17③)
- Modify: `crates/stg-harness/src/viewer.rs`(51 生产 + 351-358 测试 fixture)
- Modify: `crates/stg-harness/src/main.rs:1160,1167-1168`(diag/boss_ui 直读)
- Modify: `crates/stg-ecl-compiler/src/{lib.rs,lang/codegen.rs}`(36 处 globals/diag 测试直读)

**Interfaces(Produces,T3 消费 boss_ui 口):**

```rust
// view.rs——照 spells()/bgm_id() 既有两款先例:
pub fn boss_ui(self) -> &'w [crate::boss::BossUiSlot] { &self.body.boss_ui }
pub fn globals(self) -> &'w [i32] { &self.body.globals }     // 只读切片,绕开 get_var 的 &mut+计数副作用
pub fn diag(self) -> crate::world::DiagCounters { self.body.diag }   // Copy ~52B 按值
pub fn last_status(self) -> u16 { self.body.last_status }
```

world.rs 四字段 `pub` → `pub(crate)`(globals:154 / boss_ui:158 / diag:194 / last_status:195;
`World.body` 保持 pub——封的是字段面)。

迁移面(探查清点 49 处,全机械):
- bridge.rs:269:`g.world.body.boss_ui.get(i as usize)` → `g.world.view().boss_ui().get(i as usize)`
  (view() 的实际构造口以代码为准,harness 用的是 `w.body.view()` 同款);
- viewer.rs:51(生产):迭代改 `w.body.view().boss_ui()`;351-358(fixture 直写)改用既有
  `boss_set(slot, BossUiSlot{..})` API(world.rs:681,纯测试没用它);
- harness main.rs:`.diag.task_faults` → `view().diag().task_faults`;`.boss_ui[0].active/hp_ratio`
  → `view().boss_ui()[0].…`;
- compiler 36 处:`w.body.globals[K]` → `w.body.view().globals()[K]`、`.diag.task_faults` 同款
  (全在 `#[cfg(test)]`,`let w` 不用变 mut——只读口的红利)。

- [ ] **Step 1**: view.rs 四读口 + 各自单测(仿 `frame_and_frame_events_and_tasks_read_accessors`
  先例:非零值经读口可见——boss_set 喂一槽读回/set_var 后 globals 切片读回/制造一次
  contract_viol 后 diag().contract_viol==1 且 last_status()==STATUS_BAD_ARGS)。
- [ ] **Step 2**: 可见性收紧 → `cargo build --workspace` 列出全部破点,逐一机械迁移(上表);
  编译通过 = 迁移完备(可见性收紧的优点:漏一处都编不过,无静默残留)。
- [ ] **Step 3**: `cargo test --workspace` 全绿 + 金向量零平移(纯路由,行为不变)。
- [ ] **Step 4**: 提交:`refactor(core): D6 收口——globals/boss_ui/diag/last_status 收 pub(crate)+WorldView 四读口,49 处直读全迁`

---

### Task 3: 桥壳收敛(register_layer 判据 + 去重位 + save_state 对称化)

**Files:**
- Modify: `crates/stg-godot/src/bridge.rs`(register_layer:168-185 / save_state:209-215 / warned 位)
- Modify: `crates/stg-godot/src/frame.rs`(补 LAYER_SHOTS 非空判别纯 Rust 测试)

**Interfaces:** `register_layer` 校验语义变更(冻结面行为微调,记 T5 文档):
`multimesh_get_instance_count(rid) == cap` → **`multimesh_get_buffer(rid).len() == cap × FLOATS_PER_INSTANCE`**。

理由(实验判决 + 生产语义双赢):①headless dummy 下 instance_count 恒 0,合法注册被拒,
上传链永远无法冒烟;buffer 尺寸判据在冒烟侧可用 `set_buffer` 播种定长零缓冲过闸;
②instance_count 判不出格式错位(3D 格式 multimesh 数量对但 stride 全错),buffer 尺寸
直接锁死壳所依赖的 12 float/实例布局,**更严了而不是更松**;③get_buffer 在真渲染器上
是一次性注册期读回,成本可忽略。

- [ ] **Step 1**: 改校验 + 加去重位。现状:坏 kind 走 `warn_once(W_BAD_LAYER)`,坏 RID/尺寸
  走裸 `godot_error!` **不去重**(与文件头 FFI 律不一致——探查发现)。修法:新增 `W_BAD_MM`
  位,尺寸/坏 RID 路径改 `self.warn_once(W_BAD_MM, "register_layer:multimesh 缓冲尺寸不符(需 cap×12;坏 RID 同途),no-op")`;
  首个错误细节(期望/实际)在 warn_once 消息里给不了动态值的,以静态消息 + 返回 false 为准
  (与 W_BAD_LAYER 同款口径)。

```rust
        let got = rs.multimesh_get_buffer(multimesh_rid).len();
        let need = cap * FLOATS_PER_INSTANCE;
        if got != need {
            self.warn_once(W_BAD_MM, "register_layer:multimesh 缓冲尺寸不符(需 cap×12,含坏 RID/未播种;headless 下须先 set_buffer 播种),no-op");
            return false;
        }
```

- [ ] **Step 2**: `save_state` 对称化:`&self` → `&mut self`,None 分支
  `self.warn_once(W_NO_GAME, "save_state:尚未 new_game,返回空"); PackedByteArray::new()`
  ——与 load_state 同口径(C17② 销账)。
- [ ] **Step 3**: frame.rs 补 `LAYER_SHOTS` 非空判别测试(探查:四判别测试唯独 shots 只有
  空池 0 断言——圆心重合纪律缺口):纯 Rust 造世界,喂 `BTN_SHOT` 步进到 `shots` 池非空
  (shottype 表驱动发弹,player 相 1/3),`encode_layer(LAYER_SHOTS)` 断言 n>0 且实例 0 的
  ox/oy 与池内首活自机弹坐标一致(除 65536)、xx==1.0。
- [ ] **Step 4**: `cargo test -p stg-godot` 全绿 + 金向量零平移 + 提交:
  `fix(godot): register_layer 换 buffer 尺寸判据(headless 可测+锁 stride)+W_BAD_MM 去重+save_state 对称化`

---

### Task 4: 冒烟大扩(B18 可达面全补)

**Files:**
- Modify: `crates/stg-godot/smoke/godot_smoke.ecl`
- Modify: `crates/stg-godot/smoke/smoke.gd`

**场景扩(godot_smoke.ecl)**——现 14 行基础上,`main()` 顶层、`mark(9)` **之前**加三行
(全部无 owner 限制,探查核实;签名以 ecl-lang.md 生成段为准):

```
bg(2);
bg_phase(1);
boss_set(0, 1.0fx, 7, 3600, 2, 1);
```

注意连锁:`mark(9)` 的自动补偿会把最近 `bgm/bg/bg_phase` 常量声明注入垫片 → 中段启动分支
的 anchors 断言从 `bg==0/bg_phase==0` **改为 `bg==2/bg_phase==1`**(正常路径同值)——断言
与摆位逻辑必须一起改,注释写明推导。

**smoke.gd 追加断言(逐条,值全判别非默认):**

1. **register_layer 三路**(用 LAYER_ENEMIES,cap=256,播种 256×12=3072 浮点,便宜):

```gdscript
# 坏 kind
if b.register_layer(99, RenderingServer.multimesh_create()): fail("rl bad kind"); return
# 坏尺寸(播种 100×12 ≠ 256×12)
var bad := RenderingServer.multimesh_create()
var seed_bad := PackedFloat32Array(); seed_bad.resize(100 * 12)
RenderingServer.multimesh_set_buffer(bad, seed_bad)
if b.register_layer(b.LAYER_ENEMIES, bad): fail("rl bad size"); return
# 合法(allocate 走生产同款调用 + set_buffer 播种过 headless 闸)
var mm := RenderingServer.multimesh_create()
RenderingServer.multimesh_allocate_data(mm, 256, RenderingServer.MULTIMESH_TRANSFORM_2D, false, true)
var seed_ok := PackedFloat32Array(); seed_ok.resize(256 * 12)
RenderingServer.multimesh_set_buffer(mm, seed_ok)
if not b.register_layer(b.LAYER_ENEMIES, mm): fail("rl ok path"); return
```

2. **编码→上传链回读判别**(步进 2 帧后——静止敌在 (0,-160),spawn_enemy 出生帧不跑第 2 步落池):

```gdscript
var back := RenderingServer.multimesh_get_buffer(mm)
# 实例 0 = 唯一敌:布局 [xx,yx,0,ox, xy,yy,0,oy, custom×4](桥刀实证 12 float)
if absf(back[0] - 1.0) > 0.0001: fail("mm xx"); return      # 无旋转 cos=1,非零判别
if absf(back[7] - (-160.0)) > 0.0001: fail("mm oy"); return  # 出生 y,非默认判别
```

3. **hud_boss 判别**(boss_set 于第 2 步执行后):`active==1 && spell_id==7 && timer_frames==3600 && phase_left==2`,`hp_ratio` 近 1.0。
4. **player_pos 判别**:`absf(p.y - 384.0) < 0.0001`(出生点非零)。
5. **hud_player 加两断言**:`lives==3 && bombs==3`(正常路径分支)。
6. **anchors 三值**:`bgm==3 && bg==2 && bg_phase==1`(两分支都断言;`bg_phase_frame` 断言
  等于声明发生帧——按摆位算出精确值写死并注释推导)。
7. 中段启动分支既有断言按上文连锁修正。

- [ ] **Step 1**: 改 `.ecl` → `cargo run -p stg-harness -- check crates/stg-godot/smoke/godot_smoke.ecl` OK。
- [ ] **Step 2**: 改 smoke.gd → `bash crates/stg-godot/smoke/run-smoke.sh` → `SMOKE OK`(先跑必红
  ——新断言在旧壳上不成立的路数各自验证一次红,报告记录)。
- [ ] **Step 3**: 全绿 + 金向量零平移 + 提交:
  `test(godot): 冒烟大扩——register_layer 三路+上传链 get_buffer 回读判别+hud_boss/player_pos/anchors/hud_player 非默认断言(B18 可达面清账)`

---

### Task 5: 蓝图/账本收口(D8 乙 + follow-ups + 陈旧注释)

**Files:**
- Modify: `stg-world-design.md`(205/660 两处;38 行通则不动)
- Modify: `crates/stg-core/src/ecl/syscall.rs`(spawn_enemy 注释提及不存在的 `spawn_task`)
- Modify: `docs/follow-ups.md`、`PROGRESS.md`

- [ ] **Step 1: D8 乙案落笔**(用户拍板 2026-07-25,评审记录写进改动处):
  - 205 行句尾:"…故 `hits` 容量按最坏情况给足(全弹同帧入擦圈);溢出时确定性丢弃 + 计数
    (`diag.hits_overflow`),debug/release 行为一致,不 panic(同 P4-a 资源耗尽铁律。勘注
    2026-07-25:行 1/2 同弹双计 × 双自机的理论最坏为 4×CAP,超额由降级语义覆盖——真实弹幕
    不可达;原"debug panic"例外与 P4-a 自相矛盾,经评审废止,实现从未采纳)。"
  - 660 行:`| （内部）hits 满 | — | 丢弃（不 panic，同 P4-a） | — | \`diag.hits_overflow\` |`
    (顺手把幽灵字段名 `hits_dropped` 对齐实名)。
- [ ] **Step 2: 陈旧注释修**:syscall.rs `spawn_enemy` 附近"脚本层自行事后 spawn_task 绑定"
  改为实情:"敌任务绑定现仅 Rust host API(`spawn_entry`/`spawn_entry_named` 携
  `EclOwner::Enemy`)可达;脚本面缺口见 follow-ups A5"。
- [ ] **Step 3: follow-ups 对账**:
  - **新 A5**(本刀最重要产出之一):"脚本面无法产生 enemy-owned 任务——`spell_begin`
    (syscall.rs:707 `self_enemy_handle` 门禁)/`move_enemy_to`/弹 setter 族全被 OWNER 校验
    挡死,而语言无任何 builtin 能造 enemy-owned 任务(`spawn` 继承父 owner;金向量 boss 是
    harness 用 Rust `spawn_entry(..., EclOwner::Enemy)` 手摆的冻结遗产)。**后果:纯 .ecl 走
    正典 boot 摆不出 boss/符卡/符卡清弹 field。触发点 = Godot 场景刀设计期必须先裁**
    (语言级 builtin 如 `spawn_for(enemy, sub)` / boss 绑定糖 vs boot 面扩口),它决定场景刀
    的 boss 关卡怎么写。"
  - **B16 条目**:②子项销(留①③④,改措辞);
  - **D6 条目**:删(收口完成;`TaskPool Default 可外构空池`知会句挪入 D7 尾注保存);
  - **D8 条目**:删(乙案落地);
  - **C17 条目**:②③子项销,留①④;
  - **B18 条目**:改写为余量:"`hud_spell`/`fields_info` 非默认判别断言被 A5 阻塞(spell_begin
    不可达 → field 唯一生产路径同不可达);`visible_instances` dummy renderer 恒 0 不可
    headless 断言——三项触发点 = A5 解锁后/场景刀真渲染。其余(四读口其二/register_layer
    三路/编码上传链回读/LAYER_SHOTS 判别)已于 2026-07-25 前置债务刀清账。"
- [ ] **Step 4: PROGRESS**:史行"| 2026-07-25 | 前置债务刀 | boss_ui 结算清扫(B16②)+D6 四字段
  收口(49 处迁移)+register_layer 换 buffer 判据+冒烟大扩(B18 可达面)+D8 乙案+新 A5(enemy-owned
  任务语言缺口=场景刀设计输入) |";「现在」段待办句同步(A5 置顶)。
- [ ] **Step 5**: `cargo test --workspace` 全绿 + 金向量零平移 + 提交:
  `docs: 前置债务刀收口——D8 乙案+A5 新债+B16②/D6/C17②③/B18 对账+spawn_task 幽灵注释修正`

---

## Self-Review 备忘(计划作者已核)

- 覆盖:B16②→T1;D6(全 49 处)→T2;B18 可达面(判据/三路/回读/shots 判别)→T3+T4;
  C17②③→T3+T2;D8 乙→T5;A5 新债/注释修→T5。递延项全部在 T5 的 B18 改写里显式落账。
- 顺序依赖:T3 的 hud_boss 不动(T2 已迁);T4 依赖 T3 的新判据(播种路径才能过闸);
  T2 可见性收紧靠编译器穷举破点,无静默漏迁风险。
- 金向量零平移证据逐任务给出(T1 窗口不可达/T2 纯路由/T3T4 断层线上/T5 纯文档)。
