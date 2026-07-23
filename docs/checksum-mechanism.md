# 校验和机制 —— 字段级 hash 与"新字段默认入校验"的保证

面向：在 `stg-core` 里新增 / 修改 World 状态结构体的人。
权威规格见 `stg-world-design.md` D11、`design_doc.md` §3.6、原则 P6。
相关代码：`stg_core::checksum`（`Fnv1a64` + `Checksum` trait + 基本类型/数组 impl）、`stg-derive`（`#[derive(Checksum)]`）。

---

## 0. 一句话目的

校验和 = **World 状态的 64 位指纹**，用来把"确定性"变成可持续验证的性质：金向量逐帧跨平台对拍、rollback desync 检测、回放回归对拍。
因 `repr(C)` 有 **padding 垃圾字节**、且有**纯输出缓冲**（重演再生），不能整块哈希整块字节 → **逐字段哈希**（只喂真实字段字节，天然跳过字段间 padding；SoA 数组同类型连续、无内部 padding，整条喂）。

## 1. 核心保证：新字段默认走一轮 hash + checksum —— 怎么做到、怎么自查

> **规则：结构体的每个字段默认参与校验；要排除必须显式 opt-out。**

三条机制共同保证"**忘不了**"：

1. **derive 从字段定义生成（单一真相源）**：`#[derive(Checksum)]` 在**编译期**读结构体字段，生成
   `hash_into` = 按声明序逐字段 `field.hash_into(h)`。**加一个字段 → 生成代码自动多哈希它一行**，
   你不需要手动同步任何"哈希器"。**根本不存在"代码与哈希器不同步"这种状态**——因为哈希器就是从字段长出来的。

2. **编译期强制，漏不了（关键）**：若新字段的类型**没有**实现 `Checksum`，生成的 `field.hash_into(h)`
   **编译报错**（`trait bound ...: Checksum is not satisfied`）。所以"新增了一个不可校验的字段"
   **不会静默通过——它在编译期就红**。顺带：`Vec` / `HashMap` / 裸指针没有、也不会有 `Checksum` impl，
   加进 World 直接编译不过（与 I7"无堆容器"一致）。

3. **默认 in、opt-out 留痕**：不写任何属性 = 参与。要排除只能 `#[checksum(skip = "理由")]`，且
   **理由字符串强制**（无理由编译报错）。P6：只有纯输出缓冲（`reqs` / `hits` / `frame_events`）才允许 skip。

**怎么"知道"某结构体的字段都进了校验？** 三种自查，按可靠度：

- **不用记（结构即真相）**：字段即哈希项。没有一份独立、需要你手动对照的"哈希器清单"，所以没有"忘了同步"的可能。
- **debug 逐字段清单**：`x.checksum_fields()`（`#[cfg(debug_assertions)]`，derive 生成）返回
  `[(字段名, 该字段指纹)]`。可在测试里断言字段数 / 名字，或 desync 时逐字段比对、直接定位"哪个字段分歧"。
- **（2026-07-23 落地）哨兵守卫**：`step.rs` 的 `world_size_sentinel_guards_copy_into_field_list`
  测试钉死 `WorldBody`/`World` 尺寸（debug/release 双值）——**字段集变更即红**，红了先核对
  `copy_into` 清单/checksum skip 理由/D10 预算再更新数字（防"加字段漏拷快照""加了 skip 却漏理由"）。

## 2. 编译期 vs 运行时（两件事，各在一头）

| | 阶段 | 干什么 | 成本 |
|---|---|---|---|
| **derive（代码生成）** | **编译期** | 读字段、生成 `hash_into` / `checksum_fields` 代码 | 一次性，仅影响编译时长（syn 稍重），运行时零 |
| **`world.checksum()`（算指纹）** | **运行时** | World 字节流过 FNV，出 u64 | 见 §5 |

即："**哪些字段被哈希**"在编译期由结构体定义定死；"**给这个实例算指纹**"是运行时执行。

## 3. 四层结构（从下往上）

| 层 | 东西 | 职责 |
|---|---|---|
| 算法 | `Fnv1a64` | vendored FNV-1a 64：xor-then-乘、小端喂字节、出 u64。**手写冻结在仓库**（外部 crate 悄改算法 = 全体回放/金向量作废） |
| 契约 | `trait Checksum` | `hash_into(&self, &mut Fnv1a64)` + `checksum()->u64`。基本类型（i8..u64/bool）/ 数组 `[T;N]` 手写，结构体靠 derive |
| 生成 | `#[derive(Checksum)]` | 编译期按字段声明序生成 `hash_into`；`#[checksum(skip="理由")]` 排除（理由强制） |
| 调试 | `checksum_fields()`（debug） | 逐字段指纹清单，desync 逐字段定位 |

**组合是递归的**：`World.checksum()` → 喂 `body`/`tasks` → `body` 喂各池 → 池喂各 SoA 数组 → 数组逐元素喂 `Fx` → `Fx` 喂其 i32 小端字节。每层只管"把自己字段喂下去"，最终一个 u64 = 整个逻辑世界的指纹（无 padding、skip 可控、跨平台可复现，因全程纯整数 + 小端）。

## 4. 纪律清单（绝大多数编译期强制）

- ✅ **编译期强制**：字段类型必须 `Checksum`（否则不编译）；不能放 `Vec`/堆容器（无 impl + I7）；skip 必须带理由。
- ⚠️ **唯一行为纪律**：因"**哈希全槽、连死槽也哈希**"（不用 alive 掩码，更简单/更不易错），死槽字节必须确定
  → 要求 **World 整块清零 + 复用槽写满所有字段**（§2.4 硬规则）。M0-3 的 `define_pool!` 用 `XxxInit`
  结构体把"写满"变**编译期事实**（构造槽必须传全字段）+ debug 帧内断言复查复用槽——到 M0-3 这条也基本焊死。
- ⚠️ **当前限制**：derive **只支持 struct、不支持 enum**（遇 enum 编译报错）。引擎状态多用 `#[repr(u8)]`
  裸整数（如 `life_state: u8`）规避；真需 enum 字段则存 repr 整数手写 impl，或后续给 derive 扩 enum 支持。

## 5. 性能

**核心：它不在热路径 `step()` 里，是偶发的独立验证 pass，不占单步 0.5ms 预算。**

- **量级**：FNV-1a 逐字节（串行 xor+乘依赖链）≈ ~1 ns/byte；满 World ~1.3 MB（跳过 reqs/hits/frame_events）
  → **一次约 ~1 ms**。
- **多久一次**：CI/金向量**逐帧**（CI 不看性能，几百帧 <1s，无所谓）；联机 **每 K=20 帧采样**（摊到每帧 ~0.05ms，
  相对 16.67ms 帧预算可忽略）。rollback 重演 8 步 <4ms 的循环里**不算校验和**。
- **形态高效**：derive 生成直线 `#[inline]` 调用，SoA 数组作连续字节段流过 hasher，无逐字段额外开销。
- **"哈希全槽"代价**：即便稀疏（500 发弹活着）也哈希满 8192 槽——为"不用 alive 掩码"付的、D11 接受的钱。
- **逃生舱**：若实测 FNV 成 CI 瓶颈，换 **vendored xxHash64**（~快 5 倍）；换算法 = 一次评审 + bump `engine_ver`。

## 6. 节奏与用途（同一函数、两种节奏）

- **CI / 金向量**：逐帧校验和，x86_64 / aarch64 逐帧对拍（DoD 机制；即 `determinism-gate`）。
- **联机 rollback**：每 K=20 帧随包采样对拍，不符即 desync 处理（开发构建硬停落盘、发布构建重同步）。
- **回归**：同一回放跑两遍、跨平台各跑，断言逐点一致。

---

*出处：M0-2 校验和 derive 实现期的机制讨论沉淀。首个消费者是 `Fx`/`Angle`（已 derive）；M0-3 池、M0-4 World 挂上 `#[derive(Checksum)]` 后，逐帧指纹自动成立，金向量对拍 DoD 链即接通。*
