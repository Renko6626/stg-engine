# 弹幕颜色轴设计（弹型 × 颜色二维图集 + ECL 两参糖）

> 日期：2026-07-26　状态：设计拍板，待 writing-plans
> 关联：`docs/render-contract.md` §3（图集契约）/ `docs/ecl-lang.md`（表层语言）/
> follow-ups C11（表资产管线与 modding 遗留）、C14（常量注入）、A3（正典入口硬绑内建表）、
> B23（UV 朝向存疑）

## 1. 目标与 DoD

**问题**：内容期会出现"12 种弹型 × 每型 16 种颜色变体"的图集。颜色轴应该住在哪——
sprite 号里约定位段，还是弹池单开一个颜色字段？

**拍板**：颜色**不进弹池**。池里那个 `sprite: u16` 已经是二维图集的格号，颜色信息已经在
里面；再加 `color: u8` 是同一份信息存两遍（+8KB 池内存、自动进校验和、还要与 `SET_SPRITE`
维护一致性），P6/I7 纪律下是纯负债。

**同时采纳 ZUN 的接口形态**：ZUN 的做法是**接口分开、存储合并**——etama 图集每种弹型一行
16 色，ECL 传弹型与色号两个参，引擎算出最终 ANM 脚本号存进弹。本刀照此：**表层语言两个参，
编译器折叠成一个 appearance 值**，字节码 / syscall / 弹池 / shader / 渲染契约全不变。

**DoD**：

1. `.ecl` 里可写 `fire(BULLET_RICE, COLOR_BLUE, …)` / `batch(…)` / `set_sprite(…)`，
   编译产物与手写等价字面量**逐字节相同**；
2. 常量色的非法组合（色号越界 / 形状非法 / **图集空格**）在 `harness check` 阶段
   **带行列报错**；
3. 运行期变量色的同三类非法在 syscall 侧 `FAULT_BAD_OP`，且**先验后建**（弹不被创建）；
4. **一张非 16 色宽的 mod 形态表**（如每形 8 色）编译与运行都正确——引擎里没有任何地方
   硬编码"16"；
5. 金向量重 bless 后三平台校验和一致（CI 绿）；桥级与真工程双冒烟绿；
6. 表的 identity 与"同形同半径"由判别式单测钉死（对调半径表两项必须变红）。

## 2. mod 作者视角（本刀的主要设计约束）

设想一个 mod 作者要在本引擎注册一种新子弹（自带贴图 + 判定半径）。今天的结构盘点：

**已经通的**：`WorldTables::from_bytes` 能从磁盘载表，`appearances` 本就是变长
`Box<[AppearanceCfg]>`；`compile_for_table` 把表的 `content_hash` 焊进脚本镜像，
`start_main` 的 coherence 守卫拦住"拿 A 表编译、拿 B 表跑"。C11 资产管线本来就是照 mod
场景设计的。名字也有现成下策：mod 作者在自己的 `.ecl` 里 `const MY_SPIKE: int = 112;`，
而 `compile_units` 是"合并 AST 走单管线"，**const 在整个编译单元内跨文件可见**。

**仍卡住的三处，全在核心之外**（本刀不解决，但**本刀不得让它们变难**）：

| 卡点 | 位置 | 归属 |
|---|---|---|
| 图集纹理路径 / 网格列数写死在 GDScript 常量，`grid_rows` 硬编码 `1.0` | `playfield.gd` | 未来"图集进表"刀 |
| `new_game_at` 硬绑 `TABLES_V0`，mod 表只能走三步散装路径 | `step.rs` | follow-ups **A3** |
| `from_bytes` 按文件计数预分配 → 蓄意构造的表文件 OOM | `tables.rs` | follow-ups **C11**（明写"加载不可信 mod `.bin` 之前必补"） |

**由此得出本刀的两条硬约束**（初稿违反了，已修正）：

- **引擎里不得出现"每形 16 色"这个数**。色轴宽度是**表数据**（§4.2），不是常量。否则 mod
  表换成每形 8 色时，编译器会误报、脚本判据会算错。
- **弹型名与颜色名不进引擎**。`consts.rs` 的 ② 表符号自己的注释就写着"乙案将来搬进表符号
  段"，而 `content_tables.gd` 已立规矩——"演出名表归内容包，id→名是表现层契约，引擎不注册"。
  把 12 个弹型名焊进 `consts.rs`，等于让引擎背内容层词表，且**内建内容有名字、mod 内容没有**，
  地位不对等。

## 3. 范围裁定

**做**：表长到 12 形 × 16 色 + 空格标记 + `color_stride`；弹型/色名迁到内容包 `.ecl`
（废除旧 `APPEARANCE_*`）；`fire`/`batch`/`set_sprite` 两参糖 + 编译期三判据（判据读表）；
syscall 运行期三判据；占位图集铺成 16×12 + `playfield.gd` 网格逐层化；仓内 5 个 `.ecl`
迁移 + 金向量重 bless。

**不做**（逐条给理由与触发点）：

- **弹池 color 字段**——冗余（见 §1）。
- **三型加第四型**（`color` 专用型让形/色对调成为类型错误）——语言面变宽，typeck /
  type_rules / 手册 / VS Code 扩展 / ecl-meta 四个 sink 全跟着动；对调已被 §6.3 的
  stride 判据抬住，不值。
- **shader 染色通道**——预画变体已足够；`custom.y/z/w` 继续留空给将来的 scale/alpha。
- **逐形不同色数**（表带每形 `color_count`）——"基本规整 + 零星空格"用空格掩码表达已够；
  `color_stride` 是全表一个值，逐形不同色数留到真有需求时。
- **图集网格进表**（`atlas_cols/rows` 进 `WorldTables`，渲染端从桥读）——**触发点 = 真做
  mod 加载刀时**；本刀只把 `playfield.gd` 的 `grid_rows` 从硬编码 `1.0` 改成逐层查表，
  不改变"网格常量住 GDScript"这一现状。
- **A3 / C11 OOM / C11 乙案符号段**——三条都是"mod 加载刀"的份内事，与颜色轴正交，
  合并会让金向量重 bless 与 mod 安全面同刀评审、回滚粒度变粗。

## 4. 数据模型（断层线以下，最小改动）

### 4.1 弹池：不动

`BulletPool` 字段一个不加。`sprite: u16` 仍是最终格号，仍是渲染端唯一消费的外观量。

### 4.2 `WorldTables` 加 `color_stride`，`AppearanceCfg` 加 `valid`

```rust
pub struct WorldTables {
    ...
    /// 每种弹型占的连续色数（= 图集列数）。内建 = 16；mod 表自定义。
    /// 引擎不硬编码这个数——见 §2 硬约束。
    pub color_stride: u16,
    pub appearances: Box<[AppearanceCfg]>,
}

pub struct AppearanceCfg {
    pub radius: Fx,
    pub sprite: u16,
    pub valid: bool,   // 新增：该格图集里是否真有图（false = 空格）
}
```

### 4.3 内建表由生成式构造（12 × 16 = 192 行）

```rust
const SHAPE_RADIUS: [Fx; 12] = [...];      // 内建内容的半径，唯一真相源
const SHAPE_COLOR_MASK: [u16; 12] = [...]; // 每形 16 位，1 = 图集有这格
// 注意：这两个数组是【内建内容包的数据】，住 build_tables_v0，不是引擎结构常量。
color_stride = 16;                         // 内建内容包的图集列数
for shape in 0..12 {
    for color in 0..16 {
        let id = shape * 16 + color;
        appearances[id] = AppearanceCfg {
            radius: SHAPE_RADIUS[shape],                       // 同形 16 行必然同半径
            sprite: id as u16,                                 // identity
            valid: SHAPE_COLOR_MASK[shape] >> color & 1 == 1,
        };
    }
}
```

三条性质是**生成式结构保证**，不是靠纪律（同"const 即下标，结构上无法错序"的既有思路）：

- **同形色行半径必然相同**——半径只有 `SHAPE_RADIUS` 一个来源；
- **`appearances[i].sprite == i`（identity）**——表索引 ≡ 图集格号 ≡ 池 `sprite` 值。
  今天 4 行表已经满足（实测 `sprites == [0,1,2,3]`），本刀延续它；
- **空格由 `valid` 唯一表达**。

**空格行照样携带本形状的半径**（不是 0）：这样 `validate()` 既有的"逐行半径同域"规则一字
不用改，也不会出现"半有效"行。空格的唯一效力是 `valid == false` 导致创建被拒。

### 4.4 为什么是 identity 表而不是"引擎拆位"

替代方案是表保持 12 行、syscall 里 `shape = a / stride; color = a % stride` 拆位。**否决**：

1. **拆位会把 `set_sprite` 逼成第二个 id 域**——`set_sprite`（xform op 30）写的是池里的
   最终格号，而拆位方案的 `appearance` 是打包字段，两者数值域不同，作者要同时记两套编号。
   identity 表下两者是同一个数。
2. **拆位让核心必须认识 (形状, 颜色) 二维**（每次创建都要除模）。identity 表下核心只做
   `appearances[id]` 一次索引，**完全不认识"颜色"这回事**——颜色只活在图集布局、内容包
   词表和编译器里，全在断层线以上。

代价只是表字节：192 行 × 7B ≈ 1.3KB（全表约 840B → 约 2.1KB）。不值一提。

**追记（T7，spec 之后的人类追加，2026-07-26）**：T7 给 `set_shape`/`set_color` 两个部分设
在世界层各加了一个 op（`OP_SET_SHAPE`/`OP_SET_COLOR`，`world/transform.rs`），运行期确实
拿 stride 把 sprite 拆回 `(形, 色)` 再只改一维——字面上正是本节否决的"拆位"。区别在于
stride **不是引擎认识的常量**，而是编译器从绑定表读出、随槽数据（`args[1]`）传进来的
参数，核心自己不持有表、不在创建热路径（`SYS_CREATE_BULLET`）拆位，只在这两个显式
op 触发时才做一次取模/减法。上面第 2 条论据因此收窄为：**核心不持有表、创建路径仍是
一次索引**；"完全不认识颜色"不再对这两个 op 成立，细节见 `docs/xform-ops.md`。

### 4.5 序列化与校验

`to_bytes` / `from_bytes` 加 `color_stride`（2B）与逐行 `valid`（1B）。`content_hash` 必然
变 → 重烘焙 `tables_v0.bin`，`verify-tables` 逐位对拍照跑。

`validate()` 扩三条：

- `color_stride >= 1`，且 `appearances.len() % color_stride == 0`（表必须是整齐的矩形）；
- **每形第 0 色必须 `valid`**（理由见 §5）；
- 既有"逐行半径同域"不变（空格行带真半径，见 §4.3）。

## 5. 命名：词表归内容包，引擎只出结构

**旧的四个 `APPEARANCE_SMALL/MEDIUM/LARGE/STAR` 废除**，且**不用新的 12+16 个引擎常量替换**
（§2 硬约束二）。弹型名与色名由内容包在自己的 `.ecl` 里声明：

```ecl
// godot/ecl/demo/bullets.ecl —— 内建 demo 内容包的词表，与 mod 作者地位对等
const BULLET_RICE: int = 0;
const BULLET_BALL_S: int = 16;
...
const COLOR_RED: int = 0;
const COLOR_BLUE: int = 8;
```

`compile_units` 合并 AST 走单管线，**const 在整个编译单元内跨文件可见**，且跨文件重名在
编译期报错（带两处位置）。单文件场景（金向量 `rainbow.ecl`）自带所需 const 前奏。

**引擎只注入一个表派生常量**：编译器从**绑定的那张表**读出 `color_stride`，作为名为
`BULLET_COLOR_STRIDE` 的注入常量喂给类型检查命名空间（与 `ENGINE_CONSTS` 同一条注入通路，
只是值来自表而非 `consts.rs`）。这样脚本写"轮转全部颜色"时用 `i % BULLET_COLOR_STRIDE`，
mod 表换成 8 色也自动正确，无需脚本改一个字。

> 名字是引擎级词汇（结构），值是表数据（内容）——这条区分正是 ①/② 之分的延伸。

**每形第 0 色必须有图**（`validate()` 强制）：形状常量的值就是该形第 0 格；这条让"只写形状、
色号给 0"永远有意义，也让内容包词表里的形状名恒指向可用格。

### 5.1 内建内容包的占位词表（东方 etama 惯例，真美术到位可改名）

形状 12 项（值 = 序号 × `color_stride`，内建即 × 16），半径为占位、随美术调整：

| # | 名 | 占位半径 |
|---|---|---|
| 0 | `BULLET_RICE`（米弹） | 3 |
| 1 | `BULLET_BALL_S`（小玉） | 3 |
| 2 | `BULLET_BALL_M`（中玉） | 4 |
| 3 | `BULLET_BALL_L`（大玉） | 6 |
| 4 | `BULLET_SCALE`（鳞弹） | 4 |
| 5 | `BULLET_KUNAI`（苦无） | 4 |
| 6 | `BULLET_SHARD`（碎片） | 3 |
| 7 | `BULLET_AMULET`（札） | 5 |
| 8 | `BULLET_STAR`（星弹） | 8 |
| 9 | `BULLET_HEART`（心弹） | 6 |
| 10 | `BULLET_BUTTERFLY`（蝶弹） | 6 |
| 11 | `BULLET_DROP`（水滴） | 4 |

颜色 16 项（值 = 序号）：`COLOR_RED`(0) / `ORANGE` / `YELLOW` / `CHARTREUSE` / `GREEN` /
`SPRING` / `CYAN` / `AZURE` / `BLUE` / `VIOLET` / `MAGENTA` / `ROSE` / `WHITE` / `GRAY` /
`BLACK` / `GOLD`(15)。

### 5.2 稀疏弹型（8 色 / 4 色）用掩码表达，不压紧

真实图集里确有只做了 8 色或 4 色的弹型。**它们仍占满一整行 `color_stride` 列，用不到的列
留空**——寻址因此保持整齐矩形（`形状 × stride + 颜色`），identity 表得以保住，不需要"每形
基址 + 每形色数"的两级查表。代价只是图集里几块空白像素。

**摆位纪律：稀疏弹型的颜色必须落在语义正确的列，不得压紧到 0..n。** 例：一个只有红/绿/蓝/白
四色的弹型，掩码取 `0b0001_0001_0001_0001`（列 0/4/8/12），而不是 `0b0000_0000_0000_1111`
（列 0-3）。理由：色号语义跨弹型一致是本设计的前提（`COLOR_BLUE` 在哪种弹上都得是蓝的）；
压紧会让 `COLOR_ORANGE` 在这个弹型上实际画出绿色，色名开始撒谎，而引擎无从察觉。

**由此产生的作者约束**（写进 `docs/ecl-lang.md`）：稀疏弹型**不能盲目轮转全色**——
`for i in 0..BULLET_COLOR_STRIDE { fire(SPARSE_SHAPE, i, …) }` 会在空格列上 Fault。轮转写法
只对满色弹型安全；稀疏弹型要么显式列出可用色，要么用满色弹型做彩虹环。

### 5.3 占位期故意留空格（机制要有靶子）

`SHAPE_COLOR_MASK` 占位值：十形全 `0xFFFF`，**`BULLET_HEART` 与 `BULLET_BUTTERFLY` 取
`0x0FFF`（高 4 色为空格）**。

理由：空格是本刀最有价值的那道闸，若占位期全满，`valid` 位就是死码、三条空格判据全无真实
靶子——判别式测试只能靠伪造表，测不到"真表里真有空格"这条路径。留两形空格同时也如实反映
了"基本规整、零星空格"的真实图集形态。

## 6. 表层语言：两参糖 + 编译期三判据

### 6.1 签名变化

```
fire(shape: int, color: int, x: fx, y: fx, speed: fx, angle: angle, xf: xform|none, task: sub|none) -> int
batch(shape: int, color: int, x: fx, y: fx, n_angle: int, angle0: angle, angle_step: angle,
      n_speed: int, speed0: fx, speed_step: fx) -> int
xformdef 里：set_sprite(shape: int, color: int)
```

`builtins.rs` 的 `params`/`param_names` 加一位；照 `fire_signature_matches_plan_shape`
先例补签名防错位断言。`gen-ecl-meta` 重跑，`ecl-meta.json` / VS Code 扩展 /
`ecl-lang.md` 生成段自动跟。

### 6.2 折叠（codegen）

两参在 codegen 折叠成**一个** appearance 值压栈：

- 两参都是常量 → 编译期算成一个字面量，**零运行期开销**，字节码与今天手写单参完全同形；
- 含变量（如 `fire(BULLET_RICE, i % BULLET_COLOR_STRIDE, …)` 的同形彩虹环）→ 发一条加法。

字节码层 `SYS_CREATE_BULLET` 仍是 8 参、`SYS_CREATE_BULLETS_BATCH` 仍是 9 参、
`OP_SET_SPRITE` 仍是 1 参——**核心一行不改**。

> 这不是新机制类别：`fire` 的 `xf` 参今天就是"一个表层参降低成两个字节码参（offset + count）"，
> 两参折一参是同一条通路的反向。

### 6.3 编译期三判据（本刀主要价值）

两参**均为常量**时，编译器查绑定表，命中即报错（带行列，走已有的 `check` 诊断环）。
**判据必须在折叠之前、对两个参分别施加**：

| 判据 | 报错 |
|---|---|
| `color ∉ 0..color_stride` | 色号越界 |
| `shape % color_stride != 0` 或 `shape / color_stride >= 形数` | 不是合法弹型 |

（`形数 = appearances.len() / color_stride`，由 §4.5 的"表必须是整齐矩形"保证整除。）

| `appearances[shape + color].valid == false` | 该弹型没有这个颜色（图集空格） |

**为什么必须"先分别校验、再折叠"**（初稿的错误，记档防再犯）：若实现成"先折叠成 id、再查
表合法性"，形/色写反的 `fire(COLOR_BLUE, BULLET_RICE, …)` 会折成 `3 + 112 = 115`——那**恰好
是 7 号形状的 3 号色，一个完全合法的格**，于是静默发出错误弹型（半径也跟着错）。折叠后的
id 无法区分"作者写反了"与"作者就要 115 号格"。分别校验则一眼抓住：`color = 112` 越界。

第三条判据是"隐形弹"的正解：空格弹**照样有半径、照样参与碰撞，只是画面上什么都没有**——
玩家视角是"被看不见的东西打死"，而金向量与 headless 冒烟都抓不到它（校验和不关心贴图内容，
正是 CLAUDE.md「金向量闸门抓不了行为回归」的又一实例）。挡在编译期是最便宜的位置。

### 6.4 穿线：前端要拿到绑定的表

今天 `compile_for_table` 只从表里取 `content_hash` 透传，表本身没有下到前端。本刀把
`compile_with_options` 的 `content_hash: u64` 参**换成** `table: Option<&WorldTables>`：

- `Some(t)` → `content_hash = t.content_hash`，并启用 §6.3 三判据、注入
  `BULLET_COLOR_STRIDE`；
- `None` → `content_hash = 0`（未绑定），跳过三判据、不注入 stride 常量。

这与既有约定完全同义（模块文档已写"测试传 `0` 表示未绑定任何表"），只是把"零散的 hash"
升格成"表本身"，顺带消掉一个参。调用点约 16 处（多为测试），机械改：`0` → `None`。

模块文档原本就预告了"乙案将来从表读符号"——本刀是那条路的第一步（先读结构数据，符号段留给
未来）。

## 7. 运行期兜底（syscall）

变量色编译期查不了，syscall 侧补判据（`sys_create_bullet` / `sys_create_bullets_batch`）：
appearance 越界 → 既有 `FAULT_BAD_OP` 不变；**新增 `valid == false` → `FAULT_BAD_OP`**。
维持**先验后建**（一切校验在任何世界写之前完成，同 `spawn_enemy` 先例），不留"弹已建、色
非法"的半成品。

注意运行期收到的已是**折叠后的单个 id**，故 syscall **不需要** `color_stride`，也无法区分
形/色写反——那是编译期的职责（§6.3），运行期只保证"落到的格必须存在且有图"。

`OP_SET_SPRITE`（xform，相位 4 执行）不新增校验：它写的是池字段，运行期无表可查，且越界号
由渲染层 `% 总格数` 回卷兜底（render-contract §3 既有语义）——空格号在此只会画出空白，不构成
新风险面。此不对称**显式记档**，理由是 xform 段执行不持有表引用。

## 8. 图集与渲染契约

- `godot/assets/bullets.png`：8×1 → **16 列 × 12 行**（32px 格 → 512 × 384）。
- `playfield.gd`：新增 `ROWS` 字典，`grid_rows` 从硬编码 `1.0` 改为**逐层查表**
  （bullets = 12，其余仍 1）。**shader 零改动**（`cell = (s % cols, s / cols)` 本就是行优先）。
- `tools/gen_atlas.gd` 占位图集生成 192 格；`SHAPE_COLOR_MASK` 为 0 的格画成全透明。
- `docs/render-contract.md` §3 改表行，补一句布局约定：**bullets 层 sprite 号 =
  形状 × `color_stride` + 颜色**，并注明网格常量仍住 `playfield.gd`（进表是未来 mod 刀）。

**顺手一件（零成本）**：占位格一律画成**上下有明暗渐变**的图元。今天的占位图元全上下对称，
B23（`layer.gdshader` UV 垂直镜像存疑）因此肉眼不可判；改成上下不对称后，任何人第一次有头
启动就会立刻看出镜像与否。**这不等于判了 B23**（仍需 GPU/X 环境，B26 合并单不变），只是把
一个隐形风险变成一眼可见的风险。

## 9. 校验和 / 金向量影响

池 `sprite` 值重排（旧 `APPEARANCE_MEDIUM` 的 1 → 新体系某形 × 16 + 色）→ **金向量校验和
整体平移，需要重 bless**，同"整局流程刀因表现锚点字段平移"的先例。

纯数据重排，不引入浮点 / 时钟 / 无序容器，三平台一致性不受影响；表 `content_hash` 变化由
`verify-tables` 与 `start_main` 的 coherence 守卫照常覆盖。

## 10. 测试策略

金向量守不了本刀（它只抓跨平台分歧，不抓"三平台一致地错"），故全部靠判别式单测：

**表（core）**

- 192 行 / identity（`appearances[i].sprite == i`）/ 同形 16 行半径相同 / 空格行
  `valid == false`；判别力要求：**对调 `SHAPE_RADIUS` 两项必须变红**（圆心重合式断言对
  半径映射是瞎的——M0-7 变异检验的教训）。
- `validate()` 三条新规各一条判别腿（stride 为 0 / 行数非 stride 整数倍 / 某形第 0 色为空格）。
- `to_bytes`/`from_bytes` 往返含 `color_stride` 与 `valid`。

**syscall（core）**

- 空格色 → Fault **且弹未被创建**（照 `spawn_enemy_bad_task_script_faults_without_enemy`
  先例断言池计数不变）；id 越界 → Fault。

**编译器**

- §6.3 三条编译期错误各一条判别测试，断言**行列**正确；
- **形/色写反的判别测试**：构造出"折叠后恰好合法"的那组（如 `fire(COLOR_BLUE,
  BULLET_RICE, …)` 折叠为 115），断言它被色号越界判据拒绝——**这条专门钉死"先校验后折叠"
  的实现顺序**，是本刀最容易被后人改坏的地方；
- **mod 形态表判别测试**：构造一张 `color_stride = 8`、7 形的表，断言判据按 8 走
  （色号 9 报错、`shape = 8` 合法），且注入的 `BULLET_COLOR_STRIDE` 为 8——**钉死引擎里
  没有硬编码的 16**；
- `fire(BULLET_RICE, COLOR_BLUE, …)` 与手写等价单参的字节码**逐字节相同**；
- 变量色确实发出加法（而非静默丢弃色参）；
- `table = None` 时三判据被跳过、不注入 stride 常量（未绑定表路径不误报）；
- `set_sprite` 两参折叠同款。

**顺带可还**：B25（`fire` 的 task 号三条拒绝支路里①③零覆盖）就住在本刀要改的同一族
syscall 代码里。**列为可选尾款**，由 plan 决定是否并刀——不并也不阻塞。

## 11. 迁移清单

- **`.ecl` 5 个文件、12 个 `fire`/`batch` 调用点**：`crates/stg-harness/scenes/rainbow.ecl`、
  `crates/stg-godot/smoke/godot_smoke.ecl`、`godot/ecl/demo/{main,stage1,boss_windchime}.ecl`。
  新增 `godot/ecl/demo/bullets.ecl`（demo 内容包词表）；`rainbow.ecl`、`godot_smoke.ecl`
  是单文件单元，自带所需 const 前奏。
  `rainbow.ecl` 里 `var appearance = i % 4` 那处"有意轮转全表"的写法改为轮转**颜色**
  （`fire(BULLET_RICE, i % BULLET_COLOR_STRIDE, …)`）——它本来就是彩虹环，新体系下语义更贴。
- **Rust 侧 `APPEARANCE_*` 消费者**：`consts.rs`（② 段删四行）、`tables.rs`（定义与测试）、
  `ecl/syscall.rs`（两处测试）、`stg-godot/src/frame.rs`（一处测试）。
- **重烘焙** `tables_v0.bin` + 金向量重 bless + 双冒烟重跑。
- **文档**：`render-contract.md` §3、`ecl-lang.md`（引擎常量节要说明"弹型/色名归内容包"、
  `BULLET_COLOR_STRIDE` 是表派生常量、**稀疏弹型不能盲目轮转全色**见 §5.2）、
  `xform-ops.md`（`set_sprite` 行改两参）、
  `PROGRESS.md`（史加一行）、`follow-ups.md`（B23 追注占位图集已上下不对称；C11 追注
  "表已带 `color_stride`，乙案符号段仍开放"）。

## 12. plan 钉子（任务切分预览，writing-plans 细化）

- **T1 表（core）**：`color_stride` + `AppearanceCfg.valid` + 生成式 192 行 +
  `validate` 三条 + 序列化 + 重烘焙 + `consts.rs` 删 ② 四行。
- **T2 syscall 运行期判据（core）**：空格 → Fault，先验后建 + 判别式测试。
- **T3 编译器**：`compile_with_options` 收 `Option<&WorldTables>`、`BULLET_COLOR_STRIDE`
  注入、`builtins` 两参、codegen **先校验后折叠**、三条诊断带行列、mod 形态表测试、
  `gen-ecl-meta` 重跑。
- **T4 图集与渲染契约**：`gen_atlas.gd` 192 格（上下渐变）+ `playfield.gd` `ROWS` 逐层化
  + `render-contract` §3。
- **T5 迁移与重 bless**：内容包词表 `.ecl`、5 个脚本改写、金向量重跑、双冒烟、文档与
  PROGRESS 对账。

依赖：T1 → T2/T3（都要新表）；T4 与 T1 并行可行但要同源落地；T5 收口。
