# stg-godot 桥刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/stg-godot` gdext cdylib(WorldBridge)落地——Godot 只加载 `.so` 不编译;headless 全流程冒烟(new_game/step/save-load 续接/通道 B)打通。

**Architecture:** 薄壳厚核(spec §3/§12):`boot/frame/save` 三纯模块零 gdext 类型、cargo test 全覆盖;`bridge.rs` gdext 壳纯胶水,由 headless 冒烟盖。前置:工具链 1.92.0→1.94.0(gdext 0.5.x MSRV)。

**Tech Stack:** Rust 1.94.0(本刀 Task 1 bump)、`godot = "=0.5.4"`(feature `api-4-6`)、Godot 4.6.3(headless 冒烟)。

**Spec:** `docs/superpowers/specs/2026-07-24-stg-godot-bridge-design.md`(§3-§12;§11 拍板值、§12 内部结构)。

## Global Constraints

- **金向量逐字节不变贯穿全分支**:Task 1 工具链 bump 前后对拍全等(= 跨编译器版本确定性实证,`GOLDEN-TOOLCHAIN-OK`);其后零 core 改动,收口再拍一次全等(`GOLDEN-BRIDGE-OK`)。基线放 `.superpowers/bridge/`(未跟踪,勿 git add)。
- **缓冲布局(spec §11.2)**:12 float/实例 = `[xx, yx, 0, ox, xy, yy, 0, oy]` + `custom[sprite,0,0,0]`;活槽压实前段(池索引升序),尾部不清(靠 `visible_instances`);定点→浮点 = `raw as f32 / 65536.0`,BAM→弧度 = `raw as f32 * (TAU/65536.0)`,**只在 frame.rs**。
- **P4/FFI 铁律(spec §6)**:任何错误不得 panic 穿 FFI;调用方违约 → no-op + false + 去重日志;`load_state` 失败不动世界。
- **机械调整许可(仅两类,逐处入报告)**:①gdext 0.5.4 的 API 名/宏形态与计划代码有出入 → 以 docs.rs/godot/0.5.4 与 gdext book 为准调整,语义不变;②`.ecl` builtin 形参与计划脚本有出入 → 以 `crates/stg-ecl-compiler/src/lang/builtins.rs` 为准调整,场景语义不变。**断言语义一律不许改**。
- 分支 `feat/stg-godot`;commit 尾附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 跑绿:`cargo test --workspace` + `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings`;冒烟 `crates/stg-godot/smoke/run-smoke.sh` 打 `SMOKE OK` 退 0。
- `GODOT_BIN` 缺省 `/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64`。

---

### Task 1: 工具链 1.92.0 → 1.94.0(跨编译器金向量对拍)

**Files:**
- Modify: `rust-toolchain.toml`(channel 行)、`CLAUDE.md`(仓库结构图 "钉死 1.92.0" 文案)

- [ ] **Step 1: 建分支 + 1.92 金向量基线(动工具链之前)**

```bash
cd /data/sunyunbo/www/stg-engine
git checkout -b feat/stg-godot
mkdir -p .superpowers/bridge
cargo run --release -p stg-harness -- golden --out .superpowers/bridge/golden-192.txt
```

- [ ] **Step 2: bump**——`rust-toolchain.toml` 的 `channel = "1.92.0"` 改 `channel = "1.94.0"`(其余行不动);`CLAUDE.md` 仓库结构里 `钉死 1.92.0` 改 `钉死 1.94.0`。

- [ ] **Step 3: 全量重建重测**(rustup 会按 toolchain 文件自动拉 1.94.0,首次较久)

```bash
cargo build --workspace && cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全绿。若新版 clippy 报新 lint:逐条最小修复(不许裸 `#[allow]`,确需 allow 必须附理由注释),修复清单入报告。

- [ ] **Step 4: 跨编译器对拍**

```bash
cargo run --release -p stg-harness -- golden --out .superpowers/bridge/golden-194.txt
diff .superpowers/bridge/golden-192.txt .superpowers/bridge/golden-194.txt && echo GOLDEN-TOOLCHAIN-OK
```

Expected: `GOLDEN-TOOLCHAIN-OK`(整数模拟对编译器版本免疫的实证)。有 diff = 停,BLOCKED 附现场——这是重大发现不是小事。

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --all -- --check
git add rust-toolchain.toml CLAUDE.md Cargo.lock
git commit -m "chore(toolchain): 1.92.0→1.94.0(gdext 0.5.x MSRV)——金向量 bump 前后逐字节全等,跨编译器确定性实证

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

(Cargo.lock 若本步无变化就不加;`git status` 为准。)

---

### Task 2: crate 骨架 + `.gdextension` + headless 加载冒烟

**Files:**
- Create: `crates/stg-godot/Cargo.toml`、`src/lib.rs`、`src/bridge.rs`(最小 ping 版)、`smoke/project.godot`、`smoke/stg_godot.gdextension`、`smoke/smoke.gd`、`smoke/run-smoke.sh`
- Modify: 根 `Cargo.toml` 的 `[workspace.dependencies]`(若缺 `stg-ecl-compiler` 行则补 `stg-ecl-compiler = { path = "crates/stg-ecl-compiler" }`)

**Interfaces:**
- Produces: 可被 Godot 4.6 headless 加载注册的 `libstg_godot.so`;`WorldBridge.ping() == 42`;冒烟脚手架(Task 4 复用扩展)。

- [ ] **Step 1: crate 文件**

`crates/stg-godot/Cargo.toml`:

```toml
[package]
name = "stg-godot"
version = "0.1.0"
edition.workspace = true
rust-version = "1.94"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
stg-core = { workspace = true }
stg-ecl-compiler = { workspace = true }
godot = { version = "=0.5.4", features = ["api-4-6"] }
```

`src/lib.rs`:

```rust
//! stg-godot——WorldBridge gdext cdylib(spec 2026-07-24 §3/§12,薄壳厚核)。
//! 断层线以上:f32 只活在 frame.rs;Godot 只加载 .so 不编译(M2)。

use godot::prelude::*;

pub mod bridge;

struct StgGodotExtension;

#[gdextension]
unsafe impl ExtensionLibrary for StgGodotExtension {}
```

`src/bridge.rs`(本任务最小版,Task 4 换全量):

```rust
//! gdext 壳:WorldBridge 节点。纯胶水零逻辑(spec §12)。

use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct WorldBridge {
    base: Base<Node>,
}

#[godot_api]
impl INode for WorldBridge {
    fn init(base: Base<Node>) -> Self {
        WorldBridge { base }
    }
}

#[godot_api]
impl WorldBridge {
    /// 加载冒烟探针:注册即 42。
    #[func]
    fn ping(&self) -> i64 {
        42
    }
}
```

- [ ] **Step 2: 编译出 `.so`**

```bash
cargo build -p stg-godot && ls -la target/debug/libstg_godot.so
```

- [ ] **Step 3: 冒烟载体**

`smoke/project.godot`:

```ini
config_version=5

[application]
config/name="stg-godot-smoke"
```

`smoke/stg_godot.gdextension`:

```ini
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = 4.6
reloadable = false

[libraries]
linux.debug.x86_64 = "res://../../../target/debug/libstg_godot.so"
linux.release.x86_64 = "res://../../../target/release/libstg_godot.so"
windows.debug.x86_64 = "res://../../../target/x86_64-pc-windows-msvc/debug/stg_godot.dll"
windows.release.x86_64 = "res://../../../target/x86_64-pc-windows-msvc/release/stg_godot.dll"
```

`smoke/smoke.gd`:

```gdscript
extends SceneTree

func _init():
	if not ClassDB.class_exists("WorldBridge"):
		push_error("WorldBridge 未注册——扩展没加载")
		quit(1)
		return
	var b = ClassDB.instantiate("WorldBridge")
	if b.ping() != 42:
		push_error("ping != 42")
		quit(1)
		return
	b.free()
	print("SMOKE OK")
	quit(0)
```

`smoke/run-smoke.sh`(`chmod +x`):

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd ../../.. && cargo build -p stg-godot )
# 首跑生成 .godot/ 资源缓存(含扩展清单);失败不致命,真判定在下一步
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . --script res://smoke.gd 2>&1)
echo "$out"
grep -q "SMOKE OK" <<<"$out"
```

- [ ] **Step 4: 跑冒烟**

Run: `crates/stg-godot/smoke/run-smoke.sh`
Expected: 末行 `SMOKE OK`,退 0。若扩展未加载(class_exists 假):排查 `.gdextension` 相对路径/`--import` 缓存,**把真实踩坑经过记入报告**(Task 5 会沉淀进 `docs/bridge-adaptation-notes.md`)。

- [ ] **Step 5: 全量 + commit**

```bash
cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-godot Cargo.toml Cargo.lock
git commit -m "feat(godot): stg-godot crate 骨架——gdext 0.5.4/api-4-6 cdylib + WorldBridge 注册 + headless 加载冒烟通路

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

(`smoke/.godot/` 生成缓存**不入库**:在 `crates/stg-godot/smoke/.gitignore` 写一行 `.godot/`,该 .gitignore 入库。)

---

### Task 3: `boot` / `frame` / `save` 三纯模块 + 单测

**Files:**
- Create: `crates/stg-godot/src/boot.rs`、`src/frame.rs`、`src/save.rs`
- Modify: `src/lib.rs`(加 `pub mod boot; pub mod frame; pub mod save;`)

**Interfaces:**
- Consumes: `stg_ecl_compiler::lang::compile(src, name) -> Result<EclImage, Vec<E>>`(E 有 `.render(name)`);`World::new_game(seed, rank, &image)`;`World::{view, frame, checksum, save_bytes(&image), load_bytes(bytes,&tables,&image), take_requests}`;`WorldView::{bullets,shots,enemies,items,players,spells}`;池切片访问器 `x()/y()/sprite()/angle()/item_type()` + `iter_alive()` + `Pool::CAP`;`TABLES_V0`/`ItemTypeCfg.sprite`;`stg_core::items::ITEM_TYPE_COUNT`。
- Produces(Task 4 依赖,签名精确):
  `boot::boot(ecl_source: &str, seed: u64, rank: i32) -> Result<Game, BootError>`;
  `boot::Game { world, image, tables }`;
  `frame::{LAYER_BULLETS=0, LAYER_SHOTS=1, LAYER_ENEMIES=2, LAYER_ITEMS=3, LAYER_COUNT=4, FLOATS_PER_INSTANCE=12}`;
  `frame::layer_cap(layer: usize) -> usize`;
  `frame::encode_layer(view: WorldView, tables: &WorldTables, layer: usize, out: &mut [f32]) -> u32`(out.len == cap×12,写前缀返活数);
  `save::{save(&Game) -> Vec<u8>, load_into(&mut Game, &[u8]) -> Result<(), String>}`。

- [ ] **Step 1: 写失败测试**(各模块文件尾 `#[cfg(test)] mod tests`;先建三个空模块声明让路径存在,测试引用未实现符号编译红即 TDD 红)

`boot.rs` 测试:

```rust
    #[test]
    fn boot_ok_and_frame_zero() {
        let g = boot("sub main() { loop { wait(60); } }", 7, 2).expect("boot");
        assert_eq!(g.world.frame(), 0);
        assert_ne!(g.world.checksum(), 0);
    }

    #[test]
    fn boot_compile_error_carries_location() {
        let e = boot("sub main() { 这不是合法脚本 }", 7, 2);
        let Err(BootError::Compile(msg)) = e else {
            panic!("应为编译错误")
        };
        assert!(!msg.is_empty());
    }
```

`frame.rs` 测试(判别值:每层单实体、互异非零坐标/贴图,断言精确 float;场景脚本按机械调整许可②对 builtins.rs 核参):

```rust
    const SRC: &str = r#"
sub main() {
    _ = spawn_enemy(-96.0fx, -64.0fx, 100, 0, 0);
    _ = drop_item(32.0fx, 48.0fx, 2);
    _ = fire(1, 10.0fx, 20.0fx, 0.0fx, 0, 0, 0, -1);
    _ = fire(1, 11.0fx, 21.0fx, 0.0fx, 16384, 0, 0, -1);
    loop { wait(60); }
}
"#;

    fn stepped_game() -> crate::boot::Game {
        let mut g = crate::boot::boot(SRC, 7, 2).expect("boot");
        let input = stg_core::input::InputFrame::empty(0);
        stg_core::step::step_with_director(&mut g.world, g.tables, &g.image, &input, |_| {});
        g
    }

    #[test]
    fn bullets_layer_transform_and_custom() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_BULLETS) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_BULLETS, &mut out);
        assert_eq!(n, 2);
        // 弹0:angle=0 → cos1/sin0 精确;pos=(10,20) 速度0不动
        assert_eq!(&out[0..8], &[1.0, -0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 20.0]);
        let expect_sprite =
            stg_core::tables::TABLES_V0.appearances[1].sprite as f32;
        assert_eq!(out[8], expect_sprite);
        assert_eq!(&out[9..12], &[0.0, 0.0, 0.0]);
        // 弹1:angle=16384(90°) → cos≈0/sin≈1;压实序=池索引升序
        let b1 = &out[12..24];
        assert!(b1[0].abs() < 1e-6 && (b1[4] - 1.0).abs() < 1e-6);
        assert_eq!((b1[3], b1[7]), (11.0, 21.0));
    }

    #[test]
    fn enemies_layer_exact() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_ENEMIES) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_ENEMIES, &mut out);
        assert_eq!(n, 1);
        assert_eq!(&out[0..8], &[1.0, -0.0, 0.0, -96.0, 0.0, 1.0, 0.0, -64.0]);
        assert_eq!(out[8], 0.0, "spawn_enemy sprite 固定 0");
    }

    #[test]
    fn items_layer_sprite_join() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_ITEMS) * FLOATS_PER_INSTANCE];
        let n = encode_layer(g.world.view(), g.tables, LAYER_ITEMS, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[3], 32.0, "x 不受弹射影响");
        let expect = stg_core::tables::TABLES_V0.item_cfg[2].sprite as f32;
        assert_eq!(out[8], expect, "item_type→表 join(A1)");
    }

    #[test]
    fn empty_layer_returns_zero() {
        let g = stepped_game();
        let mut out = vec![0.0f32; layer_cap(LAYER_SHOTS) * FLOATS_PER_INSTANCE];
        assert_eq!(encode_layer(g.world.view(), g.tables, LAYER_SHOTS, &mut out), 0);
    }
```

`save.rs` 测试:

```rust
    #[test]
    fn save_load_roundtrip_checksum() {
        let mut g = crate::boot::boot("sub main() { loop { wait(60); } }", 7, 2).unwrap();
        let bytes = save(&g);
        let c0 = g.world.checksum();
        load_into(&mut g, &bytes).expect("load");
        assert_eq!(g.world.checksum(), c0);
    }

    #[test]
    fn load_bad_bytes_keeps_world_intact() {
        let mut g = crate::boot::boot("sub main() { loop { wait(60); } }", 7, 2).unwrap();
        let c0 = g.world.checksum();
        let mut bad = save(&g);
        bad[0] ^= 0xFF; // 毁 magic
        assert!(load_into(&mut g, &bad).is_err());
        assert_eq!(g.world.checksum(), c0, "失败不动世界");
    }
```

- [ ] **Step 2: 跑出编译失败**

Run: `cargo test -p stg-godot 2>&1 | head -15`
Expected: 未实现符号编译错(TDD 红)。

- [ ] **Step 3: 实现三模块**

`boot.rs`:

```rust
//! 纯 Rust:源码文本 → EclImage → World::new_game(spec §12)。零 gdext 类型。

use stg_core::ecl::image::EclImage;
use stg_core::step::World;
use stg_core::tables::{TABLES_V0, WorldTables};

pub struct Game {
    pub world: Box<World>,
    pub image: EclImage,
    pub tables: &'static WorldTables, // v1 恒 &TABLES_V0(follow-ups A3)
}

#[derive(Debug)]
pub enum BootError {
    /// 编译失败(带行列的渲染消息)。
    Compile(String),
    /// new_game 失败(TaskStartError 转述)。
    Start(String),
}

pub fn boot(ecl_source: &str, seed: u64, rank: i32) -> Result<Game, BootError> {
    let image = stg_ecl_compiler::lang::compile(ecl_source, "bridge.ecl").map_err(|errs| {
        let msg: Vec<String> = errs.iter().map(|e| e.render("bridge.ecl")).collect();
        BootError::Compile(msg.join("\n\n"))
    })?;
    let world =
        World::new_game(seed, rank, &image).map_err(|e| BootError::Start(format!("{e:?}")))?;
    Ok(Game { world, image, tables: &TABLES_V0 })
}
```

`frame.rs`:

```rust
//! 纯 Rust:四渲染层 → f32 实例缓冲编码器。**定点→浮点唯一转换点**(I1 边界)。
//! 布局(spec §11.2,GLES3 源码+headless 实测双源):
//! 12 float/实例 = [xx, yx, 0, ox, xy, yy, 0, oy] + custom[sprite, 0, 0, 0]。

use stg_core::bullets::BulletPool;
use stg_core::enemy::EnemyPool;
use stg_core::items::ItemPool;
use stg_core::math::{Angle, Fx};
use stg_core::shots::ShotPool;
use stg_core::tables::WorldTables;
use stg_core::world::WorldView;

pub const LAYER_BULLETS: usize = 0;
pub const LAYER_SHOTS: usize = 1;
pub const LAYER_ENEMIES: usize = 2;
pub const LAYER_ITEMS: usize = 3;
pub const LAYER_COUNT: usize = 4;
pub const FLOATS_PER_INSTANCE: usize = 12;

pub fn layer_cap(layer: usize) -> usize {
    match layer {
        LAYER_BULLETS => BulletPool::CAP,
        LAYER_SHOTS => ShotPool::CAP,
        LAYER_ENEMIES => EnemyPool::CAP,
        LAYER_ITEMS => ItemPool::CAP,
        _ => 0,
    }
}

#[inline]
fn fx_f32(v: Fx) -> f32 {
    v.raw() as f32 / 65536.0
}

#[inline]
fn write_instance(out: &mut [f32], slot: usize, x: f32, y: f32, cos: f32, sin: f32, sprite: u16) {
    let o = slot * FLOATS_PER_INSTANCE;
    out[o..o + FLOATS_PER_INSTANCE].copy_from_slice(&[
        cos, -sin, 0.0, x, sin, cos, 0.0, y, sprite as f32, 0.0, 0.0, 0.0,
    ]);
}

/// 活槽压实(池索引升序)写 `out` 前缀,返活数。`out.len() == layer_cap(layer)*12`。
/// 未知 layer → 0(P4-b no-op)。
pub fn encode_layer(
    view: WorldView<'_>,
    tables: &WorldTables,
    layer: usize,
    out: &mut [f32],
) -> u32 {
    let mut n: usize = 0;
    match layer {
        LAYER_BULLETS => {
            let p = view.bullets();
            let (xs, ys, angles, sprites) = (p.x(), p.y(), p.angle(), p.sprite());
            for i in p.iter_alive() {
                let rad = angles[i].raw() as f32 * (core::f32::consts::TAU / 65536.0);
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), rad.cos(), rad.sin(), sprites[i]);
                n += 1;
            }
        }
        LAYER_SHOTS => {
            let p = view.shots();
            let (xs, ys, sprites) = (p.x(), p.y(), p.sprite());
            for i in p.iter_alive() {
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, sprites[i]);
                n += 1;
            }
        }
        LAYER_ENEMIES => {
            let p = view.enemies();
            let (xs, ys, sprites) = (p.x(), p.y(), p.sprite());
            for i in p.iter_alive() {
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, sprites[i]);
                n += 1;
            }
        }
        LAYER_ITEMS => {
            let p = view.items();
            let (xs, ys, types) = (p.x(), p.y(), p.item_type());
            // A1 渲染 join:循环前提栈上 LUT(spec §2.1 性能拍板)
            let lut: [u16; stg_core::items::ITEM_TYPE_COUNT] =
                core::array::from_fn(|t| tables.item_cfg[t].sprite);
            for i in p.iter_alive() {
                let s = lut[types[i] as usize];
                write_instance(out, n, fx_f32(xs[i]), fx_f32(ys[i]), 1.0, 0.0, s);
                n += 1;
            }
        }
        _ => {}
    }
    n as u32
}
```

(`Angle` 若未直接用到 import 会告警——以编译为准删增 use。)

`save.rs`:

```rust
//! 纯 Rust:L1 存读的桥侧管道。换弹夹式载入 = 天然原子(失败不动旧世界)。

use crate::boot::Game;
use stg_core::step::World;

pub fn save(game: &Game) -> Vec<u8> {
    game.world.save_bytes(&game.image)
}

pub fn load_into(game: &mut Game, bytes: &[u8]) -> Result<(), String> {
    match World::load_bytes(bytes, game.tables, &game.image) {
        Ok(w) => {
            game.world = w;
            Ok(())
        }
        Err(e) => Err(format!("{e:?}")),
    }
}
```

- [ ] **Step 4: 跑绿 + 全量**

Run: `cargo test -p stg-godot && cargo test --workspace`
Expected: 新测全过、全量绿。测试红时先怀疑计划场景(实体是否 step 1 帧后存在/坐标是否被物理动过),**如实修场景断言的期望值须附推导注释**,不许无脑放宽。

- [ ] **Step 5: fmt/clippy + commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-godot/src
git commit -m "feat(godot): boot/frame/save 三纯模块——编译+new_game 管道、四层 12f 编码器(压实+LUT join)、换弹夹式存读;单测全覆盖零 gdext 依赖

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: WorldBridge 壳接线 + `godot_smoke.ecl` 全流程冒烟

**Files:**
- Modify: `crates/stg-godot/src/bridge.rs`(换全量)、`smoke/smoke.gd`(换全流程)
- Create: `smoke/godot_smoke.ecl`

**Interfaces:**
- Consumes: Task 3 的 `boot::boot/Game`、`frame::{encode_layer, layer_cap, LAYER_*, FLOATS_PER_INSTANCE}`、`save::{save, load_into}`;core 读口 `frame()/checksum()/take_requests()/view()/body.boss_ui`。

- [ ] **Step 1: bridge.rs 全量**(gdext API 名按机械调整许可①核;`godot_error!` 打日志)

```rust
//! gdext 壳:WorldBridge(spec §4 冻结面/§12 内部结构)。纯胶水零逻辑。
//! P4/FFI 铁律:错误不 panic 穿 FFI——no-op + false + 去重日志。

use godot::classes::RenderingServer;
use godot::prelude::*;

use crate::boot::{self, Game};
use crate::frame::{self, FLOATS_PER_INSTANCE, LAYER_COUNT};

/// 去重日志位(warned 位集)。
const W_NO_GAME: u32 = 1 << 0;
const W_BAD_LAYER: u32 = 1 << 1;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct WorldBridge {
    base: Base<Node>,
    game: Option<Game>,
    layers: [Option<Rid>; LAYER_COUNT],
    bufs: [Vec<f32>; LAYER_COUNT],
    warned: u32,
}

#[godot_api]
impl INode for WorldBridge {
    fn init(base: Base<Node>) -> Self {
        WorldBridge {
            base,
            game: None,
            layers: [None; LAYER_COUNT],
            bufs: Default::default(),
            warned: 0,
        }
    }
}

impl WorldBridge {
    fn warn_once(&mut self, bit: u32, msg: &str) {
        if self.warned & bit == 0 {
            self.warned |= bit;
            godot_error!("[stg] {msg}(同类后续不再报)");
        }
    }
}

#[godot_api]
impl WorldBridge {
    // ── 输入位与层号常量(GDScript 侧 WorldBridge.BTN_* 取用)─────
    #[constant]
    const BTN_UP: i64 = stg_core::input::BTN_UP as i64;
    #[constant]
    const BTN_DOWN: i64 = stg_core::input::BTN_DOWN as i64;
    #[constant]
    const BTN_LEFT: i64 = stg_core::input::BTN_LEFT as i64;
    #[constant]
    const BTN_RIGHT: i64 = stg_core::input::BTN_RIGHT as i64;
    #[constant]
    const BTN_SHOT: i64 = stg_core::input::BTN_SHOT as i64;
    #[constant]
    const BTN_BOMB: i64 = stg_core::input::BTN_BOMB as i64;
    #[constant]
    const BTN_FOCUS: i64 = stg_core::input::BTN_FOCUS as i64;
    #[constant]
    const LAYER_BULLETS: i64 = frame::LAYER_BULLETS as i64;
    #[constant]
    const LAYER_SHOTS: i64 = frame::LAYER_SHOTS as i64;
    #[constant]
    const LAYER_ENEMIES: i64 = frame::LAYER_ENEMIES as i64;
    #[constant]
    const LAYER_ITEMS: i64 = frame::LAYER_ITEMS as i64;

    #[func]
    fn new_game(&mut self, ecl_source: GString, seed: i64, rank: i64) -> bool {
        match boot::boot(&ecl_source.to_string(), seed as u64, rank as i32) {
            Ok(g) => {
                self.game = Some(g);
                self.warned = 0;
                true
            }
            Err(e) => {
                godot_error!("[stg] new_game 失败:{e:?}");
                false
            }
        }
    }

    #[func]
    fn step_frame(&mut self, buttons: i64) {
        let Some(game) = self.game.as_mut() else {
            self.warn_once(W_NO_GAME, "step_frame:尚未 new_game,no-op");
            return;
        };
        let mut input = stg_core::input::InputFrame::empty(game.world.frame());
        input.actions[0].buttons = buttons as u32;
        stg_core::step::step_with_director(&mut game.world, game.tables, &game.image, &input, |_| {});
        // 已注册层:编码 + 一次上传 + 可见数
        let mut rs = RenderingServer::singleton();
        for layer in 0..LAYER_COUNT {
            let Some(rid) = self.layers[layer] else { continue };
            let n = frame::encode_layer(game.world.view(), game.tables, layer, &mut self.bufs[layer]);
            rs.multimesh_set_buffer(rid, &PackedFloat32Array::from(self.bufs[layer].as_slice()));
            rs.multimesh_set_visible_instances(rid, n as i32);
        }
    }

    #[func]
    fn register_layer(&mut self, kind: i64, multimesh_rid: Rid) -> bool {
        let layer = kind as usize;
        if layer >= LAYER_COUNT {
            self.warn_once(W_BAD_LAYER, "register_layer:未知层号,no-op");
            return false;
        }
        let cap = frame::layer_cap(layer);
        let rs = RenderingServer::singleton();
        let got = rs.multimesh_get_instance_count(multimesh_rid);
        if got as usize != cap {
            godot_error!("[stg] register_layer:层 {layer} 需 instance_count=={cap},实际 {got}");
            return false;
        }
        self.bufs[layer] = vec![0.0; cap * FLOATS_PER_INSTANCE];
        self.layers[layer] = Some(multimesh_rid);
        true
    }

    #[func]
    fn take_requests(&mut self) -> Array<Dictionary> {
        let mut arr = Array::new();
        let Some(game) = self.game.as_ref() else {
            return arr;
        };
        let f = game.world.frame() as i64;
        for r in game.world.take_requests() {
            let mut d = Dictionary::new();
            d.set("id", r.id as i64);
            d.set("seq", r.seq as i64);
            d.set("frame", f);
            let mut args = Array::<i64>::new();
            for a in r.args {
                args.push(a as i64);
            }
            d.set("args", args);
            arr.push(&d);
        }
        arr
    }

    #[func]
    fn save_state(&self) -> PackedByteArray {
        match self.game.as_ref() {
            Some(g) => PackedByteArray::from(crate::save::save(g).as_slice()),
            None => PackedByteArray::new(),
        }
    }

    #[func]
    fn load_state(&mut self, bytes: PackedByteArray) -> bool {
        let Some(game) = self.game.as_mut() else {
            self.warn_once(W_NO_GAME, "load_state:尚未 new_game,no-op");
            return false;
        };
        match crate::save::load_into(game, bytes.as_slice()) {
            Ok(()) => true,
            Err(e) => {
                godot_error!("[stg] load_state 失败:{e}(世界原状不动)");
                false
            }
        }
    }

    #[func]
    fn frame(&self) -> i64 {
        self.game.as_ref().map_or(-1, |g| g.world.frame() as i64)
    }

    #[func]
    fn checksum(&self) -> i64 {
        self.game.as_ref().map_or(0, |g| g.world.checksum() as i64)
    }

    #[func]
    fn hud_player(&self) -> Dictionary {
        let mut d = Dictionary::new();
        let Some(g) = self.game.as_ref() else { return d };
        let p = &g.world.view().players()[0];
        d.set("x", p.x.raw() as f64 / 65536.0);
        d.set("y", p.y.raw() as f64 / 65536.0);
        d.set("lives", p.lives as i64);
        d.set("bombs", p.bombs as i64);
        d.set("life_pieces", p.life_pieces as i64);
        d.set("bomb_pieces", p.bomb_pieces as i64);
        d.set("power", p.power as i64);
        d.set("score", p.score as i64);
        d.set("graze", p.graze as i64);
        d.set("life_state", p.life_state as i64);
        d.set("invuln", p.invuln as i64);
        d
    }

    #[func]
    fn hud_boss(&self, i: i64) -> Dictionary {
        let mut d = Dictionary::new();
        let Some(g) = self.game.as_ref() else { return d };
        let Some(s) = g.world.body.boss_ui.get(i as usize) else { return d };
        d.set("active", s.active as i64);
        d.set("hp_ratio", s.hp_ratio.raw() as f64 / 65536.0);
        d.set("spell_id", s.spell_id as i64);
        d.set("timer_frames", s.timer_frames as i64);
        d.set("phase_left", s.phase_left as i64);
        d
    }

    #[func]
    fn hud_spell(&self, i: i64) -> Dictionary {
        let mut d = Dictionary::new();
        let Some(g) = self.game.as_ref() else { return d };
        let Some(s) = g.world.view().spells().get(i as usize) else { return d };
        d.set("active", s.active as i64);
        d.set("spell_id", s.spell_id as i64);
        d.set("frames_left", s.frames_left as i64);
        d.set("bonus_now", s.bonus_now as i64);
        d.set("capture_ok", s.capture_ok as i64);
        d.set("flags", s.flags as i64);
        d
    }

    #[func]
    fn player_pos(&self) -> Vector2 {
        let Some(g) = self.game.as_ref() else {
            return Vector2::ZERO;
        };
        let p = &g.world.view().players()[0];
        Vector2::new(p.x.raw() as f32 / 65536.0, p.y.raw() as f32 / 65536.0)
    }

    #[func]
    fn fields_info(&self) -> Array<Dictionary> {
        let mut arr = Array::new();
        let Some(g) = self.game.as_ref() else { return arr };
        let view = g.world.view();
        let p = view.fields();
        let (xs, ys, rads, lives) = (p.x(), p.y(), p.radius(), p.life());
        for i in p.iter_alive() {
            let mut d = Dictionary::new();
            d.set("x", xs[i].raw() as f64 / 65536.0);
            d.set("y", ys[i].raw() as f64 / 65536.0);
            d.set("radius", rads[i].raw() as f64 / 65536.0);
            d.set("life", lives[i] as i64);
            arr.push(&d);
        }
        arr
    }

    #[func]
    fn ping(&self) -> i64 {
        42
    }
}
```

(SpellSlot/BossUiSlot/PlayerState 字段名若与实际有出入,以代码为准同款调整并入报告。hud 的 f64 转换在壳层,非 frame.rs 唯一点的破例——它们是 UI 便利读口,spec §4 既定。)

- [ ] **Step 2: `smoke/godot_smoke.ecl`**(脚本自举,数据驱动 boot 端到端;builtin 形参按许可②核)

```
sub main() {
    _ = spawn_enemy(0.0fx, -160.0fx, 9999, 0, 100);
    loop {
        _ = fire(1, 0.0fx, -160.0fx, 1.5fx, 0, 0, 0, -1);
        _ = emit_req(64, 1, 2, 3, 4, 5, 6);
        wait(30);
    }
}
```

- [ ] **Step 3: `smoke/smoke.gd` 换全流程**

```gdscript
extends SceneTree

func fail(msg: String):
	push_error("SMOKE FAIL: " + msg)
	quit(1)

func _init():
	if not ClassDB.class_exists("WorldBridge"):
		fail("WorldBridge 未注册"); return
	var b = ClassDB.instantiate("WorldBridge")
	if b.ping() != 42: fail("ping"); return
	# 未开局违约路径:no-op 不炸
	b.step_frame(0)
	if b.frame() != -1: fail("未开局 frame 应为 -1"); return
	var src := FileAccess.get_file_as_string("res://godot_smoke.ecl")
	if not b.new_game(src, 7, 2): fail("new_game"); return
	if b.frame() != 0: fail("frame0"); return
	var reqs_seen := 0
	for i in range(120):
		b.step_frame(0)
		reqs_seen += b.take_requests().size()
	if b.frame() != 120: fail("frame120"); return
	if reqs_seen == 0: fail("通道 B 零请求——emit_req 没到达"); return
	var c120 = b.checksum()
	if c120 == 0: fail("checksum 0"); return
	var sav = b.save_state()
	if sav.size() == 0: fail("save 空"); return
	for i in range(30): b.step_frame(WorldBridge.BTN_LEFT)
	var c150 = b.checksum()
	if c150 == c120: fail("步进未改变校验和"); return
	if not b.load_state(sav): fail("load"); return
	if b.checksum() != c120: fail("载入未回到 c120"); return
	for i in range(30): b.step_frame(WorldBridge.BTN_LEFT)
	if b.checksum() != c150: fail("恢复重演 ≠ 未离开(确定性破)"); return
	if b.hud_player().get("lives", -1) < 0: fail("hud_player"); return
	b.free()
	print("SMOKE OK")
	quit(0)
```

- [ ] **Step 4: 跑冒烟 + 全量**

```bash
crates/stg-godot/smoke/run-smoke.sh
cargo test --workspace
```

Expected: `SMOKE OK` + 全量绿。冒烟任何 fail:先查场景/API 对接,断言语义不许改;真身分歧(恢复重演不等)= BLOCKED。

- [ ] **Step 5: fmt/clippy + commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/stg-godot
git commit -m "feat(godot): WorldBridge 冻结面全量接线 + godot_smoke.ecl 脚本自举 + headless 全流程冒烟(step/save-load 续接/通道 B/违约 no-op)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: 收口簿记 + 金向量终拍

**Files:**
- Modify: `CLAUDE.md`(仓库结构加 stg-godot 块 + M2 里程碑行更新)、`PROGRESS.md`(史行 + 现在段)、`docs/bridge-adaptation-notes.md`(真实新坑追加)

- [ ] **Step 1: 文档三件**

- `CLAUDE.md` 仓库结构 crates/ 块加(照既有缩进风格,置于 stg-harness 之后):
  `stg-godot/       M2 桥:WorldBridge gdext cdylib(boot/frame/save 纯模块+壳;smoke/ headless 冒烟)`;
  Milestone 地图 M2 行改为:`**M2(进行中)** stg-godot WorldBridge 已落地(2026-07-24,桥刀);余量 = 真 Godot 工程(场景/MultiMesh 节点/分发器/输入映射)`。
- `PROGRESS.md`:史表加行 `2026-07-24 stg-godot 桥刀(工具链 1.94/gdext 0.5.4/三纯模块/冻结面/headless 冒烟)`;「现在」段重写:M2 桥已通,下一步 = Godot 工程刀(读 spec §10 注意事项)或 RL 线 stg-py。
- `docs/bridge-adaptation-notes.md`:把 Task 2/4 报告里**真实踩到的坑**逐条按该文档格式追加(如 `--import` 缓存、gdext API 调整点、GDScript↔桥类型对接);没踩到的不编造。

- [ ] **Step 2: 金向量终拍 + 终验四连**

```bash
cargo run --release -p stg-harness -- golden --out .superpowers/bridge/golden-final.txt
diff .superpowers/bridge/golden-194.txt .superpowers/bridge/golden-final.txt && echo GOLDEN-BRIDGE-OK
cargo test --workspace && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
crates/stg-godot/smoke/run-smoke.sh
```

Expected: `GOLDEN-BRIDGE-OK` + 全绿 + `SMOKE OK`。

- [ ] **Step 3: commit**

```bash
git add CLAUDE.md PROGRESS.md docs/bridge-adaptation-notes.md
git commit -m "docs: M2 桥刀收口——仓库地图/进度/外接适配坑沉淀,金向量全分支逐字节稳定终拍

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review 记录(plan 作者自查)

1. **Spec 覆盖**:§3 结构→T2/T3;§4 冻结面十方法+常量→T4;§5 数据流(step 内编码+上传)→T4;§6 错误三类→T4(编译错/违约 no-op+去重日志/载入失败不动世界,冒烟盖违约路径);§8 测试与 DoD→T3 单测+T4 冒烟+T5 终拍;§9 产物→T2(.gdextension/reloadable=false;拷贝脚本属未来游戏工程,spec 原文即如此);§11.1 工具链→T1;§12 内部结构→T4 逐项。无缺口。
2. **占位符**:无 TBD;两处"机械调整许可"是显式受控通道非占位。
3. **类型一致性**:`encode_layer(view,&tables,layer,&mut [f32])->u32`/`layer_cap`/`boot::Game`/`save(&Game)` 在 T3 定义、T4 消费,签名一致;冒烟断言链(c120/c150 恢复重演)与 L2"恢复≡未离开"口径一致。
