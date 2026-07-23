# 外接前收口刀(pre-bridge sealing)设计

> **一句话**:M2 外接层动工前,把六路系统审阅(2026-07-23)裁定的"外接前必修"一次收掉——
> 快照防漏 + D6 硬风险面四项封口 + 场界常量放行 + `spawn_entry*` coherence 守卫 +
> `ENGINE_VER` 铸造 + 弹 setter 陷阱文档;并把其余审阅发现批量记档 `follow-ups.md`。
> **发现即需求**:各项的依据与裁决见审阅报告(六路 subagent 实读引证),本 spec 只定修法。

**目标**:让 WorldBridge/py env 的作者拿到一个**误用面收干净、身份素材齐全**的 stg-core。
纯可见性/守卫/测试/文档刀,**行为零变化,金向量逐位不变**(无新增 World 字段,无演化路径改动)。

**归属**:M2 前置终刀(承接刀 A / 通道 A / 通道 B)。

---

## 1. 快照防漏(审阅唯一 Critical)

`World::copy_into`(step.rs)是手写字段清单,加字段漏拷编译不报错;checksum-mechanism.md:37
承诺的"尺寸变更即红"守卫从未落地;七个字段无判别式拷贝测试(通用回归被零值遮蔽)。

1. **哨兵测试**(step.rs tests):`assert_eq!(size_of::<WorldBody>(), N)` + `size_of::<World>()`,
   `#[cfg(debug_assertions)]`/`#[cfg(not(...))]` 双值(phase_guard 只在 debug 存在);断言消息
   写明"尺寸变了 ⇒ 你加/删了字段 ⇒ 去核对 copy_into 清单 + checksum + D10 预算,再更新本数字"。
2. **补七个字段的判别式拷贝测试**:`frame`/`rng`/`players`/`diag`/`last_status`/
   `ecl_main_started`/`tables_hash`——各自 mutate 成非默认值 → `copy_into` → 断言 dst 命中
   (模式照抄既有 `globals`/`signals` 拷贝测试)。`rng` 用"走一步再比对下一抽"或比对 checksum。
3. **CLAUDE.md 自检清单第 2 条**加一问:"`copy_into` 同步了吗(哨兵测试会红)?"
4. checksum-mechanism.md:37 的承诺从"CI 守卫"改口为"哨兵测试守卫"(实现形态即测试,CI 跑测试)。

## 2. D6 硬风险面封口(四项;globals/boss_ui/diag/last_status 留 M2 顺手)

| 字段 | 修法 | 迁移 |
|---|---|---|
| `World.tasks`(step.rs:20) | `pub → pub(crate)` + 新读口 `World::tasks(&self) -> &TaskPool` | harness:1087 `w.tasks.iter_alive()` → `w.tasks().iter_alive()`(唯一外读) |
| `TaskPool::new`(ecl/task.rs:85) | `pub → pub(crate)` | 无外部调用者 |
| `WorldBody.rng`(world.rs:145) | `pub → pub(crate)` | 外部零读者,零迁移 |
| `WorldBody.frame`(world.rs:144) | `pub → pub(crate)` + `WorldBody::frame(&self) -> u32` + `World::frame()` 委派(M2 水位协议要读帧号;字段名=方法名共存,池先例) | 外部零读者 |
| `WorldBody.events`/`events_len`(world.rs:168/170) | `pub → pub(crate)` + `WorldBody::frame_events(&self) -> &[Event]`(A9 契约名,按 `events_len` 切片,幂等只读,镜像 `take_requests`)+ `World::frame_events()` 委派 | 外部零读者(cut-B 时已 grep 实证) |

- `tasks()` 交出 `&TaskPool` 即只读(mutator 全 `pub(crate)`,同 `&Pool` 之于通道 A 的论证)。
- `docs/follow-ups.md` D6 条目改写:已收 tasks/rng/frame/events 四项,剩 `globals`/`boss_ui`/
  `diag`(先补 `diag()` 读口再收,harness:1073 直读要迁)/`last_status`,触发点 M2。

## 3. 场界常量放行(一行级)

world.rs:99-103 四常量 `pub(crate) → pub`:`FIELD_HALF_W`/`FIELD_HEIGHT`/`OOB_MARGIN`/
`ENEMY_OOB_MARGIN`,各配一句面向外接消费者的 doc(单位:逻辑像素 px;场坐标系 x∈[-192,192],
y∈[0,448],D7 中轴原点)。WorldBridge 对齐 viewport 不再硬编。

## 4. `spawn_entry`/`spawn_entry_named` coherence 守卫

`start_main` 的表配对守卫(binding.rs:129-141,`image_hash≠0 ∧ tables_hash≠0 ∧ 不等 →
contract_viol + BAD_ARGS + Err(TableImageMismatch)`)提为 World 私有 helper
`check_table_coherence(&mut self, image: &EclImage) -> Result<(), TaskStartError>`,三站共用
(`start_main` 重构为调 helper——纯等价改写;`spawn_entry`/`spawn_entry_named` 入口处新调)。
判别测试:错表 image 喂 `spawn_entry` → `Err(TableImageMismatch)` + `contract_viol` 计数;
对表 → 照常派生。销 follow-ups 对应条目("spawn_entry 无守卫,留 M2 补"——本刀即 M2 前置)。

## 5. `ENGINE_VER` 铸造

`crates/stg-core/src/lib.rs`:`pub const ENGINE_VER: u32 = 1;`,doc 写明 bump 纪律——
**凡改 op 表语义/syscall 号语义/校验和算法/烘焙表内容/池布局/step 相位序,必须 bump**
(设计文档"bump engine_ver"条款的落点);回放头(M3)/联机握手(M4)从这里读。
architecture.md M3 行的"`engine_ver`"从虚指变实指(文字不用动,常量存在即兑现)。
单测:`ENGINE_VER == 1`(锚定值,bump 时必须有意识地改测试——同号表冻结纪律)。

## 6. ecl-lang.md 弹 setter 陷阱段落

「内建函数」节弹 setter 族处补明:**首参 handle 求值即丢弃,setter 恒作用于任务 owner 弹**
(`self` 语义);不能借句柄定向操纵别的弹;owner 非弹 → Fault。M2 把内建面暴露给脚本作者
前必须写清的反直觉点(codegen `is_self_bullet_setter` 已有内部文档,本项是用户面补齐)。

## 7. 记档批(follow-ups.md,一次 commit)

新增/更新以下条目(每条含出处=本轮审阅、严重度、触发点):
- **generation u16 ABA**(新):最低空位分配使热槽复用集中,65536 次回绕后跨帧长持句柄
  理论可撞别名;触发点 M5 headless 长跑前复核(估算 churn 量级,必要时 gen 扩 u32 或记文档约束)。
- **hits 溢出 debug panic 未实现**:蓝图 A5/D12 三处承诺 debug panic,实现只计数;
  且 HITS_CAP 按单自机估,行 1/2 双计 + 双自机可超;触发点 M2 顺手(改实现或改蓝图口径)。
- **ItemPool 无 sprite 数据源**:`ItemTypeCfg` 缺 sprite 列,道具外观映射在 core 侧缺位
  (弹/敌都有);触发点 M2 建桥第一周(加一列 + 表 bump)。
- **Fault/P4-b 口径不一对子**:appearance 越界 Fault vs item_type 越界静默 NULL;
  batch 负计数钳零 vs xform_cnt 负数 Fault;归并入既有 C7 作者体验条目。
- **bench 基线过期**:World 实测 1.03MB(任务池 ~108KB 从未入账),校验和/step 比值
  实测远超文档;触发点 M2 前重跑 bench 续表。
- **EclImage 装载路径拍板**:M2 走"启动时编译 `.ecl`"(harness 同款);离线序列化格式
  推迟到 modding 需求出现(避免过早冻结镜像字节格式)。
- **数学核小项**:`Angle` 缺 `FULL_TURN` 具名常量;`Angle` 派生 `Ord` 是线性序陷阱
  (无现用点);motion.rs isqrt→Fx 窄化缺 debug 护栏(正常速度不可达)。
- **池文档账目**:pool-memory-layout.md 弹池汇总虚高 ~30%;蓝图 D5 敌池 "~64B" 实为 ~74B。

## 8. 一行级文档修正(顺手,不记档)

- `field.rs` 半径校验注释:删"players 是 pub"过时前提(刀 A 已封,校验已迁 `WorldTables::validate`)。
- `stg-world-design.md` D12 表 `move_to` 行:"dur=0 → BAD_ARGS" 改为与 D5 拍板一致
  ("dur=0 = 瞬移,合法退化")——代码是对的,表行陈旧。

---

## 不做什么(划界)

- **globals/boss_ui/diag/last_status 收口**:D6 余项,M2 顺手(diag 要先造读口迁 harness)。
- **hits debug panic / ItemPool sprite / bench 重跑 / isqrt 护栏**:记档,M2 处理。
- **generation 扩宽 / PoolView\<Cols\> / from_bytes 钳制**:各随其触发点(M5/M5/modding)。
- **`sys_emit_req` 引擎 id 段强制**:世界不解释 id 是拍板语义,不改。

## 确定性与金向量论证

纯可见性(编译期)+ 新守卫(错误路径,金向量不触发:golden 走 `start_main` 且表匹配)+
测试 + 文档 + 一个新 pub 常量。无新增 World 字段、无演化路径改动 ⇒ **金向量与基线逐位全等**
(本刀恢复 byte-diff 回归闸;通道 B 刀的取值平移已是新基线)。零新依赖。

## 测试策略

1. 哨兵尺寸测试(§1)——判别:改字段集必红。
2. 七字段判别式拷贝测试(§1)——判别:从 copy_into 删对应行必红。
3. `spawn_entry` 错表 → `TableImageMismatch` + 计数;对表 → 正常派生(§4)。
4. `frame()`/`frame_events()`/`tasks()` 读口行为测试(切片界=events_len;帧号推进可见)。
5. `ENGINE_VER == 1` 锚定测试(§5)。
6. 金向量 byte-diff 全等 + workspace 全绿 + fmt/clippy + `cargo tree -p stg-core` 防火墙。

## 收尾

- `docs/follow-ups.md`:§7 批量记档 + D6 改写 + 销 spawn_entry 条目。
- `docs/architecture.md`:M2 接缝行"焊点"补"外接前收口 ✅(快照哨兵/写口封/守卫/ENGINE_VER)"。
- `PROGRESS.md`:史加一行 + 重写「现在」段(下一步:开 `stg-godot` crate)。
