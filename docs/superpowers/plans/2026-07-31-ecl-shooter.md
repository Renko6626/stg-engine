# ECL Shooter 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `fire`/`batch` 的整个发射参数集存成任务身上的可变状态，开火一句 `sh_fire(id)`。

**Architecture:** 每任务 4 个 shooter 槽，住 `TaskPool` 的并行数组（**不能住 `WorldBody`**——P1
明写 world 不知道"任务"存在）。15 个 syscall：14 个 setter + 1 个开火。开火循环住 ECL 层逐颗调
既有的 `create_bullet_with_xform`，故**对 world 层纯加法、零既有 API 改动、零 op 表改动**。

**Tech Stack:** Rust 1.94.0（edition 2024）/ `stg-core`（断层线以下，纯整数）/ `stg-ecl-compiler`

**设计依据：** [`docs/superpowers/specs/2026-07-31-ecl-shooter-design.md`](../specs/2026-07-31-ecl-shooter-design.md)
（十条人类裁定见其 §3，不得偏离）。

## Global Constraints

- **I1–I7**：`stg-core` 内不得 `f32`/`f64`/系统时钟/宿主 RNG/`HashMap`/`HashSet`；一切遍历按
  **池索引升序**（I4）；`World` 内无指针/堆容器，**快照 = 整块字节复制**（I7）。
- **P1（本刀的骨架约束）**：world 不 import 任何 ECL 类型、不知道"任务"存在。shooter 按任务键
  ⇒ **必须住 `TaskPool`**。开火时逐颗挂 `task_script` 也只能在 ECL 层做。
- **P4 三铁律**：(a) 资源耗尽→确定性降级不 panic + 计数；(b) 调用方违约→安全结果 + 计数；
  (c) 引擎自身 bug→debug 帧内断言。
- **复用槽写满**：`TaskPool::spawn` 必须把新任务的 4 个 shooter 写成默认值——这撑着
  "校验和哈希全槽不掩码"那条纪律。
- **十条人类裁定**（spec §3）中最容易被实现者"顺手改好"的四条：
  - **D-6**：ZUN 九值 aimmode 塌成 `aimed`/`ring` 两个布尔；**随机模式 6/7/8 不做**。
  - **D-7**：**fan 以基准方向为中心对称展开**（默认语义，非可选）；**ring 不居中**。
  - **D-8**：`sh_fire` **无返回值**。
  - **D-9**：**不做** `etCopy`。
- **syscall 号表 append-only**：61 已占（`SYS_DIE`，敌人死亡效果刀），本刀占 **62–76**。
- **`ENGINE_VER` 4 → 5**，在 **T1** bump（布局在 T1 就变了——上一刀的教训是 bump 拖到最后会让
  中间几个 commit 处于"存档格式变了而身份三元组没变"）。
- **金向量逐字节不变**（纯新增，无脚本调用）——**两个方向都要验**（"该同的同"，只验一个方向
  等于没验）。
- 每个任务结束前必须全绿：
  ```bash
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- **commit 结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **分支**：`feat/ecl-shooter`（从 `main` 开）。

### 金向量对拍的正确做法

仓库**无 committed 金向量基线**（CLAUDE.md「金向量闸门的能力边界」）——`determinism-gate`
只把三平台的流互相比。本地对拍只能靠 worktree 副本：

```bash
git worktree add /tmp/sh-base <本任务的 base commit>
(cd /tmp/sh-base && cargo run -q -p stg-harness -- golden --out /tmp/g-base.txt)
cargo run -q -p stg-harness -- golden --out /tmp/g-head.txt
diff -q /tmp/g-base.txt /tmp/g-head.txt
git worktree remove /tmp/sh-base
```

**绝不**在主 checkout 上 `git stash` 或 `git checkout <file>` 去拿 base——本仓吃过亏
（`git checkout` 清临时代码时把实现一起撤了）。

## 对 spec 的一处补充（写计划时查证，spec 未覆盖）

**xform 数据住调用任务的 `locals`，而 shooter 要延迟到开火时才读——这是安全的，理由如下，
但必须配测试钉住。**

`fire` 的 xform 不是全局表，而是 `task.locals[xform_off .. xform_off + cnt*3]`
（`syscall.rs::sys_create_bullet` 的 xform 解包段）。数据由编译器的
`codegen::gen_xformdef_staging` 在 **sub 入口一次性**写入（"入口直排，先于 loop 回跳点"），
区间由 `slots::allocate` 的调用图着色分配。

- 着色保证**同时活跃的 sub 拿到不相交的 locals**，故被调 sub 不会踩掉调用方的 xform 区。
- shooter 是**每任务**的，`sh_xform` 与 `sh_fire` 必在同一任务 ⇒ 同一个 `locals` 数组。
- 结论：存 `(off, cnt)`、开火时读，**安全**。不必把 16 个槽（192 B）拷进 shooter——那是槽宽的
  四倍多。

**T3 要加一条测试钉死它**：`sh_xform` 之后 `wait` 跨帧再 `sh_fire`，xform 仍生效。

## File Structure

| 文件 | 责任 | 任务 |
|---|---|---|
| `crates/stg-core/src/ecl/shooter.rs` | **新建**：`Shooter` 结构 + 默认值 + 常量 + `flags` 位 | T1 |
| `crates/stg-core/src/ecl/mod.rs` | 挂 `pub mod shooter;` | T1 |
| `crates/stg-core/src/ecl/task.rs` | `TaskPool` 加并行数组；`spawn` 重置 4 个槽 | T1 |
| `crates/stg-core/src/step.rs` | 世界尺寸哨兵 | T1 |
| `crates/stg-core/src/lib.rs` | `ENGINE_VER` 4→5 | T1 |
| `crates/stg-core/src/ecl/syscall.rs` | 号表 62–76 + 14 个 setter 派发（T2）+ `sh_fire`（T3） | T2/T3 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | 15 个表层内建 | T2/T3 |
| `docs/*` `PROGRESS.md` | 文档收口 | T4 |

---

### Task 1: `Shooter` 结构 + `TaskPool` 存储 + `ENGINE_VER`

**Files:**
- Create: `crates/stg-core/src/ecl/shooter.rs`
- Modify: `crates/stg-core/src/ecl/mod.rs`（挂模块）
- Modify: `crates/stg-core/src/ecl/task.rs`（`TaskPool` 字段 + `new` + `spawn`）
- Modify: `crates/stg-core/src/step.rs`（哨兵）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER`）

**Interfaces:**
- Consumes: `crate::math::{Fx, Angle}`（均 `repr(transparent)`）、`crate::checksum::Checksum`
  与 `crate::save::SaveBytes`（两者都有 `impl<T, const N: usize> for [T; N]` 泛型实现，故
  `[[Shooter; 4]; 256]` 自动生效，无需手工同步）
- Produces:
  - `pub struct Shooter`（`repr(C)`，**44 B**）+ `impl Default`
  - `pub const SHOOTERS_PER_TASK: usize = 4;`
  - `pub const SH_AIMED: u8 = 1 << 0;` / `SH_RING: u8 = 1 << 1;` / `SH_ABS_OFFSET: u8 = 1 << 2;`
  - `pub const SH_NO_TASK: u16 = 0xFFFF;`（哨兵，与既有 `xform::XFORM_NONE = 0xFFFF` 同惯例）
  - `TaskPool.shooters: [[Shooter; SHOOTERS_PER_TASK]; TASK_CAP]`

- [ ] **Step 1: 写失败测试**

新建 `crates/stg-core/src/ecl/shooter.rs`，先只写 `mod tests`（结构还没有，编译必红）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// 槽宽是容量账的一部分（spec §4.2：44 B × 4 × 256 = 45056 B）。变了就要重算预算表。
    #[test]
    fn shooter_is_44_bytes() {
        assert_eq!(core::mem::size_of::<Shooter>(), 44);
    }

    /// 默认值：`n_angle`/`n_speed` 是 **1×1** 而不是 0——刚重置的 shooter 开火发**一颗**弹，
    /// 是个有意义的退化，不是"什么也不发"这种要 debug 半天的静默（spec §5）。
    #[test]
    fn shooter_default_fires_exactly_one_bullet() {
        let s = Shooter::default();
        assert_eq!((s.n_angle, s.n_speed), (1, 1), "默认 1×1,不是 0");
        assert_eq!(s.flags, 0, "aimed/ring/abs_offset 三位全关");
        assert_eq!(s.xform_cnt, 0, "无 xform");
        assert_eq!(s.task_script, SH_NO_TASK, "无挂弹任务");
        assert_eq!(s.on_fire_req, 0, "不发请求");
        assert_eq!((s.off_x, s.off_y), (Fx::ZERO, Fx::ZERO));
        assert_eq!(s.polar_r, Fx::ZERO);
        assert_eq!(s.dist, Fx::ZERO);
        assert_eq!((s.speed0, s.speed_step), (Fx::ZERO, Fx::ZERO));
    }
}
```

在 `crates/stg-core/src/ecl/task.rs` 的 `mod tests` 加：

```rust
    /// 「复用槽写满」纪律：`spawn` 必须把新任务的 4 个 shooter 写成默认值。
    /// 判别腿是**先脏后建**——直接查一个刚 new 出来的池只能证明"全零构造对"，
    /// 证不了"复用时会重置"。
    #[test]
    fn spawn_resets_all_shooters_of_the_reused_slot() {
        let mut p = TaskPool::new();
        let h = p.spawn(SubId::new(0), 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();
        // 弄脏这个槽的全部 4 个 shooter
        for s in p.shooters[h as usize].iter_mut() {
            s.n_angle = 99;
            s.flags = 0xFF;
            s.dist = Fx::from_int(7);
        }
        p.kill(h as usize);
        // 复用同一个槽（最低空位分配 ⇒ 必然是它）
        let h2 = p.spawn(SubId::new(0), 0, (OWNER_STAGE, 0, 0), 0, 0).unwrap();
        assert_eq!(h2, h, "最低空位分配应复用同一槽");
        for (k, s) in p.shooters[h2 as usize].iter().enumerate() {
            assert_eq!(*s, Shooter::default(), "槽 {k} 未被重置为默认值");
        }
    }
```

> `SubId::new` / `kill` 的确切签名以 `image.rs` / `task.rs` 现有代码为准；照该文件既有测试
> 的构造惯例写。`Shooter` 需要 `PartialEq + Debug` 才能这么断言——加进 derive。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core shooter`
Expected: 编译失败——`cannot find type Shooter`。

- [ ] **Step 3: 写 `Shooter` 结构**

`crates/stg-core/src/ecl/shooter.rs`（放在 `mod tests` 之前）：

```rust
//! Shooter（预存发射参数集）——ECL 层的每任务发射器状态（shooter 刀 2026-07-31）。
//!
//! 参照 ZUN ECL 的 `et*` 族弹幕管理器（600-641）：`sh_reset` 重置编号槽 → 一堆以 `id` 打头的
//! setter 逐项配 → `sh_fire(id)` 开火。改一个字段再开一次火就是下一波。
//!
//! **为什么住 ECL 层而不是 `WorldBody`**：P1 明写 world 不 import 任何 ECL 类型、不知道
//! "任务"存在。shooter 按任务键 ⇒ 天然是 ECL 层的东西。存储挂在 `TaskPool` 的并行数组上，
//! 顺带白捡"槽复用时的重置有现成挂点"（`TaskPool::spawn`）。
//!
//! **字段顺序按 4 字节对齐紧排**（`Fx` 在前、`u16` 居中、`u8` 收尾），`repr(C)` 下恰好 44 B。
//! 改字段顺序会改槽宽 → 改快照尺寸 → 改存档格式，动前先看 `size_of` 那条测试。

use crate::math::{Angle, Fx};

/// 每任务的 shooter 槽数（spec D-1）。脚本用 `id ∈ 0..SHOOTERS_PER_TASK`。
/// 4 是"一个 boss 典型并发 2-3 种弹（主环/点射/收尾）再留一格"的取值；
/// 容量账：44 B × 4 × 256 = 45056 B，见 spec §4.2。
pub const SHOOTERS_PER_TASK: usize = 4;

/// `flags` 位：`angle0` 是**相对自机方向的偏移**（而非绝对方向）。
pub const SH_AIMED: u8 = 1 << 0;
/// `flags` 位：`n_angle` 颗**自动均分整周**，`angle_step` 转义成**逐层**偏移；
/// 否则 fan（`angle_step` 逐弹、且**以基准方向为中心**对称展开）。
pub const SH_RING: u8 = 1 << 1;
/// `flags` 位：`off_x/off_y` 是**绝对**坐标（而非相对 owner）。
pub const SH_ABS_OFFSET: u8 = 1 << 2;

/// `task_script` 的"无挂弹任务"哨兵——与既有 `crate::xform::XFORM_NONE` 同惯例。
pub const SH_NO_TASK: u16 = 0xFFFF;

#[repr(C)]
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, crate::checksum::Checksum, crate::save::SaveBytes,
)]
pub struct Shooter {
    pub off_x: Fx,
    pub off_y: Fx,
    /// 极坐标偏移的半径；与 `off_x/off_y` **永远叠加**，不存在覆盖关系（ZUN 626 明写 stacks）。
    pub polar_r: Fx,
    /// 出生后沿**各自角度**推出去的距离（ZUN 627）——逐颗方向不同，不是整体平移。
    pub dist: Fx,
    pub speed0: Fx,
    pub speed_step: Fx,
    /// 形 × color_stride + 色，折叠值（同 `fire`/`batch` 的颜色轴糖）。
    pub appearance: u16,
    pub polar_ang: Angle,
    pub angle0: Angle,
    pub angle_step: Angle,
    /// 指向**本任务 `locals`** 的 xform 区间起点（见计划「对 spec 的一处补充」）。
    pub xform_off: u16,
    /// `SH_NO_TASK` = 不挂。
    pub task_script: u16,
    /// 开火后要发的通道 B 请求 id；`0` = 不发（ZUN 608 的 sound1 归并进通道 B，spec §8）。
    pub on_fire_req: u16,
    pub n_angle: u8,
    pub n_speed: u8,
    /// `0` = 无 xform。
    pub xform_cnt: u8,
    pub flags: u8,
}

impl Default for Shooter {
    fn default() -> Self {
        Shooter {
            off_x: Fx::ZERO,
            off_y: Fx::ZERO,
            polar_r: Fx::ZERO,
            dist: Fx::ZERO,
            speed0: Fx::ZERO,
            speed_step: Fx::ZERO,
            appearance: 0,
            polar_ang: Angle::from_raw(0),
            angle0: Angle::from_raw(0),
            angle_step: Angle::from_raw(0),
            xform_off: 0,
            task_script: SH_NO_TASK,
            on_fire_req: 0,
            // **1×1 而不是 0**：刚重置的 shooter 开火发一颗弹,是个有意义的退化,
            // 不是"什么也不发"这种要 debug 半天的静默。
            n_angle: 1,
            n_speed: 1,
            xform_cnt: 0,
            flags: 0,
        }
    }
}
```

> `Angle::from_raw` 的确切构造名以 `math/angle.rs` 为准；若它有 `Angle::ZERO` 就用那个。

`crates/stg-core/src/ecl/mod.rs` 加 `pub mod shooter;`（位置按该文件既有的模块声明字母序）。

- [ ] **Step 4: `TaskPool` 加并行数组 + `spawn` 重置**

`crates/stg-core/src/ecl/task.rs`：

```rust
pub struct TaskPool {
    pub(crate) slots: [Task; TASK_CAP],
    /// 每任务 4 个发射器槽（shooter 刀 2026-07-31）。**并行数组而非塞进 `Task`**——
    /// `Task` 的 `repr(C)` 布局是精算过的（`spell_bound` 卡在对齐间隙里以免改 `size_of`），
    /// 不该为这个塞 176 B 进去。
    /// `spawn` 负责重置（复用槽写满纪律，撑"校验和哈希全槽不掩码"）。
    pub(crate) shooters: [[Shooter; SHOOTERS_PER_TASK]; TASK_CAP],
    pub(crate) alive: [u64; TASK_CAP / 64],
}
```

`new()` 里加 `shooters: [[Shooter::default(); SHOOTERS_PER_TASK]; TASK_CAP],`。

`spawn()` 里，在写完 `self.slots[idx] = Task { .. }` 之后加：

```rust
                // 复用槽写满：新任务的发射器一律回默认值。漏这步会让上一个任务的发射器
                // 参数泄漏给新任务——而且因为校验和哈希全槽,泄漏值还会进校验和。
                self.shooters[idx] = [Shooter::default(); SHOOTERS_PER_TASK];
```

顶部加 `use crate::ecl::shooter::{Shooter, SHOOTERS_PER_TASK};`。

- [ ] **Step 5: 跑测试确认两条都过**

```bash
cargo test -p stg-core -- shooter_is_44_bytes shooter_default_fires spawn_resets_all_shooters
```

`shooter_is_44_bytes` 若不是 44，**不要改断言去迁就**——回去调字段顺序（`Fx` 在前、`u16` 居中、
`u8` 收尾）。槽宽是容量账的一部分。

- [ ] **Step 6: 世界尺寸哨兵 + `ENGINE_VER`**

`crates/stg-core/src/step.rs` 的 `const EXPECTED: (usize, usize)`（两处，debug/release 各一）：
当前是 `(970144, 1084888)`。shooter 加在 `TaskPool` ⇒ **`WorldBody` 不变、`World` 增
45056**：新值 `(970144, 1129944)`。

> **实测为准**：跑一次拿真值，若与 1129944 不符，说明 `Shooter` 不是 44 B 或有对齐吸收，
> **停下来查清楚再改**，不要直接把实测值填进去了事（哨兵存在的意义就是逼人算一遍）。
> 更新时按该测试既有注释的体例，把"为什么 +45056"写清。

`crates/stg-core/src/lib.rs` 的 `ENGINE_VER` **4 → 5**，理由注释写**两条**：
① syscall 号表新增 62–76（T2/T3 落地）；② `TaskPool` 布局变更导致 `SaveBytes` 载荷编码变化
（**本任务就变了**）。`step.rs` 的 `engine_ver_anchored` 锚点测试同步。

> 为什么在 T1 就 bump：布局在**本任务**就变了。上一刀（敌人死亡效果）把 bump 拖到第三步，
> 结果中间几个 commit 处于"存档格式变了而身份三元组没变"——这次提前。

- [ ] **Step 7: 全绿 + 金向量对拍**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

金向量：**预期逐字节不变**。shooter 全是默认值、没有任何脚本读写它，且 `Checksum` 对
全零/默认数组的哈希只影响"世界校验和"这个数——**等等，它会变**：校验和哈希全槽，新增
45056 B 的默认值参与哈希 ⇒ **校验和流必然整体平移**。

**所以本任务的金向量预期是"会漂，且只因新增字段进了校验和"**，与行为无关。对拍确认它**确实
不同**（"该差的差"），并在报告里写清归因。行为无变化由"没有任何代码读写 shooter"这一事实
保证——本任务连 syscall 都还没有。

- [ ] **Step 8: 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): Shooter 结构 + TaskPool 每任务 4 槽存储 + ENGINE_VER 4→5

预存发射参数集(参照 ZUN et* 族弹幕管理器)的存储底座。本刀第 1 步:只有存储与重置,
还没有任何 syscall。

**住 ECL 层而非 WorldBody**:P1 明写 world 不 import 任何 ECL 类型、不知道"任务"存在,
shooter 按任务键 ⇒ 天然是 ECL 层的东西。挂 TaskPool 并行数组(不塞进 Task——它的 repr(C)
布局是精算过的),顺带白捡"槽复用重置有现成挂点"。

槽宽 44 B(字段按 4 字节对齐紧排),×4×256 = 45056 B。默认 n_angle/n_speed = **1×1** 而非 0:
刚重置的 shooter 开火发一颗弹,是有意义的退化,不是"什么也不发"这种要 debug 半天的静默。

复用槽写满有判别腿:测试**先弄脏再 kill 再 spawn**——只查刚 new 的池证不了"复用时会重置"。

ENGINE_VER 在**本步**就 bump,理由两条:syscall 号表将新增 62-76 + TaskPool 布局变更致
SaveBytes 载荷编码变化(**本步就变了**)。上一刀把 bump 拖到最后,中间几个 commit 处于
"存档格式变了而身份三元组没变",这次提前。

金向量**会漂**:校验和哈希全槽,新增 45056 B 默认值参与哈希 ⇒ 流整体平移。与行为无关——
本步没有任何代码读写 shooter。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 14 个 setter（syscall 62–75）+ 表层内建

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（号表 + 派发 + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（14 个内建 + 两处穷举名单）

**Interfaces:**
- Consumes: T1 的 `Shooter` / `SHOOTERS_PER_TASK` / `SH_AIMED` / `SH_RING` / `SH_ABS_OFFSET` /
  `SH_NO_TASK`；`TaskPool.shooters`
- Produces: `SYS_SH_RESET = 62` … `SYS_SH_REQ = 75`（下表）；表层 `sh_reset` … `sh_req`

| 号 | 内建 | 参数 | 写什么 |
|---|---|---|---|
| 62 | `sh_reset(id)` | — | 整槽 = `Shooter::default()` |
| 63 | `sh_sprite(id, shape, color)` | 两参**折叠**（同 `fire`） | `appearance` |
| 64 | `sh_offset(id, x, y)` | fx, fx | `off_x/off_y`，**清** `SH_ABS_OFFSET` |
| 65 | `sh_offset_abs(id, x, y)` | fx, fx | `off_x/off_y`，**置** `SH_ABS_OFFSET` |
| 66 | `sh_offset_rad(id, angle, r)` | angle, fx | `polar_ang/polar_r` |
| 67 | `sh_dist(id, d)` | fx | `dist` |
| 68 | `sh_angle(id, angle0, step)` | angle, angle | `angle0/angle_step` |
| 69 | `sh_speed(id, speed0, step)` | fx, fx | `speed0/speed_step` |
| 70 | `sh_count(id, n_angle, n_speed)` | int, int | `n_angle/n_speed`（钳 `[0,255]`） |
| 71 | `sh_aim(id, on)` | int | `SH_AIMED` 位 |
| 72 | `sh_ring(id, on)` | int | `SH_RING` 位 |
| 73 | `sh_xform(id, xf)` | XformRef | `xform_off/xform_cnt` |
| 74 | `sh_task(id, sub)` | SubRef | `task_script` |
| 75 | `sh_req(id, req_id)` | int | `on_fire_req`（钳 `[0, u16::MAX]`） |

- [ ] **Step 1: 写失败测试**

在 `syscall.rs` 的 `mod tests` 加。**五条**：

```rust
    /// 每个 setter 只改自己那一维,其余字段纹丝不动。
    /// 这条守的是"14 个派发臂没有互相串写"——串写在只测单个 setter 的测试里看不出来。
    #[test]
    fn each_shooter_setter_touches_only_its_own_field() {
        // 逐个调 setter,每次调完与 Shooter::default() 逐字段比,
        // 断言**恰好**只有该 setter 负责的字段变了。
        // (用一个 (调用, 期望差异) 的表驱动写法,避免 14 段复制粘贴。)
    }

    /// `sh_offset` 与 `sh_offset_abs` 写的是同一对字段,只是解释方式不同——**后写的赢**。
    /// 判别腿:先 abs 后 rel,`SH_ABS_OFFSET` 必须被**清掉**（不是只置不清）。
    #[test]
    fn offset_abs_flag_is_set_and_cleared_by_the_two_setters() {
        // sh_offset_abs(0, 10fx, 20fx) → flags & SH_ABS_OFFSET != 0
        // sh_offset(0, 30fx, 40fx)     → flags & SH_ABS_OFFSET == 0,且 off 已是 (30,40)
    }

    /// `sh_reset` 恢复**全部**默认（不只是清几个字段）。
    #[test]
    fn sh_reset_restores_every_field_to_default() {
        // 把 14 个 setter 全调一遍弄脏,再 sh_reset,断言整槽 == Shooter::default()
    }

    /// P4-b：`id ≥ SHOOTERS_PER_TASK` → no-op + contract_viol + STATUS_BAD_ARGS。
    /// **14 个 setter 各一腿**（表驱动）——只测其中一个的话，漏写边界检查的那几个不会红。
    #[test]
    fn every_shooter_setter_rejects_out_of_range_id() {
        // 对每个 setter：调 id = SHOOTERS_PER_TASK（恰好越界）与 id = -1，
        // 断言 contract_viol 每次 +1、last_status == STATUS_BAD_ARGS、
        // 且**四个槽全都没被改动**（no-op 的判别腿——只查 contract_viol 证不了没写脏）。
    }

    /// P4-b：`sh_count` 的参数是脚本给的任意 i32,先钳后存,不回绕不 panic。
    /// (n=-5 → 0；n=i32::MAX → 255)
    #[test]
    fn sh_count_clamps_both_ends() { /* ... */ }
```

> 世界/镜像构造照抄本模块既有 syscall 测试的惯例（`World::new` + 手拼 `Task` + `dispatch`）。
> **表驱动写法很重要**——14 个 setter 各写一段复制粘贴的测试，既臃肿又容易漏掉新加的那个。

- [ ] **Step 2: 跑测试确认失败**（符号不存在 → 编译错）

- [ ] **Step 3: 号表 + 一个取槽助手**

`syscall.rs` 号表（5x/6x 族尾部，61 = `SYS_DIE` 已占）：

```rust
// ── Shooter：预存发射参数集（62-76；shooter 刀 2026-07-31，参照 ZUN et* 族 600-641）──
//
// 每任务 4 个编号槽。`id ≥ SHOOTERS_PER_TASK` 一律 no-op + `contract_viol`（P4-b）——
// 不 Fault，因为"槽号写错"是常见笔误而非结构性违约，降级比杀任务更有用。
//
// **`sh_fire`(76) 的语义见其自身文档**；下面 14 条都只是写字段，无副作用。
pub const SYS_SH_RESET: u16 = 62;
pub const SYS_SH_SPRITE: u16 = 63;
pub const SYS_SH_OFFSET: u16 = 64;
pub const SYS_SH_OFFSET_ABS: u16 = 65;
pub const SYS_SH_OFFSET_RAD: u16 = 66;
pub const SYS_SH_DIST: u16 = 67;
pub const SYS_SH_ANGLE: u16 = 68;
pub const SYS_SH_SPEED: u16 = 69;
pub const SYS_SH_COUNT: u16 = 70;
pub const SYS_SH_AIM: u16 = 71;
pub const SYS_SH_RING: u16 = 72;
pub const SYS_SH_XFORM: u16 = 73;
pub const SYS_SH_TASK: u16 = 74;
pub const SYS_SH_REQ: u16 = 75;
```

取槽助手（14 个派发臂共用，P4-b 集中在一处）：

```rust
/// 取本任务的第 `id` 个 shooter（可变）。`id` 越界 → `None` + `contract_viol` + `BAD_ARGS`
/// （P4-b：no-op，不 Fault）。`id` 是脚本给的任意 `i32`，负数与超界同处置。
fn shooter_mut<'a>(id: i32, task_index: u16, ctx: &'a mut VmCtx) -> Option<&'a mut Shooter> {
    if id < 0 || id as usize >= SHOOTERS_PER_TASK {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return None;
    }
    Some(&mut ctx.tasks.shooters[task_index as usize][id as usize])
}
```

> `ctx.self_index` 是本任务的池索引（`vm.rs` 的 `VmCtx` 字段）。派发臂里用它。
> 若借用检查器对 `ctx.body` 与 `ctx.tasks` 同时可变借用有意见，把 `contract_viol` 的记账
> 拆到助手外层做——**不要**改成 Fault 来绕开借用问题。

- [ ] **Step 4: 14 个派发臂**

形状统一（**参数逆序弹出**，照 `sys_move_enemy_to`）。三个代表，其余照此写：

```rust
        SYS_SH_RESET => {
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                *s = Shooter::default();
            }
            Ok(())
        }
        SYS_SH_OFFSET => {
            let y = pop(task)?;
            let x = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                s.off_x = Fx::from_raw(x);
                s.off_y = Fx::from_raw(y);
                s.flags &= !SH_ABS_OFFSET; // 相对模式：清标志（与 sh_offset_abs 互为反向）
            }
            Ok(())
        }
        SYS_SH_COUNT => {
            let n_speed = pop(task)?;
            let n_angle = pop(task)?;
            let id = pop(task)?;
            if let Some(s) = shooter_mut(id, ctx.self_index, ctx) {
                // P4-b：脚本给的任意 i32，先钳进 [0,255] 再存（裸 `as u8` 会让负数回绕）。
                s.n_angle = n_angle.clamp(0, u8::MAX as i32) as u8;
                s.n_speed = n_speed.clamp(0, u8::MAX as i32) as u8;
            }
            Ok(())
        }
```

要点：

- `sh_sprite`：两参在**编译期**已被 codegen 折叠成单个 appearance 值（同 `fire`——见
  `builtins::folds_shape_color`），所以派发臂只 `pop` 出**一个** appearance。
- `sh_offset_abs`：同 `SYS_SH_OFFSET` 但 `s.flags |= SH_ABS_OFFSET;`。
- `sh_aim` / `sh_ring`：`if on != 0 { s.flags |= BIT } else { s.flags &= !BIT }`。
- `sh_xform`：表层 `XformRef` 参在 codegen 里降低成 `(off, cnt)` **两个**栈值（见
  `gen_builtin_call` 的 `ParamKind::XformRef` 分支），故派发臂 `pop` 两个。`cnt == 0` 即无 xform。
  **此处不校验区间**——校验在开火时做（与 `fire` 同口径：`fire` 也是在建弹那一刻才查
  `LOCALS` 边界）。
- `sh_task`：表层 `SubRef` 降低成一个栈值（sub 号或 -1）。`< 0` → 存 `SH_NO_TASK`；
  否则存 `u16::try_from`。**此处不校验号是否在册**——与 `fire` 的"先验后建"不同，
  因为设的时候还没建弹；校验在开火时做（T3）。
- `sh_req`：`req_id.clamp(0, u16::MAX as i32) as u16`。

- [ ] **Step 5: 14 个表层内建**

`builtins.rs` 照 `add_score` / `fire` 的形状加。三个代表：

```rust
    // ── Shooter：预存发射参数集（syscall 62-75；参照 ZUN et* 族）─────────────
    Builtin {
        name: "sh_reset",
        syscall: syscall::SYS_SH_RESET,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "重置发射器槽 id 为默认(1×1 单发、无 xform/挂弹任务/请求)",
        param_names: &["id"],
    },
    Builtin {
        name: "sh_sprite",
        syscall: syscall::SYS_SH_SPRITE,
        is_op: false,
        // 颜色轴糖：表层 shape/color 两参，codegen 折叠成单个 appearance（同 fire/batch）。
        params: &[Val(Int), Val(Int), Val(Int)],
        ret: None,
        doc: "设发射器的弹型与颜色;查外观表(越界/空格 编译期或 Fault)",
        param_names: &["id", "shape", "color"],
    },
    Builtin {
        name: "sh_xform",
        syscall: syscall::SYS_SH_XFORM,
        is_op: false,
        params: &[Val(Int), Xf],
        ret: None,
        doc: "给发射器挂 xformdef(名或 none);开火时每颗弹都带上",
        param_names: &["id", "xf"],
    },
```

**`sh_sprite` 要进 `folds_shape_color` 的名单**（`builtins.rs` 里那个判据函数）——否则两参
不会被折叠，派发臂 `pop` 出来的就不是 appearance。**注意折叠位置**：`fire`/`batch` 的折叠发生在
**前两参**，而 `sh_sprite` 的是**第 2、3 参**（第 1 参是 `id`）。看 `codegen::gen_builtin_call`
里 `folds_shape_color && i == 0` 那个判据——它硬编码了"下标 0"。**这里需要改**：把判据从
"名字在名单里且 i==0"改成"名字在名单里且 i == 该内建的折叠起始下标"，起始下标由一个
`fold_start(name) -> usize` 给出（`fire`/`batch` 返 0、`sh_sprite` 返 1）。改完 `fire`/`batch`
的行为必须一字不变——**加一条测试钉住**（编译 `fire` 与 `batch` 的产物与改动前逐字节相同）。

并更新 `builtins.rs` 里**两处穷举名单断言**（`lookup_finds_every_documented_builtin_by_name`
的 `names` 数组、`void_builtins_have_none_return_type` 的名单），14 个名字都加。

- [ ] **Step 6: 重跑元数据生成器**

```bash
cargo run -p stg-harness -- gen-ecl-meta
```
改了 `builtins.rs` 必须同步，否则两条防漂移测试红。核对 diff 只有这 14 个新内建的签名。
**手写语义节归 T4，本任务不写。**

- [ ] **Step 7: 跑测试确认五条全过 + 全绿 + 金向量对拍**

金向量**必须逐字节不变**——纯新增 syscall，无脚本调用。**差了就是有问题**（尤其要警惕
Step 5 那个折叠下标的改动误伤了 `fire`/`batch`），要查清而不是接受。

- [ ] **Step 8: 提交**

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): shooter 的 14 个 setter(syscall 62-75) + 表层内建

配置面齐活,还不能开火(sh_fire 在下一刀)。参数逆序弹出,照 sys_move_enemy_to。
id 越界一律 no-op + contract_viol(P4-b,不 Fault——槽号写错是常见笔误而非结构性违约)。

sh_offset 与 sh_offset_abs 写同一对字段、只是解释方式不同,**后写的赢**;判别腿钉死
"先 abs 后 rel 必须把标志清掉",防实现成只置不清。

颜色轴糖的折叠下标从硬编码的 0 改成按内建查——fire/batch 折叠前两参,而 sh_sprite 的
是第 2、3 参(第 1 参是 id)。配测试钉死 fire/batch 的产物逐字节不变。

四条 P4/判别测试全是**表驱动**的:14 个 setter 各写一段复制粘贴既臃肿又容易漏掉新加的。

金向量逐字节不变(纯新增,无脚本调用)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `sh_fire`（syscall 76）——开火七步

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`SYS_SH_FIRE = 76` + `sys_sh_fire` + 测试）
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`（`sh_fire` 内建 + 两处名单）

**Interfaces:**
- Consumes: T1/T2 的全部；`WorldBody::create_bullet_with_xform` / `create_bullet`；
  `math::geom::polar_to_vec`；`math::cordic::atan2`；`self_pos`（`syscall.rs` 的 owner 位置解析）；
  `WorldBody::emit_req`
- Produces: `SYS_SH_FIRE: u16 = 76`；表层 `sh_fire(id)`（**无返回值**）

- [ ] **Step 1: 写失败测试**

**最硬的一条**（带居中补偿的等价）：

```rust
    /// shooter 的 fan 与手写 `batch` 等价——**带居中补偿**。
    /// 一次押住网格序、坐标算法、速度递增、**以及居中公式本身**。
    ///
    /// 为什么必需：开火循环住 ECL 层（P1 使然，见 spec §10 末），网格逻辑有了第二份实现。
    /// 补偿写错、或实现忘了居中,这条都会红。
    #[test]
    fn shooter_fan_matches_batch_with_centering_compensation() {
        // 世界 A：shooter，n_angle=5, n_speed=3, angle0=base, angle_step=step,
        //         speed0/speed_step 若干，其余全默认（无 aim/ring/dist/polar/xform/task）
        // 世界 B：batch(shape, color, x, y, 5, base - (5-1)*step/2, step, 3, speed0, speed_step)
        // 断言两个世界的 bullets 池**逐字段相同**（x/y/vx/vy/speed/angle/sprite/radius 全比）。
        // 用 iter_alive() 按池序取，两边逐一比。
    }
```

三条判别腿：

```rust
    /// `dist` 是**逐颗沿各自角度**位移,不是整环朝同一方向平移。
    /// 判别腿：取一个 n_angle=4、angle_step=90deg 的十字环,四颗弹的位移方向必须两两不同;
    /// 错误实现（整体平移）会让四颗的 (x,y) 相对无 dist 时的偏移量**完全相同**。
    #[test]
    fn dist_pushes_each_bullet_along_its_own_angle() { /* ... */ }

    /// `sh_offset` 与 `sh_offset_rad` 同时设 → 两者**相加**（ZUN 626 明写 stacks）。
    /// 判别腿：只设其一 / 只设另一 / 两个都设，第三种的原点必须等于前两种偏移量之和。
    #[test]
    fn rect_and_polar_offsets_stack_rather_than_override() { /* ... */ }

    /// `aim` 在**开火那一刻**解析,不是 `sh_aim` 时。
    /// 判别腿：sh_aim(0,1) → 移动自机 → sh_fire，基准角必须跟着新位置变。
    /// 错误实现（设的时候就把角算死）会让两次开火的角相同。
    #[test]
    fn aim_resolves_at_fire_time_not_at_set_time() { /* ... */ }
```

其余六条：

```rust
    /// ring 精确闭合：n_angle=28 时 28 颗**铺满整 65536**、收尾无缝。
    /// 判别腿：相邻两颗的角差之和 == 65536（不是 28×(65536/28)=65520）。
    /// 防照抄 demo 的 `65536/n` 预乘写法。
    #[test]
    fn ring_distributes_exactly_around_the_full_circle() { /* ... */ }

    /// ring 模式下 `angle_step` 是**逐层**偏移（而非逐弹）——两层的同序号弹角差 == angle_step。
    #[test]
    fn ring_angle_step_offsets_layers_not_bullets() { /* ... */ }

    /// xform 延迟读安全（见计划「对 spec 的一处补充」）：
    /// `sh_xform` 之后 **wait 跨帧** 再 `sh_fire`，xform 仍生效。
    #[test]
    fn xform_survives_a_frame_boundary_between_set_and_fire() { /* ... */ }

    /// 挂弹任务：每颗弹各派一个任务，owner = (BULLET, 该弹 index/gen)。
    /// P4-a：任务池满 → 弹**保留**、pool_full[POOL_TASK] 计数（同 fire 口径）。
    #[test]
    fn sh_task_spawns_one_task_per_bullet_and_degrades_when_full() { /* ... */ }

    /// `on_fire_req`：发一条请求，`args[3]` 是**实际**创建数而非请求数。
    /// 判别腿：把弹池灌到只剩 2 格再发 5 颗的环，args[3] 必须是 2。
    #[test]
    fn on_fire_req_reports_actual_count_not_requested() { /* ... */ }

    /// P4-b：n_angle 或 n_speed 为 0、或乘积超弹池 CAP → 不发 + contract_viol
    /// （对齐 `create_bullets_batch` 的既有口径）。
    #[test]
    fn sh_fire_rejects_degenerate_grid() { /* ... */ }
```

- [ ] **Step 2: 跑测试确认失败**（`SYS_SH_FIRE` 不存在 → 编译错）

- [ ] **Step 3: 实现 `sys_sh_fire`**

号表：

```rust
/// 开火（76）：用发射器槽 `id` 的参数造弹。**无返回值**（人类裁定 D-8——本语言要求值必须
/// 消费，有返回值就得写 `_ = sh_fire(0);`，而开火是循环里最高频的语句）。
///
/// 七步见 spec §10。要点三条，都有判别式测试钉着，**别"顺手改好"**：
/// - **fan 以基准方向为中心对称展开**（D-7）；ring 不居中。
/// - `dist` 逐颗沿**各自**角度位移，不是整环平移。
/// - 直角偏移与极坐标偏移**相加**（ZUN 626 明写 stacks），不是覆盖。
pub const SYS_SH_FIRE: u16 = 76;
```

实现骨架（按 spec §10 的七步；确切类型以既有代码为准）：

```rust
fn sys_sh_fire(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let id = pop(task)?;
    // 先取一份拷贝：后面要同时可变借用 ctx.body 建弹，不能持着 shooters 的引用。
    let Some(sh) = ({ /* 越界 → contract_viol + BAD_ARGS + return Ok(()) */ }) else {
        return Ok(());
    };

    // ① 退化网格（对齐 create_bullets_batch 的既有口径）
    let total = sh.n_angle as u32 * sh.n_speed as u32;
    if sh.n_angle == 0 || sh.n_speed == 0 || total > crate::bullets::BulletPool::CAP as u32 {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return Ok(());
    }

    // ② 外观：空格格拒（同 fire——放行会造出"有判定但画面上什么都没有"的隐形弹）
    let Some(cfg) = ctx.tables.appearances.get(sh.appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }

    // ③ xform 解包（照抄 sys_create_bullet 的 LOCALS 边界校验与解包循环）
    //    区间越界 → Fault（同 fire）。

    // ④ 挂弹任务号先验（照抄 sys_create_bullet：坏号 → Fault，零副作用，弹未建）

    // ⑤ 原点（spec §10 步 2）
    let (ox, oy) = if sh.flags & SH_ABS_OFFSET != 0 { (Fx::ZERO, Fx::ZERO) } else { self_pos(task, ctx) };
    let (px, py) = polar_to_vec(sh.polar_r, sh.polar_ang);
    let origin = (ox + sh.off_x + px, oy + sh.off_y + py);

    // ⑥ 基准角（spec §10 步 3）——**在这一刻**解析 aim
    let base = if sh.flags & SH_AIMED != 0 {
        let a = crate::math::cordic::atan2(
            ctx.body.players[0].y - origin.1,
            ctx.body.players[0].x - origin.0,
        );
        Angle::from_raw(a.raw().wrapping_add(sh.angle0.raw()))
    } else {
        sh.angle0
    };

    // ⑦ 网格：角度外层、速度内层（照抄 create_bullets_batch 的序）
    //    fan:  angle_i = base + i*step - ((n-1)*step)/2
    //    ring: angle_i = base + (i*65536)/n + j*step
    //    每颗：位置 = origin + polar_to_vec(dist, angle_ij)；速度 = speed0 + j*speed_step
    //    建弹（有 xform 走 create_bullet_with_xform，否则 create_bullet）
    //    建成后若有挂弹任务 → ctx.tasks.spawn(...)，池满计数不 Fault（同 fire）
    //    统计 created

    // ⑧ on_fire_req
    if sh.on_fire_req != 0 {
        ctx.body.emit_req(
            sh.on_fire_req,
            [origin.0.raw(), origin.1.raw(), sh.appearance as i32, created as i32, 0, 0],
        );
    }
    Ok(())
}
```

**角度算术全部在 `i32` 里做再回绕成 `Angle`**（`Angle` 是 `u16`，直接加会溢出 panic）。
`(i * 65536) / n_angle` 的中间量用 `i32`：`i ≤ 255`，`255 × 65536` 未溢出 `i32`。

- [ ] **Step 4: 表层内建**

```rust
    Builtin {
        name: "sh_fire",
        syscall: syscall::SYS_SH_FIRE,
        is_op: false,
        params: &[Val(Int)],
        ret: None,
        doc: "用发射器槽 id 的参数开火;无返回值;池满走 P4-a 计数",
        param_names: &["id"],
    },
```

两处穷举名单加 `"sh_fire"`，重跑 `gen-ecl-meta`。

- [ ] **Step 5: 跑测试确认十条全过**

```bash
cargo test -p stg-core -- shooter_fan_matches_batch dist_pushes_each rect_and_polar \
  aim_resolves_at_fire ring_distributes_exactly ring_angle_step xform_survives \
  sh_task_spawns on_fire_req_reports sh_fire_rejects
cargo test --workspace
```

- [ ] **Step 6: 全绿 + 金向量对拍 + 提交**

金向量**必须逐字节不变**。

```bash
git commit -m "$(cat <<'EOF'
feat(ecl): sh_fire(76)——shooter 开火七步

用发射器槽的参数造弹。无返回值(D-8:本语言要求值必须消费,有返回值就得写 `_ = sh_fire(0);`,
而开火是循环里最高频的语句)。

**开火循环住 ECL 层**:本想扩 create_bullets_batch,但撞上 P1——world 不知道任务存在、
没法逐颗挂 task_script。结果反而更好:整个 shooter 对 world 层纯加法、零既有 API 改动。
代价是网格逻辑有第二份实现,故那条等价测试是**必需**而非顺带的。

最硬一条:shooter 的 fan ≡ batch(n, base-(n-1)*step/2, step),一次押住网格序、坐标算法、
速度递增**与居中公式本身**。三条判别腿各防一个具体错误实现:dist 逐颗沿各自角度(防整环
平移——圆心重合式测试对它是瞎的)、直角与极坐标偏移相加(防后设覆盖先设)、aim 在开火
那一刻解析(防设的时候就算死)。

ring 精确闭合:逐颗算 (i×65536)/n 把余数均摊,28 颗铺满整 65536——demo 现在手算 65536/28
=2340 只铺满 65520,收尾留 16 BAM 的缝。

xform 延迟读安全性有测试钉着(跨帧后仍生效):xform 数据住本任务 locals,由 codegen 在 sub
入口一次性 staging,slots 着色保证区间不被复用。

金向量逐字节不变。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 文档与收口

**Files:**
- Modify: `docs/ecl-lang.md`（手写语义节）
- Modify: `docs/ecl-ops.md`（syscall 号表追 15 行）
- Modify: `docs/bench-baseline.md`（内存账续表）
- Modify: `docs/follow-ups.md`（记随机 aimmode 一条）
- Modify: `PROGRESS.md`

- [ ] **Step 1: 确认生成段已是最新**

```bash
cargo run -p stg-harness -- gen-ecl-meta && git diff --stat
```
**预期无改动**（T2/T3 已各跑过）。有 diff 说明前面漏了同步，带上并在报告里说明。

- [ ] **Step 2: `docs/ecl-lang.md` 手写语义节**

生成段只有签名，语义必须手写。**本仓规矩：写之前先核实**——逐条对代码核，别照抄本计划。四块：

**(a) 是什么、怎么用**：每任务 4 个编号槽；配一遍、开多次火、改一个字段就是下一波。
配一个能编译的完整例子（风铃卡那种：`sh_sprite`/`sh_ring`/`sh_count`/`sh_speed`/`sh_xform`
配好，循环里只改 `sh_angle` 再 `sh_fire`）。

**(b) fan vs ring 的两张图**（这块最重要，是作者最容易搞混的）：
- **fan**：`n_angle` 颗按 `angle_step` 排开，**以基准方向为中心对称**——奇数路正中一颗正对、
  偶数路基准落中间两颗之间。**改颗数不用重算 `angle0`。**
- **ring**：`n_angle` 颗自动均分整周，`angle_step` **转义成逐层偏移**。
  "错开半步"写 `sh_angle(id, base, (32768 / n) as angle)`。

**(c) `sh_aim` 的语义**：`angle0` 是**相对自机方向的偏移**，故 `sh_angle(0, 15deg, ...)` 是
"打在自机右侧 15°"，`0deg` 是正打。

**(d) 三条坑**：
- 直角偏移与极坐标偏移**相加**，不是覆盖；`sh_offset` 会**清掉** `sh_offset_abs` 的标志
- `dist` 是逐颗沿**各自**角度推，不是整环平移
- **挂弹任务很吃任务槽**：`sh_count(0, 28, 1)` + `sh_task` = 一句话 28 个任务槽（池 256）。
  池满走 P4-a：**弹保留、任务丢**，表现为"一环里有几颗静默地没有该有的行为"。
  今天的 `batch` 没有 `task` 参，所以这是 shooter 新引入的压力面，**必须写进手册**。

> ⚠️ `ecl-lang.md` 的围栏示例是**真编译**的（harness 有 `every_ecl_fenced_example_in_doc_compiles`）。
> 写完务必 `cargo test -p stg-harness` 验一下。

- [ ] **Step 3: `docs/ecl-ops.md` syscall 号表追 15 行**

追到 **syscall 表**（`| 50 | add_score |` 那张，约 110 行起；**不是**上面那张 `| 50 | SPAWN |`
的 op 表，那是另一套编号）。62–76 共 15 行，照邻行的详略口径写全 P4 处置。

- [ ] **Step 4: `docs/bench-baseline.md` 内存账续表**

该文件的「内存账」节写的是 **2026-07-18 的旧值**（"World 总计 0.92 MB"），而哨兵实测早已是
1.03 MB。按该文件"更新纪律：把新表贴在旧表上方留史"的体例，续一段新的：World 现值、
shooter 占 45 KB、16 帧快照环新值。**不要改旧段**。

- [ ] **Step 5: `docs/follow-ups.md` 记一条**

ZUN 的随机 aimmode（`6` random angles / `7` random speeds / `8` 两者）本刀不做——它们消耗
**世界 RNG**，而 RNG 随快照回滚、消耗顺序直接进校验和（I3），要把"每颗弹抽几发、按什么序"
钉死成契约。按该文件既有条目体例记一条（编号取 D 组下一空号），写清为何延后与将来的形状。
顺带更新文件头「最后核实」段（看清既有体例是追加还是改写）。

- [ ] **Step 6: `PROGRESS.md`**

- **史加一行**（表格最上方，日期用 `date +%F` 取真实日期）：里程碑名 **Shooter 刀**，
  一句话概括 + 关键判据。照既有行的密度写。
- **「现在」段重写**（≤10 行，**重写不追加**）：位置换成本刀；「在飞」= 无；
  **保留 B26 余量那条现状**（`visible_instances` 断言 + 可玩性目验，都要有头环境）。

- [ ] **Step 7: 全绿收口**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```

**两个冒烟都要真跑**——本刀改了 `TaskPool` 布局与存档格式，桥面与真工程都可能受影响。

金向量：本任务纯文档，**必须逐字节不变**。

- [ ] **Step 8: 提交**

```bash
git commit -m "$(cat <<'EOF'
docs: shooter 刀收口——手册 fan/ring 两图 + 号表 15 行 + 内存账续表

ecl-lang.md 手写节:fan 以基准方向**居中**对称展开(改颗数不用重算 angle0)vs ring 自动
均分整周(angle_step 转义成逐层偏移),这是作者最容易搞混的一处;sh_aim 下 angle0 是相对
自机方向的**偏移**故 15deg 就是"打自机右侧 15 度";三条坑(两种偏移相加不覆盖、dist 逐颗
沿各自角度、**挂弹任务很吃任务槽**——sh_count(28)+sh_task 一句话 28 个槽,池满弹保留任务丢)。

最后那条是 shooter **新引入**的压力面:今天的 batch 没有 task 参,想给一环弹逐颗挂任务得写
for 循环逐颗 fire,写的时候自然会掂量;shooter 让它变成一句话。

bench-baseline 内存账按"新表贴旧表上方留史"续一段(旧段是 2026-07-18 的 0.92 MB,哨兵实测
早已 1.03 MB)。follow-ups 记随机 aimmode 一条(消耗世界 RNG,消耗序进校验和,要单独设计)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 自审记录

- **spec 覆盖**：§4 存储→T1；§5 字段→T1；§6 aimed/ring→T2(位)+T3(语义)；§7 fan 居中→T3；
  §8 on_fire_req→T1(字段)+T3(发送)；§9 十五内建→T2(14)+T3(1)；§10 开火七步→T3；
  §11 P4 七条→T2(id 越界/钳位)+T3(其余五条)；§12 测试→T2(五条)+T3(十条)；§13 破坏面→T1
  (`ENGINE_VER`/哨兵)+T4(文档)；§14 明确不做→计划全程未涉及，随机模式在 T4 记档。
- **对 spec 的一处补充**已单列一节（xform 延迟读的安全性论证 + T3 的跨帧测试）——spec 写的时候
  没查到 xform 数据住 `locals`。
- **一处 spec 没预见的连带改动**：`sh_sprite` 的颜色轴折叠下标是 **1** 而非 0，而 `codegen` 里
  那个判据硬编码了 `i == 0`。T2 Step 5 写明了改法与"`fire`/`batch` 产物逐字节不变"的押运测试。
- **类型一致性**：`Shooter` 的字段名在 T1 定义、T2 的派发臂与 T3 的开火逐一对应；
  `SH_AIMED`/`SH_RING`/`SH_ABS_OFFSET`/`SH_NO_TASK`/`SHOOTERS_PER_TASK` 五个常量在 T1 产出、
  后两个任务消费；syscall 号 62–75（T2）+ 76（T3）不重不漏。
- **金向量分两段记账**：**T1 漂**（新增 45056 B 默认值进校验和，与行为无关——本步没有任何代码
  读写 shooter）；**T2/T3/T4 逐字节不变**（纯新增，无脚本调用）。每个任务两个方向都要验。
- **本刀的判别力策略**：三条判别腿各针对一个**具体的**错误实现（整环平移 / 后设覆盖先设 /
  设时算死角度），而不是泛泛的"测一下功能"。等价测试因为网格逻辑有第二份实现而从"顺带的"
  升级成必需的。P4 与 setter 的测试一律**表驱动**——14 个各写一段复制粘贴既臃肿又会漏掉新加的。
