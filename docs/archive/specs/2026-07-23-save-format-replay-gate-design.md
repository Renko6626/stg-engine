# 存档字节格式(L1)+ 恢复重演风暴闸(L2)设计

> **一句话**:World 的字段级规范字节 `save_bytes`/`load_bytes`(乙案:stg-derive 新
> `SaveBytes` derive,与 Checksum 同源字段清单、同一 skip 口径)+ 身份头 v1 +
> harness `storm` 子命令(多点存档→恢复重演→校验和流逐位对拍);顺手裁决 **F2:保
> FNV-1a64 冻结,不换**。两线(godot 游戏 / RL)共享底座的最后一块;环形缓冲/预测未来
> 是消费侧插件(D10/D11 预决定),**不在本刀、不进 core**。

**目标**:让"状态出进程"成为一等能力——godot 随地存档/回溯的地基、RL 状态档案与断点
续训的地基;L2 风暴闸把"恢复 == 从未离开"焊成 CI 性质。

**性能账**(2026-07-23 release bench 实测,World=1.03MB):`copy_into` 31-47µs;推算
save = 字段写 ~0.1ms + 载荷 FNV ~1.6ms + 落盘 ⇒ **~2-3ms/次**,随地存档零感知。

---

## 1. L1:`SaveBytes` derive(stg-derive)

- 新 proc-macro derive `SaveBytes`,与 `#[derive(Checksum)]` **同一字段遍历机制**,生成:
  - `fn write_bytes(&self, out: &mut Vec<u8>)` —— 字段声明序、小端、无 padding(规范字节);
  - `fn read_bytes(&mut self, r: &mut SaveReader<'_>) -> Result<(), LoadError>` —— 逐字段
    读回,长度不足 → `LoadError::Truncated`。
- **skip 单一口径**:`#[checksum(skip = ...)]` 字段**同时不入档**——derive 在写读两侧都
  **省略**它们;清零语义由 **`load_bytes` 的零构造契约**承担(目标 World 堆零构造后逐字段
  读回,skip 字段保持零 = "恢复出的 World 必须无陈旧输出",与 `copy_into` 同口径;
  `phase_guard` debug-only 不入档 ⇒ debug 存 ↔ release 读天然兼容)。`read_bytes` 的
  trait 契约写明"目标须零初始化"。
- 防漏性质白拿:新增 World 字段自动入档(同 checksum 单一真相源);收口刀的尺寸哨兵 +
  "save→load→checksum 相等"测试(§4)双重兜底。
- 叶型手写 impl(core 侧 trait):整数族/`[T; N]`/`Fx`/`Angle`(raw 出入)。凡有 Checksum
  impl 的类型镜像补 SaveBytes:derive 类(PlayerState/BossUiSlot/DiagCounters/Task/
  TaskPool/Pcg32…)加 derive;`define_pool!` 宏内同步生成;`XformSegPool` 手写 Checksum
  的照样手写 SaveBytes。
- `SaveReader`(core,tables.rs C11 `Reader` 同款纪律):只读整数、越界即 `Truncated`。

## 2. L1:头格式 v1 与 World API(step.rs)

头(小端,49 B):`magic "STGW"(4) + file_ver u8 = 1 + ENGINE_VER u32 + 表 content_hash
u64 + 镜像 content_hash u64 + seed u64 + frame u32(速览用,载荷为准) + payload_len u32 +
payload_fnv u64`(vendored FNV-1a64 over 载荷,C11 完整性校验同款)。

```rust
impl World {
    /// 存档:头 + 字段级规范字节载荷。~2-3ms(大头是完整性 FNV),随地存档零感知。
    pub fn save_bytes(&self, image: &EclImage) -> Vec<u8>;
    /// 读档:头校验(magic/file_ver/ENGINE_VER/载荷 FNV)+ 表/镜像 coherence(任一侧 0 =
    /// 未绑定跳过,同 start_main 守卫口径)→ 堆零构造 + 逐字段读回。P4 式 Err 不 panic。
    pub fn load_bytes(
        bytes: &[u8],
        tables: &WorldTables,
        image: &EclImage,
    ) -> Result<Box<World>, LoadError>;
}
pub enum LoadError { BadMagic, BadFileVer, EngineVerMismatch { .. }, HashMismatch { .. },
    TablesMismatch { .. }, ImageMismatch { .. }, Truncated, TrailingBytes }
```

- 表/镜像哈希从**参数**取(World 自身的 `tables_hash` 字段照常入档并回读,load 后二者必然
  一致——header 校验先挡在前面)。
- `TrailingBytes`:载荷读毕必须恰好耗尽(规范字节无冗余)。

## 3. L2:`storm` 风暴闸(harness 子命令 + CI 短版测试)

`storm [--frames 1200] [--saves 8] [--seed N]`:

1. `build_rainbow_world(seed)`;宿主侧**独立** Pcg32(表现层那颗,I3)逐帧生成伪随机输入
   掩码(方向/射击/低速随机组合),全程记录;
2. 跑全程,逐帧记校验和流;沿途 `saves` 个均匀点各做:(a) 内存快照(`copy_into`)
   (b) `save_bytes` → `load_bytes` 往返,断言**二次 save 字节全等** + **load 后 checksum
   == 原 world checksum**;
3. 对每个点、两种恢复源(内存快照 / 磁盘往返):用记录的输入序列**重演到终点**,校验和流
   尾段与原始流**逐位对拍**;任何不等 → 非零退出 + 首分歧帧定位(desync 手术刀口径)。
4. CI 形态:harness `#[test]` 跑短版(~240 帧 × 3 点),`cargo test` 即闸;全参数版留
   手动/夜跑。

## 4. F2 裁决(本刀记档,窗口关闭)

**保 FNV-1a64,不换。** 依据(2026-07-23 实测):校验和 1.56ms/帧**无任何在线逐帧消费者**
——单机不算、联机 K=20 摊 ~70µs(实测口径,详 follow-ups F2)、CI/storm 离线;换字宽 mix 只省离线工具耐心,却要重 bless
金向量 + 多背一刀。`ENGINE_VER = 1` 的身份语义**即含 FNV-1a64**;将来真换 = bump + 过评审。
follow-ups F2 条目改口记裁决(留"若 M4 实测采样成本超预算再启"一句活口)。

## 5. 不做什么

- **环形缓冲/回退调度/预测 rollout 帮手**:消费侧插件,随 godot 线出生(D10/D11:"环归
  消费者所有")。
- 压缩/增量存档/跨 ENGINE_VER 迁移/py 绑定暴露(`save_bytes` 的 py 面随 stg-py)。
- 存档文件的向后兼容承诺:v1 就是 v1,格式变 = file_ver+1,老档拒读不迁移(测试工具阶段)。

## 6. 测试策略(判别式)

1. **往返深等价**:跑 N 帧(有弹有敌有任务)→ save → load → `checksum()` 相等(校验和
   即全字段深比较,防漏白拿)+ 二次 save 字节全等。
2. **skip 字段清零**:save 前向 reqs/events/hits 预污染 → load 后三缓冲空、`phase_guard`
   复位(收口刀"预污染 dst"教训:恢复目标先弄脏再断言)。
3. **头错误路径逐一判别**:坏 magic/坏 file_ver/改 ENGINE_VER 字节/翻转载荷一位(FNV 必
   抓)/截断/错表哈希/错镜像哈希/尾部加垃圾——八条各得其 `LoadError` 变体。
4. **storm 短版**(CI):内存源与磁盘源重演均逐位;变异检验一次(临时篡改重演输入一帧 →
   必须报分歧,证明对拍真在比)。
5. 金向量逐位不变(纯增量:新 derive/新 API/新子命令,不触 checksum 与演化路径)。
6. 全绿 + fmt/clippy + `cargo tree -p stg-core` 防火墙(零新依赖——SaveBytes 是 stg-derive
   自家 derive)。

## 7. 收尾

- follow-ups:F2 改口记裁决;bench-baseline 留给 A2 时把本刀实测数一并续表。
- CLAUDE.md 常用命令加 `storm`;architecture M3 行改口(L1/L2 落地,M3 剩余=环形调度,
  已挪 godot 线)。
- PROGRESS:史加一行 + 「现在」段(两线并行起点就绪)。
