# M0-6: 输入 + 自机移动 + 基础发弹 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans（分组检查点执行）。

**Goal:** 世界层第一块"能玩"——外部输入抽象、可操控自机（东方手感移动）、玩家阵营弹池（ShotPool）+ character-0 基础直线发弹。金向量升级为"自机边走边打"，压玩家状态 + 第 2 个池的跨平台确定性。

**Architecture:** 输入 = `stg_core::input`（`InputFrame`/`ActionInput`，POD 边界类型）。自机 = 世界侧确定性状态机（A8）：`PlayerState`(D6 全字段) 住 `WorldBody.players`；`decode_input`(相位2) 译码、`update_players`(相位4) 移动 + **角色模块静态分发**（`match character_id` → character-0 `update_shot`）。玩家弹 = `ShotPool`(D7，第 2 个 `define_pool!` 用例)，走 `create_player_shot` 写 API(P4-a)。"池即层"：bullets=EnemyBullet、shots=PlayerShot（碰撞矩阵后续按池配对）。

**Tech Stack:** Rust 1.92；stg-core（+ input/player/shots 模块，改 world/step）、stg-harness（金向量加输入）。

## Global Constraints（grill 决策 + 不变量）

- **范围**：输入 + 移动 + 基础发弹；PlayerState 全 D6 字段建、仅移动+发弹活跃；死亡/碰撞/bomb 下一块。
- **MAX_PLAYERS = 2**（`crate::MAX_PLAYERS`）；player 0 驱动，player 1 保持全零 = `life_state==ABSENT` 跳过。
- **移动东方手感**：方向 (dx,dy)∈{-1,0,1}²；低速切换；**对角归一化** `speed×INV_SQRT2`；场界钳制。
- **角色配置暂 const**（WorldTables 角色配置表将来接管）；`INV_SQRT2 = Fx::from_raw(46341)`。
- **按钮位约定**（半冻结）：`UP/DOWN/LEFT/RIGHT/SHOT/BOMB/SLOW = bit 0..6`。坐标 y 向下为正，UP = 减 y。
- **create_player_shot P4-a**：池满 → NULL + `diag.pool_full[SHOT]++` + last_status。
- I1/I4/I7、字段声明序校验和；提交结尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。

## File Structure

- 新建 `crates/stg-core/src/input.rs`（InputFrame/ActionInput + 按钮位 const）。
- 新建 `crates/stg-core/src/shots.rs`（`define_pool!{Shot,...}`）。
- 新建 `crates/stg-core/src/player.rs`（PlayerState + 常量 + spawn + character-0 update_shot 逻辑）。
- 改 `crates/stg-core/src/world.rs`（WorldBody 加 players/shots；decode_input/update_players/create_player_shot；integrate/cleanup 加 shots）。
- 改 `crates/stg-core/src/step.rs`（`World::new` spawn player 0；step 加 input 参）。
- 改 `crates/stg-core/src/lib.rs`（`pub const MAX_PLAYERS`；`pub mod input/player/shots`）。
- 改 `crates/stg-harness/src/main.rs`（金向量加脚本输入 + step 签名）。
- 改 `stg-world-design.md`（A8/D6/D7 落地注）、`CLAUDE.md`。

---

### Task 1: `input` 类型 + `ShotPool`

- [ ] `lib.rs` 加 `pub const MAX_PLAYERS: usize = 2;`；`pub mod input; pub mod shots;`。
- [ ] `crates/stg-core/src/input.rs`：

```rust
//! 输入抽象（§5）—— 断层线边界类型（POD，无 godot/浮点）。键位绑定在表现层，模拟核只见动作位。
//! Phase 1 住 stg-core；将来可拆 stg-input crate（同 stg-net@M4）。

/// 一人一帧的量化动作位（4B）。`parameters` 数组待将来传参需求再加。
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ActionInput {
    pub buttons: u16,
    pub _pad: u16,
}

/// 一帧的全体输入（§5）。**不进 World 校验和**（外部输入，非世界状态；译码后的 `players[i].input` 才入校验和）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputFrame {
    pub frame: u32,
    pub actions: [ActionInput; crate::MAX_PLAYERS],
}

impl InputFrame {
    pub fn empty(frame: u32) -> Self {
        InputFrame {
            frame,
            actions: [ActionInput::default(); crate::MAX_PLAYERS],
        }
    }
}

// ── 按钮位约定（半冻结；表现层须同意）──────────────────────────────────
pub const BTN_UP: u16 = 1 << 0;
pub const BTN_DOWN: u16 = 1 << 1;
pub const BTN_LEFT: u16 = 1 << 2;
pub const BTN_RIGHT: u16 = 1 << 3;
pub const BTN_SHOT: u16 = 1 << 4;
pub const BTN_BOMB: u16 = 1 << 5;
pub const BTN_SLOW: u16 = 1 << 6;
```

- [ ] `crates/stg-core/src/shots.rs`：

```rust
//! 自机弹池（D7 PlayerShot 层）——`define_pool!` 第 2 个实例。行为由角色模块驱动（A8）。

use crate::define_pool;
use crate::math::Fx;

define_pool! {
    Shot, cap = 1024,
    fields {
        x: Fx, y: Fx, vx: Fx, vy: Fx,
        damage: u16, radius: Fx, sprite: u16, owner: u8, flags: u8
    }
}
```

- [ ] 测试（`input.rs` + `shots.rs` tests）：按钮位互不重叠；ShotPool alloc/copy_into（复用 M0-3 的宏，快速冒烟）。
- [ ] `cargo test -p stg-core` / `clippy` 绿；commit `feat(world): input 抽象 + ShotPool（PlayerShot 层）`。

### Task 2: `PlayerState` + WorldBody 接入 + spawn

- [ ] `crates/stg-core/src/player.rs`：

```rust
//! 自机（D6/A8）——世界侧确定性状态机。PlayerState + 角色常量 + character-0 火力（"shottype 类似物"）。

use crate::math::Fx;

// ── 生死状态（本块只用 ABSENT/ALIVE；其余待碰撞那块）──────────────────
pub const LIFE_ABSENT: u8 = 0; // 全零默认 = 不在场
pub const LIFE_ALIVE: u8 = 1;

// ── 角色配置（暂 const；WorldTables 角色配置表将来接管）────────────────
pub const HIGH_SPEED: Fx = Fx::from_raw(294_912); // 4.5 px/帧
pub const LOW_SPEED: Fx = Fx::from_raw(131_072); // 2.0 px/帧
pub const INV_SQRT2: Fx = Fx::from_raw(46_341); // 0.7071（对角归一）
pub const HIT_RADIUS: Fx = Fx::from_raw(163_840); // 2.5 px
pub const GRAZE_RADIUS: Fx = Fx::from_int(16);
pub const SHOT_SPEED: Fx = Fx::from_int(12);
pub const SHOT_RADIUS: Fx = Fx::from_int(4);
pub const SHOT_CD_FRAMES: u8 = 4;
pub const SHOT_DAMAGE: u16 = 1;

/// 自机状态（D6 全字段；本块仅移动 + 发弹活跃，余字段随快照/入校验和）。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum)]
pub struct PlayerState {
    pub x: Fx,
    pub y: Fx,
    pub character_id: u8,
    pub facing: i8, // 纯表现，照样入校验和（P6）
    pub hit_radius: Fx,
    pub graze_radius: Fx,
    pub input: u16,
    pub life_state: u8,
    pub state_timer: u16,
    pub invuln: u16,
    pub bomb_phase: u8,
    pub bomb_timer: u16,
    pub shot_cd: u8,
    pub power: u16,
    pub lives: u8,
    pub bombs: u8,
    pub life_pieces: u8,
    pub bomb_pieces: u8,
    pub score: u64,
    pub graze: u32,
}

impl PlayerState {
    /// 出场初值（场底中心，Alive，3 命 3 弹）。
    pub fn spawn(character_id: u8) -> Self {
        PlayerState {
            x: Fx::ZERO,
            y: Fx::from_int(384),
            character_id,
            facing: 0,
            hit_radius: HIT_RADIUS,
            graze_radius: GRAZE_RADIUS,
            input: 0,
            life_state: LIFE_ALIVE,
            state_timer: 0,
            invuln: 0,
            bomb_phase: 0,
            bomb_timer: 0,
            shot_cd: 0,
            power: 0,
            lives: 3,
            bombs: 3,
            life_pieces: 0,
            bomb_pieces: 0,
            score: 0,
            graze: 0,
        }
    }
}
```

- [ ] `lib.rs` 加 `pub mod player;`。
- [ ] `world.rs`：`use` 引入 `PlayerState`/`ShotPool`/`ShotHandle`/`ShotInit`/`input::*`/`player::*`；WorldBody 加字段（放 bullets 后）：

```rust
    pub players: [crate::player::PlayerState; crate::MAX_PLAYERS],
    pub shots: crate::shots::ShotPool,
```

- [ ] `world.rs` 加常量与写 API：

```rust
pub const POOL_SHOT: usize = 1;

impl WorldBody {
    /// 创建一发自机弹（P4-a）。
    pub fn create_player_shot(&mut self, init: crate::shots::ShotInit) -> crate::shots::ShotHandle {
        match self.shots.alloc(init) {
            Some(h) => h,
            None => {
                self.diag.pool_full[POOL_SHOT] = self.diag.pool_full[POOL_SHOT].wrapping_add(1);
                self.last_status = STATUS_POOL_FULL;
                crate::shots::ShotHandle::NULL
            }
        }
    }
}
```

- [ ] `step.rs` 的 `World::new` 末尾（播种 rng 后）spawn player 0：

```rust
        w.body.rng = Pcg32::new(seed, RNG_SEQ);
        w.body.players[0] = crate::player::PlayerState::spawn(0);
        w
```

- [ ] 测试（`step.rs` tests）：`World::new` 后 player 0 = Alive 且在 (0,384)、player 1 = ABSENT；`create_player_shot` 池满计数。
- [ ] `cargo test -p stg-core` / `clippy` 绿；commit `feat(world): PlayerState(D6) + WorldBody 接入 + spawn player0`。

### Task 3: decode_input + update_players(移动+发弹) + shots 积分/cleanup + step 加 input

- [ ] `world.rs` 的 `decode_input` 从桩改真实（签名加 input）：

```rust
    pub(crate) fn decode_input(&mut self, input: &crate::input::InputFrame) {
        self.phase_enter(PH_DECODE);
        for i in 0..crate::MAX_PLAYERS {
            self.players[i].input = input.actions[i].buttons;
        }
    }
```

- [ ] `world.rs` 的 `update_players` 从桩改真实（移动 + 角色模块发弹）：

```rust
    pub(crate) fn update_players(&mut self) {
        self.phase_enter(PH_PLAYERS);
        for i in 0..crate::MAX_PLAYERS {
            if self.players[i].life_state == crate::player::LIFE_ABSENT {
                continue;
            }
            self.move_player(i);
            match self.players[i].character_id {
                0 => self.char0_update_shot(i),
                _ => {}
            }
        }
    }

    /// 移动（东方手感：方向 + 低速 + 对角归一 + 场界钳制）。
    fn move_player(&mut self, i: usize) {
        use crate::input::{BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SLOW, BTN_UP};
        use crate::player::{HIGH_SPEED, INV_SQRT2, LOW_SPEED};
        let inp = self.players[i].input;
        let mut dx = 0i32;
        let mut dy = 0i32;
        if inp & BTN_LEFT != 0 {
            dx -= 1;
        }
        if inp & BTN_RIGHT != 0 {
            dx += 1;
        }
        if inp & BTN_UP != 0 {
            dy -= 1; // y 向下为正，UP = 减 y
        }
        if inp & BTN_DOWN != 0 {
            dy += 1;
        }
        let sp = if inp & BTN_SLOW != 0 { LOW_SPEED } else { HIGH_SPEED };
        // 对角归一化（两轴都动时 speed×INV_SQRT2）
        let axis = if dx != 0 && dy != 0 { sp * INV_SQRT2 } else { sp };
        let p = &mut self.players[i];
        if dx > 0 {
            p.x = p.x + axis;
        } else if dx < 0 {
            p.x = p.x - axis;
        }
        if dy > 0 {
            p.y = p.y + axis;
        } else if dy < 0 {
            p.y = p.y - axis;
        }
        // 场界钳制（自机不出场）
        p.x = Fx::from_raw(p.x.raw().clamp(-192 << 16, 192 << 16));
        p.y = Fx::from_raw(p.y.raw().clamp(0, 448 << 16));
    }

    /// character-0 火力（"shottype 类似物"）：SHOT 按下且 CD 到 → 发一发直线上飞弹。
    fn char0_update_shot(&mut self, i: usize) {
        use crate::input::BTN_SHOT;
        use crate::player::{SHOT_CD_FRAMES, SHOT_DAMAGE, SHOT_RADIUS, SHOT_SPEED};
        if self.players[i].shot_cd > 0 {
            self.players[i].shot_cd -= 1;
            return;
        }
        if self.players[i].input & BTN_SHOT != 0 {
            let (px, py) = (self.players[i].x, self.players[i].y);
            self.create_player_shot(crate::shots::ShotInit {
                x: px,
                y: py,
                vx: Fx::ZERO,
                vy: -SHOT_SPEED, // 上飞
                damage: SHOT_DAMAGE,
                radius: SHOT_RADIUS,
                sprite: 0,
                owner: i as u8,
                flags: 0,
            });
            self.players[i].shot_cd = SHOT_CD_FRAMES;
        }
    }
```

- [ ] `world.rs` 的 `integrate` 末尾加自机弹积分（无 delay/life，直接 `pos+=vel`）：

```rust
        // 自机弹：pos += vel
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.shots.x[i] = self.shots.x[i] + self.shots.vx[i];
                self.shots.y[i] = self.shots.y[i] + self.shots.vy[i];
            }
        }
```

- [ ] `world.rs` 的 `cleanup` 加自机弹越界回收：

```rust
        let nw = self.shots.alive.len();
        for w in 0..nw {
            let mut bits = self.shots.alive[w];
            while bits != 0 {
                let i = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if Self::out_of_bounds(self.shots.x[i], self.shots.y[i]) {
                    self.shots.free_index(i);
                }
            }
        }
```

- [ ] `step.rs`：`step`/`step_with_director` 签名加 `input: &InputFrame`，`decode_input(input)`：

```rust
pub fn step(world: &mut World, input: &crate::input::InputFrame) {
    step_with_director(world, input, |_| {});
}

pub fn step_with_director<F: FnMut(&mut WorldBody)>(
    world: &mut World,
    input: &crate::input::InputFrame,
    mut director: F,
) {
    let b = &mut world.body;
    b.begin();
    b.decode_input(input);
    b.phase_enter(PH_DIRECTOR);
    director(b);
    b.update_players();
    // ... 其余不变
}
```

- [ ] 改 `step.rs` 既有测试：`step(&mut w)` → `step(&mut w, &InputFrame::empty(0))`；`step_with_director(&mut w, &inp, |..|)`。
- [ ] 新增测试：脚本输入驱动 player 0 移动到位（含钳制、对角归一）；SHOT 按钮发弹进 ShotPool、shot_cd 生效；空输入不动；确定性重放含玩家/shots。
- [ ] `cargo test --workspace` / `clippy` 绿；commit `feat(world): decode_input + update_players(移动+发弹) + shots 积分/cleanup`。

### Task 4: 金向量"自机边走边打" + 设计回写

- [ ] `crates/stg-harness/src/main.rs` 的 `cmd_golden`：每帧构造脚本 `InputFrame` 驱动 player 0（走方框 + 持续射击 + 周期低速），传入 `step_with_director`：

```rust
    // 在 for frame 循环里：
    let mut input = InputFrame::empty(frame);
    // 走方框：每 30 帧换向；持续射击；每 120 帧一段低速
    let dir = (frame / 30) % 4;
    let mut btn = BTN_SHOT;
    btn |= match dir { 0 => BTN_RIGHT, 1 => BTN_DOWN, 2 => BTN_LEFT, _ => BTN_UP };
    if (frame / 120) % 2 == 0 {
        btn |= BTN_SLOW;
    }
    input.actions[0].buttons = btn;
    step_with_director(&mut world, &input, |b| { /* 原敌弹环导演不变 */ });
```

（`use stg_core::input::{ActionInput, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SHOT, BTN_SLOW, BTN_UP, InputFrame};`）

- [ ] 本地 `golden` 跑两次 `diff` 应相同；观察 checksum 稳定（现在含玩家移动 + 自机弹 churn）。
- [ ] 设计回写：`stg-world-design.md` A8/D6/D7 加"M0-6 落地：输入+移动(对角归一)+character-0 直线发弹；角色配置/生死/bomb/碰撞待后续"；`CLAUDE.md` 记世界层进度。
- [ ] commit `feat(harness): 金向量自机边走边打 + 设计回写`。

### Task 5: 收口 + CI + 合并

- [ ] `cargo test --workspace` / `fmt --check` / `clippy -D warnings` / `verify-tables` 全绿。
- [ ] 依赖防火墙仍过（input/player/shots 无新外部依赖）。
- [ ] 推分支 → PR → 三平台 CI 绿（determinism-gate 现含玩家+自机弹演化）→ ff 合并 → 清理。

## Self-Review

- §5 输入：InputFrame/ActionInput（buttons u16）+ 按钮位约定 ✓；不进校验和（外部）、译码后 input 入校验和 ✓。
- A8 自机：世界侧状态机 ✓、角色模块 `match character_id` 静态分发（character-0）✓、世界管移动、角色管火力 ✓。
- D6：PlayerState 全字段 + spawn ✓；生死状态机仅 ABSENT/ALIVE（死亡待碰撞）。
- D7 ShotPool（PlayerShot 层）✓；"池即层"阵营分离 ✓。
- 移动东方手感（对角归一/低速/钳制）✓；P4-a create_player_shot ✓。
- 留后：生死/决死窗口/复活、bomb、碰撞矩阵（行1中弹/行4打敌人）、graze、敌人、WorldTables 角色配置、homing、多发弹型（角色模块扩展）、MAX_PLAYERS=2 的第 2 人接入。
