# M0-17 shottype 表 + WorldTables 骨架 —— 设计 spec

> 状态：已过 grill 拍板（2026-07-18），待实施计划。定位：火力→弹型接线（power_tier 通电）+
> A3 WorldTables 骨架落地（静态只读数据层，全家入驻）。上游：ZUN `.sht` 机制调研
> （TH07 起数据驱动；TH10+ = 5 档 × 2 态 shooterset，shooter = 发射器条目）、
> M0-16 火力定标（0.00-4.00，`power_tier()` 0..=4）、A3（两类表两条身份链）。

## 拍板纪要（grill，2026-07-18）

1. **WorldTables 骨架这刀建**，且**全家入驻**：shottype 表（新）+ `ITEM_CFG`/`DROP_TABLES`/
   `ITEM_GRAVITY`（items.rs 迁入）+ 角色参数（player.rs 的移速四常量/判定半径/擦弹半径迁入）。
   场界/相位/引擎宪法常量不属于"表"，原地不动。**内容哈希留字段占位不实现**（v0 表编译进
   二进制，二进制同一性已覆盖；文件加载刀再做真哈希——A3 双身份链）。
2. **子机（option）= 纯数据发射原点**：表里 `option_pos[tier]`（每档位子机偏移列表），
   shooter 的 `option` 号解析为"出生点 = 自机位 + 该子机偏移"；世界侧**不建 option 实体**
   （零新池零新状态，表现层将来读表自画）。
3. **发射计时器 = `shot_timer: u16`**（PlayerState 新字段，替换 `shot_cd: u8`）：持 SHOT
   累进、松手清零（ZUN 同款，首发延迟一致）；shooter 在 `shot_timer % interval == delay % interval`
   时发射；wrapping 语义（u16 按住 ~18 分钟回绕，interval 是小因数即无缝）。
4. **shooter 基础十字段**（全定点/整数）：`interval: u16`（>0，表校验钉）/ `delay: u16` /
   `dx, dy: Fx` / `angle: Angle`（直上 = 49152）/ `speed: Fx` / `damage: u16` / `radius: Fx`
   （表校验 ≤ MAX_ENTITY_RADIUS）/ `sprite: u16` / `option: u8`（0=本体，1..=子机号，
   表校验 ≤ 该档子机数）/ `flags: u8`（预留；bit0 = homing 注记，**之后单刀**）。
   **伤害上限不做、不留字段**（改主意再加，常量表加字段是小事）。音效/ANM 归 M2。
5. **组织 = ZUN 现代作同款** `shootersets[tier 0..=4][focus 0/1]`——每角色 10 槽一次到位；
   **v0 内容简化**：高/低速共享同一列表（两槽指同一 `&[Shooter]`）、只做 2-3 档真差异
   （建议：0-1 档 1 路直射 → 2-3 档 2 路 → 4 档 3 路 + 子机），内容后补不动结构。
6. **角色参数迁表**：`PlayerState::spawn` 改为从表取 hit_radius/graze_radius；移速常量迁入
   表（`update_players` 移动逻辑改读表）；player.rs 编译期断言换成**表构造处的 const 校验/
   单测校验**（D1 债收半口——常量源头集中、越界在表层拦住）。

## WorldTables 形状

```rust
// crates/stg-core/src/tables.rs（新模块，断层线以下、纯数据）
pub struct WorldTables {
    pub content_hash: u64,               // 占位 0：文件加载刀实现（A3 与 EclImage 合并哈希）
    pub characters: [CharacterCfg; 1],   // v0 一个角色；数组长度即角色数（编译期事实）
    pub item_cfg: [ItemTypeCfg; ITEM_TYPE_COUNT],
    pub drop_tables: &'static [&'static [(u8, u8)]],
    pub item_gravity: Fx,
}

pub struct CharacterCfg {
    // 移动（原 player.rs 常量）
    pub high_speed: Fx, pub low_speed: Fx, pub inv_sqrt2: Fx,
    pub hit_radius: Fx, pub graze_radius: Fx,
    // shottype（本刀新增）
    pub shot: ShotTypeCfg,
}

pub struct ShotTypeCfg {
    /// [tier 0..=4][focus 0/1] —— 10 槽；v0 高低速两槽指同一列表
    pub sets: [[&'static [Shooter]; 2]; 5],
    /// 每档位子机偏移（option 号 1..=len 查此表；v0 前几档空表）
    pub option_pos: [&'static [(Fx, Fx)]; 5],
}

pub struct Shooter { /* 基础十字段，见拍板 4 */ }

pub static TABLES_V0: WorldTables = /* 全内容 const */;
```

- **传递形态**：`&'static WorldTables` 每帧作参数传入——`step(world, tables, input)` /
  `step_with_director(world, tables, input, director)`；相位函数按需下传（P2 组装层持有
  顺序不变）。**不进 World**（I7：无引用；静态表不进快照/校验和——两机同表由表哈希/二进制
  同一性保证，不由逐帧校验和保证）。
- **表校验**：`WorldTables::validate()`（debug/测试用）：interval>0、radius 双边入
  `[0, MAX_ENTITY_RADIUS]`、option 号 ≤ 该档子机数、drop_tables 类型合法（现有 items 测试
  迁移）。单测钉 `TABLES_V0.validate()` 全过。

## 相位 3 解释器（update_players）

持 SHOT 时 `shot_timer` 自增（wrapping），松手清零；对活跃档位
`sets[player.power_tier()][focus]`（focus = BTN_SLOW 按下）逐 shooter 升序：
`shot_timer % interval == delay % interval` → 出生点 = 自机位 +（option==0 ? (dx,dy)
: option_pos[tier][option-1] + (dx,dy)）→ `polar_to_vec(speed, angle)` →
`create_player_shot`。池满 P4-a 照旧。旧 `SHOT_*` 常量与 `shot_cd` 删除。
换档/换 focus **瞬时生效**（下一次取模判定即新列表；无过渡状态）。

## 迁移波及面（全家入驻的代价，实施计划逐条列）

- `step`/`step_with_director` 签名 +`&WorldTables` → harness/全部测试造场调用点更新
  （建议 test_support 给 `step_t(w, input)` 糖默认 `&TABLES_V0`，改动集中）。
- settle（credit_item/damage_enemy 掉落展开 读 ITEM_CFG/DROP_TABLES）、integrate
  （道具重力/终速/磁吸速度）、collide？（行 5 拾取半径——查 ITEM_CFG 的调用点全改道
  `tables.item_cfg`）；`spawn_drop`/`drop_item`/`spawn_star_at`/`attract_all_items` 等
  写 API 若读表 → 签名 +`&WorldTables`（D12 既定："`&mut WorldBody`（+ `&WorldTables`）"）。
- `PlayerState::spawn(character_id)` → `spawn(character_id, &WorldTables)`；`World::new`
  相应携带（或 spawn 移出 new、由调用方注入——实施时取扰动小者）。
- items.rs 只留池定义与类型常量/哨兵/入账常数（POWER_MAX 等账本规则**不迁**——它们是
  世界规则不是内容数据；`ItemTypeCfg` 结构体定义可留 items.rs 或迁 tables.rs，取引用向清晰者）。

## 金向量

- 表接线后弹型=1 路直射与旧硬编码**不逐位等价**（interval/damage 或有差）→ 金向量流变化
  属预期，双跑 + 三平台照旧。
- **拔档压差异**：导演在特定帧直写 `players[0].power`（诊断场景合法）：如 200 帧拔到
  250（2 路档）、400 帧拔到 400（满档+子机）——换档瞬时生效入流；BTN_SLOW 已在输入脚本里
  周期性出现，focus 维度顺带入流（v0 共享列表，压索引不压内容差异）。

## 判别式单测（最小集）

- 表校验全过；`power_tier` 边界（已有 M0-16 测试）接到解释器：**逐档弹数判别**
  （tier 0 一帧齐射 1 弹 / tier 2 两弹 / tier 4 三弹+子机位坐标逐位对）；
- shot_timer：按住第 N 帧出弹相位符合 interval/delay；**松手清零**（松一帧再按，首发
  延迟与初次一致的判别腿）；
- 子机出生点 = 自机位 + option_pos 偏移（逐位）；
- 迁表回归：道具入账/掉落/磁吸全部既有测试改造后仍绿（行为零变化——同值搬家）；
- spawn 从表取判定半径（值与旧常量逐位同）。

## 变异检验 ≥3 候选

解释器 tier 索引钉死（恒 0）→ 逐档弹数测试红；shot_timer 松手不清零 → 首发延迟测试红；
option_pos 偏移不加（子机弹从自机原点出）→ 子机出生点测试红。

## 收尾义务

`stg-world-design.md`：A3 表清单落地括注 + D6/A8 自机发弹改"shottype 表驱动"一句 +
D12 签名注（`&WorldTables` 已穿线）；`docs/follow-ups.md`：homing 单刀候选入库（含
转向率存放两案）、内容哈希占位待文件加载刀；`PROGRESS.md` 史行 + 现在段。

## 验收

判别单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；World 无新字段
（`shot_timer` 替换 `shot_cd` 除外，自动入校验和）；`cargo tree` 防火墙照旧。
