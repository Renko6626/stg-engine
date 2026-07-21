# ECL 资产管线（C11 / Spec 2）设计

> **状态**：设计定稿，待写实施计划（superpowers:writing-plans）。
> **前身**：C14 常量注入（`docs/superpowers/specs/2026-07-21-ecl-const-injection-design.md`）明确把
> **coherence 不变量**甩给本 spec 焊死。相关技术债：`docs/follow-ups.md` C11 / C14 条。
> **权威约束**：CLAUDE.md 七不变量（尤其 I1 定点、I7 World 无引用）+ 烘焙字节纪律（D1/§2.1）。

## 一句话目标

让"表数据"（弹型 shottype / 道具 / 弹外观 / 角色参数）**不再硬编码进 stg-core 二进制**，而是以
一份**可加载的规范字节文件**存在：核心从字节反序列化出**owned `WorldTables`** 运行，不再认编译进去
的 `static TABLES_V0`。同时补上确定性完整性闭环——**编译 `.ecl` 时绑定的表，必须与运行时加载的表
是同一份**（`content_hash` 机制性比对），不一致就在构造期拒绝。

## 为什么现在做（现状事实）

代码勘察（2026-07-21）确认：

- `World` **不存表**；表每帧以 `&WorldTables` 引用穿线进 `step`/`step_with_director`（守 I7）。10 个相位/
  写 API 函数 + `VmCtx.tables` 吃这个引用。**这些签名不因 owned 化而变**（它们收 `&WorldTables`，
  不关心内部是 `&'static` 切片还是 `Box<[]>`）。
- `WorldTables` 内部挂 `&'static` 切片：`appearances`、`drop_tables`，以及嵌套 `ShotTypeCfg.sets`
  (`[[&'static [Shooter]; 2]; 5]`)、`option_pos`。owned 化的真正代价在这里 + 由此
  `ShotTypeCfg`/`CharacterCfg` 失去 `Copy`。
- `static TABLES_V0` 是唯一生产表；唯一生产内部消费点是 `World::new`（`step.rs:50` 取
  `TABLES_V0.characters[0]` spawn 自机）+ harness bench/golden 入口。其余 ~150 处 `&TABLES_V0` 全在测试。
- **`content_hash` 两处（`WorldTables` 与 `EclImage`）今天完全是死的**：只写 0、从没被读/比/序列化过。
  没有任何 `WorldTables`/`EclImage` 的序列化、回放头或握手存在。整层是 greenfield。

## 三层常量模型（本 spec 的架构基石）

引脚不是"两个表"，而是**三层，`②` 骑在 `③` 上**：

| # | 层 | 例子 | consumer | 进 ecl 编译？ | 进表 `content_hash`？ |
|---|---|---|---|---|---|
| ① | **引擎结构常量** | `GLOBALS_SYS_SEGMENT=16`、`GVAR_RANK=0` | 编译器 | 是 | **否**（归 `engine_ver`） |
| ② | **表符号词汇** | `APPEARANCE_SMALL/…/STAR` | 编译器 | 是 | 隐含（见下） |
| ③ | **数据表** | shottype / item cfg / appearance 的 radius,sprite / drops / 角色参数 | **sim（每帧）** | 否（不进 ecl） | **是** |

- **`②` 骑在 `③` 上**：`APPEARANCE_STAR=3` 里的 `3` 必须是 `③.appearances[3]` 的合法行。
- **关键观察 → 大幅省事**：`.ecl` 只用 **ID**（小整数），从不用 ID 背后的数据（radius/sprite 是
  **运行时**查表）。故 **ID 名字（name→id）可继续留在 `consts.rs`** 当稳定词汇（v1 甲案），真正会
  "三平台一致地错"的是 **id→数据**，那个正好被 `③` 的 `content_hash` 盖住。
- **只需一套 hash**：`②` 派生自/校验于 `③`，`③` 的 `content_hash` 隐含盖住 `②` 一致性；`①` 由
  握手/回放头的 `engine_ver` 管，不归表 hash。**不需要**给常量表单独发 hash，**不需要**把符号段塞进
  二进制格式。

### v1 明确不做（YAGNI，留将来 modding）

**乙案（表 `.bin` 自带 `[(name,id)]` 符号段，编译器从加载表读符号）不做**。它只买"非-Rust 作者定义
自己的 appearance 词汇"这一个能力，当前单角色单表无消费者。留作干净扩展点：将来出现 Godot 内容
设计师 / modding 时，符号段进格式、`compile` 吃符号段即可，不回改本 spec 的任何决定。

**文本 DSL（RON/TOML 手编数据文件）不做**。作者写 Rust（同今天），零新作者面——直接回应"会不会给
作者增加复杂度"。表源走**烘焙字节纪律**（下）。

## 架构：烘焙字节纪律（复用数学表同款）

```
harness（断层线以上，允许 float，v1 用 Fx 直写即够）：
    build_worldtables_v0() -> stg_core::tables::WorldTables   ← v0 内容单一真相源（Fx 直写）
        └─ .to_bytes() ─► tables_v0.bin（规范 i32/u16 小端，提交进 stg-core/src/tables/）
stg-core（断层线以下，无 float）：
    TABLES_V0 = WorldTables::from_bytes(include_bytes!("tables/tables_v0.bin"))   ← 内建默认表
CI：harness verify 重烘、与 commit 字节逐位比对（与 sin_quarter.bin 等同款闸门）
运行时：World::new 用 TABLES_V0 构造；golden 也能 from_bytes 另一份 .bin 证明真·文件加载
```

- **v1 无 float 风险**：现 `tables.rs` 内容全用 `Fx::from_int`/`from_raw`（i32）。harness builder 直抄
  这些 `Fx` 值 → `to_bytes` 得同一批 i32 → 金向量校验和**逐位不变**，不经过任何 f64 round-trip。
  f64 authoring（写 `4.5` 而非 `294912`）是将来 harness builder 的可选糖，非 v1。
- **`to_bytes` 属断层线安全**（already-built `WorldTables` 的 i32 → 字节，无 float），可住 stg-core。
  **float→Fx 的转换**（若将来做）只住 harness。v1 builder 不含 float，故住 harness 与住 core 皆可；
  **本 spec 定：builder 住 harness**（与数学表生成器同处，保持 stg-core 内容-free、float-ready）。

## 组件与接口

### 1. owned `WorldTables`（`stg-core/src/tables.rs`）

`&'static` 切片 → owned；由此 `ShotTypeCfg`/`CharacterCfg` 去 `Copy`、留 `Clone`。**引擎固定计数的
数组保持定长**（`item_cfg` 按 `ITEM_TYPE_COUNT` 索引、`characters` v1 恒 1）：

```rust
pub struct WorldTables {
    pub content_hash: u64,                     // 现为 LIVE：from_bytes 计算并校验，guard 读它
    pub characters: [CharacterCfg; 1],         // 定长（v1 单角色；多角色=未来）
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT], // 定长（type id 即索引，引擎固定）
    pub drop_tables: Box<[Box<[(u8, u8)]>]>,   // was &'static [&'static [..]]
    pub item_gravity: Fx,
    pub appearances: Box<[AppearanceCfg]>,     // was &'static [AppearanceCfg]
}
pub struct CharacterCfg { /* movement Fx 字段(Copy) */ pub shot: ShotTypeCfg } // 去 Copy，留 Clone
pub struct ShotTypeCfg {
    pub sets: [[Box<[Shooter]>; 2]; 5],        // was [[&'static [Shooter]; 2]; 5]
    pub option_pos: [Box<[(Fx, Fx)]>; 5],
}
```

- **`ptr::eq` 共享失效**：v0 "两焦点槽指同一列表"（`std::ptr::eq`）在 owned 后是两块独立分配、内容
  相等。该优化是内存 nicety、非语义；`tables_v0_shape` 测试的 `ptr::eq` 断言迁为**值相等**断言。
- **`static TABLES_V0` 不能持 `Box`**（static 里不能分配）→ 变 `pub static TABLES_V0: LazyLock<WorldTables>`
  （`from_bytes(include_bytes!(...))`，单次确定性初始化，无 float/clock/rng）。~150 处 `&TABLES_V0` →
  `&*TABLES_V0`（机械 sed）。**记档为一处 std 触点**（同 `World::new` 的堆分配一档，F1 no_std 清单更新）。

### 2. 规范二进制格式 + `to_bytes`/`from_bytes`（`stg-core/src/tables.rs`）

`tables_v0.bin` 布局（全小端；`content_hash` 之后即 body）：

```
magic:        [u8; 4] = b"STGT"
version:      u16     = 1               (格式版本；改布局即 bump)
_reserved:    u16     = 0
content_hash: u64     = FNV-1a64(body)  (body = 本字段之后全部字节)
── body ──
item_gravity: i32(Fx raw)
appearances:  u32 count, count × { radius:i32, sprite:u16 }
item_cfg:     u32 count, count × { score:u32, eject_speed:i32, terminal_vy:i32,
                                   magnet_speed:i32, pickup_radius:i32, attract_radius:i32 }
drop_tables:  u32 outer, outer × { u32 inner, inner × { ty:u8, qty:u8 } }
characters:   u32 count, count × {
                high_speed:i32, low_speed:i32, inv_sqrt2:i32, hit_radius:i32, graze_radius:i32,
                shot: { 5 tier × 2 focus × { u32 n, n × Shooter },  5 tier × { u32 m, m × {x:i32,y:i32} } }
              }
Shooter: interval:u16, delay:u16, dx:i32, dy:i32, angle:u16, speed:i32, damage:u16, radius:i32,
         sprite:u16, option:u8, flags:u8
```

- `to_bytes(&self) -> Vec<u8>`：写 header 占位 → 写 body → `FNV-1a64(body)` 回填 `content_hash`。复用
  `checksum::Fnv1a64`（vendored，冻结算法）。**纯 i32→字节，无 float，断层线安全**。
- `from_bytes(&[u8]) -> Result<WorldTables, TableLoadError>`：校验 magic/version → 读 body → 重算
  `FNV-1a64(body)` 断言 == 存储 `content_hash`（完整性）→ 逐字段读**整数**（绝不解析 float，守 I1）→
  末尾调 `validate()`（含新 join 校验）→ 设 `WorldTables.content_hash = 存储值`。
- **计数守卫**：`item_cfg` count 必须 == `ITEM_TYPE_COUNT`、`characters` count v1 必须 == 1，否则
  `TableLoadError::ArityMismatch`（这两层 v1 定长）。`appearances`/`drop_tables`/shooter 列表变长。
- `content_hash` 只盖 body（语义内容）；magic/version 是 framing，由 `from_bytes` 单独验。

### 3. `content_hash` 变 LIVE + coherence 守卫

- **compile 绑定表**：新增 `compile_for_table(src, file, &WorldTables)`，把 `table.content_hash` 透传到
  `codegen`→`ImageParts.content_hash`（`EclImage` 现盖真 hash）。`compile(src, file)` 保签名不变 =
  `compile_for_table(src, file, &*TABLES_V0)`（既有调用方零改）。v1 里 `&WorldTables` **只取
  `.content_hash()`**（`①②` 常量仍由 `consts.rs` `ENGINE_CONSTS` 注入，同 C14）；乙案将来从表读符号，
  故收 `&WorldTables` 而非裸 `u64`（表达"编译绑定于表"、留扩展）。
- **World 记录 `tables_hash`**：新增 `World::new_with_tables(seed, &WorldTables)`（读 `characters[0]`
  spawn 自机 + 记 `world.tables_hash = table.content_hash`）。`new(seed) = new_with_tables(seed, &*TABLES_V0)`
  （~40 测试 + 5 harness `new(seed)` 站点零改）。`tables_hash` 是稳定 u64，跨机一致，正常入校验和。
- **守卫落在 image 绑定处**（`start_main` 与其逃逸口 `start_main_with_owner` 的共同路径）：
  `image.content_hash() == self.tables_hash` 不符则 `TaskStartError::TableImageMismatch`。**`0 = 未绑定`
  逃逸**：`image.content_hash()==0`（空脚本/无绑定，如 golden 场景一的 `EclImage::empty()`）跳过比对。
  守卫住在**启动期一次**，不进 `step` 热路径。golden 场景二 `compile` 默认绑 `TABLES_V0`、世界也用
  `TABLES_V0` → hash 相等放行，无需改动。

### 4. join 校验 + 防作者迷路（`②` 骑 `③` 变机制）+ `consts.rs` 分组

**分处不可合**：③ 数据必须可加载/可变/可 mod（C11 全部意义），② 名字是编译器稳定词汇——塞一个宏
生成 = 数据硬编回二进制、反噬 C11。故 `name→id`(②) 与 `id→data`(③) **分处是对的**，但要让分处
**不能悄悄漂**。两种漂移 + 三条机制：

- **FM1 名字指向不存在的行**（`STAR=3` 但 `appearances` 无 index 3）→ **join 校验挡**：
  `WorldTables::validate()` 扩一条：**每个 `②` appearance ID 必须是 `appearances` 的合法行**
  （`APPEARANCE_STAR=3` ⟹ `appearances.len() > 3`）。烘焙期 + 加载期（`from_bytes` 调 `validate`）两道都拒。
- **FM2 名字指向错误的行**（index 3 放的不是星）→ **builder 按 const 下标赋值挡**（对内建表）：
  harness builder 写 `appearances[stg_core::consts::APPEARANCE_STAR as usize] = star_cfg;`，**不是**
  位置列表 `[row0, row1, row2, star_cfg]`。**const 即下标**，星数据 definitionally 落 STAR 槽，结构上
  无法错序。加一个外观 = 一行 const + 一行按该 const 下标赋值，两处互引、顺序不能漂。
- **忘配对**（加 const 忘加行 / 反之）→ **coverage 断言挡**：一条测试断言内建表每个 `②` appearance
  符号都有对应行、builder 按 const 下标覆盖整个命名集（无空洞）。CI 兜底。
- **加载的 mod 表**：② 词汇引擎固定，mod `.bin` 必须覆盖它（join 校验在加载期挡 FM1）；mod 在 STAR 槽
  放什么数据 = 这份 mod 对 STAR 的定义，`content_hash` 保证"对此表编的 .ecl 跑此表"，确定性不破。
  故 mod 作者相对**确定性**不会迷路；内容是否符直觉是其自身表语义,引擎不越俎。

`consts.rs` `engine_consts!` **显式分两组**：`ENGINE_STRUCTURAL`(①) 与 `TABLE_SYMBOLS`(②)；
`ENGINE_CONSTS = ①⧺②`（注入不变，仍全量注入）。分组让 `validate` 知道哪些常量是 appearance 行名
需 join 校验，也标出乙案将来要搬进表符号段的正是 `②`。**v1 `②` 值仍固定在 `consts.rs`**。
将来 `②` 长出 item 符号时，`TABLE_SYMBOLS` 每条带一个"索引哪张表"的 tag，join 校验数据驱动扩展；
v1 只有 appearances，不实装 tag。

## 数据流

```
编译期：  .ecl 源 ─ compile_for_table(_, _, &TABLES_V0) ─► EclImage{ content_hash = TABLES_V0.content_hash }
烘焙期：  build_worldtables_v0()(Fx) ─ to_bytes ─► tables_v0.bin（提交）; CI verify 逐位
构造期：  World::new_with_tables(seed, &table) ─► world.tables_hash = table.content_hash
启动期：  world.start_main(&image) ─► assert image.content_hash == world.tables_hash（或任一为 0 跳过）
每帧：    step(world, &table, &image, input) ─► 相位/syscall 读 &table（签名不变，无守卫开销）
```

## 错误处理（P4 铁律对齐）

- `TableLoadError`（`from_bytes`）：`BadMagic` / `UnsupportedVersion` / `Truncated` / `HashMismatch` /
  `ArityMismatch` / `ValidateFailed`。加载是**构造前的资产环节**（非模拟中），返 `Result` 干净，不 panic、
  不触 per-frame 确定性。
- `TaskStartError::TableImageMismatch`（`start_main`）：调用方拿错配对 → 返错、不启动（P4-b：确定性安全
  结果，不 panic）。既有 `start_main` 已返 `Result<u16, TaskStartError>`，加一枚变体即可。
- `from_bytes` 内 `content_hash` 自校不符 = 数据损坏 → `HashMismatch`（非引擎 bug，返错）。
- 热路径 `timer % interval`（interval 可能来自外部表）→ 加载期 `validate()` 已挡 `interval==0`；再补
  debug 帧内断言兜底（follow-ups C11③）。

## 测试策略（确定性项目：测试即规格）

金向量闸门只抓跨平台分歧、抓不了"一致地错"（CLAUDE.md），故正确性靠单测：

1. **金向量逐位不变**（回归基线）：两段 golden 校验和在改动前后逐位相同——证 owned 化 + from_bytes
   round-trip 未改任何值。**这是最硬的闸门**。
2. **round-trip 恒等**：`from_bytes(build_v0().to_bytes()) == build_v0()`（结构值相等）；
   `to_bytes` 幂等（二次 to_bytes 字节相同）。
3. **`content_hash` LIVE**：`TABLES_V0.content_hash != 0`；改任一 body 字节 → hash 变（判别式）。
4. **coherence 守卫判别腿**：`compile_for_table(src, &A)` 得的 image 拿去
   `new_with_tables(_, &B)`(A≠B) → `start_main` 返 `TableImageMismatch`；A==B → 放行；空 image（hash 0）
   → 任意表放行。
5. **join 校验 + 防迷路**：`appearances` 长度不覆盖某 `②` ID 的坏表 → `validate()`/`from_bytes` 拒（FM1）；
   coverage 断言：内建表每个 `②` appearance 符号有对应行、builder 按 const 下标覆盖命名集无空洞（FM2/忘配对）。
6. **格式健壮**：坏 magic / 未知 version / 截断 / hash 篡改 / arity 不符 各返对应 `TableLoadError`。
7. **CI 字节闸门**：`verify-tables` 纳入 `tables_v0.bin`（重烘逐位比对，同数学表）。
8. **文件加载端到端**：harness 用 `from_bytes` 载一份 `.bin` 跑 golden，校验和 == 内建表跑的 golden
   （**圆环自证**：字节表与内建表同源、逐位一致）。
9. **顺带补债**：`validate_rejects_bad` 加角色半径负向腿（follow-ups B14）；interval 热路径 debug 断言（C11③）。

## 决策记录（防"悄悄改"）

- **甲案定，乙案缓**：v1 `②` 名固定在 `consts.rs`，靠 `③.content_hash` 机制性绑定；符号段进格式留
  modding。理由：`.ecl` 只用 ID、不用数据，一套 hash 足堵洞。
- **一套 hash**：只 `③` 有 `content_hash`；`②` 隐含覆盖；`①` 归 `engine_ver`。
- **builder 住 harness**：与数学表生成器同处，stg-core 保持内容-free、float-ready。
- **v1 无 float**：Fx 直抄现值 → 金向量逐位不变、零 round-trip 风险。
- **守卫在 `start_main` 一次**，非 `step` 热路径；`0=未绑定`逃逸保空脚本路径。
- **定长 vs 变长**：`item_cfg`/`characters` 引擎固定计数保定长数组；`appearances`/`drop_tables`/shooter
  列表 owned 变长。多角色/多 item-type 是未来。

## 实施分组（一个 spec，plan 里清晰分组；建议顺序）

> 顺序即依赖：owned 化解锁一切；hash 依赖规范字节；守卫依赖 hash；文件加载依赖 owned+from_bytes。

- **组 A — owned 化**：结构切片 `&'static`→`Box`，去 `Copy`、`ptr::eq`→值相等，`TABLES_V0`→`LazyLock`
  经 `from_bytes(include_bytes!)`，~150 站点 `&*` sed。**门槛：金向量逐位不变。**
- **组 B — 规范字节 + hash**：`to_bytes`/`from_bytes`/`TableLoadError`；harness `build_worldtables_v0`
  （**appearances 按 `②` const 下标赋值**，防 FM2 错序）+ bake/verify 注册 `tables_v0.bin`；
  `content_hash` 变 LIVE；round-trip/格式健壮测试。
- **组 C — compile 绑定 + 守卫**：`compile_for_table` 盖 hash；`new_with_tables` 记 `tables_hash`；
  `start_main` 守卫 + `TableImageMismatch`；`0` 逃逸；守卫判别腿测试。
- **组 D — join 校验 + consts 分组 + 端到端**：`consts.rs` 分 `ENGINE_STRUCTURAL`/`TABLE_SYMBOLS`；
  `validate` join 腿；harness golden 走 `from_bytes` 端到端圆环自证；补 B14/C11③ 债。
- **收口**：`PROGRESS.md`（史加一行 + 重写「现在」）、`docs/follow-ups.md`（C11 销账、乙案/DSL 记为
  未来）、`docs/ecl-lang.md`（若需提"编译绑定表"一句）。
```
