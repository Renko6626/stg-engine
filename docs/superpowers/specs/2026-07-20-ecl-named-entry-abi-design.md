# ECL 命名入口 ABI 与 sub 符号元数据设计

**日期：** 2026-07-20

**状态：** 已确认，待实施计划

**范围：** `EclImage` 的镜像内 sub 命名、入口分类、参数 ABI、解析与任务启动接口

## 1. 背景与目标

当前 ECL 编译器在前端保留 sub 名称，但 `EclImage` 只输出扁平字节码、按声明顺序排列的
入口偏移和占位 `content_hash`。运行时以裸 `u16` script id 启动任务，无法区分：

- 一关唯一的根入口；
- 可独立派生任务的 async sub；
- 只能经同步 `CALL` 进入的普通 sub。

这导致名称在编译后丢失，外部绑定层只能依赖“第一个 sub 是 main”等约定，也可能把以
`RET` 收尾的 call-only sub 当任务根启动。本设计把名称提升为正式身份，把数字 ID 收窄为
当前镜像内的紧凑索引，并让入口类型和参数 ABI 成为 `EclImage` 的可验证元数据。

本设计的目标是：

1. 一个 `.ecl` 文件独立描述一关或一个完整场景控制单元。
2. 每个正常镜像有唯一、零参数、单例启动的 `main` 根入口。
3. 所有 async sub 都是公开命名入口；普通 sub 只允许同步调用。
4. 镜像保留所有 sub 的名称、类型、参数名称和参数类型。
5. 名称是跨编译引用的正式身份；`SubId`/`EntryId` 只在当前镜像加载期间有效。
6. 仅调整 sub 声明顺序时，符号 ID、字节码和未来的内容哈希保持不变。

## 2. 非目标

以下内容不属于本次实施范围：

- 外部二进制文件格式和反序列化；
- 对不可信字节码执行完整 CFG、指令边界和 reserved bits 校验；
- engine/schema/opcode 版本协商；
- 真实 `content_hash`、回放头和联机握手；
- ECL 跨文件依赖、include 或模块系统；
- 可见性修饰符：本版所有 async sub 都公开；
- 可持久化的数字入口 ID 或名称哈希 ID。

这些项目继续归 C11 资产管线。本次只保证可信编译器与低层 Builder 生成的镜像具有完整且
一致的命名入口 ABI。

## 3. 命名与语言规则

### 3.1 编译单元

一个 `.ecl` 文件编译成一个独立 `EclImage`。镜像内只保存源码短名称，例如 `main`、
`patrol`、`bullet_task`。关卡资产路径和不同镜像之间的命名冲突由外层资产系统处理，ECL
不引入限定名或模块名。

标识符继续沿用当前 lexer 规则：ASCII 字母或下划线开头，后续为 ASCII 字母、数字或
下划线；查找大小写敏感，不做大小写折叠或 Unicode 归一化。

### 3.2 三种 sub 类型

| 源码形态 | `SubKind` | 允许的进入方式 |
|---|---|---|
| `sub main()` | `Root` | 仅 `World::start_main` |
| `async sub name(...)` | `Async` | `spawn`、`fire(..., task)`、外部命名/ID 启动 |
| `sub name(...)` | `CallOnly` | 仅同步 `CALL` |

所有 sub 共用一个命名空间，名称必须唯一。所有 sub 均保留符号元数据，不因 `CallOnly` 而
匿名化。

### 3.3 main 规则

每个由 `.ecl` 编译得到的非空镜像必须恰好声明一个 `sub main()`：

- 必须是普通 `sub`，不能写成 `async sub main`；
- 参数必须为空；
- 不能被同步调用；
- 不能作为 `spawn` 或 `fire(..., task)` 目标；
- 不能经普通 `spawn_entry` 或 `spawn_entry_named` 启动。

缺少 main 或违反以上任意条件都是编译错误。引擎内置的 `EclImage::empty()` 是唯一允许
没有 root 的镜像。

### 3.4 async 与 call-only 规则

所有 `async sub` 自动成为公开入口，不增加 `pub` 或 export 清单。普通 sub 始终是
call-only。现有“async 不能同步调用、普通 sub 不能 spawn”的类型检查继续保留，并扩展到
root 类型。

## 4. 镜像数据模型

以下类型定义在 `stg-core::ecl::image`，供 VM、编译器和绑定层共享：

```rust
#[repr(transparent)]
pub struct SubId(u16);

#[repr(transparent)]
pub struct EntryId(u16);

pub struct ResolvedEntry<'a> {
    image: &'a EclImage,
    id: EntryId,
}

pub enum EclValueType {
    Int,
    Fx,
    Angle,
}

pub enum SubKind {
    Root,
    Async,
    CallOnly,
}

pub struct EclParam {
    pub name: String,
    pub ty: EclValueType,
}

pub struct SubMeta {
    pub name: String,
    pub code_entry: u32,
    pub kind: SubKind,
    pub params: Vec<EclParam>,
}

pub struct EclImage {
    code: Vec<u32>,
    subs: Vec<SubMeta>,
    entries: Vec<SubId>,
    root: Option<SubId>,
    content_hash: u64,
}
```

### 4.1 ID 语义

- `SubId` 覆盖镜像中的全部 sub，下标指向按名称严格升序排列的 `subs`。
- `EntryId` 只覆盖 `Async` sub，下标指向同样按名称严格升序排列的 `entries`。
- `entries[entry_id]` 得到对应 `SubId`。
- `main` 只由 `root` 指向，不进入 `entries`。
- `SubId` 与 `EntryId` 的内部整数不能由外部调用方任意构造。
- 数字 ID 不得写入关卡配置、存档或跨资源引用；持久化层必须保存名称并在镜像加载后解析。
- 对外缓存的是绑定了 `&EclImage` 的 `ResolvedEntry<'a>`，而不是可与另一镜像误配的裸
  `EntryId`。

字典序 ID 对源码声明重排稳定，但新增一个字典序更靠前的名称仍会移动后续 ID。因此数字 ID
是运行期缓存，而不是持久化 ABI。

### 4.2 封装与构造

`EclImage` 的内部表改为私有，防止调用方直接构造相互失配的 `code/subs/entries/root`。
正常镜像只能通过带结构校验的构造入口创建。构造入口返回 `Result<EclImage,
ImageBuildError>`；`EclImage::empty()` 保留为无分配、无 root 的显式哨兵。

本次结构校验包括：

- 名称合法、唯一且严格排序；
- 非空镜像有且仅有一个名为 `main` 的 `Root`；
- root 零参数且 `root` 字段指向它；
- 每个 `Async` sub 在 `entries` 中恰好出现一次；
- `entries` 不包含 `Root` 或 `CallOnly`；
- sub/entry 数量能编码为 `u16`；
- 每个参数列表不超过 VM locals 容量；
- 每个 `code_entry` 位于 code 范围内；
- Builder/codegen 的所有窄化转换为 checked conversion。

完整不可信字节码验证仍留给 C11。

### 4.3 查询 API

```rust
impl EclImage {
    pub fn symbol(&self, name: &str) -> Option<(SubId, &SubMeta)>;
    pub fn resolve_entry(&self, name: &str) -> Result<ResolvedEntry<'_>, ResolveError>;
    pub fn entry_meta(&self, id: EntryId) -> Option<&SubMeta>;
    pub fn sub_meta(&self, id: SubId) -> Option<&SubMeta>;
    pub fn root(&self) -> Option<SubId>;
    pub fn code(&self) -> &[u32];
    pub fn content_hash(&self) -> u64;
}
```

`symbol` 对 `subs` 做名称二分。`resolve_entry` 先查完整符号表，以便区分：

- `UnknownSymbol`；
- `RootRequiresStartMain`；
- `CallOnly`。

`ResolvedEntry` 内部携带来源镜像引用和 EntryId，避免把镜像 A 解析出的有效数字误用于镜像
B。它可以被绑定层缓存，但不能脱离来源镜像存活。查询不修改 World，也不触碰诊断计数。

## 5. 编译器与 Builder

### 5.1 canonical 编译顺序

编译器完成 parse/typecheck 后收集全部 sub，按名称排序并分配最终 `SubId`。codegen 也按这个
顺序生成和拼接各 sub 字节码，而不是按源码声明顺序。因此只调整 sub 声明顺序时：

- `SubId` 不变；
- `EntryId` 不变；
- code 物理布局不变；
- 完整 `EclImage` 不变。

参数元数据保持源码声明顺序，因为参数位置属于 ABI。

`CALL` 可继续回填为绝对 code pc；`OP_SPAWN` 和 `fire(..., task)` 编码 canonical `SubId`。
生成器必须按目标 `SubKind` 检查引用方式。

### 5.2 Builder 两阶段声明

当前 `ImageBuilder::add_sub` 立即返回声明顺序 `ScriptId`，与 canonical ID 冲突。Builder 改为
先声明符号、后定义代码：

```rust
let patrol = builder.declare_sub(
    "patrol",
    SubKind::Async,
    [param("speed", EclValueType::Fx)],
)?;

builder.define_sub(patrol, |code| {
    // ...
});
```

`declare_sub` 返回的 `BuilderSubRef` 是仅在当前 Builder 有效的构建期句柄，不等于最终
`SubId`。`call`、`spawn` 等 fixup 接受 `BuilderSubRef`；`build()` 完成名称排序、ID 分配、
代码拼接和结构校验后才产出正式镜像。

Builder 必须拒绝：

- 重复声明或未定义声明；
- 同一句柄重复定义；
- 非法 root 数量或 main ABI；
- CALL/Spawn 的 kind 不匹配；
- 参数、sub、entry 或 code 长度溢出。

## 6. 任务启动 API

### 6.1 参数表示

镜像保存完整参数类型，但 VM locals 仍是 `i32`。绑定层使用：

```rust
pub enum EclArg {
    Int(i32),
    Fx(Fx),
    Angle(Angle),
}
```

类型化接口先匹配 `EclValueType`，随后转换为 raw `i32`。快速接口接收已经编码好的
`&[i32]`，只校验参数数量。

### 6.2 三条公开入口

```rust
impl World {
    pub fn start_main(
        &mut self,
        image: &EclImage,
    ) -> Result<u16, TaskStartError>;

    pub fn spawn_entry(
        &mut self,
        entry: ResolvedEntry<'_>,
        args: &[i32],
        owner: EclOwner,
    ) -> Result<u16, TaskStartError>;

    pub fn spawn_entry_named(
        &mut self,
        image: &EclImage,
        name: &str,
        args: &[EclArg],
        owner: EclOwner,
    ) -> Result<u16, TaskStartError>;
}
```

`EclOwner` 是对现有 `(kind, index, generation)` 的类型化封装；构造与启动时校验 owner kind、
索引、generation。root owner 固定为 `OWNER_STAGE`，`start_main` 不接受 owner 参数。

`spawn_entry_named` 的数据流为：

```text
名称二分查询
  -> ResolvedEntry(&EclImage + EntryId)
  -> EntryId 映射 SubId
  -> 校验参数数量和逐项类型
  -> 转换为 raw i32
  -> 校验 owner
  -> 参数按声明顺序写入 locals[0..argc]
  -> 创建任务，次帧首跑
```

`spawn_entry` 从 `ResolvedEntry` 开始，跳过名称查找和逐项类型检查，但仍校验 argc、owner
和池容量。参数落 locals 的顺序与内部 `OP_SPAWN` 一致。安全公开 API 不接受脱离镜像来源的
裸 EntryId；内部实现仍防御性检查其范围。

### 6.3 main 单例状态

`World` 增加全零合法的 `ecl_main_started: u8`：

- 每个 `World` 生命周期最多成功启动一次 main；
- main 正常结束或 Fault 后也不能重新启动；
- 切关通过创建新 `World` 完成；
- 只有任务成功创建后才从 0 置为 1，池满等失败允许调用方重试；
- 该字段参与 checksum，并由 `World::copy_into` 复制；
- 重复启动是绑定层违约，不是任务 Fault。

`World` 仍不持有 `EclImage` 引用；从 `start_main` 到该 World 生命周期结束，每次 `step`
必须传入同一镜像。这是现有 `&EclImage` 穿线模型的组装层前置条件。C11 落地真实镜像身份后
再把该条件升级为可自动核对的 hash/version 契约。

### 6.4 字节码内部入口纪律

- `OP_SPAWN` 的目标必须为 `Async`，argc 必须匹配参数元数据。
- `fire(..., task)` 的目标必须为零参数 `Async`。
- `CALL` 的目标必须为 `CallOnly`。
- 没有任何普通 opcode 路径能启动 `Root`。
- VM 对损坏镜像或手工错误字节码继续做运行时 kind/argc 防御，失败转成当前任务的确定性
  Fault，不 panic。

## 7. 错误与诊断模型

### 7.1 编译错误

源码契约问题使用现有 `CompileError`，包括：

- 缺少 main；
- async main 或带参数 main；
- 重复 sub 名；
- 调用、spawn 或 fire 引用了错误 kind；
- async 参数超出 locals 容量。

### 7.2 镜像构建错误

`ImageBuildError` 表示编译器或 Builder 输出的结构不合法，例如 ID/长度溢出、表未排序、
root/entries 失配或构建期引用错误。此错误发生在镜像进入 World 之前，不产生运行时 Fault。

### 7.3 名称解析错误

`ResolveError` 至少区分：

- `UnknownSymbol`；
- `RootRequiresStartMain`；
- `CallOnly`。

解析是纯操作，不修改 World 诊断状态。

### 7.4 启动错误

`TaskStartError` 至少包含：

- `NoRoot`；
- `MainAlreadyStarted`；
- `UnknownEntry`；
- `RootRequiresStartMain`；
- `CallOnly`；
- `InvalidEntryId`（仅内部防御路径，安全公开 API 无法构造）；
- `WrongArgCount`；
- `WrongArgType { param, expected, actual }`；
- `InvalidOwner`；
- `PoolFull`。

实际启动请求失败返回 `Result`，并沿用现有确定性降级纪律：

- 调用方违约：`diag.contract_viol += 1`，`last_status = STATUS_BAD_ARGS`；
- 任务池满：`diag.pool_full[POOL_TASK] += 1`，`last_status = STATUS_POOL_FULL`；
- 不杀死任何已经存在的任务。

## 8. 兼容性与迁移

这是有意的内部 ABI 更新：

- 现有 `ScriptId` 替换为职责清晰的 `SubId`/`EntryId`；
- `EclImage` 公开字段改为私有查询接口；
- `World::spawn_task` 调用点迁移到 `start_main`、`spawn_entry` 或
  `spawn_entry_named`；
- Builder 调用点迁移到两阶段 declare/define；
- 测试和 harness 不再假定 script 0 是 main。

canonical 名称排序会改变现有任务中的 script/pc 数值及代码物理布局，因此 ECL 金向量的
checksum 很可能发生一次性变化。实施时必须核对脚本可见行为未变，再按项目纪律更新三平台
一致的金向量；不能只因“预期会变”而无检查地接受差异。

## 9. 验收标准

### 9.1 编译与符号

- 不同 sub 声明顺序生成完全相同的符号表、EntryId、字节码和镜像。
- 新增字典序靠后的名称不改变之前已有的 SubId/EntryId。
- 缺失 main、async main、带参 main 和非法 main 引用均产生定位明确的编译错误。
- Root、Async、CallOnly 的 symbol 查询结果完整且正确。
- `resolve_entry` 能区分未知、root 与 call-only。

### 9.2 参数与启动

- 类型化入口逐项检查 `int/fx/angle`，错误包含参数名、期望类型和实际类型。
- 快速入口跳过类型检查但拒绝错误 argc。
- 参数按声明顺序写入 `locals[0..argc]`。
- main 只能成功启动一次，owner 固定为 stage。
- main 结束或 Fault 后仍不可重启。
- 错误 argc、owner 和池满产生正确的 `Result` 与诊断计数；内部坏 EntryId 防御路径同样
  可测。
- `ResolvedEntry` 始终绑定来源镜像，公开快速启动 API 不接受裸 EntryId。

### 9.3 快照与 VM 防御

- `ecl_main_started` 参与 checksum 和 `copy_into`，回滚前后单例约束一致。
- `EclImage::empty()` 保持零任务、零行为。
- VM 遇到错误 SPAWN/CALL kind 或 argc 时产生确定性 Fault，不 panic。

### 9.4 回归门

- `stg-core` ECL 测试通过；
- `stg-ecl-compiler` 全测试和 doctest 通过；
- workspace Clippy `-D warnings` 通过；
- 三平台 determinism gate 通过；
- ECL 金向量若变化，必须有逐项行为核对记录。

## 10. 后续工作

本设计落地后，C11 可在不改变名称和入口语义的前提下继续增加：

- 文件头中的 engine/schema/opcode 版本；
- canonical 序列化与真实 content hash；
- 加载期完整字节码 validator；
- 回放和联机握手中的 ECL 镜像身份；
- 资产路径到单镜像短名称命名空间的外层映射。
