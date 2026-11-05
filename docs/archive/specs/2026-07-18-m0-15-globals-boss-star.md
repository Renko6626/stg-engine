# M0-15 globals + boss_ui + 消弹转星星 —— 设计 spec

> 状态：已过 grill 拍板（2026-07-18），待实施计划。定位：**M1 ECL 硬前置**（脚本状态地基 +
> boss 公告板）+ 东方消弹经济闭环。上游：A2（`globals`/`boss_ui` 形状既定）、D12（错误语义
> 表已预写）、D9 趟一（消弹挂点）、M0-12 道具扩展四步清单。

## 拍板纪要（grill，2026-07-18）

1. **消弹转化范围：一律转化**——所有 `FIELD_CLEAR_BULLETS` 消掉的弹都转星星，不加新
   flag 位（静默清屏需求出现时再议，YAGNI）。
2. **星星物理：出生即磁吸**——ZUN 系 bomb 星星经典行为：出生即 `magnet_to` 指向**存活
   自机**（升序首个 ALIVE，I4 口径），复用现有磁吸机器；**无存活自机**（决死窗口/等重生）
   → `MAGNET_NONE` + 零初速正常下落（重力/终速），走三源磁吸后收。**无散布无 RNG**。
3. **星星分值 30**（POWER=10 / 蜡=50 / POINT=100 之间的"少量但有感"档）。
4. **boss_set 形态：整槽写入** `boss_set(slot, BossUiSlot)`——与"复用槽写满"纪律同构，
   脚本层想改单字段自行先读后写；世界 API 面最简。

## A. globals（A2 照章落地）

- `WorldBody` 新字段 `globals: [i32; 1024]`（4KB，全局变量竞技场，**语义归脚本、世界自身
  不读不写**）。零初始化合法。derive 自动入校验和（P6）；**`step.rs::copy_into` 手工快照
  必须加拷贝行**（漏拷 = 回滚丢状态，判别测试钉住）。
- API（D12 错误语义表照章）：
  - `set_var(&mut self, slot: u16, val: i32)`：slot ≥ 1024 → no-op + `contract_viol` +
    `last_status = BAD_ARGS`。
  - `get_var(&mut self, slot: u16) -> i32`：slot ≥ 1024 → 返回 0 + 同上计数（取 `&mut self`
    为的就是坏槽计数入校验和——两机必须一样错）。

## B. boss_ui（A2 照章落地）

- 新数据模块 `crates/stg-core/src/boss.rs`：`MAX_BOSSES = 2`（D10 预算"boss_ui×2"）+

  ```rust
  #[repr(C)]
  #[derive(Clone, Copy, Checksum)]
  pub struct BossUiSlot {
      pub enemy: EnemyHandle, // 哪个敌人是 boss
      pub hp_ratio: Fx,       // 血条比例（脚本负责刷新）
      pub spell_id: u16,      // 当前符卡 id
      pub timer_frames: u16,  // 倒计时（脚本负责递减）
      pub phase_left: u8,     // 剩余阶段数（血条下的星星）
      pub active: u8,         // 0 = 无 boss（零初始化合法）
  }
  ```

  `Default` 手工实现（`enemy: EnemyHandle::NULL`，余零）——句柄类型如无 `Default` derive。
- `WorldBody` 新字段 `boss_ui: [BossUiSlot; MAX_BOSSES]`（`pub`——UI/表现层要读；**世界自身
  逻辑不读它**，超时/换卡判断归 boss 主控任务）。入校验和 + `copy_into` 加行。
- API：`boss_set(&mut self, slot: u8, ui: BossUiSlot)`：slot ≥ MAX_BOSSES → no-op +
  `contract_viol` + `BAD_ARGS`。**不校验 `enemy` 句柄有效性**（世界不读它；悬垂句柄由读方
  按"视同已失效"处置——P4-b 既定哲学）。

## C. 消弹转星星

- `items.rs` 扩展四步清单照走：`ITEM_STAR = 4` + `ITEM_CFG` 新行（score=30，磁吸参数同
  STD 基线）+ `credit_item` 新臂（纯加分）+ 掉落表**不动**（星星不出现在 drop table，
  只由消弹产生）。
- **挂点：settle 趟一**（唯一改状态者）。field 消弹处，每颗被消的弹在其 `(x, y)` 原位
  生成一颗星星：`item_type = STAR`、`magnet_to = 升序首个 ALIVE 自机`（无则 `MAGNET_NONE`）、
  `vx = vy = 0`、`timer = 0`。磁吸目标**每颗消弹时点查一次**（趟一内自机状态不变，实际
  同帧同值；实现上提出趟外算一次亦可——确定性等价，取实现简洁者）。
- **P4-a**：道具池满 → 该颗星星不生成 + `pool_full[ITEM]` 逐颗计数（与单发 create_* 同律，
  不短路——消弹循环本身有界）。全屏消弹 ~300 发对 512 池是真实压力源，金向量顺带覆盖
  池满降级路径（B1 族的道具池腿）。
- `FieldCleared` 聚合事件不变（count 语义照旧）；星星拾取走现有趟三 + `EVT_ITEM_PICKED`。

## 金向量

**导演零改动**——现有每 150 帧全屏消弹场自动转星星入流（一律转化的直接收益）：
消弹→星星雨→磁吸流→拾取入账→可能的道具池满降级全链进对拍。双跑照旧。

## 判别式单测（最小集）

- globals：set/get 往返；坏槽 set no-op + get 回 0 + 各计一次；**写 globals 改变校验和**；
  **快照往返保 globals**（set → snapshot → 改 → restore → 值/校验和还原）。
- boss_ui：整槽写入可读回；坏槽 no-op + 计数；写 boss_ui 改变校验和；快照往返保 boss_ui。
- 星星：消弹 N 发 → 恰 N 颗星星、各在原弹位（判别：多弹异位逐一断言）；磁吸目标 =
  ALIVE 自机索引；无 ALIVE 自机 → `MAGNET_NONE`；拾取入账 score +30；道具池满 →
  少生成 + `pool_full[ITEM]` 计数；掉落表行为不变（回归腿）。

## 变异检验 ≥3 候选

星星生成位置改场中心 → 原位测试红；`credit_item` STAR 臂分值改 0 → 入账测试红；
`copy_into` 的 globals 拷贝行删除 → 快照往返测试红。

## 收尾义务

`stg-world-design.md`：D12 成员表 `set_var/get_var/boss_set` 落地括注 + D9 趟一补"消弹
转星星"一句 + D7 道具类型表加 STAR 行；`PROGRESS.md` 史行 + 现在段；follow-ups 若产生
延后项入库。

## 验收

判别单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；新字段全部入校验和
且 `copy_into` 有拷贝行。
