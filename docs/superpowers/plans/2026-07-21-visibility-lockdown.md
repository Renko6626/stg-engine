# 可见性收口（刀 A）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让自机（`players`）与池的 `alloc`/`free` 遵守"断层线以上只能走写 API / 读访问器"的编译期纪律，收 follow-ups D1 + D4，为 M2 表现层接入焊死可见性裂缝——纯重构，金向量逐位不变。

**Architecture:** 三刀。① 先补 `set_player_power` 写 API + `players()` 只读访问器（此时 `players` 字段仍 `pub`，零破坏，独立可测）；② 再把 `WorldBody.players` 收 `pub(crate)` + 迁移 harness 四处 power 直写 + 销 D1（写口封死靠新 API 才能编译）；③ 最后把 `define_pool!` 的 `alloc`/`free` 收 `pub(crate)` + 销 D4（in-crate 调用者全不受影响）。每刀结束全绿 + 金向量与基线 `diff` 全等。

**Tech Stack:** Rust 1.92.0 / edition 2024；stg-core（断层线以下确定性内核）、stg-derive（`define_pool!` proc-macro）、stg-harness（CLI 金向量）。

## Global Constraints

- **断层线纪律**：stg-core 不得引入 float / 时钟 / 宿主 RNG / 无序容器 / 任何新依赖。本刀**零新依赖**（不引 `trybuild`），`cargo tree -p stg-core` 防火墙保持干净。
- **金向量逐位不变**：纯重构，`cargo run -q -p stg-harness -- golden` 的输出在每刀后必须与刀前基线 `diff` 全等（空 diff）。
- **P4-b**：调用方违约（越界索引）→ 确定性安全结果（no-op + `diag.contract_viol` 计数 + `last_status = STATUS_BAD_ARGS`），**不 panic**。
- **常量真实路径**：`crate::items::POWER_MAX: u16 = 400`、`crate::MAX_PLAYERS: usize = 2`、`crate::world::STATUS_BAD_ARGS: u16 = 3`（在 `world.rs` 内免限定，经 `use super::*` 也在 `mod tests` 内可见）。
- **提交结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- **不做**：完整 WorldView、`emit_req`/`reqs`（通道 B）、destroy/kill API、`PlayerState` 内部字段逐个降级、删 `power_tier()` 的 `min(4)` 防御钳。

---

## File Structure

| 文件 | 职责 | 刀 |
|---|---|---|
| `crates/stg-core/src/world.rs` | `impl WorldBody` 加 `set_player_power` + `players()`；`players` 字段 `pub→pub(crate)`；重写 line 68-72 的 D1 裂缝注释；`mod tests` 加 3 个判别式单测 | 1, 2 |
| `crates/stg-derive/src/lib.rs` | `define_pool!` 生成的 `alloc`/`free`：`pub→pub(crate)` | 3 |
| `crates/stg-harness/src/main.rs` | 四处 `players[0].power = N` → `set_player_power(0, N)` | 2 |
| `docs/follow-ups.md` | 删 D1（刀 2）、删 D4（刀 3） | 2, 3 |

**合入时（finishing-a-development-branch，非 TDD 任务）**：`PROGRESS.md` milestone 史加一行 + 重写「现在」段。

---

### Task 1: `set_player_power` 写 API + `players()` 只读访问器

**Files:**
- Modify: `crates/stg-core/src/world.rs`（`impl WorldBody` 内 `set_var` 之后 ~609 行插两方法；`mod tests` ~819 行后插三测试）
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces（刀 2/3 依赖）：
  - `WorldBody::set_player_power(&mut self, player: usize, power: u16)` —— 越界 player → no-op+计数；power 钳 `POWER_MAX`。
  - `WorldBody::players(&self) -> &[PlayerState]` —— 只读整条自机数组切片。

> 本刀不动 `players` 字段可见性（仍 `pub`），故加了未被外部调用的两方法后**零破坏**，独立可测。

- [ ] **Step 1: 抓金向量基线（在任何代码改动前，保证 pristine）**

Run:
```bash
cd /data/sunyunbo/www/stg-engine
cargo run -q -p stg-harness -- golden --out /tmp/vis-golden-baseline.txt
wc -l /tmp/vis-golden-baseline.txt
```
Expected: 生成基线文件（两段金向量逐帧校验和，非空）。刀 2/3 都与它 `diff`。

- [ ] **Step 2: 写失败测试**（`crates/stg-core/src/world.rs` 的 `mod tests` 末尾，`events_push_records_fact` 等之后插入）

```rust
/// set_player_power 钳边界：恰 POWER_MAX 原样写入；超一格被钳（判别 min 是否真在——
/// 换成裸写 `self.players[player].power = power` 即红）。
#[test]
fn set_player_power_clamps_to_power_max() {
    let mut w = crate::step::World::new(1);
    w.body.set_player_power(0, crate::items::POWER_MAX);
    assert_eq!(
        w.body.players[0].power,
        crate::items::POWER_MAX,
        "恰满档原样写入"
    );
    w.body.set_player_power(0, crate::items::POWER_MAX + 1);
    assert_eq!(
        w.body.players[0].power,
        crate::items::POWER_MAX,
        "超 POWER_MAX 被钳（防 power_tier index OOB）"
    );
}

/// set_player_power 越界 player 索引 → P4-b 确定性安全结果：no-op + contract_viol +1 +
/// last_status=BAD_ARGS（同 set_var/pulse_signal 守卫口径）。
#[test]
fn set_player_power_oob_player_is_guarded_no_op() {
    let mut w = crate::step::World::new(1);
    let last = crate::MAX_PLAYERS - 1;
    let before = w.body.players[last].power;
    let cv0 = w.body.diag.contract_viol;
    w.body.set_player_power(crate::MAX_PLAYERS, 200); // 恰过界
    assert_eq!(w.body.players[last].power, before, "越界不写任何真槽");
    assert_eq!(w.body.diag.contract_viol, cv0 + 1);
    assert_eq!(w.body.last_status, STATUS_BAD_ARGS);
}

/// players() 只读访问器：返回 MAX_PLAYERS 长切片，内容与内部一致。
#[test]
fn players_accessor_returns_full_slice() {
    let mut w = crate::step::World::new(1);
    w.body.set_player_power(0, 123);
    let ps = w.body.players();
    assert_eq!(ps.len(), crate::MAX_PLAYERS);
    assert_eq!(ps[0].power, 123);
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p stg-core --lib set_player_power 2>&1 | tail -20`
Expected: 编译失败——`no method named set_player_power`/`no method named players`（方法未实现）。

- [ ] **Step 4: 实现两方法**（`crates/stg-core/src/world.rs`，`impl WorldBody` 内 `set_var`（~609 行 `}` 之后）插入）

```rust
/// 拔火力档（导演/游戏层唯一的自机 power 外部写入口，收 D1）。P4-b：越界 player 索引
/// → no-op + contract_viol 计数 + last_status=BAD_ARGS（同 set_var/pulse_signal 口径）；
/// power 上钳 POWER_MAX（`power_tier` index OOB 的根，见本文件顶 MAX_ENTITY_RADIUS 注释）。
pub fn set_player_power(&mut self, player: usize, power: u16) {
    if player >= crate::MAX_PLAYERS {
        self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        self.last_status = STATUS_BAD_ARGS;
        return;
    }
    self.players[player].power = power.min(crate::items::POWER_MAX);
}

/// 自机只读切片（表现层读自机态的入口；通道 A 最小种子，非完整 WorldView）。
pub fn players(&self) -> &[PlayerState] {
    &self.players
}
```

- [ ] **Step 5: 跑测试确认通过 + 全 crate 绿**

Run: `cargo test -p stg-core --lib 2>&1 | tail -15`
Expected: 三新测试 PASS，stg-core 全绿。

- [ ] **Step 6: 提交**

```bash
cd /data/sunyunbo/www/stg-engine
git add crates/stg-core/src/world.rs
git commit -m "$(cat <<'EOF'
feat(world): set_player_power 写 API + players() 只读访问器（刀 A/1）

补自机唯一合法外部写入口（钳 POWER_MAX，越界 player P4-b no-op+计数）+ 通道 A
最小只读种子。此刀不动字段可见性，零破坏；判别式单测钉钳边界 + P4-b 守卫。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 封 `players` 写口 + harness 迁移 + 销 D1

**Files:**
- Modify: `crates/stg-core/src/world.rs:141`（`players` 字段 `pub→pub(crate)`）+ 68-72 行 D1 裂缝注释重写
- Modify: `crates/stg-harness/src/main.rs`（四处 `power = N` → `set_player_power(0, N)`）
- Modify: `docs/follow-ups.md`（删 D1）

**Interfaces:**
- Consumes：Task 1 的 `WorldBody::set_player_power` / `WorldBody::players`。

> 写口与 harness 迁移必须同刀落地：先迁 harness 到新写 API，再翻 `pub(crate)`，否则 harness 编不过。

- [ ] **Step 1: 迁移 harness 四处 power 直写**（`crates/stg-harness/src/main.rs`）

四处逐字替换（保留各自尾注释）：
- 行 137：`w.body.players[0].power = 400; // 满火力：四路+子机的自机弹稳态负载` → `w.body.set_player_power(0, 400); // 满火力：四路+子机的自机弹稳态负载`
- 行 239：`w.body.players[0].power = 400;` → `w.body.set_player_power(0, 400);`
- 行 813：`b.players[0].power = 250;` → `b.set_player_power(0, 250);`
- 行 816：`b.players[0].power = 400;` → `b.set_player_power(0, 400);`

（`w` 是 `Box<World>`、`b` 是导演闭包的 `&mut WorldBody`——两者都能直调 `set_player_power`。）

- [ ] **Step 2: 封 `players` 字段写口**（`crates/stg-core/src/world.rs:141`）

```rust
    pub(crate) players: [PlayerState; crate::MAX_PLAYERS],
```
（原为 `pub players: [PlayerState; crate::MAX_PLAYERS],`。）

- [ ] **Step 3: 重写 D1 裂缝注释**（`crates/stg-core/src/world.rs`，把 68-72 行"但 …留待后续。"整段替换）

old_string（精确匹配这段）:
```rust
///   位等测试钉死（原 player.rs 编译期断言已随常量迁表退役）。但
///   `WorldBody.players` 与 `PlayerState` 的字段目前都是 `pub`，任何持 `&mut World` 的上层
///   （今天是 stg-harness，将来是 stg-godot/stg-py）都能绕过 `spawn` 直接写这两个字段——这是
///   **前提**，不是强制。安全性目前只因"除 spawn 外无人写它"成立；收紧可见性（或改走访问器）
///   留待后续。
```
new_string:
```rust
///   位等测试钉死（原 player.rs 编译期断言已随常量迁表退役）。**自机写口已收紧**（刀 A，
///   2026-07-21）：`WorldBody.players` 字段为 `pub(crate)`，断层线以上只能经 `set_player_power`
///   写 API（钳 POWER_MAX）改 power、经 `players()` 只读访问器读态——与四池"只能走写 API"同为
///   类型系统强制。`PlayerState` 内部字段仍 `pub`，但数组已封 → 外部无 `&mut` 路径可达，
///   `players()` 交出的 `&PlayerState` 只读不可写。
```

- [ ] **Step 4: 全工作区编译 + 测试绿**

Run: `cargo build --workspace 2>&1 | tail -5 && cargo test --workspace 2>&1 | tail -20`
Expected: 全绿。（若 harness 有遗漏的 `players[..]` 写点未迁 → 此处 `E0616`/私有字段错会暴露。）

- [ ] **Step 5: 金向量与基线 diff 全等（纯重构回归闸）**

Run:
```bash
cargo run -q -p stg-harness -- golden --out /tmp/vis-golden-after2.txt
diff /tmp/vis-golden-baseline.txt /tmp/vis-golden-after2.txt && echo "IDENTICAL"
```
Expected: 无 diff 输出 + 打印 `IDENTICAL`（power 写值 ≤400，钳为 no-op → 逐位不变）。

- [ ] **Step 6: 销 follow-up D1**（`docs/follow-ups.md`）

删整节 `### D1. 自机半径不经写 API —— **M2 表现层接入前应解决**`（连同其正文，到下一 `###` 前）。别留"已完成"墓碑（git log 即历史）。

- [ ] **Step 7: fmt + clippy + 提交**

```bash
cd /data/sunyunbo/www/stg-engine
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
git add crates/stg-core/src/world.rs crates/stg-harness/src/main.rs docs/follow-ups.md
git commit -m "$(cat <<'EOF'
refactor(world): 封 players 写口为 pub(crate)，harness 迁 set_player_power（刀 A/2，销 D1）

WorldBody.players 字段 pub→pub(crate)：断层线以上只能经 set_player_power 写、players()
读，与四池写 API 纪律对齐。harness 四处 power 直写迁新 API（值≤400 钳无效 → 金向量与基线
逐位全等）。删 follow-up D1。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 封 `define_pool!` 的 `alloc`/`free` + 销 D4

**Files:**
- Modify: `crates/stg-derive/src/lib.rs`（`alloc` ~217 行、`free` ~242 行：`pub fn`→`pub(crate) fn`）
- Modify: `docs/follow-ups.md`（删 D4）

**Interfaces:**
- Consumes：无（独立于 Task 1/2）。
- 影响面：五个 `define_pool!` 实例（bullets/shots/enemy/field/items）的 `alloc`/`free`。stg-core 内所有调用者均 in-crate（生产代码 + `#[cfg(test)]` 测试），`pub(crate)` 后全不受影响；out-of-crate 无任何调用者（已核实 harness 零 `.alloc(`/`.free(`）。手写池 `xforms`（字段本就 `pub(crate)`，外部够不着）不在本刀范围。

- [ ] **Step 1: 收 `alloc` 可见性**（`crates/stg-derive/src/lib.rs`，`define_pool` 生成体内）

```rust
            /// 分配：写满全字段 + gen+1；池满返回 None。
            pub(crate) fn alloc(&mut self, init: #init) -> ::core::option::Option<#handle> {
```
（原 `pub fn alloc`。）

- [ ] **Step 2: 收 `free` 可见性**（同文件）

```rust
            /// 释放（清 alive 位）；句柄无效则 no-op 返回 false。
            pub(crate) fn free(&mut self, h: #handle) -> bool {
```
（原 `pub fn free`。）

- [ ] **Step 3: 全工作区编译 + 测试绿**

Run: `cargo build --workspace 2>&1 | tail -5 && cargo test --workspace 2>&1 | tail -20`
Expected: 全绿。（`define_pool!` 展开出的 `pub(crate)` alloc/free 对 in-crate 调用者透明；若某处意外从外部调用会在此暴露——预期无。）

- [ ] **Step 4: 金向量与基线 diff 全等**

Run:
```bash
cargo run -q -p stg-harness -- golden --out /tmp/vis-golden-after3.txt
diff /tmp/vis-golden-baseline.txt /tmp/vis-golden-after3.txt && echo "IDENTICAL"
```
Expected: 无 diff + `IDENTICAL`（纯可见性变更，零运行期影响）。

- [ ] **Step 5: 销 follow-up D4**（`docs/follow-ups.md`）

删整节 `### D4. define_pool! 生成的 alloc/free 是 pub —— 与 players 同族问题（D1 姊妹条）`（连同正文，到下一 `###`/`---` 前）。

- [ ] **Step 6: fmt + clippy + 依赖防火墙 + 提交**

```bash
cd /data/sunyunbo/www/stg-engine
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
cargo tree -p stg-core 2>&1 | grep -iE "godot|libm|rand|chrono|time" && echo "FIREWALL BREACH" || echo "FIREWALL OK"
git add crates/stg-derive/src/lib.rs docs/follow-ups.md
git commit -m "$(cat <<'EOF'
refactor(derive): define_pool! 的 alloc/free 收 pub(crate)（刀 A/3，销 D4）

堵池的写入逃生口：外部 &mut World 持有者不再能 bullets.alloc()/free() 绕开写 API。
D4 "直接 free 漏 xform 段" 那半由"外部 free 不了"按构造消除，无需 destroy API。
in-crate 调用者全不受影响，金向量与基线逐位全等。删 follow-up D4。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Definition of Done（合入前）

- [ ] 三刀提交齐全，`cargo build/test --workspace` 全绿。
- [ ] `cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警。
- [ ] 金向量三刀后与基线 `diff` 全等（`/tmp/vis-golden-baseline.txt`）。
- [ ] `cargo tree -p stg-core` 无 godot/libm/rand/时钟等禁项（零新依赖）。
- [ ] `docs/follow-ups.md` 中 D1、D4 已删（无墓碑）。
- [ ] `PROGRESS.md`：合入时（finishing-a-development-branch）milestone 史加一行 + 重写「现在」段。
