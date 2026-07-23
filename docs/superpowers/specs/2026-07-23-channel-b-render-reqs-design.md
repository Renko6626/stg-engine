# 通道 B / anm call（渲染请求队列）设计

> **一句话**：落地断层线的第二条向上出口——`RenderReq` 请求类型 + WorldBody `reqs` 缓冲 +
> `emit_req` 写 API（世界成员 + `SYS_EMIT_REQ` syscall + `.ecl` 内建）+ `take_requests()` 读出口，
> 外加蓝图 §207 明文的机械产出者（settle 趟二敌人死亡特效请求）。"核出请求，壳做演出"
> （ZUN anm 层思想在本架构的落点，design_doc §6.4）。

**目标**：M2 表现层前置最后一块。爆炸/音效/宣言/震屏这类**一次性离散表现事件**由核推送、
表现层分发——与通道 A（常驻状态视图，拉取式）互补，两通道合成表现层的全部供给。
金向量逐位不变（reqs 是 checksum-skip 纯输出，见 §4）。

**归属**：M2 表现层**前置推送侧刀**（承接刀 A 可见性收口 + 通道 A WorldView）。

---

## 1. 背景：§6.2/§6.3/D10/D12/P6 早已拍板的部分

两份权威文档已定死（本刀不再议）：

- **请求格式**（design_doc §6.2）：`RenderReq { id: u16, seq: u16, args: [i32; 6] }`，
  repr(C) 28 B；`(frame, seq)` 全局唯一；坐标 Q16.16 **原样**传出；**音效走同一通道**
  （id 命名空间区分），不另开机制。
- **容量与预算**（D10）：256 条/帧 × 28 B = 7 KB——**已在 D10 World 预算表内**，本刀不加账。
- **溢出**（D12）：`emit_req` 满 → 确定性丢弃 + `last_status = TRUNCATED` + `diag.reqs_dropped`。
- **校验和**（P6）：`reqs` 是三条预授权 checksum-skip 纯输出缓冲之一（`hits`/`events` 已落地，
  本刀补齐第三条）；回滚重演时确定性再生，不是状态。
- **drain 语义**（蓝图 §256）：`take_requests() -> &[RenderReq]` **幂等**（帧内多次调用同一
  切片，非消费式）；缓冲下帧 `begin` 清空；headless 无人消费 = 零成本。
- **机械产出者**（蓝图 §207）：settle 趟二 hp≤0 结算中"发死亡特效请求（reqs）"。
- **回滚水位 / `confirmed_only`**（§6.3）：表现层策略，归 M2 Godot 桥，**不在本刀**。

**id 驻留零新机制**：§6.2"字符串名编译/注册期驻留 u16"由既有设施天然满足——引擎保留 id 走
C14 consts 注册表（Rust/脚本双侧单源），脚本自由 id 走 `.ecl` `const`，全部编译成立即数，
运行时无字符串。

---

## 2. 设计

### 2.1 `RenderReq` 类型与 id 命名空间（新模块 `stg-core/src/reqs.rs`）

```rust
/// 通道 B 渲染请求（§6.2，28 B）。核出请求，壳做演出；id 语义世界不解释。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RenderReq {
    pub id: u16,        // 请求名（编译期驻留；0 = 保留无效值）
    pub seq: u16,       // 帧内自增序号（= 入缓冲索引），(frame, seq) 全局唯一
    pub args: [i32; 6], // 按 id 约定解释的裸载荷：连续量 Q16.16 raw / 离散量裸 int / 角度 BAM raw
}

pub(crate) const REQS_CAP: usize = 256;
```

**id 命名空间分区**（半冻结契约，§6）：

| 区间 | 归属 |
|---|---|
| `0` | 保留无效值（零结构体防呆；分发器忽略） |
| `1..=63` | 引擎保留（C14 `structural` 段注册，Rust/脚本双侧单源） |
| `64..` | 脚本 / mod 自由（作者自配 `.ecl` `const`，与自家分发器 handler 自成契约） |

`consts.rs` `structural` 段新增两行：

```rust
REQ_ENEMY_DEATH: u16 as int = 1;   // 敌人死亡特效（settle 机械产出，args 约定见 §2.4）
REQ_SCRIPT_BASE: u16 as int = 64;  // 脚本自由段起点（const MY_REQ: int = REQ_SCRIPT_BASE + n;）
```

### 2.2 WorldBody 缓冲字段（生而封口，不长 D6）

```rust
#[checksum(skip = "纯输出缓冲，回滚重演确定性再生（P6/§6.2 通道 B）")]
pub(crate) reqs: [RenderReq; REQS_CAP],
#[checksum(skip = "纯输出缓冲，len 随 reqs 一并 skip（通道 B）")]
pub(crate) reqs_len: u16,
```

- **`pub(crate)` 出生即封**（不同于 `events` 的 pub 残留）——读一律走 `take_requests()`，
  D6 不新增条目。
- `DiagCounters` 新增 `pub reqs_dropped: u32`（**照常入校验和**——P4-a 计数参与校验，
  同 `events_overflow`）。
- `STATUS_TRUNCATED: u16 = 4` 新常量（D12 既定名，续 `STATUS_BAD_ARGS = 3`）。
- **新字段四件套**：checksum skip（带理由，derive 强制）✓ / `copy_into` 逐字段快照补两行
  （镜像 `events`/`events_len` 处理）✓ / `begin` 清空（`reqs_len = 0`，数组不动、len 守界）✓
  / D10 预算（表内已有）✓。

### 2.3 `emit_req` 写 API（WorldBody 成员，P4-a）

```rust
/// 通道 B 推送。id 语义世界不解释（含 0——分发器侧忽略未知 id）；
/// 满 → 确定性丢弃 + TRUNCATED + 计数（D12），不 panic 不 Fault。
pub fn emit_req(&mut self, id: u16, args: [i32; 6]) {
    if (self.reqs_len as usize) < REQS_CAP {
        self.reqs[self.reqs_len as usize] = RenderReq { id, seq: self.reqs_len, args };
        self.reqs_len += 1;
    } else {
        self.diag.reqs_dropped = self.diag.reqs_dropped.wrapping_add(1);
        self.last_status = STATUS_TRUNCATED;
    }
}
```

- `seq` = 入缓冲时的索引（帧内推送序，确定性来自 I4 池序遍历 + §3.5 相位序）。
- 成功路径**不动** `last_status`（现役 API 惯例：只失败置状态）。
- 不验 id 语义（`anm_state` 同款"世界不解释"纪律）；id 值域收窄验证归 syscall 层（§2.5）。

### 2.4 机械产出者：settle 趟二死亡特效请求（蓝图 §207）

`world/settle.rs` 趟二 hp≤0 块，紧邻既有 `EVT_ENEMY_DIED` 事件推送处：

```rust
self.emit_req(
    crate::consts::REQ_ENEMY_DEATH,
    [
        self.enemies.x[e].raw(),
        self.enemies.y[e].raw(),
        self.enemies.sprite[e] as i32,
        self.enemies.score[e] as i32,
        0,
        0,
    ],
);
```

**`REQ_ENEMY_DEATH` args 约定**（引擎 id 逐位表的第一行，进 `reqs.rs` 文档 + `ecl-lang.md`）：

| 位 | 含义 | 编码 |
|---|---|---|
| `args[0]` / `args[1]` | 死亡位置 x / y | Q16.16 raw |
| `args[2]` | 敌人 `sprite`（表现层选爆炸样式） | 裸 int |
| `args[3]` | `score`（击杀分数弹出演出） | 裸 int |
| `args[4]` / `args[5]` | 保留 | 0 |

（`death_script` 是 ECL 侧概念不进表现载荷；事件 `EVT_ENEMY_DIED` 照旧并存——事件=世界事实
供 ECL 挂钩/观测，请求=表现载荷供分发器，双轨是蓝图既定。）请求发射序 = settle 遍历序
（池索引升序，I4），逐敌一条，seq 确定。

### 2.5 `SYS_EMIT_REQ` syscall（号 27，2x 写族收尾）

- `pub const SYS_EMIT_REQ: u16 = 27;`（`pulse_signal = 26` 之后，族内递增）。
- `sys_emit_req`：栈上弹 7 值（`a5..a0` 逆序、`id` 最后，同现役多参 sys_* 弹栈模式）；
  栈下溢 → `FAULT_STACK`（既有 pop 助手统一处置）。
- **id 收窄 P4-b**：栈值 i32 超出 `0..=65535` → no-op + `diag.contract_viol` +
  `last_status = BAD_ARGS`，**不 Fault**（作者违约 → 确定性安全结果）；在值域内 → `as u16`
  转交 `body.emit_req`。
- `args` 六值裸转 `[i32; 6]`，无任何解释（RawVal 契约，§2.6）。
- **无 owner 类别限制**：关卡（STAGE）/敌/弹任务皆可发——宣言、音效、震屏常由关卡任务发，
  不同于弹 setter 族的 owner 必须是 BULLET。

### 2.6 `.ecl` 表层：`ParamKind::RawVal` + `emit_req` 内建

`builtins.rs` 的 `ParamKind` 新增一臂：

```rust
/// 裸载荷参数：接受 int/fx/angle 任意型表达式，codegen 原样发射（VM 栈本就是裸 i32，
/// 零转换指令）——语义镜像 `RenderReq.args` 的不透明本质（fx 过 Q16.16 raw、angle 过
/// BAM raw、int 原样）。v1 仅 `emit_req` 六个载荷位使用。
RawVal,
```

- `typeck`：`RawVal` 位放行三型任意（**表达式仍须良型**，只免去与固定 `Ty` 的匹配）；
  `codegen`：与 `Val` 同路径求值入栈，零差异。
- 内建表新条目：

```rust
Builtin {
    name: "emit_req",
    syscall: syscall::SYS_EMIT_REQ,
    is_op: false,
    params: &[Val(Int), RawVal, RawVal, RawVal, RawVal, RawVal, RawVal],
    ret: None,   // 无返回；值消费检查按 None 处置（只能做语句）
},
```

- **固定 7 参**，不足位手写 `0`（"无隐式转换"显式文化一脉相承；嫌吵自包 `sub`）。
- **小数糖已有**：`1.5fx`（精确十进制折叠）/ `90deg` / `$self_x`（fx 引擎变量）在 RawVal
  位直接写，raw 直通——`emit_req(REQ_SHAKE, 1.5fx, 30, 90deg, 0, 0, 0)` 开箱即用。
- **不加无后缀小数字面量**：`1.5` 只在 RawVal 位合法会造出上下文相关字面量类型，
  比多打两个字符贵（评审已议决）。

### 2.7 `take_requests()` 读出口

```rust
// world.rs（WorldBody）
/// 通道 B 出口（蓝图 §256）：本帧请求切片。幂等非消费——名字沿契约叫 take，
/// 实际帧内多次调用返回同一切片；缓冲下帧 begin 清空。
pub fn take_requests(&self) -> &[RenderReq] {
    &self.reqs[..self.reqs_len as usize]
}
// step.rs（World，镜像 view() 委派）
pub fn take_requests(&self) -> &[RenderReq] {
    self.body.take_requests()
}
```

---

## 3. 不做什么（划界）

- **`reqdef` 逐 id 签名声明**：真类型强制的升级路径（纯编译期收紧 typeck，VM/结构零动），
  等脚本 req 词汇量长起来再上；RawVal 对它前向兼容。**记档不实现**。
- **表现层水位去重 / `confirmed_only` / 请求分发器**：M2 Godot 桥（§6.3/§6.4）。
- **其他机械产出者**（擦弹音效、拾取音效、自机死亡演出等）：蓝图只点名了敌死一处；
  其余等 M2 真分发器出现按需加 id，防止提前铸错约定。
- **D6（`frame_events()` 出口 + events 封口 + tasks/非池字段）**：独立后刀（评审议决押后）。
- **无后缀小数字面量**：见 §2.6。
- **`REQS_CAP` 导出 / 动态容量**：消费者只需 `take_requests().len()`。

---

## 4. 确定性与金向量论证

- `reqs`/`reqs_len` checksum-skip（P6 预授权），`take_requests` 纯读——**任何 req 推送对校验
  和流不可见**。settle 死亡请求照发，金向量场景敌人照死，**校验和流逐位不变**。
- 例外路径入账即入校验：溢出 → `diag.reqs_dropped`（金向量场景远低于 256/帧，不触发）；
  syscall 坏 id → `contract_viol`（金向量脚本不含 emit_req，不触发）。
- 金向量脚本**一字不动**（评审议决）：字节码不变 → 任务演化不变；"通道 B 落地而金向量分毫
  不动"即"纯输出"性质的实证，兼作本刀全部改动的逐位回归闸。
- 零新依赖（`cargo tree -p stg-core` 防火墙不动）；`RawVal` 是编译器编译期机制，不进内核。

---

## 5. 测试策略

行为面判别式单测钉死（金向量对 skip 缓冲天生失明，§4）：

1. **推送逐字段命中**（stg-core）：`emit_req(7, [1,2,3,4,5,6])` 后 `take_requests()` 断言
   `id==7 / seq==0 / args==[1..6]`；再推一条断言 `seq==1`（判别：id/seq/args 错位即红）。
2. **溢出 P4-a**：推满 256 后再推 → len 停 256、`reqs_dropped==1`、
   `last_status==STATUS_TRUNCATED`、前 256 条内容不受扰。
3. **begin 清空 + 幂等**：step 过帧后 `take_requests()` 为空；帧内两次调用切片指针与内容一致。
4. **校验和失明实证**：checksum → `emit_req` → checksum，两值相等（skip 属性判别式）。
5. **死亡请求**（stg-core settle 测试）：铺已知 `(x, y, sprite, score)` 的敌、打死，断言
   `REQ_ENEMY_DEATH` 且 `args==[x.raw, y.raw, sprite, score, 0, 0]`（x≠y、sprite≠score
   取判别值，防位序对调假绿）。
6. **syscall 层**（stg-core ecl 测试，builder 直构字节码）：`OP_SYS 27` 正常入缓冲；
   id 越 u16 值域 → no-op + `contract_viol`；栈下溢 → `FAULT_STACK`。
7. **表层端到端**（stg-ecl-compiler）：内联 `.ecl` 源含
   `emit_req(64, 1.5fx, -3, 90deg, 2 + 3, 0, 0)` → 编译 → step → `take_requests()` 断言
   args 命中 `[98304, -3, 16384, 5, 0, 0]`（字面量折叠 + RawVal 三型 raw 直通 + RawVal 位
   表达式求值，一次验尽；各位取判别值防错位假绿）；typeck 负例：`emit_req` 第 1 位（id）
   传 fx → 编译错误（id 位仍是 `Val(Int)`）。
8. **金向量逐位不变**：`golden` 前后 `diff` 全等（§4 论证的实证闸）。
9. **全绿 + 防火墙**：`cargo build/test --workspace`、`fmt --check`、`clippy -D warnings`、
   `cargo tree -p stg-core` 无新增。

---

## 6. 半冻结契约记档

- **`RenderReq` repr(C) 布局（2+2+24 B）+ id 分区（0 无效 / 1..=63 引擎 / 64+ 脚本）+
  引擎 id 逐位 args 约定表** = 跨语言契约（M2 Godot 分发器按此路由/解码）；改动过评审 +
  视情 bump `engine_ver`。
- 引擎 id args 编码律：**连续量 Q16.16 raw、离散量裸 int、角度 BAM raw**；每个引擎保留 id
  在 `reqs.rs` 文档表逐位列明（本刀首行 `REQ_ENEMY_DEATH`，§2.4）。
- `SYS_EMIT_REQ = 27` 入 syscall 号表冻结纪律（编号即契约）。

---

## 7. 收尾

- `docs/ecl-lang.md`：`emit_req` 内建条目 + RawVal 参数说明 + id 分区与约定表 + 小数字面量用例。
- `docs/ecl-ops.md`：syscall 27 行 + P4-b/Fault 处置。
- `CLAUDE.md`：P6 名字漂移警告改口（"`reqs` 尚未实现属 M2"→ 已落地）+ 仓库结构图加 `reqs.rs`。
- `docs/follow-ups.md`：P6 对账行"`reqs` 尚未实现（M2）"改口。
- `docs/architecture.md`：M2 接缝行"还缺"挪走通道 B（只剩新建 crate）。
- `PROGRESS.md`：合入时史加一行 + 重写「现在」段。
