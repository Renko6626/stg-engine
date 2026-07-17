# M0-14 create_bullets_batch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** N×K 网格批量发射器——环（角度插值）/列（速度插值）/多重环（双轴）一个原语通吃，ECL 密集符卡的性能语义基建。

**Architecture:** `world.rs` 写 API 区新增 `create_bullets_batch`（前置一次验证 + 两轴累加器 + 逐颗内联先段后弹 + 满额短路批量计数）；xform 验证两遍走格从 `create_bullet_with_xform` 抽为共用静态助手（纯重构先行）。上游 spec：`docs/superpowers/specs/2026-07-17-m0-14-bullets-batch-design.md`。

**Tech Stack:** Rust 2024 / stg-core。复用 `polar_to_vec`/`Angle::add_delta`/池宏。

## Global Constraints

- **迭代序 = 角度外层、速度内层 = 池槽分配序**（I4 契约，测试钉住）。
- **两轴累加器**（非乘法）：外层 `cur_angle.add_delta(angle_step)` BAM 回绕；内层 `cur_speed + speed_step` Fx 裸加（溢出 P4-c 域，`ADD_SPEED` 同律）。
- **前置一次验证**：`n_angle == 0 ∨ n_speed == 0 ∨ n_angle×n_speed > BulletPool::CAP(8192)` → `BAD_ARGS` 整体拒（实发 0 零副作用）；`xform` 非空跑走格验证（共用助手）坏则整体拒；模板 `radius` 钳制**恰计一次**。
- **满额短路**：首次 alloc 失败（池或段）即 break，剩余颗数（含本颗）一次性 `wrapping_add` 进**对应**计数器（`pool_full[POOL_BULLET]` 或 `pool_full[POOL_XFORM]`）+ `last_status = STATUS_POOL_FULL`——确定性严格等价于逐颗试（同相位无回收，首败后必然连败；xform 批弹池满路径逐颗试也是"段成功→弹失败→还段→计 BULLET"，短路等价且无段泄漏）。
- **无 RNG**（路线甲）。返回实发数 u16。段消耗账（xform 批 = N×K 段）写 docstring + xform-ops.md。
- **TDD** 每任务红绿；合入前变异检验（T5）。commit 中文 conventional + 尾签 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

---

### Task 1: `xform_args_valid` 抽助手（纯重构）

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`create_bullet_with_xform` 的两遍走格抽出）

**Interfaces:**
- Produces: `fn xform_args_valid(xform: &[crate::xform::XformSlot]) -> bool`（world.rs 私有静态纯函数——两遍 arity 走格全套：长度/未知 op/扩展槽空间/easing id/LOOP 边界；**计数处置留调用方**）。`create_bullet_with_xform` 改为 `if xform.len() > SLOTS_PER_SEG || !Self::xform_args_valid(xform) { …BAD_ARGS… }`——注意长度检查是留在调用方还是进助手**二选一并全计划一致**：**定为进助手**（助手管完整合法性，调用方只问真假）。

- [ ] **Step 0: 开分支**

```bash
git checkout -b m0-14-batch
```

- [ ] **Step 1: 重构**（无新测试——既有 create 验证测试群 + 金向量流不变即规格）

把 `create_bullet_with_xform` 里从 `let mut bad = xform.len() > …` 到第二遍 LOOP 边界检查结束的整块**原样搬**进：

```rust
    /// xform 序列合法性（两遍 arity 走格）：长度 ≤16、逐 op 已实现（扩展槽 scratch 不判）、
    /// STEP 扩展槽空间、easing id < 8、LOOP target 落边界（zero-tail = 合法 END 边界）。
    /// 纯谓词——计数/status 处置留调用方（create_bullet_with_xform 与 create_bullets_batch 共用）。
    fn xform_args_valid(xform: &[crate::xform::XformSlot]) -> bool {
        // ……原两遍走格逻辑逐字搬入，`bad` 语义翻转为返回 !bad ……
    }
```

（实现者照现函数体逐字搬移+翻转，不重写逻辑。）`create_bullet_with_xform` 验证段替换为
`if !Self::xform_args_valid(xform) { contract_viol+1; BAD_ARGS; return NULL; }`。

- [ ] **Step 2: 行为零变化实证 + Commit**

```bash
cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stg-harness -- golden --out /tmp/g-pre.txt && cargo run -p stg-harness -- golden --out /tmp/g-post.txt
diff /tmp/g-pre.txt /tmp/g-post.txt && echo IDENTICAL
git add crates/stg-core/src/world.rs
git commit -m "refactor(world): xform 验证抽 xform_args_valid 助手——batch 共用（金向量流不变实证）"
```

（pre/post 都在本分支起点后跑——纯重构对比自身前后。）

---

### Task 2: `create_bullets_batch` 本体（哑弹批全语义）

**Files:**
- Modify: `crates/stg-core/src/world.rs`（写 API 区）+ 测试（`step.rs` tests，`straight()` 同址）

**Interfaces:**
- Consumes: `xform_args_valid`、`polar_to_vec`、`Angle::add_delta`、`clamp_radius`、`XFORM_NONE`。
- Produces: `pub fn create_bullets_batch(&mut self, init: BulletInit, xform: &[XformSlot], n_angle: u16, angle0: Angle, angle_step: i16, n_speed: u16, speed0: Fx, speed_step: Fx) -> u16`。

- [ ] **Step 1: 写失败测试**（`step.rs` tests）

```rust
    /// 网格几何 + 迭代序判别：3 角 × 2 速——槽 idx = i×2+k，每颗 vx/vy 与
    /// polar_to_vec(speed0+k·Δs, angle0+i·Δa) 参考逐位相等（角度外层速度内层的序被槽号钉死）。
    #[test]
    fn batch_grid_geometry_and_slot_order() {
        use crate::math::geom::polar_to_vec;
        let mut w = World::new(1);
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF),
            &[],
            3, Angle(4096), 8192,
            2, Fx::from_int(1), Fx::from_raw(32768), // 1.0 步进 0.5
        );
        assert_eq!(n, 6);
        for i in 0..3u16 {
            for k in 0..2u16 {
                let slot = (i * 2 + k) as usize;
                let ang = Angle(4096).add_delta((8192 * i as i32) as i16);
                let spd = Fx::from_raw(65536 + 32768 * k as i32);
                let (rvx, rvy) = polar_to_vec(spd, ang);
                assert_eq!(w.body.bullets.vx[slot], rvx, "槽 {slot} vx（角外速内序）");
                assert_eq!(w.body.bullets.vy[slot], rvy, "槽 {slot} vy");
                assert_eq!(w.body.bullets.speed[slot], spd);
                assert_eq!(w.body.bullets.angle[slot], ang);
            }
        }
    }

    /// 环回绕：8-way 整环从 61440 起步——第 2 颗角度过 65536 自动回绕闭合。
    #[test]
    fn batch_ring_wraps_full_circle() {
        let mut w = World::new(1);
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF), &[],
            8, Angle(61440), 8192, 1, Fx::from_int(2), Fx::ZERO,
        );
        assert_eq!(n, 8);
        assert_eq!(w.body.bullets.angle[1], Angle(4096), "61440+8192 回绕 = 4096");
        assert_eq!(w.body.bullets.angle[7], Angle(53248));
    }

    /// 超量与零轴整体拒：实发 0 + BAD_ARGS + 零副作用。
    #[test]
    fn batch_rejects_oversize_and_zero_axis() {
        let mut w = World::new(1);
        let cv0 = w.body.diag.contract_viol;
        assert_eq!(
            w.body.create_bullets_batch(straight(0, 0, 0, 0, 1), &[], 100, Angle::ZERO, 0, 100, Fx::ONE, Fx::ZERO),
            0, "100×100 > 8192 拒"
        );
        assert_eq!(
            w.body.create_bullets_batch(straight(0, 0, 0, 0, 1), &[], 0, Angle::ZERO, 0, 5, Fx::ONE, Fx::ZERO),
            0, "零轴拒"
        );
        assert_eq!(w.body.diag.contract_viol, cv0 + 2);
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0, "零副作用");
    }

    /// 模板 radius 钳制恰计一次（不逐颗累加）。
    #[test]
    fn batch_clamps_template_radius_once() {
        let mut w = World::new(1);
        let mut init = straight(0, 100, 0, 0, 0xFFFF);
        init.radius = Fx::from_int(5000); // 越 MAX_ENTITY_RADIUS
        let cv0 = w.body.diag.contract_viol;
        let n = w.body.create_bullets_batch(init, &[], 4, Angle::ZERO, 1000, 1, Fx::ONE, Fx::ZERO);
        assert_eq!(n, 4);
        assert_eq!(w.body.diag.contract_viol, cv0 + 1, "钳制恰计一次");
        for s in 0..4 {
            assert_eq!(w.body.bullets.radius[s], crate::world::MAX_ENTITY_RADIUS);
        }
    }
```

（`Fx::ONE` 若无此常量用 `Fx::from_int(1)`——以 fx.rs 实况为准。）

- [ ] **Step 2: RED**：`cargo test -p stg-core batch_ 2>&1 | head -3` → 编译失败（方法不存在）。

- [ ] **Step 3: 最小实现**（`world.rs` 写 API 区；本任务只走哑弹路径，xform 分支下一任务补——
`xform` 非空时本任务版本先 `debug_assert!(xform.is_empty())` 占位？**不**——直接写全结构、
xform 分支正确实现但其测试归 T3；实现一次到位，测试分两任务）：

```rust
    /// N×K 网格批量发射器（性能语义原语；ECL syscall `create_bullets_batch` 直通）。
    /// 环 = n_speed=1；列 = n_angle=1；多重环 = 双轴。迭代序 = 角度外层、速度内层 = 池槽
    /// 分配序（I4 契约）。两轴累加器：角度 BAM 回绕、速度 Fx 裸加（溢出 P4-c 域）。
    /// P4：轴零/超池 cap/坏 xform → BAD_ARGS 整体拒（实发 0 零副作用）；额度内池/段满 →
    /// 尽力而为 + 满额短路（剩余批量计数，与逐颗试严格等价）。模板 radius 钳一次。
    /// **段消耗账**：xform 非空时每颗自有段拷贝——一次吃 n_angle×n_speed 个段（段池 2048）。
    /// 无 RNG（路线甲：生成器纯确定）。返回实发数。
    #[allow(clippy::too_many_arguments)] // 批量原语的天然参数面；ECL 绑定层按位打包
    pub fn create_bullets_batch(
        &mut self,
        mut init: BulletInit,
        xform: &[crate::xform::XformSlot],
        n_angle: u16,
        angle0: Angle,
        angle_step: i16,
        n_speed: u16,
        speed0: Fx,
        speed_step: Fx,
    ) -> u16 {
        let total = n_angle as u32 * n_speed as u32;
        if n_angle == 0 || n_speed == 0 || total > BulletPool::CAP as u32 {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return 0;
        }
        if !xform.is_empty() && !Self::xform_args_valid(xform) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
            self.last_status = STATUS_BAD_ARGS;
            return 0;
        }
        if Self::clamp_radius(&mut init.radius) {
            self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        }
        let mut created: u16 = 0;
        let mut cur_angle = angle0;
        'grid: for _ in 0..n_angle {
            let mut cur_speed = speed0;
            for _ in 0..n_speed {
                let (vx, vy) = crate::math::geom::polar_to_vec(cur_speed, cur_angle);
                init.speed = cur_speed;
                init.angle = cur_angle;
                init.vx = vx;
                init.vy = vy;
                // 逐颗：哑/xform 两路（per-bullet P4-a 语义与单发 API 一致；先段后弹）
                let fail_pool: usize = if xform.is_empty() {
                    init.transform_head = crate::xform::XFORM_NONE;
                    if self.bullets.alloc(init).is_some() { usize::MAX } else { POOL_BULLET }
                } else {
                    match self.xforms.alloc() {
                        None => POOL_XFORM,
                        Some(seg) => {
                            let dst = self.xforms.seg_slots_mut(seg);
                            dst[..xform.len()].copy_from_slice(xform);
                            dst[xform.len()..].fill(Default::default());
                            init.transform_head = seg;
                            init.xform_wait = 0;
                            init.xform_next = 0;
                            if self.bullets.alloc(init).is_some() {
                                usize::MAX
                            } else {
                                self.xforms.free(seg); // 先段后弹回滚（无泄漏）
                                POOL_BULLET
                            }
                        }
                    }
                };
                if fail_pool == usize::MAX {
                    created += 1;
                } else {
                    // 满额短路：同相位无回收，后续必然同败——剩余（含本颗）批量计数，
                    // 确定性严格等价于逐颗试（xform 批弹池满路径逐颗也是还段后计 BULLET）。
                    let remaining = (total - created as u32) as u32;
                    self.diag.pool_full[fail_pool] =
                        self.diag.pool_full[fail_pool].wrapping_add(remaining);
                    self.last_status = STATUS_POOL_FULL;
                    break 'grid;
                }
                cur_speed = cur_speed + speed_step;
            }
            cur_angle = cur_angle.add_delta(angle_step);
        }
        created
    }
```

- [ ] **Step 4: 全绿 + Commit**

```bash
git add crates/stg-core/src/world.rs crates/stg-core/src/step.rs
git commit -m "feat(world): create_bullets_batch——N×K 网格/双轴累加器/超量拒/radius 钳一次（哑弹批测试）"
```

---

### Task 3: xform 批 + 满额短路测试

**Files:**
- Modify: `crates/stg-core/src/step.rs`（tests——实现已在 T2 落全，本任务纯测试补齐）

- [ ] **Step 1: 写失败？——本任务测试对已落实现应直接绿；判别力用**变异自证**（每个测试
写完先跑绿，再临时注入对应缺陷证红后还原——T5 变异检验的前置小演，报告记录）：

```rust
    /// 池满尽力而为：预占到只剩 3，请求 2×3=6 → 实发 3 + pool_full[BULLET] += 3。
    #[test]
    fn batch_partial_on_pool_full() {
        let mut w = World::new(1);
        for _ in 0..(BulletPool::CAP - 3) {
            w.body.create_bullet(straight(0, 0, 0, 0, 0xFFFF));
        }
        let pf0 = w.body.diag.pool_full[POOL_BULLET];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF), &[],
            2, Angle::ZERO, 1000, 3, Fx::from_int(1), Fx::from_raw(16384),
        );
        assert_eq!(n, 3, "尽力而为发满剩余额度");
        assert_eq!(w.body.diag.pool_full[POOL_BULLET], pf0 + 3, "剩余 3 颗批量计数");
        assert_eq!(w.body.last_status, crate::world::STATUS_POOL_FULL);
    }

    /// xform 批：每颗自有段（transform_head 互异、段内容 = 序列拷贝+尾零）。
    #[test]
    fn batch_with_xform_gives_each_own_segment() {
        let mut w = World::new(1);
        let seq = [slot(5, crate::xform::OP_SET_SPEED, 131072, 0)];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF), &seq,
            2, Angle::ZERO, 16384, 2, Fx::from_int(1), Fx::from_raw(32768),
        );
        assert_eq!(n, 4);
        let heads: Vec<u16> = (0..4).map(|s| w.body.bullets.transform_head[s]).collect();
        assert_eq!(heads, vec![0, 1, 2, 3], "每颗自有段、分配序 = 槽序");
        for &seg in &heads {
            assert_eq!(w.body.xforms.seg_slots(seg)[0], seq[0], "段内容 = 拷贝");
            assert_eq!(w.body.xforms.seg_slots(seg)[1], Default::default(), "尾零");
        }
    }

    /// xform 坏序列：开跑前整体拒（实发 0、无弹无段泄漏）。
    #[test]
    fn batch_bad_xform_rejected_upfront() {
        let mut w = World::new(1);
        let bad = [slot(0, 99, 0, 0)];
        assert_eq!(
            w.body.create_bullets_batch(straight(0, 0, 0, 0, 1), &bad, 4, Angle::ZERO, 0, 1, Fx::from_int(1), Fx::ZERO),
            0
        );
        assert_eq!(w.body.last_status, crate::world::STATUS_BAD_ARGS);
        assert_eq!(w.body.bullets.iter_alive().count(), 0);
        assert_eq!(w.body.xforms.alloc().unwrap(), 0, "无段泄漏");
    }

    /// 段满短路（xform 批）：段池只剩 2，请求 2×2 → 实发 2 + pool_full[XFORM] += 2。
    #[test]
    fn batch_partial_on_segpool_full() {
        let mut w = World::new(1);
        for _ in 0..(crate::xform::SEG_CAP - 2) {
            w.body.xforms.alloc().unwrap();
        }
        let seq = [slot(0, crate::xform::OP_SET_SPRITE, 1, 0)];
        let n = w.body.create_bullets_batch(
            straight(0, 100, 0, 0, 0xFFFF), &seq,
            2, Angle::ZERO, 1000, 2, Fx::from_int(1), Fx::ZERO,
        );
        assert_eq!(n, 2);
        assert_eq!(w.body.diag.pool_full[crate::world::POOL_XFORM], 2, "剩余 2 颗计入段池计数");
    }
```

（`slot` 助手 step.rs tests 已有【M0-12 T3 引入】；没有就照 transform.rs 同款补。）

- [ ] **Step 2: 跑绿 + 判别力自证**（两处：注释掉短路 break 改逐颗——计数仍等但……选
更判别的：把 `fail_pool` 短路的批量计数改成 +1 → partial 测试红；把 xform 批的
`self.xforms.free(seg)` 回滚注释 → 段泄漏测试红【需构造弹满段活场景——若难构造就跳过
此自证，T5 有正式变异】。证据入报告，还原用反向 Edit。）

- [ ] **Step 3: Commit**

```bash
git add crates/stg-core/src/step.rs
git commit -m "test(world): batch 满额短路/xform 批自有段/坏序列整体拒/段满计数——判别测试群"
```

---

### Task 4: 金向量三压力源 + 双跑

**Files:**
- Modify: `crates/stg-harness/src/main.rs`（导演闭包续编号）

- [ ] **Step 1**：三块（续现有最大编号）：
  - 每 90 帧（offset 35）：32-way 哑弹环 `create_bullets_batch(bullet_at(0, 60), &[], 32, Angle::ZERO, 2048, 1, Fx::from_raw(78643), Fx::ZERO)`（速 1.2，2048=65536/32 整环）；
  - 每 110 帧（offset 70）：5 重速度列 `(…, &[], 1, Angle(16384), 0, 5, Fx::from_int(1), Fx::from_raw(32768))`（正下，1.0→3.0）；
  - 每 150 帧（offset 130）：3×4 网格带两槽 xform `[SET_ANG_VEL(256), END]`（段消耗路径入流，12 段/次）。
  函数文档补三句（含 offset）。
- [ ] **Step 2**：全套闸门 + 金向量双跑 diff → IDENTICAL。
- [ ] **Step 3**：

```bash
git add crates/stg-harness/src/main.rs
git commit -m "feat(harness): 金向量 batch 三压力源——32way 环/5 重列/3×4 带段网格入对拍"
```

---

### Task 5: 变异检验（无提交物；反向 Edit 还原，禁 checkout/restore/stash）

- [ ] 变异 A：迭代序对调（速度外角度内）→ `batch_grid_geometry_and_slot_order` 红。还原。
- [ ] 变异 B：角度累加 `add_delta` 改饱和（`Angle(cur.0.saturating_add(step as u16))`）→ `batch_ring_wraps_full_circle` 红。还原。
- [ ] 变异 C：超量检查删除 → `batch_rejects_oversize_and_zero_axis` 红（且不许烧机——100×100=10000 < CAP? 不：10000 > 8192，删检查后尽力而为发 8192 颗，断言 0 失败即红）。还原。
- [ ] `git diff --exit-code` = 0 + `cargo test -p stg-core` 全绿；三杀记报告。

---

### Task 6: 收尾——回写 + 文档 + 全绿门 + 终审 + 收枝

- [ ] `stg-world-design.md` §4.4/D12 syscall 清单 `create_bullets_batch` 处补括注：
  （世界侧已落地 M0-14：N×K 网格/步长直给/超量拒/尽力而为；张角→步长糖归 ECL DSL——spec 2026-07-17）。
- [ ] `docs/xform-ops.md` 消费入口节补一行：`create_bullets_batch(init, &[XformSlot], …)` ——
  批量发射器，xform 非空时**每颗自有段拷贝（一次吃 N×K 段）**。
- [ ] `PROGRESS.md`：史表顶加
  `| 2026-07-17 | M0-14 | create_bullets_batch N×K 网格发射器——环/列/多重环一个原语，ECL 性能面就绪 |`；
  现在段重写（下一步候选：`bomb · SPAWN_PATTERN+图样表 · M1 ECL`）。
- [ ] 全绿门（fmt/clippy/test/verify-tables/金向量双跑）→ 单 commit `docs:` → 终审（最强模型
  全分支复审）→ `superpowers:finishing-a-development-branch`。

---

## Self-Review 记录

- **Spec 覆盖**：验证助手共用→T1；网格/累加器/迭代序/回绕/超量零轴/radius 一次→T2；
  尽力而为/短路计数/xform 批自有段/坏序列整体拒/段满→T3；金向量三源→T4；变异→T5；
  回写+段账→T6。无缺口。
- **占位扫描**：T3 Step 2 的第二个自证给了"难构造可跳过"的显式出口（T5 有正式变异）——
  非占位而是分层安排；其余全实码。
- **类型一致性**：`xform_args_valid`（T1 产出、T2 消费）；batch 签名 8 参全计划一致；
  `POOL_BULLET`/`POOL_XFORM` 作 `fail_pool` 索引与 `usize::MAX` 哨兵。
- **已知注意**：`#[allow(clippy::too_many_arguments)]` 带理由注释——批量原语天然参数面，
  ECL 绑定层负责打包；若复审不适可议参数结构体化（YAGNI 倾向不做）。
