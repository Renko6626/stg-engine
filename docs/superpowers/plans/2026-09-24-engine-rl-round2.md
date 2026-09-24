# 引擎第二刀 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 敌人速度成为引擎维护的一等字段并进入 Tier 0；同时落地三项模拟热点优化：跳过没有活跃 STEP 的弹、ECL 镜像在加载时校验、CART_FX 极坐标改为惰性计算。

**Architecture:** 前三项只改 `stg-core` 内部，每项做一次前后对比来证明行为不变（或者只在拍板允许的地方变化）。敌人速度要改 `stg-core` 敌人池、`stg-rl` Tier 0 编码，以及 proto 的 SPEC 文字。训练仓最后切到新字段。四项共用一次 ENGINE_VER bump（22 → 23）。

**Tech Stack:** Rust 1.94（workspace，`cargo test`），PyO3 / maturin（wheel `stg_rl` 0.2.0），Python / torch（训练仓，`uv`）。

**Spec:** `docs/superpowers/specs/2026-09-24-engine-rl-round2-design.md`（stg-engine 仓）。实施前先读一遍。

## Global Constraints

- 仓库路径：引擎 `/data/sunyunbo/www/stg-engine`、训练仓 `/data/sunyunbo/www/stg-rl-train`、proto `/data/sunyunbo/www/stg-agent-proto`。**都直接在 `main` 上提交，不开分支，不 push。**
- 引擎的硬规则见 `stg-engine/CLAUDE.md`（I1–I7、P1–P6）。`stg-core` 里不得出现浮点，遍历一律按池索引升序，新池字段自动进入校验和（P6）。
- `ENGINE_VER` 从 22 改为 **23**，只在 Task 1 改一次，changelog 条目写在 `crates/stg-core/src/lib.rs` 的文档注释里。
- 弹的 `flags` 新增两位：第 5 位 `BULLET_STEP_LIVE`，第 6 位 `BULLET_POLAR_STALE`。第 7 位保持空闲。
- 敌人池新增字段 `dx: Fx, dy: Fx`。Tier 0 敌人行步长 38 → **46**，新字段 `vx @38`、`vy @42`，类型为 i32 Q16.16。HELLO 仍是 `"proto":1`。
- wheel 版本 `stg_rl` 升到 **0.2.0**（`crates/stg-py/Cargo.toml` 与 `crates/stg-py/pyproject.toml`）。
- 训练仓的 `GRAPH_VERSION` 不变，动作表不变。
- 提交信息沿用各仓现有风格（中文，`type(scope): 摘要`），末尾加：
  `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`
- 引擎测试命令：`cd /data/sunyunbo/www/stg-engine && cargo test --workspace --exclude stg-godot`。`stg-godot` 只做 `cargo check -p stg-godot`，它的测试依赖 Godot。
- 本地打 wheel：`cd /data/sunyunbo/www/stg-engine && uvx --from "maturin>=1.15,<2" maturin build --release -m crates/stg-py/Cargo.toml --out dist`。

### 前后对比脚本（Task 1 第一步创建，后续任务都用它）

路径：`/data/sunyunbo/www/stg-engine/target/round2/capture.sh`。`target/` 被 gitignore，这个脚本不入库。

```bash
#!/usr/bin/env bash
# 用法：capture.sh <输出目录>
# 用当前工作树的 release harness 跑 golden，再跑训练卡池的全部卡（各 3000 帧，seed 1，rank 2）。
set -euo pipefail
out=$1; mkdir -p "$out"
cd /data/sunyunbo/www/stg-engine
cargo build --release -q -p stg-harness
h=target/release/stg-harness
"$h" golden > "$out/golden.txt"
for d in /data/sunyunbo/www/stg-rl-train/cards/*/; do
  c=$(basename "$d")
  "$h" run "$d" --frames 3000 --seed 1 --rank 2 > "$out/run-$c.txt" 2>&1 || echo "EXIT $?" >> "$out/run-$c.txt"
done
echo "captured $(ls "$out" | wc -l) files into $out"
```

对比：`diff -r target/round2/<before> target/round2/<after>`。允许出现哪些差异，见每个任务的验收条件。

## Review Focus

以下是测试最容易漏掉、但最可能伤到使用者的几种情况，每一条都在对应任务里有测试：

1. **两个 STEP 重叠在同一颗弹上**，先结束的那个不能清掉 `BULLET_STEP_LIVE`，否则后一个会冻在中途（Task 1）。
2. **存档恢复出来的 task pc 指向非法位置**（跨镜像的存档，或者手工改过的存档）：VM 必须确定性地报 fault，不能 panic 或越界读（Task 2）。
3. **CART_FX 弹切回 POLAR（`SET_ACCEL`）或 `STOP_FX` 的那一帧**，必须先 materialize，否则 POLAR 积分会拿陈旧的 speed/angle 把弹甩向错误的方向（Task 3）。
4. **敌人在 `move_to` 插值途中、时停期间、瞬移那一帧**，Tier 0 的 `vx/vy` 分别等于插值位移、0、仅 phase 5 的位移（Task 4）。
5. **镜像的 env**：训练侧读到的敌人 `vx` 必须取负，`vy` 不变（Task 5）。

---

### Task 1: 跳过没有活跃 STEP 的弹 + ENGINE_VER 23

**Files:**
- Modify: `crates/stg-core/src/bullets.rs`（在 `BULLET_BOUNCE_MASK` 后加常量）
- Modify: `crates/stg-core/src/world/transform.rs`（`run_transforms`、`fire_op` 的 STEP 分支、`tick_steps`；测试模块追加测试）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 改为 23，追加 changelog 条目）
- Create: `target/round2/capture.sh`（不入库）

**Interfaces:**
- Produces: `pub const BULLET_STEP_LIVE: u8 = 1 << 5;`（`stg_core::bullets`）；`ENGINE_VER = 23`。

- [ ] **Step 1: 建对比脚本，抓修改前的基线**

写入上文「前后对比脚本」，`chmod +x`，然后运行：
`target/round2/capture.sh target/round2/base`
预期：`captured 87 files`（1 个 golden，加 86 个 run 文件；`cards/` 下的 `README.md` 和 `density.json` 不是目录，glob 不会匹配到）。实际数量以运行结果为准，记下来。

- [ ] **Step 2: 写失败的测试**

追加到 `world/transform.rs` 的 `mod tests`，沿用该模块已有的 `slot` / `xf_bullet` / `test_support::step_t` 辅助函数：

```rust
    /// §2 跳过位：STEP 武装即置位、插值走完那帧清零；期间弹照常推进。
    #[test]
    fn step_live_bit_set_on_arm_and_cleared_on_finish() {
        use crate::bullets::BULLET_STEP_LIVE;
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_SPEED, Fx::from_int(1).raw(), 0),
                slot(0, OP_STEP_SPEED, Fx::from_int(3).raw(), 2), // frames=2
            ],
        );
        assert_eq!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0, "未发射前无位");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 武装
        assert_ne!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0, "武装帧置位");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // tick 1/2
        assert_ne!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0, "进行中保持");
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2)); // tick 2/2 = 完成
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(3));
        assert_eq!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0, "完成那帧清零");
    }

    /// 两个 STEP 重叠：短的先完成时位必须保留，直到长的也完成（Review Focus 1）。
    #[test]
    fn step_live_bit_survives_until_last_overlapping_step_finishes() {
        use crate::bullets::BULLET_STEP_LIVE;
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_STEP_ANGLE, 16384, 4), // 4 帧
                slot(0, OP_STEP_SPEED, Fx::from_int(2).raw(), 1), // 1 帧
            ],
        );
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0)); // 两个都武装
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1)); // SPEED 完成，ANGLE 1/4
        assert_eq!(w.body.bullets.speed[i], Fx::from_int(2));
        assert_ne!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0, "ANGLE 仍在进行");
        for f in 2..=4u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        assert_eq!(w.body.bullets.angle[i], Angle(16384), "长 STEP 走完到终值");
        assert_eq!(w.body.bullets.flags[i] & BULLET_STEP_LIVE, 0);
    }

    /// LOOP 重新武装 STEP 时再次置位。
    #[test]
    fn step_live_bit_rearmed_by_loop() {
        use crate::bullets::BULLET_STEP_LIVE;
        let mut w = crate::step::World::new(1);
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_STEP_SPEED, Fx::from_int(2).raw(), 1), // 槽 0-1
                slot(3, OP_SET_SPRITE, 0, 0),                     // 槽 2，wait 3
                slot(0, OP_LOOP, 0, 2),                           // 槽 3：跳回槽 0 一次
            ],
        );
        let mut seen_clear_then_set = false;
        let mut was_clear = false;
        for f in 0..10u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            let live = w.body.bullets.flags[i] & BULLET_STEP_LIVE != 0;
            if was_clear && live {
                seen_clear_then_set = true;
            }
            was_clear = !live;
        }
        assert!(seen_clear_then_set, "LOOP 重新武装后位应再次置上");
    }
```

注意：`slot` 的参数是 `(wait, op, a0, a1)`。`xf_bullet` 在序列之后怎么补 END、LOOP 的 `args[1]` 计数语义（`fire_loop`：2 表示「写回 1 并跳转」），以现有测试 `loop_rearms_step_from_new_start` 为准。如果上面的序列按实际语义跑不出「清零之后再置位」，就参照那个测试调整序列，但断言的意图不能变。

- [ ] **Step 3: 运行，确认失败**

`cargo test -p stg-core step_live_bit` → 编译失败，`BULLET_STEP_LIVE` 未定义。

- [ ] **Step 4: 实现**

`bullets.rs`，放在 `BULLET_BOUNCE_MASK` 之后：

```rust
/// `flags` 位 5：本弹可能有进行中的 STEP 插值（引擎第二刀 §3）。`fire_op` 武装 STEP 时置位，
/// `tick_steps` 扫完一遍发现没有活跃 STEP 时清零；`run_transforms` 见 0 就不调 `tick_steps`。
/// 只是跳过扫描的提示位：多置无害（多扫一次），漏置不会发生（STEP 只由 `fire_op` 武装）。
/// 进校验和（P6）；Tier 0 不读 `flags`。
pub const BULLET_STEP_LIVE: u8 = 1 << 5;
```

`transform.rs`：
- `run_transforms`：把 `self.tick_steps(i);` 改成
  ```rust
  if self.bullets.flags[i] & crate::bullets::BULLET_STEP_LIVE != 0 {
      self.tick_steps(i); // 与游标并发：wait/WAIT_SIGNAL 期间插值照走
  }
  ```
- `fire_op` 的 STEP 分支：在 `ext.args[1] = STEP_ACTIVE;` 之后（`ext` 的借用结束后）加
  `self.bullets.flags[i] |= crate::bullets::BULLET_STEP_LIVE;`
- `tick_steps`：维护一个 `live` 标志。
  ```rust
  fn tick_steps(&mut self, i: usize) {
      let seg = self.bullets.transform_head[i];
      if seg as usize >= SEG_CAP {
          // P4-b：伪造越界段号——advance_cursor 负责计数+终止，这里只需不 panic。
          self.bullets.flags[i] &= !crate::bullets::BULLET_STEP_LIVE;
          return;
      }
      let fired_end = (self.bullets.xform_next[i] as usize).min(SLOTS_PER_SEG);
      let mut live = false;
      let mut s = 0usize;
      while s < fired_end {
          let main = self.xforms.seg_slots(seg)[s];
          if main.op == OP_END {
              break; // （原注释保留）
          }
          let is_step = main.op == OP_STEP_SPEED || main.op == OP_STEP_ANGLE;
          if is_step && s + 1 < SLOTS_PER_SEG {
              let ext = self.xforms.seg_slots(seg)[s + 1];
              if ext.args[1] & STEP_ACTIVE != 0 {
                  self.tick_one_step(i, seg, s, main, ext);
                  live |= self.xforms.seg_slots(seg)[s + 1].args[1] & STEP_ACTIVE != 0;
              }
          }
          s += 1 + ARITY[main.op as usize] as usize;
      }
      if !live {
          self.bullets.flags[i] &= !crate::bullets::BULLET_STEP_LIVE;
      }
  }
  ```
  原来 END 那一行的长注释要保留。

`lib.rs`：`ENGINE_VER` 改为 23，在变更史末尾按现有格式追加一条：

```
/// **22 → 23**（引擎第二刀，2026-09-24，spec `2026-09-24-engine-rl-round2-design.md`）：
/// ① 弹 `flags` 位 5 `BULLET_STEP_LIVE`（跳过无活跃 STEP 的弹）、位 6 `BULLET_POLAR_STALE`
/// （CART_FX 极坐标惰性回填，读取前 materialize）；② ECL 镜像在加载时校验代码
/// （坏 op / 越界操作数 / 非法跳转目标等从运行时 fault 改为加载错误），两个指令预算合成一个倒数；
/// ③ 敌人池新增 `dx`/`dy`（本帧积分阶段的实际位移）。校验和与存档载荷均变化，旧回放失效。
```

- [ ] **Step 5: 运行测试**

`cargo test --workspace --exclude stg-godot` → 全部 PASS。有测试断言 `ENGINE_VER == 22` 的（例如 `stg-rl` 的 build_info 测试，或 fixture 里的 `engine_ver`），改成 23。

- [ ] **Step 6: 前后对比**

`target/round2/capture.sh target/round2/t1`，然后 `diff -r target/round2/base target/round2/t1`。
验收：只允许校验和相关的字段不同（golden 的逐帧 checksum、run 输出里如果有 checksum 行）。位置、事件、段结束帧这些必须逐字节相同。如果有其他差异，停下来报告，不要提交。

- [ ] **Step 7: 提交**

```bash
git add crates/stg-core/src/bullets.rs crates/stg-core/src/world/transform.rs crates/stg-core/src/lib.rs
git commit -m "perf(core): 弹 flags 位 5 BULLET_STEP_LIVE——无活跃 STEP 的弹跳过 tick_steps；ENGINE_VER 23"
```
（其他因 ENGINE_VER 改动的测试 / fixture 一并 add。）

---

### Task 2: ECL 镜像加载时校验 + 预算合成一个倒数

**Files:**
- Modify: `crates/stg-core/src/ecl/image.rs`（`ImageBuildError` 新变体、`validate_code`，在 `try_from_parts` 末尾调用；测试）
- Modify: `crates/stg-core/src/ecl/vm.rs`（`exec` 拆成外层包装 + `exec_inner`；删掉循环里的 `op_implemented` 检查；测试）
- Modify: `crates/stg-core/src/ecl/mod.rs`（fuzz smoke，第 40–130 行）
- Modify: `crates/stg-ecl-compiler/src/lib.rs`（如果 `ImageBuildError` 被穷尽 match，需要补新变体的消息）
- Modify: 文档 `docs/ecl-ops.md`（25、55–60、342–349、361 行）、`docs/ecl-lang/8-errors.md`（23–39、60 行）、`crates/stg-harness/src/run.rs:43-44`、`docs/superpowers/specs/2026-07-18-m1-ecl-vm-design.md`（文末追加修订记录）
- Modify: `docs/superpowers/specs/2026-09-24-engine-rl-round2-design.md` §4.2（见 Step 6 的说明）

**Interfaces:**
- Consumes: `ops::{ARITY, op_implemented, OP_*}`、`syscall::syscall_implemented(u16) -> bool`、`SubKind`、`LOCALS`（= 64）。
- Produces: `ImageBuildError::BadCode { pc: u32, reason: BadCodeReason }`，其中
  `pub enum BadCodeReason { UnknownOp(u8), ReservedBits, TruncatedOperand, JumpTarget(u32), NotInstructionBoundary(u32), CallTarget(u32), SpawnTarget(u32), SpawnArgc(u32), LocalIndex(u32), Syscall(u32) }`，派生 `Clone, Debug, PartialEq, Eq`。

**关于 spec §4.2 的一处修正（执行时要改 spec 原文）**：存档恢复的 pc 可能不在指令边界上（跨镜像存档，或者被改过的存档）。循环里只保留取指越界检查的话，操作数读取 `ctx.code[opnd_start]` 就可能越界而 panic，违反 P4。所以**保留取指越界和操作数越界两个检查**（都是 `PC_OOB`，分支预测几乎零成本），只删掉 `op_implemented` 这次查表：未实现的 op 的 ARITY 是 0，并且会落进 `match` 的 `_ => Fault(FAULT_BAD_OP)` 分支，行为和原来一样。CALL / SPAWN / PUSHL / POPL / SYS 自己的运行时检查也保留，它们只在各自的 op 上才执行，不在每条指令的路径上。所以现有 vm 单测（直接把裸 `code` 交给 `VmCtx`、不经过镜像）的运行时 fault 断言**全部照旧成立，不用改**。

- [ ] **Step 1: 写失败的测试（校验器）**

追加到 `image.rs` 的 `mod tests`：

```rust
    /// 以单个 Root sub（入口 0）包装 code，走 try_from_parts（同 ecl/mod.rs fuzz 的镜像形状）。
    fn parts_one_sub(code: Vec<u32>) -> Result<EclImage, ImageBuildError> {
        EclImage::try_from_parts(ImageParts {
            code,
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        })
    }
```

sub 0 的 kind 是 Root，所以 `CALL 0`（要求 CallOnly）和 `SPAWN 0`（要求 Async）都应被拒。用例：

```rust
    use crate::ecl::ops::*;

    #[test]
    fn validate_accepts_minimal_program() {
        assert!(parts_one_sub(vec![OP_PUSHI as u32, 5, OP_POP as u32, OP_END as u32]).is_ok());
    }

    #[test]
    fn validate_rejects_unknown_op() {
        let e = parts_one_sub(vec![200, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::UnknownOp(200) });
    }

    #[test]
    fn validate_rejects_reserved_high_bits() {
        let e = parts_one_sub(vec![(1 << 8) | OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::ReservedBits });
    }

    #[test]
    fn validate_rejects_truncated_operand() {
        let e = parts_one_sub(vec![OP_END as u32, OP_PUSHI as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 1, reason: BadCodeReason::TruncatedOperand });
    }

    #[test]
    fn validate_rejects_jump_out_of_range_and_into_operand() {
        let e = parts_one_sub(vec![OP_JMP as u32, 99, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::JumpTarget(99) });
        // 跳到 PUSHI 的操作数字（下标 3）上
        let e = parts_one_sub(vec![OP_JMP as u32, 3, OP_PUSHI as u32, 7, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::NotInstructionBoundary(3) });
    }

    #[test]
    fn validate_rejects_bad_local_index_and_syscall() {
        let e = parts_one_sub(vec![OP_PUSHL as u32, 64, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::LocalIndex(64) });
        let e = parts_one_sub(vec![OP_SYS as u32, 9999, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::Syscall(9999) });
    }

    #[test]
    fn validate_rejects_call_to_non_callonly_and_bad_spawn() {
        // sub 0 是 Root：CALL 0 → CallTarget；SPAWN 0 argc=1 → SpawnTarget；SPAWN argc=65 → SpawnArgc
        let e = parts_one_sub(vec![OP_CALL as u32, 0, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::CallTarget(0) });
        let e = parts_one_sub(vec![OP_SPAWN as u32, 0, 1, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::SpawnTarget(0) });
        let e = parts_one_sub(vec![OP_SPAWN as u32, 0, 65, OP_END as u32]).unwrap_err();
        assert_eq!(e, ImageBuildError::BadCode { pc: 0, reason: BadCodeReason::SpawnArgc(65) });
    }

    #[test]
    fn validate_allows_falling_off_the_end() {
        // 不要求以 END 结尾：落出末尾仍由运行时 PC_OOB 处理
        assert!(parts_one_sub(vec![OP_PUSHI as u32, 1, OP_POP as u32]).is_ok());
    }
```

SPAWN 的检查顺序：先 `argc > 64` → `SpawnArgc`，再查 id / kind / arity → `SpawnTarget`。

- [ ] **Step 2: 运行，确认失败**

`cargo test -p stg-core validate_` → 编译失败（`BadCode` 未定义）。

- [ ] **Step 3: 实现 `validate_code`**

在 `image.rs` 里加：

```rust
/// 加载闸的代码校验（引擎第二刀 §4）：从 0 起按 ARITY 线性解码整条 `code`，
/// 先定指令边界，再逐条查静态可判的合法性。运行期仍保留取指与操作数越界检查（存档可带回任意 pc）。
fn validate_code(code: &[u32], subs: &[RuntimeSubMeta], param_types_of: impl Fn(u16) -> Option<usize>)
    -> Result<(), ImageBuildError>
```

参数形式按 `try_from_parts` 里实际可用的数据结构来定（`runtime_subs`、各 sub 的参数个数）。要点：
1. 第一趟：`pc = 0` 起，`head = code[pc]`；`head >> 8 != 0` → `ReservedBits`；`!op_implemented(op)` → `UnknownOp(op)`；`pc + 1 + ARITY[op] > len` → `TruncatedOperand`；把 `pc` 记进一个 `Vec<bool>` 边界表，然后 `pc += 1 + arity`。
2. 每个 sub 的 `code_entry`、每个 mark 的 ip，都必须是边界，否则报 `NotInstructionBoundary(ip)`，`pc` 取该 ip。
3. 第二趟逐条检查操作数：
   - JMP / JZ：`t >= len` → `JumpTarget(t)`；`!boundary[t]` → `NotInstructionBoundary(t)`。
   - CALL：id 不是合法 u16 SubId，或者 kind != CallOnly → `CallTarget(raw)`。
   - SPAWN：`argc > 64` → `SpawnArgc(argc)`；id 非法、kind != Async、或参数个数 != argc → `SpawnTarget(raw)`。
   - PUSHL / POPL：`idx >= 64` → `LocalIndex(idx)`。
   - SYS：`u16::try_from(n)` 失败或 `!syscall_implemented(n)` → `Syscall(n)`。
4. 在 `try_from_parts` 返回 `Ok(Self{..})` 之前调用。按 VM 里 CALL / SPAWN 的做法，用现成的 `sub_id` / `sub_meta` / `param_types` 查询方法；只有已经构造出来的 `Self` 才有这些方法的话，就先构造 `img`，再 `validate_code(&img)?; Ok(img)`。

`ImageBuildError` 如果实现了 `Display`，要给 `BadCode` 写一条人读的消息（包含 pc 和原因）。

- [ ] **Step 4: 预算合成一个倒数**

`vm.rs`：把现有的 `exec` 改名为 `fn exec_inner(task: &mut Task, ctx: &mut VmCtx, left: &mut u32) -> Exec`，循环头改成：

```rust
        // 预算门：进 exec 时已取 left = min(任务内 1024, 全局余额)——任一耗尽都在下一条指令处报错，
        // 恰在第 1025 条（或全局余额用尽后的下一条）触发，语义同合并前（引擎第二刀 §4.3）。
        if *left == 0 {
            return Exec::Fault(FAULT_BUDGET);
        }
        *left -= 1;
```

删掉 `task_count` 和 `*ctx.budget -= 1`，也删掉 `if !ops::op_implemented(op) { .. }` 这一块（保留取指和操作数越界检查）。新的外层函数：

```rust
/// 从 `task.pc` 起解释执行，直到 `WAIT` 让出 / `END` 完成 / Fault。
pub(crate) fn exec(task: &mut Task, ctx: &mut VmCtx) -> Exec {
    let n0 = TASK_BUDGET.min(*ctx.budget);
    let mut left = n0;
    let r = exec_inner(task, ctx, &mut left);
    *ctx.budget -= n0 - left;
    r
}
```

先 `grep -n "ctx.budget\|\.budget" crates/stg-core/src/ecl/*.rs`，确认 `exec_inner` 内部（包括 syscall）没有别处读写 `ctx.budget`。如果有，停下来报告。

- [ ] **Step 5: fuzz smoke 改为走校验器**

`ecl/mod.rs:40-130` 目前用 `test_image` 包 64 个随机字（它会 `.expect`，现在会 panic）。改为：直接调用 `EclImage::try_from_parts`，被拒就 `continue`（统计被拒的个数）；通过的照原来的方式跑，并断言：
- 不 panic；
- 如果 fault，fault 码必须属于 `{FAULT_PC_OOB, FAULT_STACK, FAULT_BUDGET, FAULT_DIV_ZERO, FAULT_CALL_DEPTH, FAULT_BAD_OP}`（BAD_OP 仍可能来自 syscall 内部的动态检查）。

再加一条确定性的用例：构造一段合法镜像，手动把 task 的 pc 设到一个操作数字上（模拟存档带回的坏 pc），`exec` 必须返回 fault、不 panic（Review Focus 2）。

- [ ] **Step 6: 文档**

- `docs/ecl-ops.md`：fault 表里 0 / 1 两行注明「未知 op、越界操作数、非法跳转目标、CALL/SPAWN 目标不对，**在加载时报编译错误**；运行时只剩存档恢复出来的坏 pc 与动态检查」；25、55–60 行里「运行时 Fault(0)」改为「加载时拒绝」。
- `docs/ecl-lang/8-errors.md` 23–39、60 行同样修改。
- `stg-harness/src/run.rs:43-44` 的提示文字：BAD_OP 改为「坏 op（通常在加载时就已拒绝；运行时出现说明是动态检查）」这类说法。
- VM 设计文档末尾追加：`> 修订（2026-09-24，引擎第二刀）：静态可判的非法指令改在加载时拒绝；两个预算合成一个倒数，1025 边界不变。`
- 本刀 spec §4.2：把「已经被校验器覆盖的检查从循环里删掉：op_implemented、操作数越界、……」改为本任务开头说明的「保留取指与操作数越界，只删 op_implemented；各 op 自己的检查保留」，并写明理由（存档 pc 不可信，P4 禁止 panic）。

- [ ] **Step 7: 全量测试 + 全部卡能编译**

`cargo test --workspace --exclude stg-godot` → PASS。
`target/round2/capture.sh target/round2/t2 && diff -r target/round2/t1 target/round2/t2` → **完全没有差异**。任何一张卡出现编译错误或差异，都说明校验器过严，或者有卡依赖运行时 fault：停下来报告。
另外编译 `scenes/` 下全部 `.ecl`：`for f in scenes/*.ecl; do target/release/stg-harness check "$f" || echo "FAIL $f"; done`。`check` 子命令的用法以 `stg-harness check --help` 或 `main.rs` 为准，如果它不接受单文件，就用 `run --frames 1`。

- [ ] **Step 8: 提交**

```bash
git add -A crates/stg-core crates/stg-ecl-compiler crates/stg-harness docs
git commit -m "perf(ecl): 镜像加载时校验代码（坏 op/越界操作数/非法跳转/CALL·SPAWN 目标/局部下标/syscall 号）+ 两个指令预算合成一个倒数"
```

---

### Task 3: CART_FX 极坐标惰性化

**Files:**
- Modify: `crates/stg-core/src/bullets.rs`（常量）
- Modify: `crates/stg-core/src/world/motion.rs`（`polar_of_vel`、`backfill_polar`、`materialize_polar`，以及各 setter 与模式切换的调用点）
- Modify: `crates/stg-core/src/world/integrate.rs`（CART 分支、反弹的 CART 分支；测试）
- Modify: `crates/stg-core/src/world/transform.rs`（`OP_ADD_SPEED`、STEP 起始值）
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`self_vel` 的 BULLET 分支）
- Modify: `crates/stg-godot/src/frame.rs:107-110`、`crates/stg-harness/src/run.rs:200-201`
- 可能涉及：`crates/stg-core/src/world/view.rs`（如果需要给外部读取方提供 `bullet_polar(i)`）

**Interfaces:**
- Consumes: `BULLET_CART_FX`、`BULLET_POLAR_FX`、`BACKFILL_MIN_SPEED`。
- Produces:
  - `pub const BULLET_POLAR_STALE: u8 = 1 << 6;`
  - `pub fn polar_of_vel(vx: Fx, vy: Fx, stored_angle: Angle) -> (Fx, Angle)`（`stg_core::world::motion`，公开，纯函数）：回填规则的唯一实现，`speed = isqrt(len_sq)`，`speed >= BACKFILL_MIN_SPEED` 时 `angle = atan2(vy, vx)`，否则取 `stored_angle`。
  - `pub(crate) fn materialize_polar(&mut self, i: usize)`（`WorldBody`）。
  - 外部读取方用的纯视图：`BulletView`（或当前 `view.bullets()` 返回的类型）上加 `pub fn polar(&self, i: usize) -> (Fx, Angle)`。位为 1 时返回 `polar_of_vel(vx, vy, angle)`，否则返回存储值。

- [ ] **Step 1: 列出全部读取点**

运行：
`grep -rn "bullets\.speed\[\|bullets\.angle\[\|\.speed()\[\|\.angle()\[" crates --include=*.rs | grep -v "/tests/"`
逐条分类，记在提交说明里：①写入且不读另一个分量；②读取（需要先 materialize）；③世界外的读取方（改用 `polar()`）。spec §5 列出的调用点必须全部出现在分类里。凡是 `bullets.speed[` 或 `bullets.angle[` 的**读取**，在 debug 下都要能被断言抓到：在 `materialize_polar` 之后才能读。做法见 Step 4 的最后一点。

- [ ] **Step 2: 写失败的测试**

`integrate.rs` 测试模块：

```rust
    /// §5 对拍：速度全程高于阈值时，惰性结果与逐帧回填逐位相同。
    #[test]
    fn lazy_polar_matches_eager_backfill_while_fast() {
        use crate::bullets::BULLET_POLAR_STALE;
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 200);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.vx[i] = Fx::from_int(2);
        w.body.bullets.vy[i] = Fx::from_int(-3);
        w.body.set_gravity_at(i, Fx::ZERO, Fx::from_raw(16384)); // ay = 0.25：vy −3 → +2，|v| ≥ 2 全程高于阈值
        for f in 0..20u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            assert_ne!(w.body.bullets.flags[i] & BULLET_POLAR_STALE, 0, "积分后应标脏");
            let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
            let (sp, ang) = crate::world::motion::polar_of_vel(vx, vy, w.body.bullets.angle[i]);
            assert_eq!(ang, crate::math::cordic::atan2(vy, vx), "帧 {f}");
            assert_eq!(sp.raw(), crate::math::isqrt::isqrt(crate::math::geom::len_sq(vx, vy) as u64) as i32);
        }
        w.body.materialize_polar(i);
        assert_eq!(w.body.bullets.flags[i] & BULLET_POLAR_STALE, 0);
        let (vx, vy) = (w.body.bullets.vx[i], w.body.bullets.vy[i]);
        assert_eq!(w.body.bullets.angle[i], crate::math::cordic::atan2(vy, vx));
    }

    /// 新语义锁定：降到阈值以下再回升，期间没人读 ⇒ materialize 取当前方向（旧实现会取当前方向——
    /// 两者在此一致）；期间被读过一次 ⇒ 低速时的读取保留读取前存储的角度。
    #[test]
    fn lazy_polar_low_speed_keeps_stored_angle() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 200);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.vx[i] = Fx::ZERO;
        w.body.bullets.vy[i] = Fx::from_raw(-2048); // 1/32 px/帧：低于阈值
        w.body.bullets.angle[i] = crate::math::Angle(1234);
        w.body.set_gravity_at(i, Fx::ZERO, Fx::ZERO);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        w.body.materialize_polar(i);
        assert_eq!(w.body.bullets.angle[i], crate::math::Angle(1234), "低速不改角度");
        assert_eq!(w.body.bullets.speed[i].raw(), 2048);
    }

    /// Review Focus 3：CART → POLAR 切换帧必须用 materialize 后的值，位移与旧的逐帧回填实现一致。
    #[test]
    fn switch_cart_to_polar_uses_fresh_polar() {
        let mut w = crate::step::World::new(1);
        let h = bullet_at(&mut w, 0, 200);
        let i = w.body.bullets.get(h).unwrap();
        w.body.bullets.vx[i] = Fx::from_int(3);
        w.body.bullets.vy[i] = Fx::ZERO;
        w.body.set_gravity_at(i, Fx::ZERO, Fx::from_int(1)); // 每帧 vy += 1
        for f in 0..3u32 {
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
        }
        // 此时 v = (3, 3)，存储的 angle/speed 是陈值
        w.body.set_accel_at(i, Fx::ZERO); // 切 POLAR
        let (x0, y0) = (w.body.bullets.x[i], w.body.bullets.y[i]);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(3));
        let dx = w.body.bullets.x[i] - x0;
        let dy = w.body.bullets.y[i] - y0;
        let (evx, evy) = crate::math::geom::polar_to_vec(
            Fx::from_raw(crate::math::isqrt::isqrt(crate::math::geom::len_sq(Fx::from_int(3), Fx::from_int(3)) as u64) as i32),
            crate::math::cordic::atan2(Fx::from_int(3), Fx::from_int(3)),
        );
        assert_eq!((dx, dy), (evx, evy), "POLAR 首帧应沿 45° 以 |v| 前进");
    }
```

另外改 `cart_fx_gravity_parabola_flips_vy_and_tracks_angle`（`integrate.rs` 约 517 行）：在断言 angle/speed 之前先调 `w.body.materialize_polar(i);`，并把注释改成「materialize 后与参考逐位相等」。

在 `syscall.rs` 的测试里，给 `self_velocity_vars_dispatch_to_bullet_pool`（约 2831 行）配一条新用例：CART_FX 弹积分 1 帧后读 `$self_angle`，值等于 `atan2(vy, vx)`，并且读完后 `BULLET_POLAR_STALE == 0`（读取会回写）。

- [ ] **Step 3: 运行，确认失败**

`cargo test -p stg-core lazy_polar switch_cart` → 编译失败。

- [ ] **Step 4: 实现**

`bullets.rs`：

```rust
/// `flags` 位 6：极坐标（`speed`/`angle`）是陈值，需按 `vx/vy` 回填后才能读（引擎第二刀 §5）。
/// 只有 CART_FX 积分与 CART 反弹置它；`materialize_polar` 清。世界外读取方用视图的 `polar()`。
pub const BULLET_POLAR_STALE: u8 = 1 << 6;
```

`motion.rs`：

```rust
/// 回填规则的唯一实现（纯函数；视图与 materialize 共用，保证两边逐位一致）。
pub fn polar_of_vel(vx: Fx, vy: Fx, stored_angle: Angle) -> (Fx, Angle) {
    let sp = Fx::from_raw(isqrt(len_sq(vx, vy) as u64) as i32);
    debug_assert!(sp.raw() >= 0, "backfill: isqrt 窄化回绕成负 speed（|v| 越界，vx={vx:?} vy={vy:?}）");
    let ang = if sp.raw() >= BACKFILL_MIN_SPEED.raw() { atan2(vy, vx) } else { stored_angle };
    (sp, ang)
}
```

`backfill_polar` 改成调用它，写回两个字段后清掉 `BULLET_POLAR_STALE`（原来的长注释移到 `polar_of_vel` 上）。新增：

```rust
    /// 极坐标若为陈值则按回填规则写回（引擎第二刀 §5）。读写 speed/angle 之前、离开 CART 模式之前调用。
    #[inline]
    pub(crate) fn materialize_polar(&mut self, i: usize) {
        if self.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE != 0 {
            self.backfill_polar(i);
        }
    }
```

调用点（每处在函数开头加 `self.materialize_polar(i);`）：`set_speed_at`、`set_angle_at`、`turn_at`、`aim_at_player_at`（它读 speed）、`set_ang_vel_at`、`set_accel_at`、`stop_fx_at`。`set_gravity_at` 不需要加。`set_bullet_vel` 已经调了 `backfill_polar`，会顺带清位。

`integrate.rs`：CART 分支里 `self.backfill_polar(i);` 改为 `self.bullets.flags[i] |= crate::bullets::BULLET_POLAR_STALE;`。`bounce_bullet` 的两个 `else` 分支同样改。

`transform.rs`：`OP_ADD_SPEED` 读 `speed` 之前 `self.materialize_polar(i);`；STEP 分支计算 `start` 之前 `self.materialize_polar(i);`。`tick_one_step` 走 `set_speed_at` / `set_angle_at`，已经被覆盖。

`syscall.rs`：`self_vel` 的 BULLET 分支需要可变访问。把签名改成 `fn self_vel(task: &Task, ctx: &mut VmCtx)`，在 BULLET 分支里先 `ctx.body.materialize_polar(i);` 再读；四个调用点跟着改。如果 `ctx.body` 不是 `&mut WorldBody`，就报告，改用 `polar_of_vel` 纯计算，并在提交说明里写明「ECL 读取不回写」。

视图：给 `view.bullets()` 返回的类型加 `polar(i)`（见 Interfaces）。`stg-godot/src/frame.rs:107-110` 把 `angles[i]` 改成 `p.polar(i).1`；`stg-harness/src/run.rs:200-201` 的 `angle_bam` / `speed_raw` 改用 `b.polar(i)`。

**debug 防漏**：在 `integrate_bullets` 的 POLAR 分支开头加
`debug_assert_eq!(self.bullets.flags[i] & crate::bullets::BULLET_POLAR_STALE, 0, "POLAR 积分读到陈值极坐标（漏 materialize）");`
在 `refresh_vel_from_polar` 开头加同样的断言。所有「读 speed/angle 再写 v」的路径最后都经过 `refresh_vel_from_polar`，漏掉的调用点会在 debug 测试和 harness debug 运行里暴露。

- [ ] **Step 5: 运行测试**

`cargo test --workspace --exclude stg-godot` → PASS。`cargo check -p stg-godot` → 通过。
再用 debug 构建跑全部卡，确认断言不触发：
`cargo build -q -p stg-harness && for d in /data/sunyunbo/www/stg-rl-train/cards/*/; do target/debug/stg-harness run "$d" --frames 3000 --seed 1 --rank 2 >/dev/null 2>/tmp/err.txt || { echo "FAIL $d"; head -5 /tmp/err.txt; }; done`
预期没有 FAIL 输出。

- [ ] **Step 6: 前后对比**

`target/round2/capture.sh target/round2/t3 && diff -r target/round2/t2 target/round2/t3`
验收：checksum 可以变；run 输出中**位置、事件、段结束帧必须不变**。harness 的 dump 已经改走 `polar()`，所以 speed / angle 列也应当不变。如果 angle 列有差异，只能出现在「弹速低于阈值、之后又回升」的弹上，并且要逐条说明。

- [ ] **Step 7: 提交**

```bash
git add -A crates/stg-core crates/stg-godot crates/stg-harness
git commit -m "perf(core): CART_FX 极坐标惰性回填——flags 位 6 BULLET_POLAR_STALE，读写前 materialize；视图 polar() 供渲染/harness"
```

---

### Task 4: 敌人速度一等字段 + Tier 0 敌人行 46 字节 + proto SPEC + wheel 0.2.0

**Files:**
- Modify: `crates/stg-core/src/enemy.rs`（池字段 `dx: Fx, dy: Fx`，放在 `x, y, vx, vy` 那一行之后单独一行，加注释）
- Modify: 全部 `EnemyInit {` 构造处，共 23 处（`grep -rn "EnemyInit {" crates`），补 `dx: Fx::ZERO, dy: Fx::ZERO`
- Modify: `crates/stg-core/src/world/integrate.rs`（`integrate` 的时停分支、`integrate_enemies`；测试）
- Modify: `crates/stg-core/src/step.rs:2350-2364`（池大小哨兵）
- Modify: `crates/stg-rl/src/layout.rs`（`off::enemy::VX = 38`、`VY = 42`；`ENEMIES.stride = 46`，`fields` 追加 `f!("vx", Fx, 38)`、`f!("vy", Fx, 42)`）
- Modify: `crates/stg-rl/src/encode.rs:195-225`、`crates/stg-rl/src/vec_env.rs:57,451`（用 `ENEMIES.stride`）
- Modify: `crates/stg-rl/tests/*`、`crates/stg-rl/tests/fixtures/proto_v1_hello.json`
- Modify: `crates/stg-py/Cargo.toml`、`crates/stg-py/pyproject.toml`（版本 0.2.0）、`Cargo.lock`
- Modify: proto 仓 `SPEC.md:216-240`、`c/sa_layout.h:13`、`c/sa_encode.c:143-145`，以及 `src/` 下的 schema（如果写死了敌人字段表）
- Modify: `docs/follow-ups.md`（新增一条 D25，见 Step 7）、`docs/rl-perf-roadmap.md`（§2、§3、§5 标为已落地）、`PROGRESS.md`（按惯例记一笔）

**Interfaces:**
- Consumes: 无（与 Task 1–3 无接口依赖，只共用 ENGINE_VER 23）。
- Produces: 敌人池 `dx()` / `dy()` 视图访问器（由宏生成）；`stg_rl.OFFSETS["enemies"]["vx"] == 38`、`["vy"] == 42`；`ENEMIES stride == 46`；wheel `stg_rl-0.2.0`。

- [ ] **Step 1: 写失败的测试（core）**

`integrate.rs` 测试模块。先把 `enemy.rs` 测试模块里的 `enemy_at(x, y, hp) -> EnemyInit`（约 103 行，穷尽的全字段 Init）改成 `pub(crate)`，模块声明改成 `#[cfg(test)] pub(crate) mod tests`，这样就可以用 `crate::enemy::tests::enemy_at` 引用它（Step 3 给它补上 `dx`/`dy` 之后照常可用）。`create_enemy` 见 `world.rs:489`，`move_enemy_to(h, x, y, dur, easing)` 见 `world.rs:619`。时停：`w.body.freeze_left = [5, 0]`（下标 0 是 C 组，也就是场景冻结；`begin` 每帧先减 1，所以初值取 5 能保证下一帧 `scene_frozen()` 为真）。

```rust
    /// dx/dy = 本帧 phase 5 的实际位移：匀速积分、move_to 插值、到站吸附。
    #[test]
    fn enemy_dxdy_is_phase5_displacement() {
        let mut w = crate::step::World::new(1);
        let mut init = crate::enemy::tests::enemy_at(0, 100, 10);
        init.vx = Fx::from_int(2);
        let h = w.body.create_enemy(init);
        let i = h.index as usize;
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!((w.body.enemies.dx[i], w.body.enemies.dy[i]), (Fx::from_int(2), Fx::ZERO), "匀速");
        // move_to 插值：4 帧从当前位置到 +40,+0（linear，easing 0）；vx 残值 2 不应出现在 dx 里
        let x0 = w.body.enemies.x[i];
        w.body.move_enemy_to(h, x0 + Fx::from_int(40), w.body.enemies.y[i], 4, 0);
        for f in 1..=4u32 {
            let before = w.body.enemies.x[i];
            crate::world::test_support::step_t(&mut w, &InputFrame::empty(f));
            assert_eq!(w.body.enemies.dx[i], w.body.enemies.x[i] - before, "插值帧 {f}");
            assert_ne!(w.body.enemies.dx[i], w.body.enemies.vx[i], "插值帧不等于积分器 vx");
        }
    }

    /// 瞬移（dur=0，phase 2 之外直接调用等价）不计入；时停帧为 0。
    #[test]
    fn enemy_dxdy_excludes_teleport_and_is_zero_when_frozen() {
        let mut w = crate::step::World::new(1);
        let h = w.body.create_enemy(crate::enemy::tests::enemy_at(0, 100, 10));
        let i = h.index as usize;
        w.body.move_enemy_to(h, Fx::from_int(50), Fx::from_int(100), 0, 0); // 瞬移
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(0));
        assert_eq!(w.body.enemies.dx[i], Fx::ZERO, "瞬移不计入");
        // 时停：按 world.rs 里 freeze 测试（约 2236 行）的做法把 freeze_left[0] 置为正数
        w.body.enemies.vx[i] = Fx::from_int(3);
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(1));
        assert_eq!(w.body.enemies.dx[i], Fx::from_int(3));
        w.body.freeze_left = [5, 0];
        crate::world::test_support::step_t(&mut w, &InputFrame::empty(2));
        assert_eq!(w.body.enemies.dx[i], Fx::ZERO, "时停帧 dx = 0");
    }
```

如果 `create_enemy` 会改写 init 里的某些字段（例如 `speed/angle` 由 vx/vy 回填），断言只针对 `dx/dy` 和位置，不受影响。

- [ ] **Step 2: 运行，确认失败**

`cargo test -p stg-core enemy_dxdy` → 编译失败（没有 `dx` 字段）。

- [ ] **Step 3: 实现（core）**

`enemy.rs`：

```rust
        // ── 本帧实际位移（引擎第二刀 §6）：phase 5 积分前后位置之差，由 integrate_enemies 写；
        //    瞬移（phase 2 的 move_to dur=0）不计入；时停帧为 0。与 vx/vy（积分器状态）不同：
        //    move_to 插值期间 vx/vy 是残值，dx/dy 才是真实位移。Tier 0 敌人行的 vx/vy 取自这里。
        dx: Fx, dy: Fx,
```

`integrate_enemies` 的循环体：开头 `let (x0, y0) = (self.enemies.x[i], self.enemies.y[i]);`，在位置更新（`if mv_active {..} else {..}`）之后写 `self.enemies.dx[i] = self.enemies.x[i] - x0; self.enemies.dy[i] = self.enemies.y[i] - y0;`。

`integrate()`：`if !scene {..}` 那段补一个 `else` 分支，把所有存活敌人的 `dx/dy` 清零（按存活掩码、池索引升序遍历，写法同 `integrate_enemies` 的位扫）。

23 处 `EnemyInit` 补字段；`step.rs` 的尺寸哨兵按新大小更新（每只敌多 8 字节）。

`cargo test -p stg-core` → PASS。

- [ ] **Step 4: Tier 0（stg-rl）测试先行**

在 `crates/stg-rl/tests/encode.rs` 加：

```rust
/// 敌人行 46 字节，vx/vy 在 38/42，取自池的 dx/dy（Q16.16 原值）。
#[test]
fn enemy_row_carries_dxdy_as_vx_vy() {
    let st = stg_rl::layout::ENEMIES.stride;
    assert_eq!(st, 46);
    let mut w = World::new(1);
    let h = enemy(&mut w, 30, 60, 500, 700);
    w.body.enemies.vx[h.index as usize] = Fx::from_int(2);
    run_step(&mut w, 0);
    let mut rows = vec![0u8; 256 * st];
    assert_eq!(write_enemies(&w, &mut rows), 1);
    let r0 = &rows[0..st];
    assert_eq!(rd_i32(r0, off::enemy::VX), Fx::from_int(2).raw());
    assert_eq!(rd_i32(r0, off::enemy::VY), 0);
    assert_eq!(rd_i32(r0, off::enemy::X), Fx::from_int(32).raw(), "位置同帧已积分");
}
```

（`stg_rl::layout::ENEMIES` 的实际路径以 `crates/stg-rl/src/lib.rs` 的导出为准。如果 `w.body.enemies` 在集成测试里不可见，就用该文件已有的公开写口设置速度；找不到这样的写口时，改为在 ECL 里让敌人匀速移动。）同一文件里的 `enemies_rows_boss_collidable_id` 写死了 `256 * 38` 和 `rows[0..38]`、`rows[38..76]`，改为按 `st` 计算。

再在 `crates/stg-rl/tests/vec_env.rs` 加一条**对拍**测试（Review Focus 4 的端到端版本）：用一段内联 ECL 建场景。场景里有一只敌以 `move_to` 插值移动 30 帧、一只敌匀速移动、一只敌在第 20 帧瞬移 100 px。ECL 语法以 `docs/ecl-lang.md` 为准，参照 `tests/` 或 `scenes/` 里已有的敌人脚本来写。`VecEnv` 设 n=1、frame_skip=1，step 40 次。每一步都按 `id` 匹配上一步的行，断言：
- 对上且位移 ≤ 16 px 时，`vx`/`vy` 等于坐标差（逐位）；
- 瞬移那一步，`vx` 不等于坐标差（坐标差约 100 px）；
- 新出现的敌，第一步的 `vx`/`vy` 就是它本帧的积分位移（匀速的那只等于它的速度）。

- [ ] **Step 5: 实现（stg-rl + stg-py）**

`layout.rs`：`off::enemy` 加 `pub const VX: usize = 38; pub const VY: usize = 42;`；`ENEMIES` 的 `stride: 46`，`fields` 末尾追加 `f!("vx", Fx, 38), f!("vy", Fx, 42)`。
`encode.rs::write_enemies`：`let st = ENEMIES.stride;`，行切片改为 `rows[k * st..(k + 1) * st]`，追加
`put_i32(r, off::enemy::VX, e.dx()[i].raw()); put_i32(r, off::enemy::VY, e.dy()[i].raw());`
`vec_env.rs:57,451`：`38` 改为 `ENEMIES.stride`。
更新 `tests/fixtures/proto_v1_hello.json` 中 enemies 表的 stride 和字段表（跑 `tests/layout.rs`，按失败提示更新，或者用现有的重生成方式）。
`stg-py` 两个版本号改为 0.2.0，`cargo build` 会更新 `Cargo.lock`。`crates/stg-py/tests/test_smoke.py:18` 里 `"enemies": 38` 改为 46；如果有断言 `build_info()["version"]` 或 `engine_ver` 的地方，一并更新。打出 wheel 之后，在临时 venv 里跑这组 Python 测试：`uv venv /tmp/sunyunbo/round2-venv && uv pip install --python /tmp/sunyunbo/round2-venv dist/stg_rl-0.2.0-*.whl pytest numpy && /tmp/sunyunbo/round2-venv/bin/pytest crates/stg-py/tests -q`。这一步放到 Step 8 打完 wheel 之后执行。
`cargo test --workspace --exclude stg-godot` → PASS。

- [ ] **Step 6: proto 仓**

`/data/sunyunbo/www/stg-agent-proto`：
- `SPEC.md` 敌人行表格追加两行：`vx | Fx | 38 | 本帧实际位移 x（px/帧）：phase 5 积分前后之差；瞬移不计入；时停为 0；frame_skip>1 时取最后一帧。后端不提供时写 0。`，`vy` 同理，偏移 42。表头处的 stride 改为 46。
- `c/sa_layout.h`：`SA_ENEMY_STRIDE 46`，如果有字段偏移宏，追加 `SA_ENEMY_VX 38`、`SA_ENEMY_VY 42`。
- `c/sa_encode.c` 的敌人行编码：两列写 0，加注释 `/* vx/vy：部署侧速度仍由 sa_model.c 按 id 差分，见 stg-engine docs/follow-ups.md D25 */`。
- `grep -rn "38\|hp_max\|\"id\"" src c tests | grep -i enem`，把写死的步长或字段表同步掉。
- 按 README 跑 proto 的测试（Python `pytest` 以及 C 编译测试，如果有的话），全部通过。
- 提交：`feat(spec): 敌人行追加 vx/vy（stride 38→46）——stg-engine 引擎第二刀；C 侧暂写 0`

- [ ] **Step 7: 引擎文档**

- `docs/follow-ups.md`：在 D24 之后新增 `### D25. 引擎第二刀遗留（2026-09-24）`，写入 spec §7 的第 1、2 条（部署侧敌人速度仍是差分、th06nc 写 0、差异点、触发点；`pack_handle` 回绕随之处理，并指向 525 行已有的条目）。
- `docs/rl-perf-roadmap.md`：§2、§3、§5 的标题后加「**已落地（ENGINE_VER 23 / stg_rl 0.2.0）**」，并在文首的「已落地」行补上这三项。§3 注明「保留了取指与操作数越界检查」。
- `PROGRESS.md`：按文件已有格式记一笔「引擎第二刀」。

- [ ] **Step 8: 前后对比 + 打 wheel**

`target/round2/capture.sh target/round2/t4 && diff -r target/round2/t3 target/round2/t4` → 只允许 checksum 变化。harness 的 run 输出不含 Tier 0；如果 dump 里有敌人行，也不应该出现 dx 以外的变化。
打 wheel（命令见 Global Constraints）→ `dist/stg_rl-0.2.0-*.whl`。

- [ ] **Step 9: 提交（引擎）**

```bash
git add -A crates docs PROGRESS.md Cargo.lock
git commit -m "feat(core,rl): 敌人池 dx/dy（本帧实际位移）进 Tier 0 敌人行 vx/vy（stride 38→46）；stg_rl 0.2.0"
```

---

### Task 5: 训练仓切到引擎的敌人速度

**Files:**
- Modify: `src/stgtrain/envwrap.py`（第 103–125 行删掉 `enemy_velocity`；约 383–386、415、468 行删掉 prev 状态；`_decode_enemies` 改为读字段；import 时断言版本）
- Delete: `tests/test_enemy_velocity.py` 中测试差分函数的用例；`test_v3_uses_enemy_velocity_in_closest_approach` 这类测特征化的用例保留，移到合适的测试文件
- Modify: `tests/test_envwrap.py`（`enemy_rows` 辅助函数、两个依赖差分的测试、第 139 行 `_fx` 探针里的 `(2,256,38)` 可以保留，它只测解码函数，与步长无关）
- Modify: `docs/experiments.md`（记一节）、`docs/perf-baseline.md`（待办第 3 条和 h2d 说明）

**Interfaces:**
- Consumes: `stg_rl` 0.2.0：`OFFSETS["enemies"]["vx"/"vy"]`、敌人行步长 46（通过 `stg_rl` 暴露的 stride 读取，不要写死）。
- Produces: `RawObs.enemies[..., 4:6]` 的语义变为引擎给出的本帧位移（px/帧），镜像时 vx 取负。

**本地环境**：先装 Task 4 打出的 wheel：
`cd /data/sunyunbo/www/stg-rl-train && uv pip install --reinstall /data/sunyunbo/www/stg-engine/dist/stg_rl-0.2.0-*.whl`
之后所有命令都用 `uv run --no-sync ...`，否则 `uv` 会按 lockfile 装回 0.1.1。**不要改 `pyproject.toml` 和 `uv.lock` 里的 wheel URL**：0.2.0 的 release 要由用户推 tag 生成，这一步在计划外，由主会话和用户确认后再做。

- [ ] **Step 1: 写失败的测试**

在 `tests/test_envwrap.py` 的「敌人表」一节（约 241 行起）加：

```python
def test_enemy_velocity_read_from_engine_fields_and_mirrored():
    """敌人 vx/vy 直接取 Tier 0 字段（Q16.16），镜像 env 的 vx 取负、vy 不变（Review Focus 5）。"""
    w = ring(mirror=False)
    n = w.n
    t = enemy_rows(n, [[(11, 10.0, 20.0, 8.0, True, 1.5, -0.25)]] * n)
    enemies, emask = w._decode_enemies(t, torch.full((n,), 1, dtype=torch.int32), 1)
    assert enemies[:, 0, 4:6].tolist() == [[1.5, -0.25]] * n
    assert emask[:, 0].all()
    # 镜像在 _decode 里做：直接走一次完整 step，看镜像 env 的符号
    w2 = ring(mirror=False)
    w2.reset()
    w2.mirrored[:] = torch.arange(w2.n) % 2 == 1
    raw = w2._copy_in()
    raw["enemies"] = enemy_rows(w2.n, [[(11, 10.0, 20.0, 8.0, True, 1.5, -0.25)]] * w2.n)
    raw["enemies_count"] = torch.full((w2.n,), 1, dtype=torch.int32)
    raw["emax"] = 1
    obs = w2._decode(raw)
    assert obs.enemies[0, 0, 4:6].tolist() == [1.5, -0.25]
    assert obs.enemies[1, 0, 4:6].tolist() == [-1.5, -0.25]
```

`enemy_rows`（同一文件约 244 行）要跟着改：步长改为 `stg_rl.STRIDES["enemies"]`；每行元组可选地多带两项 `(vx, vy)`，缺省为 0，写入 `off("vx")`、`off("vy")`（4 字节有符号小端）。`ring`、`_copy_in`、`_decode(raw)` 的实际签名以 `envwrap.py` 和本文件已有的用法为准；`_copy_in()` 返回的 dict 如果还包含别的键，照原样保留。

以下两个现有测试依赖被删除的差分状态，要改写：
- `test_stale_enemy_rows_never_match_and_are_zeroed`：去掉速度匹配相关的部分，只保留「count 之后的陈旧行清零、掩码只覆盖前 count 行」的断言；陈旧行里写上非零的 vx，并断言输出里被清零。
- `test_narrowed_enemy_decode_matches_full_width_reference`：参照值 `ref_v` 改为直接从全宽字节表解码 `vx/vy`（`_fx(t, _off("enemies", "vx"))` 等）再乘以 `live`；删掉 `enemy_velocity` 的 import 和 `_enemy_prev_valid`。

另外 `tests/test_envwrap.py` 里的端到端测试（真的建 `VecEnv` 跑几步的那些）会随新 wheel 自动覆盖新路径，不需要改。

- [ ] **Step 2: 运行，确认失败**

`uv run --no-sync pytest tests/test_envwrap.py -q -k engine_fields` → FAIL（当前实现还在差分，读出来是 0）。

- [ ] **Step 3: 实现**

- 文件顶部 import `stg_rl` 之后加：
  ```python
  _STG_RL_MIN = (0, 2, 0)   # 敌人行带 vx/vy（引擎第二刀）
  if tuple(int(x) for x in stg_rl.build_info()["version"].split(".")[:3]) < _STG_RL_MIN:
      raise ImportError(f"stg_rl >= 0.2.0 required (敌人速度字段), got {stg_rl.build_info()['version']}")
  ```
- 删掉 `enemy_velocity`、`TELEPORT_PX`（如果没有其他使用者；先 `grep -rn TELEPORT_PX src scripts tests eval`），删掉 `_prev_enemy_ids`、`_prev_enemy_xy`、`_enemy_prev_valid` 及其在 reset（约 415 行）和 done 处理（约 468 行）中的维护代码。
- `_decode_enemies`：删掉 `h2d_enemy_velocity` 计时段和 `eids`；`rows` 里的 `evel[..., 0], evel[..., 1]` 改为 `_fx(en, _off("enemies", "vx")), _fx(en, _off("enemies", "vy"))`。docstring 改为「速度由引擎给出（本帧实际位移，瞬移不计入）」，删掉关于差分的描述。
- 镜像那一行（`enemies[..., 4] *= sign`）保留。
- `grep -rn "enemy_velocity\|h2d_enemy_velocity" src scripts eval tests magnus`：删掉或更新剩余的引用（例如 perf 阶段名单）。

- [ ] **Step 4: 全量测试**

`uv run --no-sync pytest -q` → 全部 PASS（按 `pyproject.toml` / README 的默认选择；需要 GPU 的测试会自动跳过）。
再跑一次 CPU 冒烟训练，确认端到端能跑通：`uv run --no-sync python -m stgtrain.train --help` 找到冒烟入口（README 或 `run.sh` 里 CPU smoke 的写法），跑一次，预期正常结束。

- [ ] **Step 5: 文档**

- `docs/experiments.md` 末尾（「评测集沿革」之前，或者按文件的节次习惯）加一小节「观测语义变化：敌人速度改由引擎给出（2026-09-24，stg_rl 0.2.0）」，写明三处差异（出生帧有值、小于 16 px 的瞬移不再算作速度、`frame_skip > 1` 取最后一帧），`GRAPH_VERSION` 不变、旧 checkpoint 可加载，严格对比需要重训。
- `docs/perf-baseline.md`：「性能待办」第 3 条后注明 §2、§3、§5 与敌人速度已在 `stg_rl` 0.2.0 落地，A100 A/B 待测；表格中 h2d 的说明「敌人速度」改为「（0.2.0 起由引擎给出）」。

- [ ] **Step 6: 提交**

```bash
git add -A src tests docs
git commit -m "feat(envwrap): 敌人速度改读引擎 Tier 0 字段（stg_rl 0.2.0），删掉按 id 差分"
```
注意：`docs/experiments.md` 在开工前已经有用户未提交的改动（`git diff` 能看到第五至七节）。**不要把那些改动混进这次提交**：用 `git add -p docs/experiments.md` 只暂存本任务新增的那一节。如果做不到，就只提交 src/tests/perf-baseline，把 experiments 的这一节留给主会话处理。`scripts/diag/` 是未跟踪文件，不要动。
