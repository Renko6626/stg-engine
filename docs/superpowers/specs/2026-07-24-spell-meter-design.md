# 符卡计器机构(spell meter)设计

> **一句话**:把符卡的**记账**收归引擎机构(`SpellState`:计时/bonus 衰减/资格作废/结算
> 入分/事件),**控制流**留在脚本(宣言/弹幕/换卡)——ZUN 忠实分工,杀掉"每张卡手写
> 轮询计时记分"的 ECL 样板。含 **A2 拍板范围修订**(评审记录见 §1)。

**目标**:让一张符卡的脚本 = `spell_begin(...)` + 弹幕行为 + `while spell_timer() >= 0`
+ 换卡——记账零样板;收卡/破卡成为**世界事实**(`EVT_SPELL_*`,RL episode 正典边界)+
**表现请求**(通道 B,宣言/结算演出);`boss_ui` 由机构自动喂,rainbow.ecl 的 `timer_ui`
轮询循环整个删除。

**归属**:两线(godot 关卡内容 / RL 训练场景)共同的内容层前置;2026-07-24 评审对话拍板。

---

## 1. A2 拍板范围修订(评审记录)

原拍板(stg-world-design A2):"boss 阶段"即 `boss_ui` 公告板,**世界自身不读**,超时/
换卡判断都是 boss 主控任务的事。本次修订(2026-07-24 对话评审):

- **维持**:换卡/阶段切换/弹幕行为归脚本;`boss_ui` 仍是表现公告板、世界逻辑不读**它**。
- **修订**:符卡**记账**(计时递减/bonus 衰减/miss·bomb 作废资格/超时判定/结算入分)收归
  引擎新机构 `SpellState`——性质同道具经济(M0-12):机械、数据驱动、确定性、无回调(P5),
  逐帧推进挂 settle 尾趟(§3.5 相位数不变,settle"三趟"扩为"三趟+符卡趟",本 spec 即
  该内容修订的评审记录)。
- 依据:ZUN 实机分工即如此(ECL 宣言,引擎记账);全脚本方案迫使每卡作者重写轮询记账,
  且脚本层无自机资源读口,丑且易错帧。
- 收尾时在 stg-world-design A2 段落加一行修订注记(状态注记先例:emit_req 已落地注)。

## 2. `SpellState`(新模块 `stg-core/src/spell.rs`,字段簇入 WorldBody)

```rust
/// 符卡计器槽(每 boss 槽一个,MAX_BOSSES=2)。全字段 POD;checksum/copy_into/SaveBytes
/// 三件套照新字段四件套清单落(尺寸哨兵会逼)。
#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub struct SpellSlot {
    pub active: u8,        // 0=空闲
    pub flags: u8,         // bit0 = SPELL_SURVIVAL(耐久卡:超时即收卡判定点)
    pub capture_ok: u8,    // 资格:1=仍可收卡;miss/bomb 即时清 0
    pub _pad: u8,          // 显式占位(P6 全量校验,零初始化合法)
    pub spell_id: u16,
    pub boss_index: u16,   // 绑定 boss 敌句柄(宣言者 owner)
    pub boss_gen: u16,
    pub frames_left: u16,  // 时限余帧;0 即超时判定点
    pub bonus_now: u32,    // 当前 bonus(分)
    pub bonus_floor: u32,  // 衰减地板 = bonus0 / 10(整数除,begin 时定格)
    pub dec_per_frame: u32, // = (bonus0 - floor) / time_limit(整数除,begin 时定格)
}
pub const SPELL_SURVIVAL: u8 = 1 << 0;
```

`WorldBody` 新增 `pub(crate) spells: [SpellSlot; crate::boss::MAX_BOSSES]`(生而封口;
读经通道 A:`WorldView::spells(self) -> &'w [SpellSlot]` 新访问器)。

## 3. 逐帧推进规则(settle 符卡趟,机械、按槽升序)

对每个 `active` 槽:

1. **资格轮询作废**(先于一切):玩家 0 `life_state != LIFE_ALIVE`(中弹入决死窗即作废,
   deathbomb 救回命也不还资格——ZUN 语义)或 `bomb_phase != 0` ⇒ `capture_ok = 0`。
   (轮询式:对帧内事件序零依赖,确定性平凡。co-op 语义随 B3 族后议,v1 只看玩家 0。)
2. **bonus 衰减**:`bonus_now = max(bonus_floor, bonus_now.saturating_sub(dec_per_frame))`
   (线性,无宽限段——v1 简形;ZUN 分段衰减记 follow-up 候选)。
3. **boss 死亡自动收口**:绑定句柄失效 **或绑定敌带 `ENEMY_DYING` 旗**(致死在本帧
   settle 趟二、回收在相位 9——查 dying 旗才能**当帧**结算,不落后一帧)⇒ 按"HP 路径
   结束"结算(§4)。
4. **超时判定**:`frames_left == 0` ⇒ 耐久卡按"收卡判定点"结算;普通卡按"超时失败"结算。
   否则 `frames_left -= 1`。
5. **`boss_ui` 自动喂**(active 期间,机械覆写绑定槽):`enemy`/`spell_id`/`timer_frames =
   frames_left`/`active = 1`,`hp_ratio` = 绑定敌 `hp/hp_max` 真除(定点);**`phase_left`
   不动**(阶段规划归脚本,仍走 `boss_set`——机构只覆写其余字段,boss_set 在非符卡段照旧全权)。

## 4. 结束路径矩阵(全部由机构结算,原子完成:付分+事件+req+清槽)

| 路径 | 触发 | 资格在 | 资格失 |
|---|---|---|---|
| **HP 路径** | 脚本 `spell_end()`(打过血线由脚本判,A2 维持项)或 boss 死亡(§3.3) | **CAPTURED**:玩家 0 `score += bonus_now` | FAILED(reason=资格失) |
| **超时·普通卡** | `frames_left` 归零 | FAILED(reason=超时) | FAILED(reason=超时) |
| **超时·耐久卡** | `frames_left` 归零 | **CAPTURED**(耐久卡的收卡点就是活到超时) | FAILED(reason=资格失) |

- 事件(A5 世界事实,入大事记):`EVT_SPELL_DECLARED = 6`(begin 时,`data=[spell_id,
  bonus0]`)、`EVT_SPELL_CAPTURED = 7`(`data=[spell_id, 实付 bonus]`)、
  `EVT_SPELL_FAILED = 8`(`data=[spell_id, reason]`,reason:1=资格失 2=超时)。
  `a_index/a_gen` = boss 句柄。**RL episode 边界正典信号即此三事件。**
- 通道 B req(表现,双轨同敌死先例):`REQ_SPELL_DECLARE = 2`(`args=[spell_id, bonus0,
  time_limit, survival_flag, 0, 0]`)、`REQ_SPELL_RESULT = 3`(`args=[spell_id, captured,
  实付 bonus, reason, 0, 0]`)——C14 structural 注册,reqs.rs 约定表各加一行。
- 清槽:全字段归零(复用槽写满纪律;下一卡 begin 时全字段重写)。

## 5. syscall 与 `.ecl` 表层

| 号 | 名 | 参(声明序) | 语义 |
|---|---|---|---|
| `SYS_SPELL_BEGIN = 28` | `spell_begin` | `slot, spell_id, time_limit, bonus0, flags` | owner 必须 ENEMY(misuse → Fault,`self_enemy_handle` 先例);槽越界/时限 ≤0/bonus0 <0/槽已 active → P4-b no-op+计数;成功即定格衰减参数 + 发 DECLARED 事件/req |
| `SYS_SPELL_END = 29` | `spell_end` | (无参) | 按 owner 句柄找绑定槽走 HP 路径结算;无绑定槽 → P4-b no-op+计数(重复调用安全) |
| `SYS_SPELL_TIMER = 11` | `spell_timer` | (无参)→ int | 读族:owner 绑定槽的 `frames_left`;无绑定槽返回 **-1**(脚本等待惯用:`while spell_timer() >= 0 { wait(1); }`) |

builtins 三条(`spell_begin` 5×Val(Int)、`spell_end` 0 参 ret None、`spell_timer` ret Int);
ecl-lang.md 新节「符卡」给创作范式(含"每张卡=具名 async sub"约定——RL 单卡训练/F3 单卡
预览的地基);ecl-ops.md 号表三行。

## 6. rainbow.ecl 狗粮化(刀 3)

`timer_ui` 轮询循环删除,风铃卡改 `spell_begin(0, SPELL_WINDCHIME, 3600, 100000, 0)` +
主控 `while spell_timer() >= 0` 等待——**金向量流预期变化**(见 §7),脚本行数净减。
`SPELL_WINDCHIME` 用脚本侧 `const`(卡 id 词汇归脚本/关卡资产,引擎不注册)。

## 7. 确定性与金向量论证

- WorldBody 新增 `spells` 字段簇 ⇒ **校验和取值平移**(通道 B 刀同款预期);rainbow 狗粮化
  ⇒ **演化本身变更**(删循环/入分)。两者叠加:**金向量与基线 byte-diff 必然不同,且这次连
  行为都变**——回归判据 = 判别式单测全家 + 跨平台 CI 对拍 + storm 闸(重演对 spells 字段
  自动覆盖:checksum/SaveBytes/copy_into 三件套即防漏,尺寸哨兵逼清单)。
- 机构全整数(I1)/帧计时(I6)/POD 入 World(I7)/无回调(P5)/按槽升序(I4)/P4 全表
  (D12 加三行)。衰减参数 begin 时一次整除定格,逐帧只有减法与比较。

## 8. 不做什么

- **ZUN 分段衰减曲线/宽限期**:v1 线性;真做关卡内容嫌糙再升级(参数已隔离在 begin 计算)。
- **收卡史/战绩统计**:跨局元数据,断层线上归外层(G6 划界维持)。
- **spell_id → 名字/立绘**:表现资产,godot 线(G4 维持)。
- **co-op 资格语义**(谁 miss 作废/分给谁):v1 玩家 0;记 B3 族。
- **脚本读自机资源 syscall**(原 G1):被本机构溶解后降为独立小件,不随本刀。
- **phase_left 自动化/多段血条机构**:阶段规划归脚本(A2 维持项)。

## 9. 测试策略(判别式)

1. **衰减曲线**:begin(限 100 帧, bonus 1000)→ 推进 N 帧断言 `bonus_now` 精确值(整除
   定格系数可手算);地板钳制(推超限后 == floor 不再降)。
2. **资格作废三触发**:中弹入决死窗/bomb 起爆/两者都无——三世界分别推进,断言 capture_ok
   0/0/1(判别:轮询字段取值可区分)。
3. **结束矩阵六格**:HP 路径×资格在/失、超时普通卡、超时耐久卡×资格在/失、boss 死亡路径
   ——逐格断言 score 增量(实付 bonus 精确值 vs 0)+ 事件 kind/data + req id/args。
4. **boss_ui 自动喂**:active 期间 timer/spell_id/hp_ratio 逐帧命中(hp 打掉一截后 ratio
   变化可判);`phase_left` 经 boss_set 写后不被覆写。
5. **syscall 边界**:STAGE owner 调 spell_begin → Fault;槽越界/重复 begin → P4-b 计数;
   spell_end 无绑定 → no-op;spell_timer 无绑定 → -1。
6. **表层端到端**(ecl-compiler):内联脚本 begin→等待→end,断言事件序与分数。
7. **storm/save 兼容**:spells 字段随快照/存档往返(深等价 checksum 测试自动覆盖——
   新基建白拿,storm 短版照跑)。
8. 尺寸哨兵按四件套清单更新;金向量跨平台 CI 照绿(取值平移+行为变更皆预期,见 §7)。

## 10. 收尾

- stg-world-design A2 段修订注记一行;D12 表加三行(spell_begin/end/timer)。
- reqs.rs 约定表 +2 行;events.rs 事件文档;ecl-lang.md 符卡节;ecl-ops.md 三行;
  CLAUDE.md 仓库结构图 spell.rs;PROGRESS 史行+「现在」。
- follow-ups:G1 读资源 syscall 降级记档;ZUN 分段衰减记候选。
