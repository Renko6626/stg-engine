# 弹幕颜色轴 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `.ecl` 作者用「弹型 + 颜色」两个参数发弹，底层仍只存一个 `sprite` 格号；非法组合（含图集空格）在编译期带行列报错。

**Architecture:** 采纳 ZUN 形态——**接口分开、存储合并**。外观表长成 `形数 × color_stride` 的整齐矩形，表索引 ≡ 图集格号 ≡ 池 `sprite`（identity）；表层语言 `fire`/`batch`/`set_sprite` 收两个参，**编译器折叠成一个 appearance 值**，字节码 / syscall / 弹池 / shader 全不改。色轴宽度 `color_stride` 是**表数据**而非引擎常量，弹型名/色名由**内容包 `.ecl` 的 `const`** 提供而非引擎注册表——两条都是为了 mod 作者与内建内容地位对等。

**Tech Stack:** Rust 1.94.0（edition 2024, resolver 3）/ `stg-core`（断层线以下，纯整数）/ `stg-ecl-compiler`（表层语言）/ GDScript（Godot 4.6）

**Spec:** `docs/superpowers/specs/2026-07-26-bullet-color-axis-design.md`

## Global Constraints

- **I1–I7 不分阶段持有**：本刀新增代码在 `stg-core` 内**不得**出现 `f32`/`f64`、系统时钟、宿主 RNG、`HashMap`/`HashSet`；确定性容器用 `BTreeMap`/`BTreeSet`。
- **引擎里不得出现"每形 16 色"这个数**。`16` 只允许出现在 `build_tables_v0`（内建内容包的数据）与占位图集生成器里；一切判据从 `WorldTables::color_stride` 读。
- **弹型名与颜色名不进 `stg-core`**：内容包 `.ecl` 的 `const` 提供。
- **P4-b 口径**：脚本传坏参 → `FAULT_BAD_OP`，且**先验后建**（任何世界写之前完成校验）。
- **表格式变更须 bump**：`TABLE_VERSION` 从 `2` 改 `3`。
- **金向量无 committed 基线**（CI 只做三平台互比），所以"重 bless"= 本地跑通 + CI 三平台绿；但 `crates/stg-core/src/tables/tables_v0.bin` **必须重烘焙并 commit**，`verify-tables` 会逐位对拍。
- **每个任务结束前**必须全绿：
  ```bash
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- **commit 结尾**附：`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **分支**：开短命特性分支 `feat/bullet-color-axis`，合入前 CI 绿。

## File Structure

| 文件 | 职责 | 任务 |
|---|---|---|
| `crates/stg-core/src/tables.rs` | `AppearanceCfg.valid` / `WorldTables.color_stride` / 12×16 生成 / validate / 序列化 | T1 |
| `crates/stg-core/src/tables/tables_v0.bin` | 重烘焙产物（committed） | T1 |
| `crates/stg-core/src/ecl/syscall.rs` | 运行期空格判据（先验后建） | T2 |
| `crates/stg-ecl-compiler/src/lang/mod.rs` | `compile_with_options` 收 `Option<&WorldTables>` + 注入 `BULLET_COLOR_STRIDE` | T3 |
| `crates/stg-ecl-compiler/src/lang/typeck.rs` `typeck/exprs.rs` `typeck/typed_ast.rs` | 表穿线 + 形/色三判据 + `const_val` helper | T3 / T4 |
| `crates/stg-ecl-compiler/src/lang/builtins.rs` | `fire` 7→8 参、`batch` 9→10 参 | T4 |
| `crates/stg-ecl-compiler/src/lang/codegen.rs` | 两参折叠（常量折字面量 / 变量发 ADD）+ xform staging 折叠 | T4 |
| `crates/stg-ecl-compiler/src/lang/xform_map.rs` `slots.rs` | `set_sprite` 两参（`OpFold2`） | T4 |
| `crates/stg-core/src/consts.rs` | 删 ② 段四个 `APPEARANCE_*` | T4 |
| `godot/ecl/demo/bullets.ecl` | **新建**：demo 内容包词表 | T4 |
| `crates/stg-harness/scenes/rainbow.ecl` 等 5 个 `.ecl` | 调用点迁移 | T4 |
| `godot/tools/gen_atlas.gd` `godot/scripts/playfield.gd` | 16×12 占位图集 + `ROWS` 逐层化 | T5 |
| `docs/{render-contract,ecl-lang,xform-ops}.md` `PROGRESS.md` `docs/follow-ups.md` | 文档收口 | T6 |

---

### Task 1: 外观表长成 12×16 整齐矩形

**Files:**
- Modify: `crates/stg-core/src/tables.rs`
- Modify（产物，重烘焙）: `crates/stg-core/src/tables/tables_v0.bin`

**Interfaces:**
- Consumes: 无（首任务）
- Produces:
  - `pub struct AppearanceCfg { pub radius: Fx, pub sprite: u16, pub valid: bool }`
  - `WorldTables` 新增 `pub color_stride: u16`
  - 内建表：`color_stride == 16`，`appearances.len() == 192`，`appearances[i].sprite == i as u16`
  - `TABLE_VERSION == 3`

- [ ] **Step 1: 写失败测试——替换 `appearances_v0_shape`**

在 `crates/stg-core/src/tables.rs` 的 `#[cfg(test)] mod tests` 里，**删除**现有的
`appearances_v0_shape` 测试（它断言 `len() == 4`、`sprites == vec![0,1,2,3]`、四个
`APPEARANCE_*` 值），换成：

```rust
    /// 内建 appearance 表 = 12 形 × 16 色的整齐矩形（identity + 同形同半径 + 空格掩码）。
    /// 判别力：对调 `SHAPE_RADIUS` 中两个**不同**的值必须让本测试变红。
    #[test]
    fn appearances_v0_is_12x16_grid() {
        let t = &*TABLES_V0;
        assert_eq!(t.color_stride, 16, "内建内容包图集 16 列");
        assert_eq!(t.appearances.len(), 12 * 16);

        // identity：表索引 ≡ 图集格号 ≡ 池 sprite 值
        for (i, a) in t.appearances.iter().enumerate() {
            assert_eq!(a.sprite as usize, i, "第 {i} 行 sprite 必须等于行号（identity）");
        }

        // 逐形半径钉死（判别腿：SHAPE_RADIUS 错序即红）
        let expect = [3, 3, 4, 6, 4, 4, 3, 5, 8, 6, 6, 4];
        for shape in 0..12usize {
            for color in 0..16usize {
                assert_eq!(
                    t.appearances[shape * 16 + color].radius,
                    Fx::from_int(expect[shape]),
                    "形 {shape} 色 {color} 半径应为 {}px（同形 16 行必然同半径）",
                    expect[shape]
                );
            }
        }

        // 空格掩码：HEART(9)/BUTTERFLY(10) 高 4 色为空格，其余全有图
        for shape in 0..12usize {
            for color in 0..16usize {
                let want = !matches!(shape, 9 | 10) || color < 12;
                assert_eq!(
                    t.appearances[shape * 16 + color].valid,
                    want,
                    "形 {shape} 色 {color} 的 valid 与掩码不符"
                );
            }
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core appearances_v0_is_12x16_grid`
Expected: 编译失败——`WorldTables` 没有 `color_stride` 字段、`AppearanceCfg` 没有 `valid`。

- [ ] **Step 3: 改结构体**

`crates/stg-core/src/tables.rs`，`WorldTables` 加字段（放在 `appearances` 之前）：

```rust
    /// 每种弹型占的连续色数（= 图集列数）。内建 = 16；mod 表自定义。
    /// **引擎不得硬编码这个数**——一切形/色判据从这里读（spec §2 硬约束一）。
    pub color_stride: u16,
    /// 弹外观表（索引 = appearance id = 图集格号 = 池 sprite 值，identity）。
    pub appearances: Box<[AppearanceCfg]>,
```

`AppearanceCfg` 加字段：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
    /// 该格图集里是否真有图。`false` = 空格：创建被拒（P4-b Fault），
    /// **不是**"半径为 0 的弹"——空格行照样带本形状的半径，见 spec §4.3。
    pub valid: bool,
}
```

- [ ] **Step 4: 改 `build_tables_v0` 的 appearance 段**

把现有的 `let mut appearances = [AppearanceCfg { .. }; 4];` 连同其后四条
`appearances[APPEARANCE_* as usize] = ...` 赋值**整段替换**为：

```rust
    // ── 内建内容包的弹型数据（**不是引擎结构常量**：mod 表自带自己的一份）──────
    // 12 形 × 16 色的整齐矩形。稀疏弹型（HEART/BUTTERFLY）仍占满 16 列，
    // 用不到的列由掩码标成空格，寻址因此保持 `形 × stride + 色`（spec §5.2）。
    const BUILTIN_COLOR_STRIDE: u16 = 16;
    const SHAPE_RADIUS: [i32; 12] = [3, 3, 4, 6, 4, 4, 3, 5, 8, 6, 6, 4];
    //  第 9/10 形（HEART/BUTTERFLY）只做了低 12 色，高 4 色留空格
    const SHAPE_COLOR_MASK: [u16; 12] = [
        0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0x0FFF, 0x0FFF,
        0xFFFF,
    ];

    let stride = BUILTIN_COLOR_STRIDE as usize;
    let mut appearances = Vec::with_capacity(SHAPE_RADIUS.len() * stride);
    for (shape, &r) in SHAPE_RADIUS.iter().enumerate() {
        for color in 0..stride {
            appearances.push(AppearanceCfg {
                radius: Fx::from_int(r),
                sprite: (shape * stride + color) as u16, // identity
                valid: SHAPE_COLOR_MASK[shape] >> color & 1 == 1,
            });
        }
    }
```

并把结构体构造处的 `appearances: Box::new(appearances),` 改为
`color_stride: BUILTIN_COLOR_STRIDE,` + `appearances: appearances.into_boxed_slice(),`。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p stg-core appearances_v0_is_12x16_grid`
Expected: PASS。

> 若报 `APPEARANCE_SMALL` 等未使用：**保留** `consts.rs` 的 ② 段不动（T4 才删），
> 但 `tables.rs` 顶部若因删掉赋值而使 `pub use crate::consts::{APPEARANCE_*}` 变成
> 未使用导入，暂时保留该 `pub use`（它是对外再导出，不会触发 unused 警告）。

- [ ] **Step 6: 写序列化往返的失败测试**

在 `mod tests` 里加：

```rust
    /// 规范字节往返必须带上 `color_stride` 与逐行 `valid`（防"新字段没进格式"）。
    #[test]
    fn bytes_roundtrip_carries_stride_and_valid() {
        let t = build_tables_v0();
        let back = WorldTables::from_bytes(&t.to_bytes()).expect("往返必须成功");
        assert_eq!(back.color_stride, t.color_stride);
        assert_eq!(back.appearances.len(), t.appearances.len());
        for (i, (a, b)) in t.appearances.iter().zip(back.appearances.iter()).enumerate() {
            assert_eq!(a.valid, b.valid, "第 {i} 行 valid 未往返");
            assert_eq!(a.sprite, b.sprite, "第 {i} 行 sprite 未往返");
            assert_eq!(a.radius, b.radius, "第 {i} 行 radius 未往返");
        }
        // 空格行确实存在（否则本测试对 valid 无判别力）
        assert!(back.appearances.iter().any(|a| !a.valid), "内建表必须含空格行");
    }
```

- [ ] **Step 7: 跑测试确认失败**

Run: `cargo test -p stg-core bytes_roundtrip_carries_stride_and_valid`
Expected: FAIL——`valid` 全为 `from_bytes` 造出的默认值 / 或 `color_stride` 不等。

- [ ] **Step 8: 改序列化格式并 bump 版本**

`crates/stg-core/src/tables.rs`：

1. `const TABLE_VERSION: u16 = 2;` → `= 3;`（格式变更，见 Global Constraints）。
2. `to_bytes` 里 `item_gravity` 之后、`appearances.len()` 之前插一行，并给每行补 `valid`：

```rust
        out.extend_from_slice(&self.item_gravity.raw().to_le_bytes());
        out.extend_from_slice(&self.color_stride.to_le_bytes());
        out.extend_from_slice(&(self.appearances.len() as u32).to_le_bytes());
        for a in self.appearances.iter() {
            out.extend_from_slice(&a.radius.raw().to_le_bytes());
            out.extend_from_slice(&a.sprite.to_le_bytes());
            out.push(u8::from(a.valid));
        }
```

3. `from_bytes` 对称：

```rust
        let item_gravity = r.fx()?;
        let color_stride = r.u16()?;

        let na = r.u32()? as usize;
        let mut appearances = Vec::with_capacity(na);
        for _ in 0..na {
            appearances.push(AppearanceCfg {
                radius: r.fx()?,
                sprite: r.u16()?,
                valid: r.u8()? != 0,
            });
        }
```

4. `from_bytes` 末尾构造 `WorldTables { ... }` 处加 `color_stride,`。

- [ ] **Step 9: 跑测试确认通过**

Run: `cargo test -p stg-core bytes_roundtrip_carries_stride_and_valid`
Expected: PASS。

- [ ] **Step 10: 写 `validate` 三条新规的失败测试**

```rust
    /// validate 新三条：stride 非零 / 行数是 stride 整数倍 / 每形第 0 色必须有图。
    #[test]
    fn validate_rejects_bad_color_grid() {
        // ① stride 为 0
        let mut bad = build_tables_v0();
        bad.color_stride = 0;
        assert!(!bad.validate(), "color_stride = 0 必须被拒");

        // ② 行数不是 stride 的整数倍（矩形被破坏）
        let mut ragged = build_tables_v0();
        let mut rows = ragged.appearances.to_vec();
        rows.pop();
        ragged.appearances = rows.into_boxed_slice();
        assert!(!ragged.validate(), "行数非 stride 整数倍必须被拒");

        // ③ 某形第 0 色是空格（形状名会指向不可用格）
        let mut hole = build_tables_v0();
        let mut rows = hole.appearances.to_vec();
        rows[2 * 16].valid = false; // 第 2 形第 0 色
        hole.appearances = rows.into_boxed_slice();
        assert!(!hole.validate(), "某形第 0 色为空格必须被拒");

        // 判别力反证：原表必须通过
        assert!(build_tables_v0().validate(), "内建表本身必须过 validate");
    }
```

- [ ] **Step 11: 跑测试确认失败**

Run: `cargo test -p stg-core validate_rejects_bad_color_grid`
Expected: FAIL——三个坏表当前都会被判为合法。

- [ ] **Step 12: 实现 `validate` 三条**

在 `WorldTables::validate` **开头**（现有 appearance 半径检查之前）插入：

```rust
        // 颜色轴：表必须是 `形数 × color_stride` 的整齐矩形
        let stride = self.color_stride as usize;
        if stride == 0 || self.appearances.is_empty() || self.appearances.len() % stride != 0 {
            return false;
        }
        // 每形第 0 色必须有图——内容包词表里的弹型名恒指向可用格（spec §5）
        if self
            .appearances
            .chunks_exact(stride)
            .any(|shape_row| !shape_row[0].valid)
        {
            return false;
        }
```

- [ ] **Step 13: 跑测试确认通过**

Run: `cargo test -p stg-core validate_rejects_bad_color_grid`
Expected: PASS。

- [ ] **Step 14: 重烘焙 `tables_v0.bin` 并对拍**

```bash
cargo run -p stg-harness -- bake-tables
cargo run -p stg-harness -- verify-tables
```
Expected: `verify-tables: 全部表与 commit 字节一致 ✔`（bake 已写入新字节，verify 随即对拍自身）。
文件 `crates/stg-core/src/tables/tables_v0.bin` 应变大（约 840B → 约 2.1KB）。

- [ ] **Step 15: 修好因表变形而失效的既有测试**

Run: `cargo test --workspace`
逐个修（**只改断言口径，不改被测行为**）：

- `tables.rs::builtin_appearances_exactly_cover_table_symbols`——原断言"appearances 长度
  恰等于 ② 命名集大小"。现在 ② 仍是 4 个符号而表 192 行，改为**只断言覆盖**：
  ```rust
        for c in crate::consts::TABLE_SYMBOLS {
            assert!(
                (c.value as usize) < TABLES_V0.appearances.len(),
                "② 符号 {} 必须落在 appearances 内",
                c.name
            );
        }
  ```
  （"恰覆盖无空洞"这条随 T4 删掉 ② 段一并消失，此处先降级为覆盖断言。）
- `tables.rs::validate_rejects_table_symbol_without_appearance_row`——构造的坏表把
  `appearances` 换成 1 行，现在会先被 Step 12 的"矩形/第 0 色"规则拒掉，断言仍成立
  （`!validate()`），**但要把 `color_stride` 一并设为 1** 才是在测 join 那条腿：
  ```rust
        t.color_stride = 1;
        t.appearances = Box::new([AppearanceCfg { radius: Fx::from_int(2), sprite: 0, valid: true }]);
  ```
- 任何直接构造 `AppearanceCfg { .. }` 字面量的地方（`tables.rs` 内多处坏表构造、
  `ecl/syscall.rs` 测试若有）补 `valid: true`。

- [ ] **Step 16: 全绿并提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/stg-core/src/tables.rs crates/stg-core/src/tables/tables_v0.bin
git commit -m "$(cat <<'EOF'
feat(core): 外观表长成 12×16 整齐矩形——color_stride 进表 + 逐行 valid

表索引 ≡ 图集格号 ≡ 池 sprite（identity）；同形 16 行半径由 SHAPE_RADIUS 单源
生成，稀疏弹型占满 stride、用不到的列由掩码标成空格。色轴宽度是表数据不是引擎
常量（mod 表可自定义）。TABLE_VERSION 2→3，tables_v0.bin 重烘焙。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: syscall 运行期空格判据（先验后建）

**Files:**
- Modify: `crates/stg-core/src/ecl/syscall.rs`（`sys_create_bullet` / `sys_create_bullets_batch` 及其测试）

**Interfaces:**
- Consumes: T1 的 `AppearanceCfg.valid`
- Produces: 两个 syscall 对空格 id 返回 `Err(FAULT_BAD_OP)`，且**弹未被创建**

- [ ] **Step 1: 写失败测试**

在 `crates/stg-core/src/ecl/syscall.rs` 的 `#[cfg(test)] mod tests` 里加两条。
**空格 id 取内建表里第 9 形第 12 色 = `9 * 16 + 12 = 156`**（T1 掩码 `0x0FFF` 的第一个空格）：

```rust
    /// 空格 appearance（图集该格没有图）→ Fault，且**弹未被创建**（先验后建）。
    /// 这是"隐形弹"（有判定无图像）的运行期闸；编译期同款判据见 lang::typeck。
    #[test]
    fn create_bullet_blank_cell_faults_without_creating() {
        const BLANK: i32 = 9 * 16 + 12; // 第 9 形（掩码 0x0FFF）的第一个空格
        assert!(
            !TABLES_V0.appearances[BLANK as usize].valid,
            "前提：{BLANK} 必须是空格行，否则本测试无判别力"
        );
        let mut h = harness_with_bullet_owner(); // 见下方 Step 3 的说明
        let before = h.world.body().bullets.alive_count();
        let r = call_create_bullet(&mut h, BLANK, 0, 0, 0, 0, 0, 0, -1);
        assert_eq!(r, Err(FAULT_BAD_OP), "空格 appearance 必须 Fault");
        assert_eq!(
            h.world.body().bullets.alive_count(),
            before,
            "Fault 时不得留下半成品弹（先验后建）"
        );
    }

    /// 批量入口同款（两族同构，别只补一边）。
    #[test]
    fn create_bullets_batch_blank_cell_faults_without_creating() {
        const BLANK: i32 = 9 * 16 + 12;
        let mut h = harness_with_bullet_owner();
        let before = h.world.body().bullets.alive_count();
        let r = call_create_bullets_batch(&mut h, BLANK, 0, 0, 4, 0, 4096, 1, 65536, 0);
        assert_eq!(r, Err(FAULT_BAD_OP));
        assert_eq!(h.world.body().bullets.alive_count(), before);
    }
```

> **写测试前先读**同文件里既有的
> `spawn_enemy_bad_task_script_faults_without_enemy` 与
> `sys_create_bullet_bad_task_script_faults_before_creating`——**照抄它们的 harness
> 构造方式与调用辅助函数**（本文件已有一套压栈调 syscall 的测试脚手架，名字以那两条
> 测试里实际使用的为准；上面的 `harness_with_bullet_owner` /
> `call_create_bullet` / `alive_count` 是占位写法，**必须替换成该文件里真实存在的
> 同款助手**）。若没有现成的"数活弹"手段，用 `iter_alive().count()`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core blank_cell_faults`
Expected: FAIL——空格 id 当前被当作合法行，弹被正常创建。

- [ ] **Step 3: 实现判据**

`sys_create_bullet` 里，紧跟现有的表查找之后：

```rust
    let Some(cfg) = ctx.tables.appearances.get(appearance as usize) else {
        return Err(FAULT_BAD_OP);
    };
    // 空格格（图集该格没有图）→ 拒。放行会造出"有判定但画面上什么都没有"的隐形弹，
    // 而金向量/冒烟都抓不到它（校验和不关心贴图内容）。先验后建：此处尚未写世界。
    if !cfg.valid {
        return Err(FAULT_BAD_OP);
    }
```

`sys_create_bullets_batch` 里同样位置插入**逐字相同**的三行。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p stg-core blank_cell_faults`
Expected: PASS（两条）。

- [ ] **Step 5: 全绿并提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/stg-core/src/ecl/syscall.rs
git commit -m "$(cat <<'EOF'
feat(core): 空格 appearance 运行期拒收——隐形弹的最后一道闸

fire/batch 两族同构补 valid 判据,先验后建(Fault 时不留半成品弹)。
编译期同款判据随两参糖落地(见 lang::typeck)。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 编译器穿线——前端拿到绑定的表 + 注入 `BULLET_COLOR_STRIDE`

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/mod.rs`（`compile_with_options` / `compile_for_table`）
- Modify: `crates/stg-ecl-compiler/src/lang/typeck.rs`（`check` 收表并存进 `Checker`）
- Modify: 全部 `compile_with_options` 调用点（约 13 处，多为测试）

**Interfaces:**
- Consumes: T1 的 `WorldTables::color_stride`
- Produces:
  - `pub fn compile_with_options(src, file, options, engine_consts: &[EngineConst], table: Option<&WorldTables>) -> Result<CompiledEcl, Vec<CompileError>>`
    （**`content_hash: u64` 参被 `table` 取代**：`Some(t)` → 用 `t.content_hash`；`None` → `0` = 未绑定）
  - `pub fn check(prog, engine_consts, table: Option<&WorldTables>) -> Result<TypedInfo, Vec<CompileError>>`
  - 绑定表时脚本可见常量 `BULLET_COLOR_STRIDE`（`int`，值 = `table.color_stride`）

- [ ] **Step 1: 写失败测试**

在 `crates/stg-ecl-compiler/src/lang/typeck/tests.rs`（或该 crate 里既有的 lang 测试模块，
以实际存在者为准）加：

```rust
    /// 绑定表时注入表派生常量 `BULLET_COLOR_STRIDE`（值来自表，不是引擎硬编码）。
    #[test]
    fn bound_table_injects_color_stride_const() {
        let src = "sub main() { var w: int = BULLET_COLOR_STRIDE; _ = w; }";
        let img = crate::lang::compile(src, "t.ecl").expect("绑定内建表应能引用 stride 常量");
        let _ = img;
    }

    /// 未绑定表（table = None）时不注入——脚本引用它应报"未知标识符"，
    /// 而不是悄悄拿到某个默认值。
    #[test]
    fn unbound_table_does_not_inject_color_stride() {
        let src = "sub main() { var w: int = BULLET_COLOR_STRIDE; _ = w; }";
        let errs = crate::lang::compile_with_options(
            src,
            "t.ecl",
            crate::lang::CompileOptions { debug_info: crate::lang::DebugInfo::None },
            stg_core::consts::ENGINE_CONSTS,
            None,
        )
        .expect_err("未绑定表不得注入 stride 常量");
        assert!(!errs.is_empty());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-ecl-compiler color_stride`
Expected: 编译失败（`compile_with_options` 第 5 参仍是 `u64`）+ 第一条测试报未知标识符。

- [ ] **Step 3: 改 `compile_with_options` 签名与实现**

`crates/stg-ecl-compiler/src/lang/mod.rs`：

```rust
pub fn compile_with_options(
    src: &str,
    file: &str,
    options: CompileOptions,
    engine_consts: &[stg_core::consts::EngineConst],
    table: Option<&stg_core::tables::WorldTables>,
) -> Result<CompiledEcl, Vec<CompileError>> {
    let content_hash = table.map_or(0, |t| t.content_hash);
    // 表派生常量：名字是引擎级词汇（结构），值来自绑定的那张表（内容）。
    // 未绑定表时不注入——脚本引用它会得到"未知标识符"，而不是一个撒谎的默认值。
    let mut consts: Vec<stg_core::consts::EngineConst> = engine_consts
        .iter()
        .map(|c| stg_core::consts::EngineConst::new(c.name, c.ty, c.value))
        .collect();
    if let Some(t) = table {
        consts.push(stg_core::consts::EngineConst::new(
            "BULLET_COLOR_STRIDE",
            stg_core::ecl::image::EclValueType::Int,
            i32::from(t.color_stride),
        ));
    }

    let program = parse(src, file)?;
    if let Err(mut errors) = entryck::check(&program) {
        attach_src_lines(&mut errors, src);
        return Err(errors);
    }
    let typed = match typeck::check(&program, &consts, table) {
        Ok(t) => t,
        Err(mut errors) => {
            attach_src_lines(&mut errors, src);
            return Err(errors);
        }
    };
    // …以下 slots / codegen / debug 段保持原样，`codegen::generate(..., content_hash)` 不变
```

`compile_for_table` 改为传 `Some(table)`：

```rust
pub fn compile_for_table(
    src: &str,
    file: &str,
    table: &stg_core::tables::WorldTables,
) -> Result<EclImage, Vec<CompileError>> {
    compile_with_options(
        src,
        file,
        CompileOptions { debug_info: DebugInfo::None },
        stg_core::consts::ENGINE_CONSTS,
        Some(table),
    )
    .map(|ce| ce.image)
}
```

- [ ] **Step 4: `typeck::check` 收表**

`crates/stg-ecl-compiler/src/lang/typeck.rs`：签名加第三参并存进 `Checker`：

```rust
pub fn check(
    prog: &Program,
    engine_consts: &[EngineConst],
    table: Option<&stg_core::tables::WorldTables>,
) -> Result<TypedInfo, Vec<CompileError>> {
    let mut c = Checker {
        // …既有字段原样…
        table,
    };
```

`Checker` 结构体（`typeck/checker.rs`）加字段，并给它加生命周期参数
（若 `Checker` 目前无生命周期，加 `<'t>` 并让 `check` 内部实例化）：

```rust
    /// 绑定的世界表（`None` = 未绑定，跳过一切依赖表的判据）。
    pub(crate) table: Option<&'t stg_core::tables::WorldTables>,
```

- [ ] **Step 5: 更新全部调用点**

Run: `cargo build -p stg-ecl-compiler --all-targets 2>&1 | head -40`
把每处 `compile_with_options(..., 0)` 改成 `..., None)`，
`..., TABLES_V0.content_hash)` 改成 `..., Some(&TABLES_V0))`；
`typeck::check(&prog, ENGINE_CONSTS)` 改成 `typeck::check(&prog, ENGINE_CONSTS, None)`
（纯类型检查的单测不需要表）。**逐条改，不要批量 sed**——有的测试是故意测"未绑定"语义。

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p stg-ecl-compiler color_stride`
Expected: PASS（两条）。

- [ ] **Step 7: 全绿并提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/stg-ecl-compiler/src/lang/
git commit -m "$(cat <<'EOF'
refactor(ecl): 编译器前端收 Option<&WorldTables> 取代裸 content_hash

前端从此拿得到绑定的表本体(形/色判据的前提),并注入表派生常量
BULLET_COLOR_STRIDE——名字是引擎词汇、值来自表,mod 表换成 8 色时脚本
`i % BULLET_COLOR_STRIDE` 自动正确。未绑定表则不注入(报未知标识符,
不给撒谎的默认值)。顺带消掉一个参。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: 两参糖 + 编译期三判据 + 全仓脚本迁移（原子）

> **为什么是一个任务**：改 `fire`/`batch` 的 arity 是破坏性变更——签名一改，全仓 `.ecl`
> 与编译器单测立刻失效，无法半做。词表与 `consts.rs` ② 段的删除也必须同刀，否则脚本没
> 名字可用。

**Files:**
- Modify: `crates/stg-ecl-compiler/src/lang/builtins.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`、`typeck/typed_ast.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`
- Modify: `crates/stg-ecl-compiler/src/lang/xform_map.rs`、`slots.rs`
- Modify: `crates/stg-core/src/consts.rs`（删 ② 四行）、`crates/stg-core/src/tables.rs`（删对应 `pub use`）
- Create: `godot/ecl/demo/bullets.ecl`
- Modify: `crates/stg-harness/scenes/rainbow.ecl`、`crates/stg-godot/smoke/godot_smoke.ecl`、`godot/ecl/demo/{main,stage1,boss_windchime}.ecl`

**Interfaces:**
- Consumes: T3 的 `Checker.table`、T1 的 `color_stride`/`valid`
- Produces:
  - 表层 `fire(shape, color, x, y, speed, angle, xf, task)`（8 参）
  - 表层 `batch(shape, color, x, y, n_angle, angle0, angle_step, n_speed, speed0, speed_step)`（10 参）
  - xformdef `set_sprite(shape, color)`（2 参，折叠进 `args[0]`）
  - `pub(crate) fn const_val(a: &CallArg) -> Option<i32>`（`typeck/typed_ast.rs`）

- [ ] **Step 1: 写失败测试——三条编译期判据 + 折叠等价 + mod 形态表**

在 `crates/stg-ecl-compiler` 的 lang 测试模块加：

```rust
    fn compile_err_msgs(src: &str) -> Vec<String> {
        crate::lang::compile(src, "t.ecl")
            .expect_err("应当编译失败")
            .into_iter()
            .map(|e| e.msg)
            .collect()
    }

    /// 判据①：色号越界。
    #[test]
    fn color_out_of_range_is_compile_error() {
        let msgs = compile_err_msgs("sub main() { _ = fire(0, 99, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(msgs.iter().any(|m| m.contains("色号")), "实际: {msgs:?}");
    }

    /// 判据②：弹型不是 stride 的倍数。
    #[test]
    fn shape_not_on_stride_boundary_is_compile_error() {
        let msgs = compile_err_msgs("sub main() { _ = fire(5, 0, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(msgs.iter().any(|m| m.contains("弹型")), "实际: {msgs:?}");
    }

    /// 判据③：图集空格——本刀最有价值的一道闸（隐形弹）。
    #[test]
    fn blank_atlas_cell_is_compile_error() {
        // 第 9 形（掩码 0x0FFF）第 12 色 = 空格
        let msgs = compile_err_msgs("sub main() { _ = fire(144, 12, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(msgs.iter().any(|m| m.contains("空格")), "实际: {msgs:?}");
    }

    /// **形/色写反**：折叠后 3+112=115 恰是另一个合法格——必须靠"先分别校验"抓住，
    /// 而不是折叠后查 id。本测试专门钉死实现顺序，别把它优化掉。
    #[test]
    fn swapped_shape_and_color_is_caught_by_stride_check() {
        // 写反 = fire(色号 8 当弹型, 弹型 112 当色号)
        let msgs = compile_err_msgs("sub main() { _ = fire(8, 112, 0fx, 0fx, 0fx, 0deg, none, none); }");
        assert!(
            msgs.iter().any(|m| m.contains("色号")),
            "写反必须被色号越界抓住（115 折叠后是合法格，查 id 抓不到）；实际: {msgs:?}"
        );
    }

    /// 常量对折叠成单个字面量：与"直接写折叠后单参"的字节码逐字节相同（DoD 1）。
    #[test]
    fn const_pair_folds_to_single_literal() {
        let two = crate::lang::compile(
            "sub main() { _ = fire(16, 3, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .unwrap();
        // 参照物：手写一个「已经折叠好」的等价脚本——用 const 让源码里只出现一个整数
        let one = crate::lang::compile(
            "const A: int = 19; sub main() { _ = fire(A, 0, 1fx, 2fx, 3fx, 0deg, none, none); }",
            "t.ecl",
        )
        .unwrap();
        assert_eq!(two.code, one.code, "常量对必须折成与单字面量相同的字节码");
    }
```

> `two.code` / `one.code` 的字段名以 `EclImage` 实际的字节码容器字段为准
> （读 `crates/stg-core/src/ecl/image.rs`）；若不可直接比，改比
> `format!("{:?}", img)` 的字节段或该 crate 既有的镜像对比助手。

再加一条 mod 形态表测试（钉死"引擎里没有硬编码的 16"）：

```rust
    /// mod 形态表：7 形 × 8 色。判据必须按表的 stride 走，不是按内建的 16。
    #[test]
    fn mod_shaped_table_drives_checks_by_its_own_stride() {
        let mut t = stg_core::tables::build_tables_v0();
        t.color_stride = 8;
        let rows: Vec<stg_core::tables::AppearanceCfg> = (0..7 * 8)
            .map(|i| stg_core::tables::AppearanceCfg {
                radius: stg_core::math::Fx::from_int(4),
                sprite: i as u16,
                valid: true,
            })
            .collect();
        t.appearances = rows.into_boxed_slice();
        t.content_hash = 0; // 未参与本测试

        // 色号 9 在 8 色表里越界（在内建 16 色表里合法——判别力所在）
        let e = crate::lang::compile_for_table(
            "sub main() { _ = fire(0, 9, 0fx, 0fx, 0fx, 0deg, none, none); }",
            "t.ecl",
            &t,
        )
        .expect_err("8 色表里色号 9 必须越界");
        assert!(e.iter().any(|x| x.msg.contains("色号")));

        // 弹型 8（= 1 × stride）在 8 色表里合法
        crate::lang::compile_for_table(
            "sub main() { _ = fire(8, 1, 0fx, 0fx, 0fx, 0deg, none, none); }",
            "t.ecl",
            &t,
        )
        .expect("8 色表里弹型 8 是第 1 形，必须合法");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-ecl-compiler -- --nocapture fire`
Expected: 全部失败——`fire` 目前只收 7 参，参数个数不匹配。

- [ ] **Step 3: 改 `builtins.rs` 签名**

`fire` 条目：

```rust
        // 丙方案 8 参 syscall 的表层化：xf/task 两位标识符参数收窄成 (off,cnt)/script；
        // shape/color 两位反向——表层两参、codegen 折叠成单个 appearance 值（颜色轴刀）。
        params: &[Val(Int), Val(Int), Val(Fx), Val(Fx), Val(Fx), Val(Angle), Xf, Sub],
        ret: Some(Int),
        doc: "发一颗弹;shape/color 查外观表(越界/空格 编译期或 Fault);xf/task 为 xformdef/sub 名或 none;返弹句柄,失败 -1",
        param_names: &["shape", "color", "x", "y", "speed", "angle", "xf", "task"],
```

`batch` 条目：`params` 头部插一个 `Val(Int)`（共 10 项），
`param_names` 头部把 `"appearance"` 换成 `"shape", "color"`，
`doc` 改 `"N-way 批量发环;shape/color 同 fire;返实际创建数"`。

同文件里断言签名形状的测试（`fire_signature_matches_plan_shape` 等）同步更新，
并**补一条防两位对调的断言**：

```rust
        assert_eq!(b.param_names[0], "shape");
        assert_eq!(b.param_names[1], "color");
        assert!(matches!(b.params[0], Val(Int)) && matches!(b.params[1], Val(Int)));
```

- [ ] **Step 4: 加 `const_val` helper**

`crates/stg-ecl-compiler/src/lang/typeck/typed_ast.rs` 末尾：

```rust
/// 一个已判型实参的编译期常量值（`IntLit` 或 `const` 折叠出的 `ConstRef`）；
/// `LocalRef`/`Binary`/`Cast`/`Call` 等运行期表达式一律 `None`。
/// 形/色判据与 mark 锚点扫描共用（DRY：`codegen::anchor_const_arg` 改调本函数）。
pub(crate) fn const_val(a: &CallArg) -> Option<i32> {
    match a {
        CallArg::Val(TypedExpr { kind, .. }) => match kind {
            TypedExprKind::IntLit(v) | TypedExprKind::ConstRef(v) => Some(*v),
            _ => None,
        },
        CallArg::XformRef(_) | CallArg::SubRef(_) => None,
    }
}
```

`codegen.rs` 的 `anchor_const_arg` 改为：

```rust
fn anchor_const_arg(args: &[CallArg]) -> Option<i32> {
    crate::lang::typeck::typed_ast::const_val(args.first()?)
}
```

- [ ] **Step 5: 在 typeck 实现三判据**

`crates/stg-ecl-compiler/src/lang/typeck/exprs.rs`，在 `check_builtin_call_args` 的
`if ok { Some(out) } else { None }` **之前**插入：

```rust
        if ok && matches!(b.name, "fire" | "batch") {
            self.check_shape_color(b.name, &out, args, span);
        }
```

并在同 `impl` 块内新增：

```rust
    /// 形/色两参判据（颜色轴刀 spec §6.3）。**必须在折叠之前对两个参分别施加**：
    /// 折叠后的 id 无法区分"作者写反了"与"作者就要那个格"——写反的
    /// `fire(COLOR_BLUE, BULLET_RICE, …)` 折成 3+112=115，恰是 7 号形的 3 号色，
    /// 一个完全合法的格，于是静默发出错误弹型（半径也跟着错）。
    ///
    /// 只在**两参都是编译期常量**且**绑定了表**时施加；运行期色由 syscall 兜底。
    fn check_shape_color(&mut self, name: &str, out: &[CallArg], args: &[Expr], span: Span) {
        let Some(table) = self.table else { return };
        let (Some(shape), Some(color)) = (const_val(&out[0]), const_val(&out[1])) else {
            return; // 变量参：运行期由 syscall 的 valid 判据兜底
        };
        let stride = i32::from(table.color_stride);
        let shape_span = expr_span(&args[0]).unwrap_or(span);
        let color_span = expr_span(&args[1]).unwrap_or(span);

        if stride <= 0 {
            return; // 坏表：validate 的职责，此处不重复报错
        }
        if !(0..stride).contains(&color) {
            self.err(
                color_span,
                format!(
                    "'{name}' 的色号 {color} 越界：当前表每种弹型 {stride} 色，合法范围 0..{}",
                    stride - 1
                ),
            );
            return;
        }
        let shapes = (table.appearances.len() as i32) / stride;
        if shape < 0 || shape % stride != 0 || shape / stride >= shapes {
            self.err(
                shape_span,
                format!(
                    "'{name}' 的弹型 {shape} 不是合法弹型：必须是 {stride} 的倍数且小于 {}",
                    shapes * stride
                ),
            );
            return;
        }
        let id = (shape + color) as usize;
        if !table.appearances[id].valid {
            self.err(
                color_span,
                format!(
                    "弹型 {shape} 没有 {color} 号颜色（图集空格）——放行会造出有判定但看不见的弹"
                ),
            );
        }
    }
```

（顶部 `use` 补 `crate::lang::typeck::typed_ast::const_val;`。）

- [ ] **Step 6: 在 codegen 实现折叠**

`crates/stg-ecl-compiler/src/lang/codegen.rs` 的 `gen_builtin_call`：把
`for (i, (a, pk)) in args.iter().zip(bi.params.iter()).enumerate()` 循环改成
索引推进式，并在头部插入折叠分支：

```rust
        let discard_first_handle = is_self_bullet_setter(bi.name);
        // 颜色轴糖：表层 (shape, color) 两参 → 字节码单个 appearance 值。
        let folds_shape_color = matches!(bi.name, "fire" | "batch");
        let mut i = 0usize;
        while i < args.len() {
            if folds_shape_color && i == 0 {
                match (const_val(&args[0]), const_val(&args[1])) {
                    // 常量对：折成单个字面量——与手写单参字节码逐字节相同（零运行期开销）
                    (Some(s), Some(c)) => b.push_i(s + c),
                    _ => {
                        let (CallArg::Val(se), CallArg::Val(ce)) = (&args[0], &args[1]) else {
                            unreachable!("typeck 已保证 fire/batch 前两参是 Val")
                        };
                        self.gen_expr(b, slots, se);
                        self.gen_expr(b, slots, ce);
                        b.add();
                    }
                }
                i = 2;
                continue;
            }
            let (a, pk) = (&args[i], &bi.params[i]);
            match (a, pk) {
                // …既有四个 match 臂原样搬进来（`i == 0 && discard_first_handle` 那条不变）…
            }
            i += 1;
        }
```

（顶部 `use` 补 `const_val`。）

- [ ] **Step 7: `set_sprite` 改两参**

`xform_map.rs`：给 `XformOp` 加变体并改 `set_sprite` 行：

```rust
pub(crate) enum XformOp {
    /// (op 字节, 实参个数, 物理槽数)。
    Op(u8, usize, usize),
    /// 表层收 2 个常量参、**折叠进 `args[0]`** 的 op（颜色轴糖：`set_sprite(shape, color)`）。
    /// 核心侧 `OP_SET_SPRITE` 只读 `args[0]`——若按 `Op(_, 2, 1)` 直落 `args[1]`，
    /// 颜色会被无声丢弃，故必须走本变体。
    OpFold2(u8, usize),
}
```

```rust
        "set_sprite" => Some(XformOp::OpFold2(xform::OP_SET_SPRITE, 1)),
```

`physical_len` 的 match 补臂：`Some(XformOp::OpFold2(_, p)) => p,`。

`slots.rs` 第 ~328 行 `Some(crate::lang::xform_map::XformOp::Op(..)) => {}` 改为
`Some(crate::lang::xform_map::XformOp::Op(..) | crate::lang::xform_map::XformOp::OpFold2(..)) => {}`。

`codegen.rs` 的 `gen_xformdef_staging` 里，`match` 补一臂（放在 `Op` 臂之后）：

```rust
                    Some(crate::lang::xform_map::XformOp::OpFold2(op, _physical)) => {
                        if s.args.len() != 2 {
                            self.err(
                                s.span,
                                format!("xform 操作 '{}' 期待 2 个参数，实际 {}", s.op_name, s.args.len()),
                            );
                            built.push(XformSlot::default());
                            continue;
                        }
                        let mut vals = [0i32; 2];
                        let mut ok = true;
                        for (i, a) in s.args.iter().enumerate() {
                            match const_eval::evaluate(a, &self.consts) {
                                Ok((_ty, v)) => vals[i] = v,
                                Err(e) => {
                                    self.err(s.span, format!("xformdef 槽参数必须是编译期常量：{}", e.msg));
                                    ok = false;
                                }
                            }
                        }
                        if !ok {
                            built.push(XformSlot::default());
                            continue;
                        }
                        // 折叠：核心只读 args[0]
                        built.push(XformSlot { wait: s.wait, op, _pad: 0, args: [vals[0] + vals[1], 0] });
                    }
```

- [ ] **Step 8: 建内容包词表 `godot/ecl/demo/bullets.ecl`**

```ecl
// demo 内容包的弹型/颜色词表——**引擎不注册这些名字**（mod 作者用自己的一份，
// 地位对等；见 spec §5）。值 = 形号 × BULLET_COLOR_STRIDE / 色序号。
// 图集布局见 docs/render-contract.md §3；空格见下方注释。

const BULLET_RICE: int = 0;        // 米弹   r=3
const BULLET_BALL_S: int = 16;     // 小玉   r=3
const BULLET_BALL_M: int = 32;     // 中玉   r=4
const BULLET_BALL_L: int = 48;     // 大玉   r=6
const BULLET_SCALE: int = 64;      // 鳞弹   r=4
const BULLET_KUNAI: int = 80;      // 苦无   r=4
const BULLET_SHARD: int = 96;      // 碎片   r=3
const BULLET_AMULET: int = 112;    // 札     r=5
const BULLET_STAR: int = 128;      // 星弹   r=8
const BULLET_HEART: int = 144;     // 心弹   r=6  ← 只做了低 12 色,高 4 色是空格
const BULLET_BUTTERFLY: int = 160; // 蝶弹   r=6  ← 同上
const BULLET_DROP: int = 176;      // 水滴   r=4

const COLOR_RED: int = 0;
const COLOR_ORANGE: int = 1;
const COLOR_YELLOW: int = 2;
const COLOR_CHARTREUSE: int = 3;
const COLOR_GREEN: int = 4;
const COLOR_SPRING: int = 5;
const COLOR_CYAN: int = 6;
const COLOR_AZURE: int = 7;
const COLOR_BLUE: int = 8;
const COLOR_VIOLET: int = 9;
const COLOR_MAGENTA: int = 10;
const COLOR_ROSE: int = 11;
const COLOR_WHITE: int = 12;       // ← HEART/BUTTERFLY 从这里开始是空格
const COLOR_GRAY: int = 13;
const COLOR_BLACK: int = 14;
const COLOR_GOLD: int = 15;
```

- [ ] **Step 9: 迁移全部 `.ecl` 调用点**

```bash
grep -rn "fire(\|batch(\|set_sprite(" --include=*.ecl . | grep -v "^./target"
```

逐处把首参 `appearance` 拆成 `shape, color`：

- `crates/stg-harness/scenes/rainbow.ecl`（**单文件单元**，自带 const 前奏——把
  Step 8 里用得到的那几行 const 复制到文件顶部）：
  - `fire(APPEARANCE_MEDIUM, …)` → `fire(BULLET_BALL_M, COLOR_CYAN, …)`
  - `var appearance: int = i % 4;` + `batch(appearance, …)` 那处**改为轮转颜色**
    （它本来就是彩虹环，新体系下语义更贴）：
    ```ecl
    var color: int = i % BULLET_COLOR_STRIDE;
    _ = batch(BULLET_RICE, color, $self_x, $self_y, ways, base, astep, 1, speed, 0fx);
    ```
    注意 `BULLET_RICE` 是满色形，轮转全色安全；**不要**拿 `BULLET_HEART` 这类稀疏形
    轮转（会撞空格 Fault）。
- `crates/stg-godot/smoke/godot_smoke.ecl`（单文件单元，同样自带 const 前奏）。
- `godot/ecl/demo/{main,stage1,boss_windchime}.ecl`——同目录已有 `bullets.ecl`，
  `compile_units` 合并 AST，**const 跨文件可见，不要重复声明**（重名会编译期报错）。

- [ ] **Step 10: 删 `consts.rs` 的 ② 段**

`crates/stg-core/src/consts.rs`：把 `table_symbols { … }` 里四行 `APPEARANCE_*` 删除，
留空段 `table_symbols { }`（宏仍生成空的 `TABLE_SYMBOLS`）。

`crates/stg-core/src/tables.rs` 顶部删掉
`pub use crate::consts::{APPEARANCE_LARGE, APPEARANCE_MEDIUM, APPEARANCE_SMALL, APPEARANCE_STAR};`。

修 `consts.rs` 自己的两条测试：`engine_consts_registry_exposes_named_ids` 里对
`APPEARANCE_STAR` 的断言删掉（改用某个 ① 常量，如 `REQ_BGM == 5u16`）；
`consts_split_into_structural_and_table_symbols` 里对 `TABLE_SYMBOLS` 含
`APPEARANCE_*` 的断言改为：

```rust
        assert!(TABLE_SYMBOLS.is_empty(), "② 段已清空——弹型名归内容包(颜色轴刀)");
```

再全仓找残留：`grep -rn "APPEARANCE_" --include=*.rs --include=*.ecl . | grep -v ./target`
——`ecl/syscall.rs` 与 `stg-godot/src/frame.rs` 的测试里若用到，改成字面量行号
（如 `TABLES_V0.appearances[1]`）并加一句注释说明"1 = 0 号形第 1 色"。

- [ ] **Step 11: 跑全套测试**

Run: `cargo test --workspace`
Expected: Step 1 的六条新测试全 PASS；既有编译器单测里凡带 `fire(`/`batch(` 的
源码字符串都要补色参（逐条改，报错信息会直接点名行）。

- [ ] **Step 12: 端到端确认金向量与 `.ecl` 编译**

```bash
cargo run -p stg-harness -- check crates/stg-harness/scenes/rainbow.ecl
cargo run -p stg-harness -- check godot/ecl/demo
cargo run -p stg-harness -- golden --out /tmp/sunyunbo/claude-1007/-data-sunyunbo-www-stg-engine/667d22c4-808e-4278-ad2a-685e6be954ef/scratchpad/c.txt
```
Expected: 两条 `check` 无错；`golden` 正常跑完输出逐帧校验和（**值会与旧版不同——
sprite 重排导致的整体平移，这是预期**；无 committed 基线，故无需更新任何文件）。

- [ ] **Step 13: 全绿并提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add -A crates/ godot/ecl/
git commit -m "$(cat <<'EOF'
feat(ecl): 弹型+颜色两参糖——编译器折叠 + 编译期三判据 + 全仓迁移

表层 fire/batch/set_sprite 收 (shape, color) 两参,codegen 折叠成一个
appearance 值:常量对折成字面量(字节码与手写单参逐字节相同),变量色发一条
加法。字节码/syscall/弹池/shader 一行不改。

编译期三判据(色号越界/弹型非法/图集空格)全从表读 stride,**先分别校验两参
再折叠**——折叠后的 id 无法区分写反(3+112=115 恰是另一合法格)。

弹型名/色名退出引擎(consts.rs ② 段清空),改由内容包 .ecl const 提供,
mod 作者与内建 demo 地位对等。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: 占位图集铺成 16×12 + 网格逐层化

**Files:**
- Modify: `godot/tools/gen_atlas.gd`
- Modify: `godot/scripts/playfield.gd`
- Modify（产物）: `godot/assets/bullets.png`

**Interfaces:**
- Consumes: T1 的 12 形 × 16 色布局与空格掩码
- Produces: `bullets.png` = 512 × 384（16 列 × 12 行 × 32px）；`Playfield.ROWS` 逐层网格行数

- [ ] **Step 1: `gen_atlas.gd` 支持多行**

现有 `_atlas()` 只画单行（`Image.create(cols * cell, cell, …)`）。改成收行数：

```gdscript
func _atlas(path: String, cols: int, cell: int, painter: Callable, rows: int = 1) -> void:
	var img := Image.create(cols * cell, rows * cell, false, Image.FORMAT_RGBA8)
	img.fill(Color(0, 0, 0, 0))
	for r in rows:
		for c in cols:
			painter.call(img, c * cell, r * cell, cell, r * cols + c)
	var err := img.save_png(path)
	assert(err == OK, "save_png 失败: " + path)
```

**注意 painter 签名多了一个 `oy`（行偏移）**：`_cell_shot`/`_cell_enemy`/`_cell_item`/
`_cell_player`/`_cell_hitbox`/`_disc` 全部要加 `oy` 参并把 `img.set_pixel(ox + x, y, …)`
改成 `img.set_pixel(ox + x, oy + y, …)`。

- [ ] **Step 2: 弹层画 12 形 × 16 色**

```gdscript
# 12 形 × 16 色(值源 crates/stg-core/src/tables.rs build_tables_v0 的
# SHAPE_RADIUS / SHAPE_COLOR_MASK;两边必须同源,改一边要改另一边)
const SHAPE_RADIUS := [6.0, 6.0, 8.0, 12.0, 8.0, 8.0, 6.0, 10.0, 14.0, 12.0, 12.0, 8.0]
const SHAPE_COLOR_MASK := [
	0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
	0xFFFF, 0x0FFF, 0x0FFF, 0xFFFF,
]

func _hue_color(i: int) -> Color:
	# 16 色占位:色相环 12 色 + 白/灰/黑/金(与内容包词表 COLOR_* 同序)
	if i == 12: return Color(1, 1, 1)
	if i == 13: return Color(0.6, 0.6, 0.65)
	if i == 14: return Color(0.15, 0.15, 0.2)
	if i == 15: return Color(0.95, 0.8, 0.3)
	return Color.from_hsv(float(i) / 12.0, 0.85, 0.95)

func _cell_bullet(img: Image, ox: int, oy: int, cell: int, i: int) -> void:
	var shape := i / 16
	var color := i % 16
	if SHAPE_COLOR_MASK[shape] >> color & 1 == 0:
		return # 空格:整格透明(踩到它的脚本会在编译期/运行期被拒)
	_disc_shaded(img, ox, oy, cell, SHAPE_RADIUS[shape], _hue_color(color))
```

`_disc_shaded` 是**上下有明暗渐变**的圆（占位图元必须上下不对称——这样任何人第一次
有头启动就能一眼看出 B23 的 UV 是否上下镜像；见 spec §8）：

```gdscript
func _disc_shaded(img: Image, ox: int, oy: int, cell: int, r: float, col: Color) -> void:
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2(x + 0.5 - cx, y + 0.5 - cx).length()
			if d >= r + 1.5:
				continue
			# 上半格亮、下半格暗——**故意上下不对称**(B23 判决的肉眼靶子)
			var k := 1.25 - 0.6 * (float(y) / float(cell))
			var c := Color(col.r * k, col.g * k, col.b * k)
			if d < r * 0.5:
				img.set_pixel(ox + x, oy + y, Color(minf(c.r + 0.5, 1.0), minf(c.g + 0.5, 1.0), minf(c.b + 0.5, 1.0)))
			elif d < r:
				img.set_pixel(ox + x, oy + y, c)
			else:
				img.set_pixel(ox + x, oy + y, Color(c.r, c.g, c.b, clampf(r + 1.5 - d, 0.0, 1.0)))
```

`_init` 里弹层那行改为：

```gdscript
	_atlas("res://assets/bullets.png", 16, 32, _cell_bullet, 12)
```

- [ ] **Step 3: 重新生成图集**

```bash
source scripts/find-godot.sh
"$GODOT_BIN" --headless --path godot --script res://tools/gen_atlas.gd
```
Expected: 打印 `ATLAS OK`；`godot/assets/bullets.png` 变成 512×384。

验证尺寸：`file godot/assets/bullets.png`（应显示 `512 x 384`）。

- [ ] **Step 4: `playfield.gd` 网格逐层化**

```gdscript
const COLS := { 0: 16, 1: 4, 2: 4, 3: 8 }
const ROWS := { 0: 12, 1: 1, 2: 1, 3: 1 }
```

`_make_layer` 里：

```gdscript
	mat.set_shader_parameter("grid_cols", float(COLS[kind]))
	mat.set_shader_parameter("grid_rows", float(ROWS[kind]))
```

- [ ] **Step 5: 跑双冒烟**

```bash
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```
Expected: 两个都打印各自的 OK（无 `SMOKE FAIL`）。
若真工程冒烟因 demo 脚本的弹型/色号踩到空格而 Fault，回 T4 Step 9 把该调用点改成满色形。

- [ ] **Step 6: 提交**

```bash
cargo fmt --all
cargo test --workspace
git add godot/tools/gen_atlas.gd godot/scripts/playfield.gd godot/assets/bullets.png
git commit -m "$(cat <<'EOF'
feat(godot): 弹图集铺成 16 列 × 12 行 + 网格逐层化

gen_atlas 支持多行,弹层按 12 形 × 16 色生成,空格格整格透明。占位图元改为
上下明暗渐变——故意不对称,让 B23(UV 垂直镜像存疑)在首次有头启动时一眼可见
(不等于判了 B23,仍需 GPU/X 环境)。playfield 的 grid_rows 从硬编码 1.0 改为
逐层查 ROWS。shader 零改动。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: 文档与收口

**Files:**
- Modify: `docs/render-contract.md`、`docs/ecl-lang.md`、`docs/xform-ops.md`
- Modify: `PROGRESS.md`、`docs/follow-ups.md`

**Interfaces:**
- Consumes: T1–T5 全部
- Produces: 文档与代码一致；`gen-ecl-meta` 两个 sink 同步

- [ ] **Step 1: 重跑元数据生成**

```bash
cargo run -p stg-harness -- gen-ecl-meta
git diff --stat
```
Expected: `ecl-meta.json`、VS Code 扩展数据、`ecl-lang.md` 的生成段自动更新出
`fire`/`batch` 的新签名（`shape`/`color` 两参）。

- [ ] **Step 2: `render-contract.md` §3**

把 bullets 行改成：

```
| bullets | assets/bullets.png | 32×32 | 16×12 | tables appearances[].sprite（identity：id 即格号） |
```

并在 §3 末尾补一段：

```markdown
**bullets 层的二维布局（颜色轴刀，2026-07-26）**：`sprite 号 = 弹型 × color_stride + 颜色`，
其中 `color_stride` 是 `WorldTables` 的字段（内建 = 16），**不是引擎常量**——mod 表可自定义
列数。表索引 ≡ 图集格号 ≡ 池 `sprite` 值（identity），故 `set_sprite` 与 `fire` 写的是同一个
数域。稀疏弹型（只做了 8 色/4 色）仍占满一整行，用不到的列是**空格**：表里 `valid = false`，
创建时被拒（编译期报错 / 运行期 Fault），绝不会造出"有判定但看不见"的弹。
网格常量仍住 `playfield.gd`（进表是未来 mod 加载刀的事）。
```

同时更新 §3 里"UV 垂直朝向存疑"那段——占位图元已改为上下不对称，把"判决前占位图元全
上下对称，无观感差异"改成"占位图元已改为上下明暗渐变，有头启动可一眼判别（判决程序见
follow-ups B23/B26）"。

- [ ] **Step 3: `ecl-lang.md` 手写节**

在「引擎常量」节补：

```markdown
**弹型名与颜色名不是引擎常量**——它们归**内容包**，由你自己的 `.ecl` 用 `const` 声明
（示例见 `godot/ecl/demo/bullets.ecl`）。同一编译单元（= 同一目录）内 `const` 跨文件可见，
所以整局脚本只需要在一个文件里声明一次。这样 mod 作者与内建内容地位对等。

引擎只注入一个**表派生**常量 `BULLET_COLOR_STRIDE`（值 = 当前绑定表的每形色数，内建 16）。
写"轮转全部颜色"用它，别硬编码 16：`fire(BULLET_RICE, i % BULLET_COLOR_STRIDE, …)`。

⚠️ **稀疏弹型不能盲目轮转全色**：只做了 8 色/4 色的弹型，其余列是图集空格，
`for i in 0..BULLET_COLOR_STRIDE` 撞上去会 Fault。轮转写法只对满色弹型安全；
稀疏弹型要显式列出可用色。
```

- [ ] **Step 4: `xform-ops.md`**

op 30 那行改为：

```
| 30 | `SET_SPRITE` | sprite id | — | 1 | 换贴图（表层写 `set_sprite(弹型, 颜色)`，编译期折叠进 args[0]） | ✅ |
```

- [ ] **Step 5: `follow-ups.md` 对账**

- **B23** 追一句：「占位图集已于颜色轴刀改为上下明暗渐变（`gen_atlas.gd::_disc_shaded`），
  首次有头启动即可肉眼判别镜像与否；判决程序与修法不变。」
- **C11** 的"仍留给未来（modding）"追一句：「表已带 `color_stride`（颜色轴刀，2026-07-26），
  乙案的**符号段**（表自带 `[(name,id)]` 词表）仍开放——弹型名现由内容包 `.ecl` `const`
  提供，乙案落地后可退位。」
- **C14** 追一句：注入面现为「①⧺② ⧺ 表派生（`BULLET_COLOR_STRIDE`）」，② 段已清空。

- [ ] **Step 6: `PROGRESS.md`**

「里程碑史」表格最上方加一行：

```
| 2026-07-26 | **弹幕颜色轴刀** | 弹型×颜色二维图集(表长成 12×16 整齐矩形/identity/空格掩码)+ECL 两参糖(编译器折叠,字节码零改)+编译期三判据(先分别校验再折叠,防写反)+color_stride 进表与词表归内容包(mod 对等)+图集 16×12 占位上下不对称;金向量因 sprite 重排整体平移 |
```

并把「现在」段的「下一阶段候选」里那条**弹幕颜色轴刀**删掉（已完成）。

- [ ] **Step 7: 最终全绿**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p stg-harness -- verify-tables
cargo run -p stg-harness -- check godot/ecl/demo
bash crates/stg-godot/smoke/run-smoke.sh
bash godot/smoke/run-smoke.sh
```
Expected: 全部通过。

- [ ] **Step 8: 提交**

```bash
git add -A docs/ PROGRESS.md editors/
git commit -m "$(cat <<'EOF'
docs: 颜色轴刀收口——渲染契约/作者手册/xform 表/follow-ups 对账 + PROGRESS

render-contract §3 记二维布局与空格语义;ecl-lang 说明词表归内容包、
BULLET_COLOR_STRIDE 是表派生常量、稀疏弹型不可盲目轮转;B23 追注占位图元
已上下不对称;C11 追注 color_stride 已落地、乙案符号段仍开放。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## 自审记录（写完计划后的核对）

- **Spec 覆盖**：§4.1 弹池不动（全刀无池改动 ✔）；§4.2/4.3 → T1；§4.5 序列化与 validate → T1；
  §5 命名与词表 → T4 Step 8/10；§5.2 稀疏摆位纪律 → T4 Step 8 注释 + T6 Step 3 手册；
  §5.3 占位空格靶子 → T1 掩码 + T4 Step 1 判据测试；§6.1 签名 → T4 Step 3；§6.2 折叠 →
  T4 Step 6；§6.3 三判据（含"先校验后折叠"）→ T4 Step 5 + Step 1 的 `swapped_*` 测试；
  §6.4 穿线 → T3；§7 运行期兜底 → T2；§8 图集与契约 → T5 + T6；§9 校验和 → T4 Step 12
  （无 committed 基线，故只需跑通 + CI 三平台）；§10 测试 → 各任务 Step 1；§11 迁移 → T4/T6。
- **已知偏离 spec 的任务切分**：spec §12 把「`consts.rs` 删 ② 四行」放在 T1，本计划移到
  T4——因为删掉常量会立刻让 `rainbow.ecl` 编不过，T1 必须自身全绿。
- **未覆盖的 spec 可选项**：§10「顺带可还 B25」未排任务（spec 明写"由 plan 决定"，
  本计划**不并**，理由：T4 已是本刀最大的原子任务，再塞会让复审面过宽）。

---

### Task 7: `set_sprite` 细化成三个 op——全设 / 只改形 / 只改色

> **本任务是计划外追加**（用户在 T4 收工后提出）。**执行顺序：T7 → T5 → T6**——T6 的文档收口
> 必须一并覆盖 T7 新增的两个 op。
>
> **两条人类裁定，实现时不得偏离**：
> 1. **不做"跨形状安全"的保守判据**。曾提议"`set_color(c)` 要求 c 在所有弹型上都有图"，被
>    否决：太激进，会因为图集里两个稀疏弹型就把 12..15 号色在所有弹型上禁掉。**部分设允许
>    落到空格**，结果是该弹变透明，由作者负责。只保留"值本身非法"的检查。
> 2. **syscall 对称面本刀不做**（任务弹改自身外观是另一个需求，今天本来就没有）。

**Files:**
- Modify: `crates/stg-core/src/xform.rs`（两个新 op 常量 + 已知 op 列表 + 号表测试）
- Modify: `crates/stg-core/src/world/transform.rs`（两个解释臂 + 判别式测试）
- Modify: `crates/stg-core/src/lib.rs`（`ENGINE_VER` 1 → 2）
- Modify: `crates/stg-ecl-compiler/src/lang/xform_map.rs`（新变体 + 两个 op 名）
- Modify: `crates/stg-ecl-compiler/src/lang/slots.rs`（新变体的 match 臂）
- Modify: `crates/stg-ecl-compiler/src/lang/codegen.rs`（新变体的 staging 臂）
- Modify: `crates/stg-ecl-compiler/src/lang/atlas.rs`（两个单轴判据）
- Test: 上述各文件的 `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes（T1/T3/T4 产出）：`WorldTables::color_stride`；`AppearanceCfg.valid`；
  `codegen::generate(..., table: Option<&WorldTables>)` 与 `Gen` 的 `self.table` 字段；
  `lang::atlas::{check_shape_color, ShapeColorError, Blame}`；
  `lang::xform_map::XformOp::{Op, OpFold2}`；`codegen` 的 `eval_slot_args` / `push_scratch_slots` 助手。
- Produces：
  - `stg_core::xform::{OP_SET_SHAPE = 32, OP_SET_COLOR = 33}`
  - 表层 `set_shape(shape)` / `set_color(color)`（xformdef 内，各 1 参、1 物理槽）
  - `stg_core::ENGINE_VER == 2`

> 下文引用的行号是 T4 收工时（`b911f05`）的状态，**仅供定位**，以你读到的实际代码为准。

- [ ] **Step 1: 写失败测试——运行期语义（本任务的招牌）**

在 `crates/stg-core/src/world/transform.rs` 的 `mod tests` 里加。**这条是本任务的判别式核心**：
它必须能区分"只改了该改的那一维"与"整个 sprite 被覆写"。

```rust
    /// 招牌语义：`SET_COLOR` 保形、`SET_SHAPE` 保色。
    /// 判别力：若任一 op 退化成"整个 sprite = args[0]"（即 SET_SPRITE 的行为），
    /// 期望值 41/57 会变成 9/48，本测试立刻红。
    #[test]
    fn set_color_preserves_shape_and_set_shape_preserves_color() {
        const STRIDE: i32 = 16;
        let mut w = World::new();
        // 起点：第 2 形第 5 色 = 2*16+5 = 37
        let i = xf_bullet(
            &mut w,
            &[
                slot(0, OP_SET_COLOR, 9, STRIDE),          // 只换色 → 2*16+9 = 41
                slot(1, OP_SET_SHAPE, 3 * STRIDE, STRIDE), // 只换形 → 3*16+9 = 57
            ],
        );
        w.body.bullets.sprite[i] = (2 * STRIDE + 5) as u16;

        w.body.run_transforms();
        assert_eq!(w.body.bullets.sprite[i], 41, "SET_COLOR 必须保住形状位");

        w.body.run_transforms();
        assert_eq!(w.body.bullets.sprite[i], 57, "SET_SHAPE 必须保住颜色位");
    }

    /// P4-b：坏 stride（手工构造的槽，编译器不会产出）→ 计 contract_viol 且 sprite 不变，
    /// 不 panic、不除零。
    #[test]
    fn partial_sprite_ops_reject_bad_stride() {
        let mut w = World::new();
        let i = xf_bullet(&mut w, &[slot(0, OP_SET_COLOR, 9, 0)]);
        w.body.bullets.sprite[i] = 37;
        let before = w.body.diag.contract_viol;

        w.body.run_transforms();

        assert_eq!(w.body.bullets.sprite[i], 37, "坏 stride 必须 no-op");
        assert_eq!(
            w.body.diag.contract_viol,
            before + 1,
            "坏 stride 必须计一次契约违规"
        );
    }
```

> `World::new()` / `xf_bullet` / `slot` 的确切写法**照抄本文件既有测试**（它们已有一套
> 构造挂变换弹的助手）。`w.body` 的路径同理——以文件里的实际写法为准。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p stg-core set_color_preserves_shape`
Expected: 编译失败——`OP_SET_COLOR` / `OP_SET_SHAPE` 不存在。

- [ ] **Step 3: 加两个 op 常量**

`crates/stg-core/src/xform.rs`，紧跟 `OP_SET_LIFE`：

```rust
pub const OP_SET_SPRITE: u8 = 30;
pub const OP_SET_LIFE: u8 = 31;
/// 只换形状、保住颜色位（颜色轴刀 T7）。`args[0]` = 形状基址，`args[1]` = 色轴宽度
/// （**由编译器从绑定表写入**——引擎不知道"颜色"是什么，只是拿两个操作数做取模）。
pub const OP_SET_SHAPE: u8 = 32;
/// 只换颜色、保住形状位（同上，`args[0]` = 色号，`args[1]` = 色轴宽度）。
pub const OP_SET_COLOR: u8 = 33;
```

把两个新号加进本文件那张"已实现 op"的判定列表（`OP_SET_SPRITE | OP_SET_LIFE | …` 那处），
并更新号表断言测试（该文件底部有 `assert_eq!((OP_SET_SPRITE, OP_SET_LIFE), (30, 31));` 一类
的钉号测试，照同款加一条 `assert_eq!((OP_SET_SHAPE, OP_SET_COLOR), (32, 33));`）。

- [ ] **Step 4: 实现两个解释臂**

`crates/stg-core/src/world/transform.rs`，紧跟 `OP_SET_SPRITE` 那一臂：

```rust
            // 部分设：把 sprite 拆回 (形, 色) 再只改一维。stride 从槽里来（编译器写入），
            // 故世界层不需要表、也不认识"颜色"这回事（spec §4.4）。
            OP_SET_SHAPE => {
                let stride = slot.args[1];
                if stride <= 0 {
                    // P4-b：手工构造的坏槽（编译器不会产出）——确定性 no-op + 计数，不除零
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                } else {
                    let cur = self.bullets.sprite[i] as i32;
                    self.bullets.sprite[i] = (slot.args[0] + cur % stride) as u16;
                }
            }
            OP_SET_COLOR => {
                let stride = slot.args[1];
                if stride <= 0 {
                    self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
                } else {
                    let cur = self.bullets.sprite[i] as i32;
                    self.bullets.sprite[i] = (cur - cur % stride + slot.args[0]) as u16;
                }
            }
```

`self.diag.contract_viol` 的确切写法照本文件/`world.rs` 既有的计数惯例。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p stg-core -- set_color_preserves_shape partial_sprite_ops_reject_bad_stride`
Expected: PASS（两条）。

- [ ] **Step 6: bump `ENGINE_VER`**

`crates/stg-core/src/lib.rs:29`：`pub const ENGINE_VER: u32 = 1;` → `= 2;`

理由写进该行上方注释：**op 清单变更**（新增 `OP_SET_SHAPE`/`OP_SET_COLOR`）——按 CLAUDE.md
「改动前自检清单」第 3 条，改 op 清单须过评审 + bump。`step.rs` 的存档头会自动带上新值，
其版本不符分支（`step.rs:228`）已有测试覆盖，无需改动。

- [ ] **Step 7: 写编译期判据的失败测试**

在 `crates/stg-ecl-compiler/src/lang/atlas.rs` 的 `mod tests` 加：

```rust
    /// 单轴判据：只查"值本身合不合法"，**不查空格**（人类裁定：部分设允许落到空格）。
    #[test]
    fn single_axis_checks_reject_bad_values_but_allow_blank_landings() {
        let t = &*stg_core::tables::TABLES_V0;

        // 色号越界 → 拒
        assert!(check_color_only(t, 99).is_err());
        assert!(check_color_only(t, -1).is_err());
        // 合法色号 → 过，**即使它在某些弹型上是空格**（12 号色在心弹/蝶弹上没有图）
        assert!(check_color_only(t, 12).is_ok(), "空格落点是作者的责任，不是编译错误");

        // 形状基址非法（不是 stride 的倍数 / 越界）→ 拒
        assert!(check_shape_only(t, 5).is_err());
        assert!(check_shape_only(t, 12 * 16).is_err());
        // 合法形状 → 过，**即使它是稀疏弹型**（第 9 形有空格色）
        assert!(check_shape_only(t, 9 * 16).is_ok(), "稀疏弹型仍可作 set_shape 目标");
    }
```

在 `crates/stg-ecl-compiler/src/lang/mod.rs` 的测试模块加端到端腿：

```rust
    /// xformdef 里的单轴 op：坏值编译期拒、空格落点放行。
    #[test]
    fn partial_sprite_ops_reject_bad_values_only() {
        let bad = compile_err_msgs("xformdef X { set_color(99); } sub main() { wait(1); }");
        assert!(bad.iter().any(|m| m.contains("色号")), "实际: {bad:?}");

        let bad2 = compile_err_msgs("xformdef X { set_shape(5); } sub main() { wait(1); }");
        assert!(bad2.iter().any(|m| m.contains("弹型")), "实际: {bad2:?}");

        // 会落到空格的写法必须**编译通过**（裁定：允许，由作者负责）
        crate::lang::compile(
            "xformdef X { set_color(12); } sub main() { wait(1); }",
            "t.ecl",
        )
        .expect("部分设落到空格是允许的，不得报编译错误");
    }

    /// 编译器把绑定表的 stride 写进 args[1]——没有表就无从得知，必须报错而不是猜。
    #[test]
    fn partial_sprite_ops_require_a_bound_table() {
        let errs = crate::lang::compile_with_options(
            "xformdef X { set_color(3); } sub main() { wait(1); }",
            "t.ecl",
            crate::lang::CompileOptions { debug_info: crate::lang::DebugInfo::None },
            stg_core::consts::ENGINE_CONSTS,
            None,
        )
        .expect_err("未绑定表时 set_color 无法确定色轴宽度，必须报错");
        assert!(!errs.is_empty());
    }
```

> `compile_err_msgs` 是 T4 加的助手，已在该模块存在；`xformdef` 的表层语法与 `wait(1)`
> 的最小 main 照抄该模块既有测试。

- [ ] **Step 8: 跑测试确认失败**

Run: `cargo test -p stg-ecl-compiler -- partial_sprite_ops single_axis_checks`
Expected: 编译失败——`check_color_only` / `check_shape_only` 不存在，`set_color` 不是已知 op 名。

- [ ] **Step 9: 实现单轴判据**

`crates/stg-ecl-compiler/src/lang/atlas.rs`——复用既有的 `ShapeColorError` 变体与**逐字相同的
措辞**（既有测试靠"色号"/"弹型"字样断言）：

```rust
/// 单轴判据（部分设专用）：只查值本身是否合法，**不查空格**。
/// 部分设的落点取决于弹当时的另一维（运行期状态），编译期不可知；人类裁定允许落到空格
/// （结果是该弹变透明，由作者负责），故这里刻意**没有** `valid` 检查。
pub(crate) fn check_color_only(table: &WorldTables, color: i32) -> Result<(), ShapeColorError> {
    let stride = i32::from(table.color_stride);
    if stride <= 0 {
        return Ok(()); // 坏表：validate 的职责，不在此重复报错
    }
    if !(0..stride).contains(&color) {
        return Err(ShapeColorError::ColorOutOfRange { color, stride });
    }
    Ok(())
}

pub(crate) fn check_shape_only(table: &WorldTables, shape: i32) -> Result<(), ShapeColorError> {
    let stride = i32::from(table.color_stride);
    if stride <= 0 {
        return Ok(());
    }
    let shapes = (table.appearances.len() as i32) / stride;
    if shape < 0 || shape % stride != 0 || shape / stride >= shapes {
        return Err(ShapeColorError::BadShape { shape, stride, shapes });
    }
    Ok(())
}
```

> `ShapeColorError` 变体的**确切字段**以 `atlas.rs` 现有定义为准；若既有 `BadShape` 的字段
> 与上面不同，照既有的填，别改它（`fire`/`batch` 的测试依赖现有措辞）。

- [ ] **Step 10: 表层接线——新的 `XformOp` 变体**

`crates/stg-ecl-compiler/src/lang/xform_map.rs`：

```rust
pub(crate) enum XformOp {
    /// (op 字节, 实参个数, 物理槽数)。
    Op(u8, usize, usize),
    /// 表层收 2 个常量参、**折叠进 `args[0]`** 的 op（`set_sprite(shape, color)`）。
    OpFold2(u8, usize),
    /// 表层收 1 个常量参，**`args[1]` 由编译器写入绑定表的 `color_stride`**
    /// （`set_shape` / `set_color`）——引擎据此在运行期把 sprite 拆回两维。
    OpWithStride(u8, usize),
}
```

```rust
        "set_sprite" => Some(XformOp::OpFold2(xform::OP_SET_SPRITE, 1)),
        "set_shape" => Some(XformOp::OpWithStride(xform::OP_SET_SHAPE, 1)),
        "set_color" => Some(XformOp::OpWithStride(xform::OP_SET_COLOR, 1)),
```

`physical_len` 的 match 补臂：`Some(XformOp::OpWithStride(_, p)) => p,`。
`slots.rs` 那处接受合法 op 的 match 臂把 `OpWithStride(..)` 一并纳入（与 `OpFold2` 同处）。

- [ ] **Step 11: codegen staging 臂**

`crates/stg-ecl-compiler/src/lang/codegen.rs` 的 `gen_xformdef_staging`，在 `OpFold2` 臂之后加：

```rust
                    Some(crate::lang::xform_map::XformOp::OpWithStride(op, physical)) => {
                        let Some(vals) = self.eval_slot_args(s, 1) else {
                            built.push(XformSlot::default());
                            continue;
                        };
                        // stride 必须来自绑定的表——没有表就无从得知，报错而不是猜一个默认值
                        let Some(table) = self.table else {
                            self.err(
                                s.span,
                                format!(
                                    "xform 操作 '{}' 需要绑定的外观表才能确定色轴宽度",
                                    s.op_name
                                ),
                            );
                            built.push(XformSlot::default());
                            continue;
                        };
                        let check = if op == xform::OP_SET_COLOR {
                            crate::lang::atlas::check_color_only(table, vals[0])
                        } else {
                            crate::lang::atlas::check_shape_only(table, vals[0])
                        };
                        if let Err(e) = check {
                            self.err(s.span, e.message(&s.op_name));
                            built.push(XformSlot::default());
                            continue;
                        }
                        built.push(XformSlot {
                            wait: s.wait,
                            op,
                            _pad: 0,
                            args: [vals[0], i32::from(table.color_stride)],
                        });
                        push_scratch_slots(&mut built, physical);
                    }
```

> `e.message(&s.op_name)`、`self.eval_slot_args`、`push_scratch_slots` 的**确切签名以现有
> 代码为准**（都是 T4 加的）；`xform` 需要在本文件 `use`。

- [ ] **Step 12: 跑测试确认通过**

Run: `cargo test -p stg-ecl-compiler -- partial_sprite_ops single_axis_checks`
Expected: PASS（四条）。

- [ ] **Step 13: 全绿并提交**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p stg-harness -- check godot/ecl/demo
git add -A crates/
git commit -m "$(cat <<'EOF'
feat(xform): set_sprite 细化成三个 op——全设/只改形/只改色

部分设需要运行期把 sprite 拆回 (形,色),而相位 4 拿不到表。做法:编译器把绑定表的
color_stride 写进槽的 args[1],引擎只做两个操作数的取模——世界层仍然不认识"颜色"
这回事(spec §4.4),零新增穿线。坏 stride 按 P4-b 计 contract_viol 后 no-op。

人类裁定:部分设**不查空格**。落点取决于弹当时的另一维(运行期状态),编译期不可知;
曾提议的"跨形状安全"保守判据被否决(太激进,会因两个稀疏弹型就禁掉大片正常用法)。
落到空格 = 该弹变透明,由作者负责。只保留"值本身非法"的检查。

op 清单变更 → 按 CLAUDE.md 自检清单第 3 条 bump ENGINE_VER 1→2。

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

> 文档（`docs/xform-ops.md` 两行 + `ecl-lang.md` 作者提示 + `follow-ups.md` 记裁定）
> **不在本任务**——统一归 T6 收口，避免两处写同一段。
